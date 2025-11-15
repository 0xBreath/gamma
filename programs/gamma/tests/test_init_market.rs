// Example LiteSVM test: https://github.com/brimigs/anchor-escrow-with-litesvm/blob/main/tests/litesvm-tests.rs

use litesvm::LiteSVM;
use {
    anchor_lang::{
        prelude::AccountMeta, solana_program::instruction::Instruction, system_program,
        InstructionData, ToAccountMetas,
    },
    common::constants::{MARKET_SEED, OUTCOME_MINT_SEED, VAULT_SEED},
    solana_sdk::{
        program_pack::Pack,
        pubkey::Pubkey,
        signer::keypair::{Keypair, Signer},
        transaction::Transaction,
    },
};

#[test]
fn test_init_market() {
    let program_id = gamma::id();
    let mut svm = LiteSVM::new();
    let bytes = include_bytes!("../../../target/deploy/gamma.so");
    svm.add_program(program_id, bytes);

    let admin = Keypair::new();
    let label = "test_market".to_string();
    let market = Pubkey::find_program_address(&[&MARKET_SEED, label.as_bytes()], &program_id).0;
    let market_vault = Pubkey::find_program_address(&[&VAULT_SEED, market.as_ref()], &program_id).0;
    let outcome_mint_a =
        Pubkey::find_program_address(&[&OUTCOME_MINT_SEED, market.as_ref(), &[0]], &program_id).0;
    let outcome_mint_b =
        Pubkey::find_program_address(&[&OUTCOME_MINT_SEED, market.as_ref(), &[1]], &program_id).0;

    svm.airdrop(&admin.pubkey(), 100_000_000_000).unwrap();
    let balance = svm.get_balance(&admin.pubkey()).unwrap();
    assert_eq!(balance, 100_000_000_000);

    let mut accounts_ctx = gamma::accounts::InitMarket {
        system_program: system_program::ID,
        rent: anchor_lang::solana_program::sysvar::rent::ID,
        token_program: anchor_spl::token::ID,
        admin: admin.pubkey(),
        market,
        market_vault,
    }
    .to_account_metas(None);
    accounts_ctx.push(AccountMeta {
        pubkey: outcome_mint_a,
        is_signer: false,
        is_writable: true,
    });
    accounts_ctx.push(AccountMeta {
        pubkey: outcome_mint_b,
        is_signer: false,
        is_writable: true,
    });
    let ix = Instruction::new_with_bytes(
        program_id,
        &gamma::instruction::InitMarket {
            num_outcomes: 2,
            scale: 100_000,
            label,
        }
        .data(),
        accounts_ctx,
    );

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&admin.pubkey()),
        &[&admin],
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx).unwrap();

    let market_account = svm.get_account(&market).unwrap();
    assert_eq!(market_account.data.len(), gamma::state::Market::SIZE);

    let outcome_mint_a_account = svm.get_account(&outcome_mint_a).unwrap();
    assert_eq!(
        outcome_mint_a_account.data.len(),
        spl_token::state::Mint::LEN
    );

    let outcome_mint_b_account = svm.get_account(&outcome_mint_b).unwrap();
    assert_eq!(
        outcome_mint_b_account.data.len(),
        spl_token::state::Mint::LEN
    );
}
