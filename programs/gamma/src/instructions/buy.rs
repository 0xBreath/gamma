use crate::state::Market;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount};
use common::check_condition;
use common::constants::{MARKET_SEED, OUTCOME_MINT_DECIMALS, OUTCOME_MINT_SEED, VAULT_SEED};
use common::errors::ErrorCode;

#[derive(Accounts)]
#[instruction(outcome_index: u8, amount_in: u64)]
pub struct Buy<'info> {
    /// Payer providing SOL
    #[account(mut)]
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

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn buy(ctx: Context<Buy>, outcome_index: u8, amount_in: u64) -> Result<()> {
    // Basic validation
    let market_key = ctx.accounts.market.key();
    let mut market = ctx.accounts.market.load_mut()?;
    let idx = outcome_index as usize;
    let num_outcomes = market.num_outcomes as usize;

    let now = Clock::get()?.unix_timestamp;
    check_condition!(now < market.resolve_at, MarketExpired);

    check_condition!(amount_in > 0, DepositIsZero);
    check_condition!(num_outcomes > 0, OutcomeBelowZero);
    check_condition!(idx < num_outcomes, InvalidOutcomeIndex);

    let (expected_mint_key, _) = Pubkey::find_program_address(
        &[OUTCOME_MINT_SEED, market_key.as_ref(), &[idx as u8]],
        ctx.program_id,
    );
    check_condition!(
        ctx.accounts.outcome_mint.key() == expected_mint_key,
        InvalidMintSeed
    );

    // Transfer SOL from user -> market vault
    anchor_lang::system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.user.to_account_info(),
                to: ctx.accounts.market_vault.to_account_info(),
            },
        ),
        amount_in,
    )
    .map_err(|_| error!(ErrorCode::TransferFailed))?;

    let amount_out = market.buy_outcome(idx, amount_in)?;

    // --- Mint outcome tokens to user via CPI, signed by market PDA ---
    //
    // We assume the outcome_mint authority is the market PDA created with seeds: [MARKET_SEED, label.as_bytes()]
    // and that `market.bump` matches the PDA bump for that seed. Adjust seeds if you used a different mint authority.
    //
    let label = market.label.clone();
    let signer_seeds: &[&[&[u8]]] = &[&[MARKET_SEED, label.as_bytes(), &[market.bump]]];

    drop(market);

    let cpi_accounts = MintTo {
        mint: ctx.accounts.outcome_mint.to_account_info(),
        to: ctx.accounts.user_outcome_token_account.to_account_info(),
        authority: ctx.accounts.market.to_account_info(), // market PDA as mint authority
    };

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        cpi_accounts,
        signer_seeds,
    );

    msg!("amount_out: {}", amount_out);

    // minted_u64 may be zero in edge cases — handle it gracefully (still OK to call mint_to with 0).
    // token::mint_to(cpi_ctx, amount_out).map_err(|_| error!(ErrorCode::TokenMintFailed))?;
    token::mint_to(cpi_ctx, amount_out)?;

    Ok(())
}
