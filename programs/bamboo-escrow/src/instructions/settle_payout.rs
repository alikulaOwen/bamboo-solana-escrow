//! Handler for releasing payout to recipient and refunding anti-griefing bond.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::{SEED_ORDER, SEED_VAULT};
use crate::errors::BambooEscrowError;
use crate::events::OrderSettledEvent;
use crate::state::{EscrowStatus, OrderAccount};

#[derive(Accounts)]
#[instruction(order_id: [u8; 32])]
pub struct SettlePayout<'info> {
    #[account(
        mut,
        seeds = [SEED_ORDER, order_id.as_ref()],
        bump = order.bump
    )]
    pub order: Account<'info, OrderAccount>,

    pub relayer: Signer<'info>,

    /// CHECK: Maker account receiving the 0.01 SOL anti-griefing bond refund
    #[account(mut, constraint = maker.key() == order.maker)]
    pub maker: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [SEED_VAULT, order_id.as_ref()],
        bump
    )]
    pub token_vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub recipient_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub relayer_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

pub fn process_settle_payout(ctx: Context<SettlePayout>, order_id: [u8; 32]) -> Result<()> {
    let order = &mut ctx.accounts.order;

    require!(order.is_locked(), BambooEscrowError::OrderNotActive);
    require!(
        ctx.accounts.relayer.key() == order.assigned_relayer,
        BambooEscrowError::UnauthorizedRelayer
    );

    order.status = EscrowStatus::Settled;

    let (payout_amount, host_fee) = order
        .calculate_split()
        .ok_or(BambooEscrowError::MathOverflow)?;

    let order_seeds: &[&[u8]] = &[SEED_ORDER, order_id.as_ref(), &[order.bump]];
    let signer_seeds = &[&order_seeds[..]];

    // 1. Transfer net payout to recipient
    let payout_cpi = Transfer {
        from: ctx.accounts.token_vault.to_account_info(),
        to: ctx.accounts.recipient_token_account.to_account_info(),
        authority: order.to_account_info(),
    };
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            payout_cpi,
            signer_seeds,
        ),
        payout_amount,
    )?;

    // 2. Transfer host fee to relayer if nonzero
    if host_fee > 0 {
        let fee_cpi = Transfer {
            from: ctx.accounts.token_vault.to_account_info(),
            to: ctx.accounts.relayer_token_account.to_account_info(),
            authority: order.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                fee_cpi,
                signer_seeds,
            ),
            host_fee,
        )?;
    }

    // 3. Return 0.01 SOL anti-griefing bond to maker
    let bond = order.anti_griefing_bond;
    **order.to_account_info().try_borrow_mut_lamports()? -= bond;
    **ctx.accounts.maker.try_borrow_mut_lamports()? += bond;

    emit!(OrderSettledEvent {
        order_id,
        relayer: ctx.accounts.relayer.key(),
        recipient: ctx.accounts.recipient_token_account.key(),
        payout_amount,
        host_fee,
        bond_refunded: bond,
    });

    Ok(())
}
