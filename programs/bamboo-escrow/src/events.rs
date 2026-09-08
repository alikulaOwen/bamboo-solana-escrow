//! Emitted protocol events following Squads v4 indexing standards.

use anchor_lang::prelude::*;

#[event]
pub struct OrderLockedEvent {
    pub order_id: [u8; 32],
    pub maker: Pubkey,
    pub token_mint: Pubkey,
    pub amount: u64,
    pub fiat_recipient_hash: [u8; 32],
    pub expiry_timestamp: i64,
    pub anti_griefing_bond: u64,
}

#[event]
pub struct OrderSettledEvent {
    pub order_id: [u8; 32],
    pub relayer: Pubkey,
    pub recipient: Pubkey,
    pub payout_amount: u64,
    pub host_fee: u64,
    pub bond_refunded: u64,
}

#[event]
pub struct OrderRefundedEvent {
    pub order_id: [u8; 32],
    pub maker: Pubkey,
    pub amount: u64,
    pub bond_refunded: u64,
}
