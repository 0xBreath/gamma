use anchor_lang::prelude::*;

/// Seed to derive [`Market`] PDA
#[constant]
pub const MARKET_SEED: &[u8] = b"market";

#[constant]
pub const VAULT_SEED: &[u8] = b"vault";

#[constant]
pub const OUTCOME_MINT_SEED: &[u8] = b"mint";
