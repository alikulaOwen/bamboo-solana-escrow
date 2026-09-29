use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Mint, Token, TokenAccount, Transfer};

declare_id!("BamEscrow1111111111111111111111111111111111");

pub const SEED_ORDER: &[u8] = b"order";
pub const SEED_VAULT: &[u8] = b"vault";

pub const MAX_HOST_FEE_BPS: u16 = 25; // 0.25% protocol ceiling
pub const ANTI_GRIEFING_BOND_LAMPORTS: u64 = 10_000_000; // 0.01 SOL refundable bond
pub const MIN_DURATION_SECONDS: i64 = 900; // 15 minutes minimum
pub const MAX_DURATION_SECONDS: i64 = 604_800; // 7 days maximum
pub const SETTLEMENT_WINDOW_SECONDS: i64 = 1_800; // 30-minute challenge window
pub const DISPUTE_TIMEOUT_SECONDS: i64 = 1_209_600; // 14-day failsafe dispute timeout

#[program]
pub mod bamboo_escrow {
    use super::*;

    /// Locks crypto collateral into an isolated PDA vault.
    /// Uses maker-seeded PDAs [b"order", maker, nonce] to completely eliminate order squatting.
    pub fn lock_order(
        ctx: Context<LockOrder>,
        nonce: u64,
        amount: u64,
        fiat_recipient_hash: [u8; 32],
        duration_seconds: i64,
        host_fee_bps: u16,
    ) -> Result<()> {
        require!(
            duration_seconds >= MIN_DURATION_SECONDS && duration_seconds <= MAX_DURATION_SECONDS,
            BambooError::InvalidDuration
        );
        require!(host_fee_bps <= MAX_HOST_FEE_BPS, BambooError::HostFeeExceedsCeiling);
        require!(amount > 0, BambooError::InvalidAmount);

        let clock = Clock::get()?;
        let order = &mut ctx.accounts.order_state;

        // Compute canonical 32-byte order ID matching bamboo-core
        let mut order_id = [0u8; 32];
        let mut hasher = anchor_lang::solana_program::hash::Hasher::default();
        hasher.hash(b"BAMBOO_ORDER_V1");
        hasher.hash(ctx.accounts.maker.key().as_ref());
        hasher.hash(ctx.accounts.token_mint.key().as_ref());
        hasher.hash(&amount.to_le_bytes());
        hasher.hash(&fiat_recipient_hash);
        hasher.hash(ctx.accounts.assigned_relayer.key().as_ref());
        hasher.hash(&nonce.to_le_bytes());
        order_id.copy_from_slice(hasher.result().as_ref());

        order.order_id = order_id;
        order.maker = ctx.accounts.maker.key();
        order.assigned_relayer = ctx.accounts.assigned_relayer.key();
        order.mediator = ctx.accounts.mediator.key();
        order.token_mint = ctx.accounts.token_mint.key();
        order.amount = amount;
        order.fiat_recipient_hash = fiat_recipient_hash;
        order.nonce = nonce;
        order.created_at = clock.unix_timestamp;
        order.expiry = clock.unix_timestamp + duration_seconds;
        order.host_fee_bps = host_fee_bps;
        order.status = EscrowStatus::Locked;
        order.settlement_requested_at = 0;
        order.disputed_at = 0;
        order.dispute_bond_amount = 0;
        order.mpesa_receipt = [0u8; 32];
        order.bump = ctx.bumps.order_state;
        order.vault_bump = ctx.bumps.token_vault;

        // 1. Transfer 0.01 SOL anti-griefing micro-bond from maker to order PDA
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.maker.to_account_info(),
                    to: order.to_account_info(),
                },
            ),
            ANTI_GRIEFING_BOND_LAMPORTS,
        )?;

        // 2. Transfer token collateral from Maker to Token Vault PDA
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.maker_token_account.to_account_info(),
                    to: ctx.accounts.token_vault.to_account_info(),
                    authority: ctx.accounts.maker.to_account_info(),
                },
            ),
            amount,
        )?;

        emit!(OrderLockedEvent {
            order_id,
            maker: order.maker,
            assigned_relayer: order.assigned_relayer,
            amount,
            fiat_recipient_hash,
            expiry: order.expiry,
            nonce,
        });

        Ok(())
    }

    /// Relayer posts proof of fiat disbursement (e.g. M-Pesa receipt) and initiates challenge window W.
    pub fn request_settlement(
        ctx: Context<RequestSettlement>,
        mpesa_receipt: [u8; 32],
    ) -> Result<()> {
        let order = &mut ctx.accounts.order_state;
        let clock = Clock::get()?;

        require!(order.status == EscrowStatus::Locked, BambooError::InvalidStatus);
        require!(
            clock.unix_timestamp <= order.expiry - SETTLEMENT_WINDOW_SECONDS,
            BambooError::SettlementWindowCutoffPassed
        );

        order.status = EscrowStatus::SettlementRequested;
        order.settlement_requested_at = clock.unix_timestamp;
        order.mpesa_receipt = mpesa_receipt;

        emit!(SettlementRequestedEvent {
            order_id: order.order_id,
            mpesa_receipt,
            settlement_requested_at: order.settlement_requested_at,
        });

        Ok(())
    }

    /// Finalizes the order after challenge window W elapses without dispute.
    /// Transfers USDC to relayer, refunds 0.01 SOL bond to maker, and reclaims account rent.
    pub fn finalize(ctx: Context<Finalize>) -> Result<()> {
        let order = &ctx.accounts.order_state;
        let clock = Clock::get()?;

        require!(order.status == EscrowStatus::SettlementRequested, BambooError::InvalidStatus);
        require!(
            clock.unix_timestamp >= order.settlement_requested_at + SETTLEMENT_WINDOW_SECONDS,
            BambooError::ChallengeWindowActive
        );

        let total_amount = order.amount;
        let host_fee = (total_amount as u128 * order.host_fee_bps as u128 / 10_000) as u64;
        let payout = total_amount.checked_sub(host_fee).ok_or(BambooError::MathOverflow)?;

        let maker_key = order.maker;
        let nonce_bytes = order.nonce.to_le_bytes();
        let seeds: &[&[u8]] = &[
            SEED_ORDER,
            maker_key.as_ref(),
            &nonce_bytes,
            &[order.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        // 1. Transfer principal to Relayer token account
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.token_vault.to_account_info(),
                    to: ctx.accounts.relayer_token_account.to_account_info(),
                    authority: ctx.accounts.order_state.to_account_info(),
                },
                signer_seeds,
            ),
            payout,
        )?;

        // 2. Transfer host fee to Fee Treasury
        if host_fee > 0 {
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.token_vault.to_account_info(),
                        to: ctx.accounts.fee_treasury_token_account.to_account_info(),
                        authority: ctx.accounts.order_state.to_account_info(),
                    },
                    signer_seeds,
                ),
                host_fee,
            )?;
        }

        // 3. Close token vault PDA
        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: ctx.accounts.token_vault.to_account_info(),
                destination: ctx.accounts.maker.to_account_info(),
                authority: ctx.accounts.order_state.to_account_info(),
            },
            signer_seeds,
        ))?;

        // 4. Refund 0.01 SOL anti-griefing micro-bond to maker
        **ctx.accounts.order_state.to_account_info().try_borrow_mut_lamports()? -= ANTI_GRIEFING_BOND_LAMPORTS;
        **ctx.accounts.maker.try_borrow_mut_lamports()? += ANTI_GRIEFING_BOND_LAMPORTS;

        emit!(OrderFinalizedEvent {
            order_id: order.order_id,
            payout,
            host_fee,
        });

        Ok(())
    }

    /// Maker flags non-receipt during window W.
    /// Requires a maker dispute bond to prevent free-riding extortion (Flaw A).
    pub fn raise_dispute(ctx: Context<RaiseDispute>, dispute_bond_amount: u64) -> Result<()> {
        let order = &mut ctx.accounts.order_state;
        let clock = Clock::get()?;

        require!(order.status == EscrowStatus::SettlementRequested, BambooError::InvalidStatus);
        require!(
            clock.unix_timestamp < order.settlement_requested_at + SETTLEMENT_WINDOW_SECONDS,
            BambooError::ChallengeWindowExpired
        );

        // Require dispute bond (at least 5% of order amount)
        let min_bond = order.amount / 20;
        require!(dispute_bond_amount >= min_bond, BambooError::InsufficientDisputeBond);

        // Deposit dispute bond into vault
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.maker_token_account.to_account_info(),
                    to: ctx.accounts.token_vault.to_account_info(),
                    authority: ctx.accounts.maker.to_account_info(),
                },
            ),
            dispute_bond_amount,
        )?;

        order.status = EscrowStatus::Disputed;
        order.disputed_at = clock.unix_timestamp;
        order.dispute_bond_amount = dispute_bond_amount;

        emit!(OrderDisputedEvent {
            order_id: order.order_id,
            disputed_at: order.disputed_at,
            dispute_bond_amount,
        });

        Ok(())
    }

    /// 2-of-3 Mediator Resolution: Resolves a disputed order in favor of Relayer or Maker.
    /// Requires signatures from 2 of [Maker, Relayer, Mediator].
    pub fn resolve_dispute_multisig(
        ctx: Context<ResolveDisputeMultisig>,
        award_to_relayer: bool,
    ) -> Result<()> {
        let order = &ctx.accounts.order_state;
        require!(order.status == EscrowStatus::Disputed, BambooError::InvalidStatus);

        let maker_key = order.maker;
        let nonce_bytes = order.nonce.to_le_bytes();
        let seeds: &[&[u8]] = &[
            SEED_ORDER,
            maker_key.as_ref(),
            &nonce_bytes,
            &[order.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let total_vault_tokens = order.amount + order.dispute_bond_amount;

        if award_to_relayer {
            // Relayer proved payout: Relayer gets principal + slashed dispute bond
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.token_vault.to_account_info(),
                        to: ctx.accounts.relayer_token_account.to_account_info(),
                        authority: ctx.accounts.order_state.to_account_info(),
                    },
                    signer_seeds,
                ),
                total_vault_tokens,
            )?;
        } else {
            // Maker was defrauded: Maker gets principal + dispute bond returned
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.token_vault.to_account_info(),
                        to: ctx.accounts.maker_token_account.to_account_info(),
                        authority: ctx.accounts.order_state.to_account_info(),
                    },
                    signer_seeds,
                ),
                total_vault_tokens,
            )?;
        }

        // Close token vault
        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: ctx.accounts.token_vault.to_account_info(),
                destination: ctx.accounts.maker.to_account_info(),
                authority: ctx.accounts.order_state.to_account_info(),
            },
            signer_seeds,
        ))?;

        // Return anti-griefing bond to maker
        **ctx.accounts.order_state.to_account_info().try_borrow_mut_lamports()? -= ANTI_GRIEFING_BOND_LAMPORTS;
        **ctx.accounts.maker.try_borrow_mut_lamports()? += ANTI_GRIEFING_BOND_LAMPORTS;

        emit!(DisputeResolvedEvent {
            order_id: order.order_id,
            awarded_to_relayer: award_to_relayer,
        });

        Ok(())
    }

    /// Permissionless timelock refund if relayer never disbursed fiat or if dispute timed out (14 days).
    pub fn claim_refund(ctx: Context<ClaimRefund>) -> Result<()> {
        let order = &ctx.accounts.order_state;
        let clock = Clock::get()?;

        let can_refund = match order.status {
            EscrowStatus::Locked => clock.unix_timestamp >= order.expiry,
            EscrowStatus::Disputed => clock.unix_timestamp >= order.disputed_at + DISPUTE_TIMEOUT_SECONDS,
            _ => false,
        };
        require!(can_refund, BambooError::TimelockActive);

        let maker_key = order.maker;
        let nonce_bytes = order.nonce.to_le_bytes();
        let seeds: &[&[u8]] = &[
            SEED_ORDER,
            maker_key.as_ref(),
            &nonce_bytes,
            &[order.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let total_vault_tokens = ctx.accounts.token_vault.amount;

        // Refund all tokens in vault back to maker
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.token_vault.to_account_info(),
                    to: ctx.accounts.maker_token_account.to_account_info(),
                    authority: ctx.accounts.order_state.to_account_info(),
                },
                signer_seeds,
            ),
            total_vault_tokens,
        )?;

        // Close token vault
        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: ctx.accounts.token_vault.to_account_info(),
                destination: ctx.accounts.maker.to_account_info(),
                authority: ctx.accounts.order_state.to_account_info(),
            },
            signer_seeds,
        ))?;

        // Return anti-griefing micro-bond to maker
        **ctx.accounts.order_state.to_account_info().try_borrow_mut_lamports()? -= ANTI_GRIEFING_BOND_LAMPORTS;
        **ctx.accounts.maker.try_borrow_mut_lamports()? += ANTI_GRIEFING_BOND_LAMPORTS;

        emit!(OrderRefundedEvent {
            order_id: order.order_id,
        });

        Ok(())
    }
}

