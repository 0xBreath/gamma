// LiteSVM docs: https://www.anchor-lang.com/docs/testing/litesvm
// Example LiteSVM test: https://github.com/brimigs/anchor-escrow-with-litesvm/blob/main/tests/litesvm-tests.rs

use anchor_lang::AccountDeserialize;
use anchor_spl::associated_token::{get_associated_token_address, spl_associated_token_account};
use common::constants::D9_U128;
use gamma::types::FixedSizeString;
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
fn test_market() {
    let program_id = gamma::id();
    let mut svm = LiteSVM::new();
    let bytes = include_bytes!("../../../target/deploy/gamma.so");
    svm.add_program(program_id, bytes);

    let admin = Keypair::new();
    let user = Keypair::new();
    let label = FixedSizeString::new("test_market");
    let market = Pubkey::find_program_address(&[&MARKET_SEED, label.as_bytes()], &program_id).0;
    let market_vault = Pubkey::find_program_address(&[&VAULT_SEED, market.as_ref()], &program_id).0;
    let outcome_mint_a =
        Pubkey::find_program_address(&[&OUTCOME_MINT_SEED, market.as_ref(), &[0]], &program_id).0;
    let outcome_mint_b =
        Pubkey::find_program_address(&[&OUTCOME_MINT_SEED, market.as_ref(), &[1]], &program_id).0;

    let airdrop_lamports_amount = 100_000_000_000;
    svm.airdrop(&admin.pubkey(), airdrop_lamports_amount)
        .unwrap();
    let balance = svm.get_balance(&admin.pubkey()).unwrap();
    assert_eq!(balance, airdrop_lamports_amount);

    svm.airdrop(&user.pubkey(), airdrop_lamports_amount)
        .unwrap();
    let balance = svm.get_balance(&user.pubkey()).unwrap();
    assert_eq!(balance, airdrop_lamports_amount);

    let deposit_amount = 100_000_000;
    let resolve_at = std::time::Instant::now().elapsed().as_secs() as i64 + 10;

    // init_market
    {
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
                resolve_at,
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

    // buy outcome A
    {
        let user_outcome_a_token_pda =
            get_associated_token_address(&user.pubkey(), &outcome_mint_a);
        let accounts_ctx = gamma::accounts::Buy {
            user: user.pubkey(),
            market,
            market_vault,
            outcome_mint: outcome_mint_a,
            user_outcome_token_account: user_outcome_a_token_pda,
            token_program: anchor_spl::token::ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None);
        let create_ata_ix =
            spl_associated_token_account::instruction::create_associated_token_account(
                &user.pubkey(),
                &user.pubkey(),
                &outcome_mint_a,
                &spl_token::ID,
            );
        let buy_ix = Instruction::new_with_bytes(
            program_id,
            &gamma::instruction::Buy {
                outcome_index: 0,
                amount_in: deposit_amount,
            }
            .data(),
            accounts_ctx,
        );

        let tx = Transaction::new_signed_with_payer(
            &[create_ata_ix, buy_ix],
            Some(&user.pubkey()),
            &[&user],
            svm.latest_blockhash(),
        );
        svm.send_transaction(tx).unwrap();

        let user_outcome_a_token_account = svm.get_account(&user_outcome_a_token_pda).unwrap();
        let user_outcome_a_tokens = anchor_spl::token::TokenAccount::try_deserialize(
            &mut user_outcome_a_token_account.data.as_ref(),
        )
        .unwrap()
        .amount;
        assert!(user_outcome_a_tokens > 0);
        println!(
            "user_outcome_a_tokens after buying A: {}",
            user_outcome_a_tokens
        );

        let user_lamports = svm.get_balance(&user.pubkey()).unwrap();
        assert!(user_lamports < airdrop_lamports_amount);
        let spent_lamports = airdrop_lamports_amount - user_lamports;
        assert_eq!(spent_lamports, deposit_amount + 2044280);

        let market_account = svm.get_account(&market).unwrap();
        let market =
            gamma::state::Market::try_deserialize(&mut market_account.data.as_ref()).unwrap();
        let outcome_a_price = market.outcome_price(0).unwrap();
        println!(
            "outcome_a_price after buying A: {}",
            outcome_a_price as f64 / D9_U128 as f64
        );
        let outcome_b_price = market.outcome_price(1).unwrap();
        assert_eq!(outcome_b_price, 0);
    }

    // buy outcome B
    let outcome_b_price_after_buying_b = {
        let user_outcome_b_token_pda =
            get_associated_token_address(&user.pubkey(), &outcome_mint_b);
        let accounts_ctx = gamma::accounts::Buy {
            user: user.pubkey(),
            market,
            market_vault,
            outcome_mint: outcome_mint_b,
            user_outcome_token_account: user_outcome_b_token_pda,
            token_program: anchor_spl::token::ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None);
        let create_ata_ix =
            spl_associated_token_account::instruction::create_associated_token_account(
                &user.pubkey(),
                &user.pubkey(),
                &outcome_mint_b,
                &spl_token::ID,
            );
        let buy_ix = Instruction::new_with_bytes(
            program_id,
            &gamma::instruction::Buy {
                outcome_index: 1,
                amount_in: deposit_amount,
            }
            .data(),
            accounts_ctx,
        );

        let tx = Transaction::new_signed_with_payer(
            &[create_ata_ix, buy_ix],
            Some(&user.pubkey()),
            &[&user],
            svm.latest_blockhash(),
        );
        svm.send_transaction(tx).unwrap();

        let user_outcome_b_token_account = svm.get_account(&user_outcome_b_token_pda).unwrap();
        let user_outcome_b_tokens = anchor_spl::token::TokenAccount::try_deserialize(
            &mut user_outcome_b_token_account.data.as_ref(),
        )
        .unwrap()
        .amount;

        assert!(user_outcome_b_tokens > 0);
        println!(
            "user_outcome_b_tokens after buying B: {}",
            user_outcome_b_tokens
        );

        let user_lamports = svm.get_balance(&user.pubkey()).unwrap();
        assert!(user_lamports < airdrop_lamports_amount);

        let market_account = svm.get_account(&market).unwrap();
        let market =
            gamma::state::Market::try_deserialize(&mut market_account.data.as_ref()).unwrap();
        let outcome_a_price = market.outcome_price(0).unwrap();
        println!(
            "outcome_a_price after buying B: {}",
            outcome_a_price as f64 / D9_U128 as f64
        );
        let outcome_b_price = market.outcome_price(1).unwrap();
        println!(
            "outcome_b_price after buying B: {}",
            outcome_b_price as f64 / D9_U128 as f64
        );
        outcome_b_price
    };

    // sell outcome A
    {
        let user_outcome_a_token_pda =
            get_associated_token_address(&user.pubkey(), &outcome_mint_a);

        // sell 100% of outcome A tokens
        let user_outcome_token_account_raw = svm.get_account(&user_outcome_a_token_pda).unwrap();
        let user_outcome_a_balance = anchor_spl::token::TokenAccount::try_deserialize(
            &mut user_outcome_token_account_raw.data.as_ref(),
        )
        .unwrap()
        .amount;

        let accounts_ctx = gamma::accounts::Sell {
            user: user.pubkey(),
            market,
            market_vault,
            outcome_mint: outcome_mint_a,
            user_outcome_token_account: user_outcome_a_token_pda,
            token_program: anchor_spl::token::ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None);
        let sell_ix = Instruction::new_with_bytes(
            program_id,
            &gamma::instruction::Sell {
                outcome_index: 0,
                burn_amount: user_outcome_a_balance,
            }
            .data(),
            accounts_ctx,
        );

        let user_lamports_before = svm.get_balance(&user.pubkey()).unwrap();
        let market_vault_lamports_before = svm.get_balance(&market_vault).unwrap();

        let tx = Transaction::new_signed_with_payer(
            &[sell_ix],
            Some(&user.pubkey()),
            &[&user],
            svm.latest_blockhash(),
        );
        svm.send_transaction(tx).unwrap();

        let market_account = svm.get_account(&market).unwrap();
        let market =
            gamma::state::Market::try_deserialize(&mut market_account.data.as_ref()).unwrap();
        let outcome_a_price = market.outcome_price(0).unwrap();
        println!(
            "outcome_a_price after selling A: {}",
            outcome_a_price as f64 / D9_U128 as f64
        );
        // supply of outcome A is zero so price should be zero
        assert_eq!(outcome_a_price, 0);
        let outcome_b_price = market.outcome_price(1).unwrap();
        // outcome B price should not have changed
        assert_eq!(outcome_b_price_after_buying_b, outcome_b_price);

        let user_outcome_token_account_raw = svm.get_account(&user_outcome_a_token_pda).unwrap();
        let user_outcome_a_balance_after = anchor_spl::token::TokenAccount::try_deserialize(
            &mut user_outcome_token_account_raw.data.as_ref(),
        )
        .unwrap()
        .amount;
        assert_eq!(user_outcome_a_balance_after, 0);

        let user_lamports_after = svm.get_balance(&user.pubkey()).unwrap();
        assert!(user_lamports_after > user_lamports_before);
        let market_vault_lamports_after = svm.get_balance(&market_vault).unwrap();
        assert!(market_vault_lamports_after < market_vault_lamports_before);
        // user gains what market_vault lost minus 5000 lamports for tx fee
        assert_eq!(
            market_vault_lamports_before - market_vault_lamports_after - 5000,
            user_lamports_after - user_lamports_before
        );
    }
}
