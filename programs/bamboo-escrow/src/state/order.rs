//! Order account definitions, layout, and state transition logic.

use anchor_lang::prelude::*;

#[repr(u8)]
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum EscrowStatus {
    #[default]
    None = 0,
    Locked = 1,
    Settled = 2,
    Refunded = 3,
}

#[account]
#[derive(Default, Debug)]
pub struct OrderAccount {
    /// Canonical 32-byte order identifier
    pub order_id: [u8; 32],
    /// Maker (creator) pubkey
    pub maker: Pubkey,
    /// Authorized relayer pubkey
    pub assigned_relayer: Pubkey,
    /// SPL Token Mint being escrowed
    pub token_mint: Pubkey,
    /// Associated PDA token vault
    pub token_vault: Pubkey,
    /// Total token amount locked
    pub amount: u64,
    /// 32-byte salted recipient hash for fiat payout
    pub fiat_recipient_hash: [u8; 32],
    /// Deposited anti-griefing micro-bond in lamports (0.01 SOL)
    pub anti_griefing_bond: u64,
    /// Unix timestamp when timelock expires
    pub expiry_timestamp: i64,
    /// Relayer fee in basis points (max 25)
    pub host_fee_bps: u16,
    /// Current lifecycle status
    pub status: EscrowStatus,
    /// Canonical PDA bump seed
    pub bump: u8,
}

impl OrderAccount {
    /// Account layout space: 8 discriminator + field sizes
    pub const LEN: usize = 32   // order_id
        + 32                    // maker
        + 32                    // assigned_relayer
        + 32                    // token_mint
        + 32                    // token_vault
        + 8                     // amount
        + 32                    // fiat_recipient_hash
        + 8                     // anti_griefing_bond
        + 8                     // expiry_timestamp
        + 2                     // host_fee_bps
        + 1                     // status (u8 representation)
        + 1;                    // bump

    pub fn is_locked(&self) -> bool {
        self.status == EscrowStatus::Locked
    }

    pub fn is_expired(&self, current_time: i64) -> bool {
        current_time > self.expiry_timestamp
    }

    /// Computes the host fee and net payout amount safely
    pub fn calculate_split(&self) -> Option<(u64, u64)> {
        let fee = (self.amount as u128)
            .checked_mul(self.host_fee_bps as u128)?
            .checked_div(crate::constants::BPS_DENOMINATOR)? as u64;
        let payout = self.amount.checked_sub(fee)?;
        Some((payout, fee))
    }
}
