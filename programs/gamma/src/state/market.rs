use anchor_lang::prelude::*;
use common::check_condition;
use common::constants::common::*;
use common::constants::MAX_OUTCOMES;
use common::errors::ErrorCode;
use common::utils::{Decimal, Rounding};
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

    pub undistributed_fees: u64,

    /// The admin of the market who can mutate it
    pub admin: Pubkey,

    /// Token users will deposit and withdraw from the market in exchange for shares (i.e. USDC)
    pub deposit_mint: Pubkey,

    pub label: FixedSizeString,

    /// Number of outcomes (N)
    pub num_outcomes: u8,

    pub bump: u8,

    pub vault_bump: u8,

    /// Padding for zero copy alignment
    pub _padding: [u8; 13],
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
    /// required_r_i = invariant / ∏_{j != i} r_j
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

    pub fn buy_outcome(&mut self, outcome_index: usize, amount_in: u64) -> Result<u64> {
        // Update reserve
        self.reserves[outcome_index] = self.reserves[outcome_index]
            .checked_add(amount_in)
            .ok_or(error!(ErrorCode::MathOverflow))?;

        // --- Compute minted tokens using quadratic cost C(s) = 1/2 * s^2 ---
        // supply s is stored as plain token units (u64)
        // We'll work in D18 decimals:
        // s0 (D18) = Decimal::from_plain(s0_u64)
        // A (token amount) -> D9 via from_token_amount -> convert to D18 by multiplying by ONE_E9 (D9)
        // Compute s_new = sqrt( s0^2 + 2 * A_in_D18 )
        // minted = floor( s_new - s0 ) converted to token units

        // current supply
        let s0_u64 = self.supplies[outcome_index];
        let s0_dec = Decimal::from_plain(s0_u64)?;

        // payment as Decimal D9 (since token amounts often D9) then convert to D18:
        let a_d9 = Decimal::from_token_amount(amount_in)?;
        // convert D9 -> D18 by multiplying by ONE_E9 (D9) producing D18 (D9 * D9 = D18)
        // Decimal::ONE_E9 exists on your type
        let a_d18 = a_d9.mul(&Decimal::ONE_E9)?; // now in D18

        // s0^2 (keep at D18): (s0_dec * s0_dec) / ONE_E18  => result D18
        let s0_sq = s0_dec.mul(&s0_dec)?.div(&Decimal::ONE_E18)?;

        // compute 2 * A_in_D18 (D18 * D18 = D36 ; divide by ONE_E18 -> D18)
        let two_d18 = Decimal::from_plain(2)?;
        let two_a_d18 = a_d18.mul(&two_d18)?.div(&Decimal::ONE_E18)?;

        // rhs = s0^2 + 2 * A
        let rhs = s0_sq.add(&two_a_d18)?;

        // s_new = sqrt(rhs)  (nth_root with n=2), returns D18
        let s_new = rhs.nth_root(2)?;

        // delta = s_new - s0_dec  (D18)
        let delta = s_new.sub(&s0_dec)?;

        // minted amount -> convert D18 -> token units (D9) using to_token_amount
        let amount_out = delta.to_token_amount(Rounding::Floor)?.0;

        // Update supply (checked)
        self.supplies[outcome_index] = self.supplies[outcome_index]
            .checked_add(amount_out)
            .ok_or(error!(ErrorCode::MathOverflow))?;

        // Recompute invariant (efficient/incremental update could be used, but recompute for correctness)
        self.recompute_invariant()?;

        Ok(amount_out)
    }

    pub fn sell_outcome(
        &mut self,
        outcome_index: usize,
        burn_amount: u64,
        vault_lamports: u64,
    ) -> Result<u64> {
        let supply_before = self.supplies[outcome_index];
        check_condition!(burn_amount <= supply_before, BurnIsMoreThanSupply);

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
            self.supplies[outcome_index] = self.supplies[outcome_index]
                .checked_sub(burn_amount)
                .ok_or(error!(ErrorCode::MathOverflow))?;
            self.recompute_invariant()
                .map_err(|_| error!(ErrorCode::MathOverflow))?;
            return Ok(0);
        }

        // Ensure vault has enough lamports
        check_condition!(vault_lamports >= refund_u64, InsufficientVaultFunds);

        // --- apply fee (fee stays in market vault) ---
        let fee = (refund_u64 as u128)
            .checked_mul(FEE_BPS as u128)
            .ok_or(error!(ErrorCode::MathOverflow))?
            / 10_000u128;
        let fee_u64 = fee as u64;
        let net_payout_u64 = refund_u64
            .checked_sub(fee_u64)
            .ok_or(error!(ErrorCode::MathOverflow))?;

        self.undistributed_fees = self
            .undistributed_fees
            .checked_add(fee_u64)
            .ok_or(error!(ErrorCode::MathOverflow))?;

        // --- Update market state: decrease reserve by full refund (refund includes fee that remains in vault)
        self.reserves[outcome_index] = self.reserves[outcome_index]
            .checked_sub(refund_u64)
            .ok_or(error!(ErrorCode::MathOverflow))?;

        // decrease supply by burned tokens
        self.supplies[outcome_index] = self.supplies[outcome_index]
            .checked_sub(burn_amount)
            .ok_or(error!(ErrorCode::MathOverflow))?;

        self.recompute_invariant()?;

        Ok(net_payout_u64)
    }
}
