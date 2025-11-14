use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke_signed;
use anchor_lang::solana_program::system_instruction;
use anchor_spl::token::{self, Burn, Mint, Token, TokenAccount};

use crate::state::Market;
use common::check_condition;
use common::constants::{common::*, seeds::*};
use common::errors::ErrorCode;
use common::utils::{Decimal, Rounding};

#[derive(Accounts)]
#[instruction(outcome_index: u8, burn_amount: u64, label: String)]
pub struct Sell<'info> {
    /// user who holds the outcome tokens and will receive SOL back
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(mut)]
    pub market: AccountLoader<'info, Market>,

    /// CHECK: PDA check
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

pub fn sell(ctx: Context<Sell>, outcome_index: u8, burn_amount: u64, label: String) -> Result<()> {
    // --- basic validation ---
    let mut market = ctx.accounts.market.load_mut()?;
    let idx = outcome_index as usize;
    let n = market.num_outcomes as usize;

    check_condition!(burn_amount > 0, BurnIsZero);
    check_condition!(n > 0, OutcomeBelowZero);
    check_condition!(idx < n, InvalidOutcomeIndex);

    // Ensure user actually has enough tokens in their ATA (safety)
    check_condition!(
        ctx.accounts.user_outcome_token_account.amount >= burn_amount,
        InsufficientFunds
    );

    // Ensure burn_amount <= current supply
    let supply_before = market.supplies[idx];
    check_condition!(burn_amount <= supply_before, BurnIsMoreThanSupply);

    // Safety cap: do not allow removing > MAX_WITHDRAW_BPS of the outcome reserve in one call
    // Compute max allowed delta in token units based on supplies or reserve fraction
    // We'll apply this cap on token amount using supply proportion:
    let max_burn_allowed = ((supply_before as u128)
        .checked_mul(MAX_WITHDRAW_BPS as u128)
        .ok_or(error!(ErrorCode::MathOverflow))?
        / 10_000u128) as u64;

    if burn_amount > max_burn_allowed {
        return Err(error!(ErrorCode::BurnIsMoreThanSupply));
    }

    // --- Burn tokens from user's ATA (user signs) ---
    let cpi_accounts = Burn {
        mint: ctx.accounts.outcome_mint.to_account_info(),
        from: ctx.accounts.user_outcome_token_account.to_account_info(),
        authority: ctx.accounts.user.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
    // Note: user signs, so no PDA signer required for burn
    token::burn(cpi_ctx, burn_amount)?;

    // --- Compute refund using quadratic bonding curve C(s) = 1/2 * s^2 ---
    //
    // Convert supplies to Decimal D18:
    // s0 = Decimal::from_plain(supply_before)
    // delta_s = Decimal::from_plain(burn_amount)
    //
    // C(s) = 0.5 * s^2  (we handle scaling with Decimal arithmetic)
    //
    // refund_dec = C(s0) - C(s0 - delta_s)  (both D18)
    // refund_lamports = refund_dec.to_token_amount(Floor)  (u64 lamports)
    //

    // s0 (D18)
    let s0_dec = Decimal::from_plain(supply_before)?;
    // s1 = s0 - delta (D18)
    let delta_dec = Decimal::from_plain(burn_amount)?;
    // ensure s0 >= delta
    let s1_dec = s0_dec.sub(&delta_dec)?;

    // C(s0) : compute s0^2 -> (s0 * s0) / ONE_E18 = D18
    let s0_sq = s0_dec.mul(&s0_dec)?.div(&Decimal::ONE_E18)?;
    // multiply by 1/2: (s0_sq * 1) / 2
    let half = Decimal::from_plain(1u64)?.div(&Decimal::from_plain(2u64)?)?; // equals 0.5 in D18
    let c_s0 = s0_sq.mul(&half)?.div(&Decimal::ONE_E18)?; // s0_sq is D18; multiply by half (D18) => D36 then /D18 -> D18

    // C(s1)
    let s1_sq = s1_dec.mul(&s1_dec)?.div(&Decimal::ONE_E18)?;
    let c_s1 = s1_sq.mul(&half)?.div(&Decimal::ONE_E18)?;

    // refund in D18
    let refund_dec = c_s0.sub(&c_s1)?;
    // Convert D18 -> lamports (u64), floor rounding
    let refund_tokens = refund_dec.to_token_amount(Rounding::Floor)?;
    let refund_u64 = refund_tokens.0;

    // If nothing to refund (due to rounding), return early
    if refund_u64 == 0 {
        // update supplies only and recompute invariant
        market.supplies[idx] = market.supplies[idx]
            .checked_sub(burn_amount)
            .ok_or(error!(ErrorCode::MathOverflow))?;
        market
            .recompute_invariant()
            .map_err(|_| error!(ErrorCode::MathOverflow))?;
        return Ok(());
    }

    // --- apply fee (fee stays in market vault) ---
    let fee = (refund_u64 as u128)
        .checked_mul(FEE_BPS as u128)
        .ok_or(error!(ErrorCode::MathOverflow))?
        / 10_000u128;
    let fee_u64 = fee as u64;
    let net_payout_u64 = refund_u64
        .checked_sub(fee_u64)
        .ok_or(error!(ErrorCode::MathOverflow))?;

    // Ensure vault has enough lamports
    let vault_lamports = ctx.accounts.market_vault.to_account_info().lamports();
    check_condition!(vault_lamports >= refund_u64, InsufficientVaultFunds);

    // --- Update market state: decrease reserve by full refund (refund includes fee that remains in vault)
    market.reserves[idx] = market.reserves[idx]
        .checked_sub(refund_u64)
        .ok_or(error!(ErrorCode::MathOverflow))?;

    // decrease supply by burned tokens
    market.supplies[idx] = market.supplies[idx]
        .checked_sub(burn_amount)
        .ok_or(error!(ErrorCode::MathOverflow))?;

    // Recompute invariant
    market
        .recompute_invariant()
        .map_err(|_| error!(ErrorCode::MathOverflow))?;

    // --- Transfer net payout from vault PDA to user (invoke_signed) ---
    let seeds: &[&[u8]] = &[MARKET_SEED, label.as_bytes(), &[market.bump]];
    let signer_seeds: &[&[&[u8]]] = &[seeds];

    let ix = system_instruction::transfer(
        &ctx.accounts.market_vault.key(),
        &ctx.accounts.user.key(),
        net_payout_u64,
    );

    // Note: because market_vault is a PDA, we must sign with PDA seeds (market PDA)
    invoke_signed(
        &ix,
        &[
            ctx.accounts.market_vault.to_account_info().clone(),
            ctx.accounts.user.to_account_info().clone(),
            ctx.accounts.system_program.to_account_info().clone(),
        ],
        signer_seeds,
    )
    .map_err(|_| error!(ErrorCode::VaultTransferFailed))?;

    // fee remains in vault; if you want to route fee to admin, implement additional transfer

    Ok(())
}
