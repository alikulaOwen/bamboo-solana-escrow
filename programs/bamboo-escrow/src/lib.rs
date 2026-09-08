pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;
pub use instructions::*;

declare_id!("BamEscrow1111111111111111111111111111111111");

#[program]
pub mod bamboo_escrow {
    use super::*;

    /// Locks SPL tokens and deposits 0.01 SOL anti-griefing micro-bond into PDA escrow
    pub fn lock_order(
        ctx: Context<LockOrder>,
        order_id: [u8; 32],
        amount: u64,
        fiat_recipient_hash: [u8; 32],
        duration_seconds: i64,
        host_fee_bps: u16,
    ) -> Result<()> {
        instructions::lock_order::process_lock_order(
            ctx,
            order_id,
            amount,
            fiat_recipient_hash,
            duration_seconds,
            host_fee_bps,
        )
    }

    /// Committed relayer triggers release upon fiat confirmation, routes fee and refunds bond
    pub fn settle_payout(ctx: Context<SettlePayout>, order_id: [u8; 32]) -> Result<()> {
        instructions::settle_payout::process_settle_payout(ctx, order_id)
    }

    /// Permissionless refund after expiration timelock has passed
    pub fn claim_refund(ctx: Context<ClaimRefund>, order_id: [u8; 32]) -> Result<()> {
        instructions::claim_refund::process_claim_refund(ctx, order_id)
    }
}
