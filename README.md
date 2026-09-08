# Project Bamboo: Spoke 2 – Solana SVM Anchor Escrow

This repository contains the Solana SVM escrow program (`bamboo_escrow`) for **Project Bamboo**, engineered strictly following the **Squads Protocol v4** enterprise architecture standard.

## Features
- **Squads Protocol v4 Architecture**: Decoupled instruction handlers (`lock_order`, `settle_payout`, `claim_refund`), encapsulated account states (`OrderAccount`), centralized constants (`SEED_ORDER`, `SEED_VAULT`), and structured error codes.
- **PDA Order Isolation**: Each order lives in its own dedicated PDA derived from `[b"order", order_id]`.
- **Anti-Griefing Micro-Bond**: Enforces a 0.01 SOL (`10_000_000` lamports) bond deposited into the order PDA and refunded upon settlement or timelock expiry.
- **Atomic Multi-Instruction Jupiter CPI**: Composable with Jupiter DEX swap instructions in a single atomic transaction.

## Program Structure
```text
programs/bamboo-escrow/src/
├── lib.rs              # Anchor entry point & #[program] routing
├── constants.rs        # PDA seeds, fee ceilings, and bond constants
├── errors.rs           # Squads v4 custom error codes
├── events.rs           # Emitted event definitions
├── state/
│   ├── mod.rs          # State module re-exports
│   └── order.rs        # OrderAccount layout, space calculation & split logic
└── instructions/
    ├── mod.rs          # Instruction router & client accounts re-exports
    ├── lock_order.rs   # LockOrder accounts & process_lock_order
    ├── settle_payout.rs# SettlePayout accounts & process_settle_payout
    └── claim_refund.rs # ClaimRefund accounts & process_claim_refund
```

## Build and Test
```bash
# Check compilation
cargo check

# Build Anchor program
anchor build
```
