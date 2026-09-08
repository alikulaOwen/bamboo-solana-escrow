//! Custom program error codes following Squads v4 error formatting.

use anchor_lang::prelude::*;

#[error_code]
pub enum BambooEscrowError {
    #[msg("Host fee ceiling exceeded: maximum protocol allowance is 25 bps (0.25%)")]
    HostFeeCeilingExceeded,

    #[msg("Order is not currently in the locked/active state")]
    OrderNotActive,

    #[msg("Signer is not the assigned relayer for this order")]
    UnauthorizedRelayer,

    #[msg("Timelock is still active; refund cannot be claimed until expiry")]
    TimelockStillActive,

    #[msg("Numerical overflow occurred during fee or payout calculation")]
    MathOverflow,

    #[msg("Invalid token mint provided for order escrow")]
    InvalidTokenMint,

    #[msg("Invalid order PDA bump")]
    InvalidBump,
}
