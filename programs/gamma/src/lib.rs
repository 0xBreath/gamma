#![allow(unexpected_cfgs)]
#![allow(
    deprecated,
    reason = "Anchor internally calls AccountInfo::realloc (see PR #3803)"
)]
use anchor_lang::prelude::*;

use instructions::*;
use types::*;

pub mod instructions;
pub mod state;
pub mod types;

declare_id!("JDP9AsSqpzeea8yqscvMHU7gkvC7QR16UF35hf74tAFG");

#[program]
pub mod gamma {
    use super::*;

    /// Create a new market with N outcomes
    pub fn init_market<'info>(
        ctx: Context<'_, '_, 'info, 'info, InitMarket<'info>>,
        num_outcomes: u8,
        scale: u64,
        resolve_at: i64,
        label: FixedSizeString,
    ) -> Result<()> {
        instructions::init_market(ctx, num_outcomes, scale, resolve_at, label)
    }

    /// Buy into a single outcome with SOL and receive liquid-stake tokens for that position
    pub fn buy(ctx: Context<Buy>, outcome_index: u8, amount_in: u64) -> Result<()> {
        instructions::buy(ctx, outcome_index, amount_in)
    }

    /// Sell out of a single outcome by burning the liquid-stake token for that position and receiving SOL in return
    pub fn sell(ctx: Context<Sell>, outcome_index: u8, burn_amount: u64) -> Result<()> {
        instructions::sell(ctx, outcome_index, burn_amount)
    }
}
