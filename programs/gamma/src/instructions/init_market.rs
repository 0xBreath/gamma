use anchor_lang::prelude::*;
use anchor_spl::token::Token;
use spl_math::uint::U256;

use crate::state::Market;
use crate::types::{FixedSizeString, MAX_PADDED_STRING_LENGTH};
use anchor_lang::solana_program::rent::ACCOUNT_STORAGE_OVERHEAD;
use common::constants::{
    MARKET_SEED, MAX_OUTCOMES, OUTCOME_MINT_DECIMALS, OUTCOME_MINT_SEED, VAULT_SEED,
};
use common::{check_condition, errors::ErrorCode};

#[derive(Accounts)]
#[instruction(num_outcomes: u8, scale: u64, label: String)]
pub struct InitMarket<'info> {
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
    pub token_program: Program<'info, Token>,

    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        init,
        payer = admin,
        space = Market::SIZE,
        seeds = [MARKET_SEED, label.as_ref()],
        bump
    )]
    pub market: AccountLoader<'info, Market>,

    /// CHECK: Default account with no data that stores lamports for the [`Market`]
    #[account(
        init,
        payer = admin,
        space = ACCOUNT_STORAGE_OVERHEAD as usize,
        seeds = [VAULT_SEED, market.key().as_ref()],
        bump,
    )]
    pub market_vault: UncheckedAccount<'info>,
}

pub fn init_market(
    ctx: Context<InitMarket>,
    num_outcomes: u8,
    scale: u64,
    label: String,
) -> Result<()> {
    let mut market = ctx.accounts.market.load_init()?;

    check_condition!(num_outcomes as usize <= MAX_OUTCOMES, TooManyOutcomes);

    check_condition!(label.len() <= MAX_PADDED_STRING_LENGTH, InvalidLabelLength);

    market.admin = *ctx.accounts.admin.key;
    market.num_outcomes = num_outcomes;
    market.scale = scale;
    market.bump = ctx.bumps.market;
    market.vault_bump = ctx.bumps.market_vault;
    market.label = FixedSizeString::new(&label);

    let bump = market.bump;
    let market_key = ctx.accounts.market.key();

    // Market PDA seeds
    let market_seeds: &[&[u8]] = &[MARKET_SEED, label.as_bytes(), &[bump]];
    let signer_seeds: &[&[&[u8]]] = &[market_seeds];

    let remaining = ctx.remaining_accounts;

    check_condition!(remaining.len() == num_outcomes as usize, InvalidMintCount);

    for (i, acct) in remaining.iter().enumerate() {
        // Unchecked -> Mint
        let mint_info = acct.clone();
        // let token_program_info = ctx.accounts.token_program.to_account_info().clone();

        let expected_seed = Pubkey::create_program_address(
            &[OUTCOME_MINT_SEED, market_key.as_ref(), &[i as u8], &[bump]],
            ctx.program_id,
        )
        .map_err(|_| ErrorCode::InvalidMintSeed)?;

        check_condition!(mint_info.key() == expected_seed, InvalidMintSeed);

        let ix = spl_token::instruction::initialize_mint2(
            &spl_token::id(),
            &mint_info.key(),
            &market_key,
            None,
            OUTCOME_MINT_DECIMALS,
        )?;

        anchor_lang::solana_program::program::invoke_signed(
            &ix,
            &[
                mint_info.clone(),
                // rent_info.to_account_info()
            ],
            signer_seeds,
        )?;

        // anchor_spl::token_interface::initialize_mint2(
        //     CpiContext::new_with_signer(
        //         token_program_info.clone(),
        //         anchor_spl::token_interface::InitializeMint2 {
        //             mint: mint_info.clone(),
        //         },
        //         signer_seeds,
        //     ),
        //     OUTCOME_MINT_DECIMALS,
        //     &mint_info.key(),
        //     None
        // )?;
    }

    // Compute initial invariant
    // product(reserves[0..num_outcomes]) = 0 as all reserves = 0
    // But we compute it properly so later it is easy to modify the logic.
    let n = num_outcomes as usize;
    let mut prod = U256::from(1u64);
    for i in 0..n {
        let r = U256::from(market.reserves[i]);
        prod = prod.checked_mul(r).ok_or(error!(ErrorCode::MathOverflow))?;
    }

    market.set_invariant_u256(prod);

    Ok(())
}
