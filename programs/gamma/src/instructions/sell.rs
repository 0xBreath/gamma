use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Mint, Token, TokenAccount};

use crate::state::Market;
use common::check_condition;
use common::constants::{common::*, seeds::*};
use common::errors::ErrorCode;

#[derive(Accounts)]
#[instruction(outcome_index: u8, burn_amount: u64)]
pub struct Sell<'info> {
    /// user who holds the outcome tokens and will receive SOL back
    #[account(
        mut,
        constraint = user_outcome_token_account.owner == user.key()
    )]
    pub user: Signer<'info>,

    #[account(mut)]
    pub market: AccountLoader<'info, Market>,

    /// CHECK: PDA check and mint account check within token program CPI
    #[account(
        mut,
        seeds = [VAULT_SEED, market.key().as_ref()],
        bump,
    )]
    pub market_vault: UncheckedAccount<'info>,

    /// Outcome SPL token to mint to user. Authority must be the market PDA.
    #[account(
        mut,
        mint::decimals = OUTCOME_MINT_DECIMALS,
        mint::authority = market,
        seeds = [OUTCOME_MINT_SEED, market.key().as_ref(), &[outcome_index]],
        bump,
    )]
    pub outcome_mint: Account<'info, Mint>,

    #[account(
        mut,
        associated_token::mint = outcome_mint,
        associated_token::authority = user,
        associated_token::token_program = outcome_mint.to_account_info().owner,
    )]
    pub user_outcome_token_account: Account<'info, TokenAccount>,

    /// Token program for burn CPI
    pub token_program: Program<'info, Token>,

    /// System program for lamport transfer
    pub system_program: Program<'info, System>,
}

pub fn sell(ctx: Context<Sell>, outcome_index: u8, burn_amount: u64) -> Result<()> {
    let mut market = ctx.accounts.market.load_mut()?;
    let idx = outcome_index as usize;
    let n = market.num_outcomes as usize;

    let now = Clock::get()?.unix_timestamp;
    check_condition!(now < market.resolve_at, MarketExpired);
    check_condition!(burn_amount > 0, BurnIsZero);
    check_condition!(n > 0, OutcomeBelowZero);
    check_condition!(idx < n, InvalidOutcomeIndex);
    check_condition!(
        ctx.accounts.user_outcome_token_account.amount >= burn_amount,
        InsufficientFunds
    );

    // Ensure vault has enough lamports
    let vault_lamports = ctx.accounts.market_vault.to_account_info().lamports();

    // Ensure burn_amount <= current supply
    let supply_before = market.supplies[idx];
    check_condition!(burn_amount <= supply_before, BurnIsMoreThanSupply);

    // Safety cap: do not allow removing > MAX_WITHDRAW_BPS of the outcome reserve in one call
    // Compute max allowed delta in token units based on supplies or reserve fraction
    // We'll apply this cap on token amount using supply proportion:
    // let max_burn_allowed = ((supply_before as u128)
    //     .checked_mul(MAX_WITHDRAW_BPS as u128)
    //     .ok_or(error!(ErrorCode::MathOverflow))?
    //     / 10_000u128) as u64;

    // if burn_amount > max_burn_allowed {
    //     return Err(error!(ErrorCode::BurnIsMoreThanSupply));
    // }

    // burn user's outcome tokens
    token::burn(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Burn {
                mint: ctx.accounts.outcome_mint.to_account_info(),
                from: ctx.accounts.user_outcome_token_account.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        ),
        burn_amount,
    )?;

    // compute payout then update market reserves, supplies, and invariant
    let net_payout_u64 = market.sell_outcome(idx, burn_amount, vault_lamports)?;

    // market_vault PDA signs for lamport transfer from self
    ctx.accounts.market_vault.sub_lamports(net_payout_u64)?;
    ctx.accounts.user.add_lamports(net_payout_u64)?;

    // fee remains in vault; if you want to route fee to admin, implement additional transfer

    Ok(())
}
