//! Error codes for the program.
//!
//! Custom error for Anchor programs start at 6000. i.e. here Unauthorized error would be 6000 and
//! InvalidProgramCount would be 6001.

use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Too many outcomes")]
    TooManyOutcomes,

    #[msg("Outcome is below zero")]
    OutcomeBelowZero,

    #[msg("Account Not Signer")]
    AccountNotSigner,

    #[msg("Account Not Writable")]
    AccountNotWritable,

    #[msg("Account Not Executable")]
    AccountNotExecutable,

    #[msg("Missing Remaining Account")]
    MissingRemainingAccount,

    #[msg("Invalid Token Program")]
    InvalidTokenProgram,

    #[msg("Math Overflow")]
    MathOverflow,

    #[msg("Invalid Account Owner")]
    InvalidAccountOwner,

    #[msg("Invalid outcome index")]
    InvalidOutcomeIndex,

    #[msg("Transfer failed")]
    TransferFailed,

    #[msg("Token mint failed")]
    TokenMintFailed,

    #[msg("Invalid mint count")]
    InvalidMintCount,

    #[msg("Invalid mint seed")]
    InvalidMintSeed,
}

/// Check a condition and return an error if it is not met.
///
/// # Arguments
/// * `condition` - The condition to check.
/// * `error` - The error to return if the condition is not met.
#[macro_export]
macro_rules! check_condition {
    ($condition:expr, $error:expr) => {
        if !$condition {
            return Err(error!(ErrorCode::$error));
        }
    };
}