// ============================================================================
// INSTRUCTION CONTEXTS
// ============================================================================

#[derive(Accounts)]
#[instruction(nonce: u64)]
pub struct LockOrder<'info> {
    #[account(mut)]
    pub maker: Signer<'info>,

    /// CHECK: Relayer key validated by maker input
    pub assigned_relayer: AccountInfo<'info>,

    /// CHECK: Neutral protocol arbiter for 2-of-3 dispute multisig
    pub mediator: AccountInfo<'info>,

    pub token_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = maker,
        space = 8 + OrderAccount::LEN,
        seeds = [SEED_ORDER, maker.key().as_ref(), nonce.to_le_bytes().as_ref()],
        bump
    )]
    pub order_state: Account<'info, OrderAccount>,

    #[account(
        init,
        payer = maker,
        token::mint = token_mint,
        token::authority = order_state,
        seeds = [SEED_VAULT, order_state.key().as_ref()],
        bump
    )]
    pub token_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = maker_token_account.owner == maker.key(),
        constraint = maker_token_account.mint == token_mint.key()
    )]
    pub maker_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct RequestSettlement<'info> {
    pub relayer: Signer<'info>,
    #[account(
        mut,
        has_one = assigned_relayer @ BambooError::UnauthorizedRelayer
    )]
    pub order_state: Account<'info, OrderAccount>,
    pub assigned_relayer: Signer<'info>,
}

