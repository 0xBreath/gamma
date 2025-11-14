use anchor_lang::prelude::*;
use common::check_condition;
use common::constants::MAX_OUTCOMES;
use common::errors::ErrorCode;
use spl_math::uint::U256;

use crate::types::FixedSizeString;

#[account(zero_copy)]
#[derive(InitSpace, Default)]
#[repr(C)]
pub struct Market {
    /// invariant = geometric mean * scale^N
    /// This never changes except at initialization.
    /// This is a u256 but raw so it can impl Pod
    pub invariant: [u8; 32],

    /// Reserves for each outcome, fixed-point scaled.
    /// All values stored as u64 but promoted to u128 for math.
    pub reserves: [u64; MAX_OUTCOMES],

    pub supplies: [u64; MAX_OUTCOMES],

    /// Precision scalar (e.g., 1e6 or 1e12)
    /// Used so geometric mean calculations stay stable.
    pub scale: u64,

    pub initialized_at: u64,

    /// The admin of the market who can mutate it
    pub admin: Pubkey,

    /// Token users will deposit and withdraw from the market in exchange for shares (i.e. USDC)
    pub deposit_mint: Pubkey,

    pub label: FixedSizeString,

    /// Number of outcomes (N)
    pub num_outcomes: u8,

    pub bump: u8,

    /// Padding for zero copy alignment
    pub _padding: [u8; 14],
}

impl Market {
    pub const SIZE: usize = 8 + Market::INIT_SPACE;
}

impl Market {
    /// Convert stored invariant bytes -> U256 (big-endian)
    #[inline(always)]
    pub fn invariant_u256(&self) -> U256 {
        // U256::from_big_endian expects big-endian ordering
        U256::from_big_endian(&self.invariant)
    }

    /// Write a U256 into the stored invariant bytes
    #[inline(always)]
    pub fn set_invariant_u256(&mut self, v: U256) {
        let mut buf = [0u8; 32];
        v.write_as_big_endian(&mut buf);
        self.invariant = buf;
    }

    /// Recompute the invariant as the product of active reserves:
    /// invariant = ∏_{i=0..num_outcomes-1} reserves[i]
    /// Returns the new invariant (U256) or MathOverflow error.
    pub fn recompute_invariant(&mut self) -> Result<U256> {
        let n = self.num_outcomes as usize;
        check_condition!(n <= MAX_OUTCOMES, InvalidOutcomeIndex);

        let mut prod = U256::from(1u64);

        // multiply all active reserves into prod
        for i in 0..n {
            let r = U256::from(self.reserves[i]);
            prod = prod.checked_mul(r).ok_or(error!(ErrorCode::MathOverflow))?;
        }

        self.set_invariant_u256(prod);
        Ok(prod)
    }

    /// Compute product of reserves excluding index `idx`:
    /// returns ∏_{j != idx} reserves[j] as U256
    pub fn product_except(&self, idx: usize) -> Result<U256> {
        let n = self.num_outcomes as usize;
        check_condition!(n <= MAX_OUTCOMES, InvalidOutcomeIndex);
        check_condition!(idx < n, InvalidOutcomeIndex);

        let mut prod = U256::from(1u64);
        for i in 0..n {
            if i == idx {
                continue;
            }
            let r = U256::from(self.reserves[i]);
            prod = prod.checked_mul(r).ok_or(error!(ErrorCode::MathOverflow))?;
        }
        Ok(prod)
    }

    /// Compute required reserve (U256) for outcome idx to satisfy the invariant:
    ///
    ///     required_r_i = invariant / ∏_{j != i} r_j
    ///
    /// If product_except == 0, this returns 0 (degenerate case).
    pub fn required_reserve_for(&self, idx: usize) -> Result<U256> {
        // validate
        let n = self.num_outcomes as usize;
        check_condition!(n <= MAX_OUTCOMES, InvalidOutcomeIndex);
        check_condition!(idx < n, InvalidOutcomeIndex);

        let inv = self.invariant_u256();
        let denom = self.product_except(idx)?;

        if denom.is_zero() {
            // degenerate product: other reserves include a zero -> required is zero to avoid div by zero
            return Ok(U256::zero());
        }

        let req = inv
            .checked_div(denom)
            .ok_or(error!(ErrorCode::MathOverflow))?;
        Ok(req)
    }

    /// Compute how many raw units (u64) must be added to outcome idx to restore the invariant:
    ///
    /// returns 0 if already satisfied; clamps to u64::MAX if delta > u64::MAX
    pub fn required_delta(&self, idx: usize) -> Result<u64> {
        let n = self.num_outcomes as usize;
        check_condition!(n <= MAX_OUTCOMES, InvalidOutcomeIndex);
        check_condition!(idx < n, InvalidOutcomeIndex);

        let cur = U256::from(self.reserves[idx]);
        let req = self.required_reserve_for(idx)?;

        if req <= cur {
            return Ok(0u64);
        }
        let delta = req - cur;

        // clamp to u64::MAX, though a delta that large indicates something is off
        if delta > U256::from(u64::MAX) {
            Ok(u64::MAX)
        } else {
            Ok(delta.as_u64())
        }
    }
}
