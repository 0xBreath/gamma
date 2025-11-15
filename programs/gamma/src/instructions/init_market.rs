use anchor_lang::prelude::*;
use anchor_spl::token::Token;
use solana_program::program_pack::Pack;
use spl_math::uint::U256;
use spl_token::solana_program;

use crate::state::Market;
use crate::types::{FixedSizeString, MAX_PADDED_STRING_LENGTH};
use anchor_lang::solana_program::rent::ACCOUNT_STORAGE_OVERHEAD;
use anchor_lang::system_program;
use common::constants::{
    MARKET_SEED, MAX_OUTCOMES, OUTCOME_MINT_DECIMALS, OUTCOME_MINT_SEED, VAULT_SEED,
};
use common::{check_condition, errors::ErrorCode};

#[derive(Accounts)]
#[instruction(num_outcomes: u8, scale: u64, label: FixedSizeString)]
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
        seeds = [MARKET_SEED, label.as_bytes()],
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

pub fn init_market<'info>(
    // ctx: Context<InitMarket>,
    ctx: Context<'_, '_, 'info, 'info, InitMarket<'info>>,
    num_outcomes: u8,
    scale: u64,
    label: FixedSizeString,
) -> Result<()> {
    let mut market = ctx.accounts.market.load_init()?;

    check_condition!(num_outcomes as usize <= MAX_OUTCOMES, TooManyOutcomes);

    check_condition!(
        label.value.len() <= MAX_PADDED_STRING_LENGTH,
        InvalidLabelLength
    );

    let bump = ctx.bumps.market;
    let market_key = ctx.accounts.market.key();

    // Market PDA seeds
    let market_signer_seeds: &[&[&[u8]]] = &[&[MARKET_SEED, label.as_bytes(), &[bump]]];

    market.admin = *ctx.accounts.admin.key;
    market.num_outcomes = num_outcomes;
    market.scale = scale;
    market.bump = ctx.bumps.market;
    market.vault_bump = ctx.bumps.market_vault;
    market.label = label;

    let remaining = ctx.remaining_accounts;

    check_condition!(remaining.len() == num_outcomes as usize, InvalidMintCount);

    for (i, acct) in remaining.iter().enumerate() {
        // Unchecked -> Mint
        let mint_info = acct.clone();
        let rent_info = ctx.accounts.rent.to_account_info().clone();

        // get PDA + bump exactly how off-chain code does
        let (expected_key, mint_bump) = Pubkey::find_program_address(
            &[OUTCOME_MINT_SEED, market_key.as_ref(), &[i as u8]],
            ctx.program_id,
        );

        check_condition!(mint_info.key() == expected_key, InvalidMintSeed);

        let mint_signer_seeds: &[&[&[u8]]] = &[&[
            OUTCOME_MINT_SEED,
            market_key.as_ref(),
            &[i as u8],
            &[mint_bump],
        ]];

        let mint_space = spl_token::state::Mint::LEN;
        let rent_lamports = Rent::get()?.minimum_balance(mint_space);

        system_program::create_account(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info().clone(),
                system_program::CreateAccount {
                    from: ctx.accounts.admin.to_account_info(),
                    to: mint_info.clone(),
                },
                mint_signer_seeds,
            ),
            rent_lamports,
            mint_space as u64,
            &ctx.accounts.token_program.key(),
        )?;

        anchor_spl::token_interface::initialize_mint(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info().clone(),
                anchor_spl::token_interface::InitializeMint {
                    mint: mint_info.clone(),
                    rent: rent_info.clone(),
                },
                market_signer_seeds,
            ),
            OUTCOME_MINT_DECIMALS,
            &market_key,
            None,
        )?;
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
