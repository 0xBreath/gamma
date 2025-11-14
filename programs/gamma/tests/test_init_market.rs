#![cfg(feature = "test-sbf")]

use {
    anchor_lang::{solana_program::instruction::Instruction, InstructionData, ToAccountMetas},
    mollusk_svm::{result::Check, Mollusk},
};

// #[test]
// fn test_init_market() {
//     let program_id = gamma::id();

//     let mollusk = Mollusk::new(&program_id, "gamma");

//     let instruction = Instruction::new_with_bytes(
//         program_id,
//         &gamma::instruction::Initialize {}.data(),
//         gamma::accounts::Initialize {}.to_account_metas(None),
//     );

//     mollusk.process_and_validate_instruction(&instruction, &[], &[Check::success()]);
// }
