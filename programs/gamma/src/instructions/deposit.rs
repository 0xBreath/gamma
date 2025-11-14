use crate::state::Market;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount};
use common::check_condition;
use common::constants::{MARKET_SEED, OUTCOME_MINT_SEED, VAULT_SEED};
use common::errors::ErrorCode;
use common::utils::{Decimal, Rounding};

#[derive(Accounts)]
#[instruction(outcome_index: u8, amount_in: u64, label: String)]
pub struct Deposit<'info> {
    /// Payer providing SOL
    #[account(mut)]
    pub user: Signer<'info>,

    /// Market zero-copy account PDA. Seeds: [MARKET_SEED, label.as_ref()]
    #[account(mut)]
    pub market: AccountLoader<'info, Market>,

    /// CHECK: PDA check
    #[account(
        mut,
        seeds = [VAULT_SEED, market.key().as_ref()],
        bump,
    )]
    pub market_vault: UncheckedAccount<'info>,

    /// Outcome SPL Mint to mint tokens to user. Authority must be market PDA.
    #[account(
        mut,
        seeds = [OUTCOME_MINT_SEED, market.key().as_ref(), &[outcome_index]],
        bump,
    )]
    pub outcome_mint: Account<'info, Mint>,

    /// User's ATA for the outcome mint
    #[account(mut)]
    pub user_outcome_ata: Account<'info, TokenAccount>,

    /// Programs
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn deposit(
    ctx: Context<Deposit>,
    outcome_index: u8,
    amount_in: u64,
    label: String,
) -> Result<()> {
    // Basic validation
    let mut market = ctx.accounts.market.load_mut()?;
    let idx = outcome_index as usize;
    let n = market.num_outcomes as usize;
    check_condition!(n > 0, OutcomeBelowZero);
    check_condition!(idx < n, InvalidOutcomeIndex);

    // Transfer SOL from user -> market vault
    // NOTE: this uses native lamports. If you plan to use SPL collateral (USDC), replace with token CPI.
    let ix = anchor_lang::solana_program::system_instruction::transfer(
        &ctx.accounts.user.key(),
        &ctx.accounts.market_vault.key(),
        amount_in,
    );
    anchor_lang::solana_program::program::invoke(
        &ix,
        &[
            ctx.accounts.user.to_account_info(),
            ctx.accounts.market_vault.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
    )
    .map_err(|_| error!(ErrorCode::TransferFailed))?;

    // Update reserve (safe checked add)
    market.reserves[idx] = market.reserves[idx]
        .checked_add(amount_in)
        .ok_or(error!(ErrorCode::MathOverflow))?;

    // --- Compute minted tokens using quadratic cost C(s) = 1/2 * s^2 ---
    // supply s is stored as plain token units (u64)
    // We'll work in D18 decimals:
    // s0 (D18) = Decimal::from_plain(s0_u64)
    // A (token amount) -> D9 via from_token_amount -> convert to D18 by multiplying by ONE_E9 (D9)
    // Compute s_new = sqrt( s0^2 + 2 * A_in_D18 )
    // minted = floor( s_new - s0 ) converted to token units

    // current supply
    let s0_u64 = market.supplies[idx];
    let s0_dec = Decimal::from_plain(s0_u64)?;

    // payment as Decimal D9 (since token amounts often D9) then convert to D18:
    let a_d9 = Decimal::from_token_amount(amount_in)?;
    // convert D9 -> D18 by multiplying by ONE_E9 (D9) producing D18 (D9 * D9 = D18)
    // Decimal::ONE_E9 exists on your type
    let a_d18 = a_d9.mul(&Decimal::ONE_E9)?; // now in D18

    // s0^2 (keep at D18): (s0_dec * s0_dec) / ONE_E18  => result D18
    let s0_sq = s0_dec.mul(&s0_dec)?.div(&Decimal::ONE_E18)?;

    // compute 2 * A_in_D18 (D18 * D18 = D36 ; divide by ONE_E18 -> D18)
    let two_dec = Decimal::from_plain(2)?;
    let two_a_d18 = a_d18.mul(&two_dec)?.div(&Decimal::ONE_E18)?;

    // rhs = s0^2 + 2 * A
    let rhs = s0_sq.add(&two_a_d18)?;

    // s_new = sqrt(rhs)  (nth_root with n=2), returns D18
    let s_new = rhs.nth_root(2)?;

    // delta = s_new - s0_dec  (D18)
    let delta = s_new.sub(&s0_dec)?;

    // minted amount -> convert D18 -> token units (D9) using to_token_amount
    let token_result = delta.to_token_amount(Rounding::Floor)?;
    let minted_u64 = token_result.0;

    // Update supply (checked)
    market.supplies[idx] = market.supplies[idx]
        .checked_add(minted_u64)
        .ok_or(error!(ErrorCode::MathOverflow))?;

    // Recompute invariant (efficient/incremental update could be used, but recompute for correctness)
    market
        .recompute_invariant()
        .map_err(|_| error!(ErrorCode::MathOverflow))?;

    // --- Mint outcome tokens to user via CPI, signed by market PDA ---
    //
    // We assume the outcome_mint authority is the market PDA created with seeds: [MARKET_SEED, label.as_ref()]
    // and that `market.bump` matches the PDA bump for that seed. Adjust seeds if you used a different mint authority.
    //
    let seeds: &[&[u8]] = &[MARKET_SEED, label.as_bytes(), &[market.bump]];
    let signer_seeds: &[&[&[u8]]] = &[seeds];

    let cpi_accounts = MintTo {
        mint: ctx.accounts.outcome_mint.to_account_info(),
        to: ctx.accounts.user_outcome_ata.to_account_info(),
        authority: ctx.accounts.market.to_account_info(), // market PDA as mint authority
    };

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        cpi_accounts,
        signer_seeds,
    );

    // minted_u64 may be zero in edge cases — handle it gracefully (still OK to call mint_to with 0).
    token::mint_to(cpi_ctx, minted_u64).map_err(|_| error!(ErrorCode::TokenMintFailed))?;

    Ok(())
}
