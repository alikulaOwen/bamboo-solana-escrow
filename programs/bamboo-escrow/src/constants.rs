//! Global constants and PDA seed definitions following Squads v4 architecture.

/// PDA seed prefix for Escrow Order accounts
pub const SEED_ORDER: &[u8] = b"order";

/// PDA seed prefix for Token Vault accounts
pub const SEED_VAULT: &[u8] = b"vault";

/// Maximum allowable host fee ceiling in basis points: 25 bps (0.25%)
pub const MAX_HOST_FEE_BPS: u16 = 25;

/// Basis points denominator (100.00% = 10,000 bps)
pub const BPS_DENOMINATOR: u128 = 10_000;

/// Required anti-griefing micro-bond: 0.01 SOL (10,000,000 lamports)
pub const ANTI_GRIEFING_BOND_LAMPORTS: u64 = 10_000_000;
