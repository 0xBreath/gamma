#![allow(unexpected_cfgs)]
#![allow(
    deprecated,
    reason = "Anchor internally calls AccountInfo::realloc (see PR #3803)"
)]
use anchor_lang::prelude::*;

use instructions::*;

pub mod instructions;
pub mod state;
pub mod types;

declare_id!("JDP9AsSqpzeea8yqscvMHU7gkvC7QR16UF35hf74tAFG");

#[program]
pub mod gamma {
    use super::*;

    pub fn init_market(
        ctx: Context<InitMarket>,
        num_outcomes: u8,
        scale: u64,
        label: String,
    ) -> Result<()> {
        instructions::init_market(ctx, num_outcomes, scale, label)
    }

    pub fn buy(ctx: Context<Buy>, outcome_index: u8, amount_in: u64) -> Result<()> {
        instructions::buy(ctx, outcome_index, amount_in)
    }
}