#[derive(Accounts)]
pub struct Finalize<'info> {
    pub relayer: Signer<'info>,

    /// CHECK: Account receives rent and micro-bond refund
    #[account(mut, address = order_state.maker)]
    pub maker: AccountInfo<'info>,

    #[account(
        mut,
        has_one = assigned_relayer @ BambooError::UnauthorizedRelayer,
        close = maker
    )]
    pub order_state: Account<'info, OrderAccount>,

    #[account(
        mut,
        seeds = [SEED_VAULT, order_state.key().as_ref()],
        bump = order_state.vault_bump
    )]
    pub token_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = relayer_token_account.mint == order_state.token_mint
    )]
    pub relayer_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = fee_treasury_token_account.mint == order_state.token_mint
    )]
    pub fee_treasury_token_account: Account<'info, TokenAccount>,

    pub assigned_relayer: AccountInfo<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct RaiseDispute<'info> {
    pub maker: Signer<'info>,

    #[account(
        mut,
        has_one = maker @ BambooError::UnauthorizedMaker
    )]
    pub order_state: Account<'info, OrderAccount>,

    #[account(
        mut,
        seeds = [SEED_VAULT, order_state.key().as_ref()],
        bump = order_state.vault_bump
    )]
    pub token_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = maker_token_account.owner == maker.key(),
        constraint = maker_token_account.mint == order_state.token_mint
    )]
    pub maker_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ResolveDisputeMultisig<'info> {
    /// Signer 1: Must be either Maker, Relayer, or Mediator
    pub authority_1: Signer<'info>,
    /// Signer 2: Must be a different party from [Maker, Relayer, Mediator]
    pub authority_2: Signer<'info>,

    /// CHECK: Receives refund if maker wins
    #[account(mut, address = order_state.maker)]
    pub maker: AccountInfo<'info>,

    #[account(
        mut,
        close = maker,
        constraint = (
            (authority_1.key() == order_state.maker || authority_1.key() == order_state.assigned_relayer || authority_1.key() == order_state.mediator) &&
            (authority_2.key() == order_state.maker || authority_2.key() == order_state.assigned_relayer || authority_2.key() == order_state.mediator) &&
            authority_1.key() != authority_2.key()
        ) @ BambooError::InvalidMultisigSigners
    )]
    pub order_state: Account<'info, OrderAccount>,

    #[account(
        mut,
        seeds = [SEED_VAULT, order_state.key().as_ref()],
        bump = order_state.vault_bump
    )]
    pub token_vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub relayer_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub maker_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ClaimRefund<'info> {
    /// Permissionless caller triggering timelock expiry
    pub caller: Signer<'info>,

    /// CHECK: Account receives rent and micro-bond refund
    #[account(mut, address = order_state.maker)]
    pub maker: AccountInfo<'info>,

    #[account(
        mut,
        close = maker
    )]
    pub order_state: Account<'info, OrderAccount>,

    #[account(
        mut,
        seeds = [SEED_VAULT, order_state.key().as_ref()],
        bump = order_state.vault_bump
    )]
    pub token_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = maker_token_account.owner == order_state.maker,
        constraint = maker_token_account.mint == order_state.token_mint
    )]
    pub maker_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

