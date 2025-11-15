use spl_math::uint::U256;

// Constants for scaling
pub const ONE_U256: U256 = U256([1, 0, 0, 0]); // 1
pub const D9_U256: U256 = U256([1_000_000_000, 0, 0, 0]); // 1e9 (D9)
pub const D18_U256: U256 = U256([1_000_000_000_000_000_000, 0, 0, 0]); // 1e18 (D18)
pub const D9_U128: u128 = 1_000_000_000; // 1e9 (D9)
pub const D18_U128: u128 = 1_000_000_000_000_000_000; // 1e18 (D18)

pub const MAX_OUTCOMES: usize = 16;
pub const OUTCOME_MINT_DECIMALS: u8 = 9;

/// MAX_TVL_FEE is the maximum fee that can be set for the TVL fee, D18{1/year} -> 10% annually in D18.
pub const MAX_TVL_FEE: u128 = 100_000_000_000_000_000;

/// DAY_IN_SECONDS is the number of se conds in a day.
pub const DAY_IN_SECONDS: u64 = 86400;

/// YEAR_IN_SECONDS is the number of seconds in a year.
pub const YEAR_IN_SECONDS: u64 = 365 * DAY_IN_SECONDS;

/// LN_2 is the natural logarithm of 2, 693147180559945309. Used in reward token calculations. In D18.
pub const LN_2: u128 = 693_147_180_559_945_309;

// Tunables (adjust or move to Market)
pub const FEE_BPS: u64 = 10; // 0.1%
pub const MAX_WITHDRAW_BPS: u64 = 50_00; // 50% of outcome reserve allowed per tx (in basis points; 10000 = 100%)

pub const MIN_MARKET_DURATION: i64 = 1;
