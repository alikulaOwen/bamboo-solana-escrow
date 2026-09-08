//! Handler for permissionless refund of expired orders and bond return.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::{SEED_ORDER, SEED_VAULT};
use crate::errors::BambooEscrowError;
use crate::events::OrderRefundedEvent;
use crate::state::{EscrowStatus, OrderAccount};

#[derive(Accounts)]
#[instruction(order_id: [u8; 32])]
pub struct ClaimRefund<'info> {
    #[account(
        mut,
        seeds = [SEED_ORDER, order_id.as_ref()],
        bump = order.bump
    )]
    pub order: Account<'info, OrderAccount>,

    #[account(mut, constraint = maker.key() == order.maker)]
    pub maker: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_VAULT, order_id.as_ref()],
        bump
    )]
    pub token_vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub maker_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

pub fn process_claim_refund(ctx: Context<ClaimRefund>, order_id: [u8; 32]) -> Result<()> {
    let clock = Clock::get()?;
    let order = &mut ctx.accounts.order;

    require!(order.is_locked(), BambooEscrowError::OrderNotActive);
    require!(
        order.is_expired(clock.unix_timestamp),
        BambooEscrowError::TimelockStillActive
    );

    order.status = EscrowStatus::Refunded;

    let order_seeds: &[&[u8]] = &[SEED_ORDER, order_id.as_ref(), &[order.bump]];
    let signer_seeds = &[&order_seeds[..]];

    // 1. Refund full SPL token amount to maker
    let cpi_accounts = Transfer {
        from: ctx.accounts.token_vault.to_account_info(),
        to: ctx.accounts.maker_token_account.to_account_info(),
        authority: order.to_account_info(),
    };
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        ),
        order.amount,
    )?;

    // 2. Refund 0.01 SOL anti-griefing micro-bond to maker
    let bond = order.anti_griefing_bond;
    **order.to_account_info().try_borrow_mut_lamports()? -= bond;
    **ctx.accounts.maker.try_borrow_mut_lamports()? += bond;

    emit!(OrderRefundedEvent {
        order_id,
        maker: ctx.accounts.maker.key(),
        amount: order.amount,
        bond_refunded: bond,
    });

    Ok(())
}