// ============================================================================
// STATE & ENUMS
// ============================================================================

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum EscrowStatus {
    Locked,
    SettlementRequested,
    Disputed,
    Finalized,
    Refunded,
}

#[account]
pub struct OrderAccount {
    pub order_id: [u8; 32],
    pub maker: Pubkey,
    pub assigned_relayer: Pubkey,
    pub mediator: Pubkey,
    pub token_mint: Pubkey,
    pub amount: u64,
    pub fiat_recipient_hash: [u8; 32],
    pub nonce: u64,
    pub created_at: i64,
    pub expiry: i64,
    pub settlement_requested_at: i64,
    pub disputed_at: i64,
    pub dispute_bond_amount: u64,
    pub mpesa_receipt: [u8; 32],
    pub host_fee_bps: u16,
    pub status: EscrowStatus,
    pub bump: u8,
    pub vault_bump: u8,
}

impl OrderAccount {
    pub const LEN: usize = 32   // order_id
        + 32                    // maker
        + 32                    // assigned_relayer
        + 32                    // mediator
        + 32                    // token_mint
        + 8                     // amount
        + 32                    // fiat_recipient_hash
        + 8                     // nonce
        + 8                     // created_at
        + 8                     // expiry
        + 8                     // settlement_requested_at
        + 8                     // disputed_at
        + 8                     // dispute_bond_amount
        + 32                    // mpesa_receipt
        + 2                     // host_fee_bps
        + 1                     // status
        + 1                     // bump
        + 1;                    // vault_bump
}

