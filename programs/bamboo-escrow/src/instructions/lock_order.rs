//! Handler for locking an SPL token order with 0.01 SOL anti-griefing bond.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke, system_instruction};
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

use crate::constants::{ANTI_GRIEFING_BOND_LAMPORTS, MAX_HOST_FEE_BPS, SEED_ORDER, SEED_VAULT};
use crate::errors::BambooEscrowError;
use crate::events::OrderLockedEvent;
use crate::state::{EscrowStatus, OrderAccount};

#[derive(Accounts)]
#[instruction(order_id: [u8; 32])]
pub struct LockOrder<'info> {
    #[account(
        init,
        payer = maker,
        space = 8 + OrderAccount::LEN,
        seeds = [SEED_ORDER, order_id.as_ref()],
        bump
    )]
    pub order: Account<'info, OrderAccount>,

    #[account(mut)]
    pub maker: Signer<'info>,

    /// CHECK: Relayer authorized to settle this order
    pub assigned_relayer: AccountInfo<'info>,

    pub token_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = maker_token_account.mint == token_mint.key(),
        constraint = maker_token_account.owner == maker.key()
    )]
    pub maker_token_account: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = maker,
        token::mint = token_mint,
        token::authority = order,
        seeds = [SEED_VAULT, order_id.as_ref()],
        bump
    )]
    pub token_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn process_lock_order(
    ctx: Context<LockOrder>,
    order_id: [u8; 32],
    amount: u64,
    fiat_recipient_hash: [u8; 32],
    duration_seconds: i64,
    host_fee_bps: u16,
) -> Result<()> {
    require!(
        host_fee_bps <= MAX_HOST_FEE_BPS,
        BambooEscrowError::HostFeeCeilingExceeded
    );

    let clock = Clock::get()?;
    let order = &mut ctx.accounts.order;

    order.order_id = order_id;
    order.maker = ctx.accounts.maker.key();
    order.assigned_relayer = ctx.accounts.assigned_relayer.key();
    order.token_mint = ctx.accounts.token_mint.key();
    order.token_vault = ctx.accounts.token_vault.key();
    order.amount = amount;
    order.fiat_recipient_hash = fiat_recipient_hash;
    order.anti_griefing_bond = ANTI_GRIEFING_BOND_LAMPORTS;
    order.expiry_timestamp = clock.unix_timestamp + duration_seconds;
    order.host_fee_bps = host_fee_bps;
    order.status = EscrowStatus::Locked;
    order.bump = ctx.bumps.order;

    // 1. Transfer SPL token deposit into vault PDA
    let cpi_accounts = Transfer {
        from: ctx.accounts.maker_token_account.to_account_info(),
        to: ctx.accounts.token_vault.to_account_info(),
        authority: ctx.accounts.maker.to_account_info(),
    };
    let cpi_program = ctx.accounts.token_program.to_account_info();
    token::transfer(CpiContext::new(cpi_program, cpi_accounts), amount)?;

    // 2. Deposit 0.01 SOL anti-griefing micro-bond into order PDA
    let ix = system_instruction::transfer(
        &ctx.accounts.maker.key(),
        &order.key(),
        ANTI_GRIEFING_BOND_LAMPORTS,
    );
    invoke(
        &ix,
        &[
            ctx.accounts.maker.to_account_info(),
            order.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
    )?;

    emit!(OrderLockedEvent {
        order_id,
        maker: ctx.accounts.maker.key(),
        token_mint: ctx.accounts.token_mint.key(),
        amount,
        fiat_recipient_hash,
        expiry_timestamp: order.expiry_timestamp,
        anti_griefing_bond: ANTI_GRIEFING_BOND_LAMPORTS,
    });

    Ok(())
}