// ============================================================================
// EVENTS & ERRORS
// ============================================================================

#[event]
pub struct OrderLockedEvent {
    pub order_id: [u8; 32],
    pub maker: Pubkey,
    pub assigned_relayer: Pubkey,
    pub amount: u64,
    pub fiat_recipient_hash: [u8; 32],
    pub expiry: i64,
    pub nonce: u64,
}

#[event]
pub struct SettlementRequestedEvent {
    pub order_id: [u8; 32],
    pub mpesa_receipt: [u8; 32],
    pub settlement_requested_at: i64,
}

#[event]
pub struct OrderFinalizedEvent {
    pub order_id: [u8; 32],
    pub payout: u64,
    pub host_fee: u64,
}

#[event]
pub struct OrderDisputedEvent {
    pub order_id: [u8; 32],
    pub disputed_at: i64,
    pub dispute_bond_amount: u64,
}

#[event]
pub struct DisputeResolvedEvent {
    pub order_id: [u8; 32],
    pub awarded_to_relayer: bool,
}

#[event]
pub struct OrderRefundedEvent {
    pub order_id: [u8; 32],
}

#[error_code]
pub enum BambooError {
    #[msg("Host fee exceeds protocol ceiling (25 bps)")]
    HostFeeExceedsCeiling,
    #[msg("Order lock duration must be between 15m and 7d")]
    InvalidDuration,
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Current order status does not permit this instruction")]
    InvalidStatus,
    #[msg("Relayer cannot request settlement within window W of expiry")]
    SettlementWindowCutoffPassed,
    #[msg("Signer does not match committed relayer")]
    UnauthorizedRelayer,
    #[msg("Signer does not match order maker")]
    UnauthorizedMaker,
    #[msg("Challenge window W is still active")]
    ChallengeWindowActive,
    #[msg("Challenge window W has expired; order must be finalized")]
    ChallengeWindowExpired,
    #[msg("Dispute bond must be at least 5% of order amount")]
    InsufficientDisputeBond,
    #[msg("Two distinct authorized signers required (Maker, Relayer, or Mediator)")]
    InvalidMultisigSigners,
    #[msg("Timelock has not yet expired")]
    TimelockActive,
    #[msg("Arithmetic overflow")]
    MathOverflow,
}
