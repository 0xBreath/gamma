use anchor_lang::prelude::*;
use common::check_condition;
use common::constants::common::*;
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

    /// Outcome mint token supplies for each outcome, fixed-point scaled.
    /// All values stored as u64 but promoted to u128 for math.
    /// Each outcome has a unique mint but all have the same decimals, so this is safe to apply generic math to.
    pub supplies: [u64; MAX_OUTCOMES],

    /// Precision scalar (e.g., 1e6 or 1e12)
    /// Used so geometric mean calculations stay stable.
    pub scale: u64,

    pub initialized_at: u64,

    /// When the market will resolve and halt trading
    pub resolve_at: i64,

    /// Lamports held in the market_vault not yet claimed by the fee recipient
    pub undistributed_fees: u64,

    /// The admin of the market who can mutate it
    pub admin: Pubkey,

    pub label: FixedSizeString,

    /// Number of outcomes (N)
    pub num_outcomes: u8,

    /// Bump for this [`Market`]
    pub bump: u8,

    /// Bump for market_vault which contains SOL reserves on behalf of the [`Market`]
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
        let n = self.num_outcomes as usize;
        check_condition!(outcome_index < n, InvalidOutcomeIndex);
        check_condition!(amount_in > 0, DepositIsZero);

        // Get current invariant k = ∏ reserves[i]
        let k = self.invariant_u256();
        let is_first_trade = k.is_zero();

        if is_first_trade {
            // First trade: initialize all reserves to scale
            for i in 0..n {
                self.reserves[i] = self.scale;
            }

            // Add user's deposit to the bought outcome's reserve
            self.reserves[outcome_index] = self.reserves[outcome_index]
                .checked_add(amount_in)
                .ok_or(error!(ErrorCode::MathOverflow))?;

            // Set initial invariant k = ∏ reserves[i]
            self.recompute_invariant()?;

            // Mint tokens 1:1 for first trade
            let amount_out = amount_in;
            self.supplies[outcome_index] = amount_out;

            return Ok(amount_out);
        }

        // Geometric mean AMM (Balancer-style with equal weights)
        // Invariant: k = ∏ reserves[i]
        //
        // When buying outcome i, we add lamports to reserve[i]
        // This increases k, and we mint tokens proportionally
        //
        // For minimal cross-impact, tokens minted should be:
        // tokens_out = supply[i] × (Δreserve / old_reserve)
        //
        // This means the token supply grows proportionally to the reserve,
        // keeping the price relatively stable for that outcome

        let old_reserve = self.reserves[outcome_index];
        check_condition!(old_reserve > 0, ReserveIsZero);

        let old_supply = self.supplies[outcome_index];

        // Add user's deposit to reserve
        let new_reserve = old_reserve
            .checked_add(amount_in)
            .ok_or(error!(ErrorCode::MathOverflow))?;

        self.reserves[outcome_index] = new_reserve;

        // Calculate tokens to mint: supply × (amount_in / old_reserve)
        let amount_out = if old_supply == 0 {
            // If no supply yet, mint 1:1
            amount_in
        } else {
            // Mint proportional to reserve increase
            ((old_supply as u128)
                .checked_mul(amount_in as u128)
                .ok_or(error!(ErrorCode::MathOverflow))?
                .checked_div(old_reserve as u128)
                .ok_or(error!(ErrorCode::MathOverflow))?) as u64
        };

        // Update supply
        self.supplies[outcome_index] = self.supplies[outcome_index]
            .checked_add(amount_out)
            .ok_or(error!(ErrorCode::MathOverflow))?;

        // Recompute invariant (it increases as we add liquidity)
        self.recompute_invariant()?;

        Ok(amount_out)
    }

    pub fn sell_outcome(
        &mut self,
        outcome_index: usize,
        burn_amount: u64,
        vault_lamports: u64,
    ) -> Result<u64> {
        let n = self.num_outcomes as usize;
        check_condition!(outcome_index < n, InvalidOutcomeIndex);
        check_condition!(burn_amount > 0, BurnIsZero);

        let supply_before = self.supplies[outcome_index];
        let reserve_before = self.reserves[outcome_index];

        check_condition!(burn_amount <= supply_before, BurnIsMoreThanSupply);
        check_condition!(supply_before > 0, SupplyIsZero);

        // Geometric mean AMM sell formula (inverse of buy)
        // When buying: tokens_minted = supply × (amount_in / reserve)
        // When selling: refund = reserve × (burn_amount / supply)
        //
        // This maintains the reserve-to-supply ratio and ensures symmetry

        // Calculate refund: reserve × (burn_amount / supply)
        let refund_u64 = ((reserve_before as u128)
            .checked_mul(burn_amount as u128)
            .ok_or(error!(ErrorCode::MathOverflow))?
            .checked_div(supply_before as u128)
            .ok_or(error!(ErrorCode::MathOverflow))?) as u64;

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

    /// Compute normalized percentage of total liquidity for each outcome.
    /// Returns [u64; MAX_OUTCOMES] where each value represents the percentage
    /// of total reserves that outcome holds, scaled by 1e9 (i.e., 100% = 1_000_000_000).
    ///
    /// For example, if outcome 0 has 30% of total liquidity, the returned value
    /// at index 0 would be 300_000_000.
    pub fn liquidity_percentages(&self) -> Result<[u64; MAX_OUTCOMES]> {
        let n = self.num_outcomes as usize;
        check_condition!(n <= MAX_OUTCOMES, InvalidOutcomeIndex);

        // Compute total reserves across all active outcomes
        let mut total: u128 = 0;
        for i in 0..n {
            total = total
                .checked_add(self.reserves[i] as u128)
                .ok_or(error!(ErrorCode::MathOverflow))?;
        }

        // Initialize result array with zeros
        let mut percentages = [0u64; MAX_OUTCOMES];

        // Handle edge case: if total is zero, all percentages are zero
        if total == 0 {
            return Ok(percentages);
        }

        // Compute percentage for each active outcome
        // percentage = (reserve / total) * 1e9
        // We use 1e9 scaling to maintain precision (100% = 1_000_000_000)

        for i in 0..n {
            let reserve = self.reserves[i] as u128;
            let percentage = reserve
                .checked_mul(D9_U128)
                .ok_or(error!(ErrorCode::MathOverflow))?
                .checked_div(total)
                .ok_or(error!(ErrorCode::MathOverflow))?;

            // Clamp to u64::MAX if somehow exceeds (shouldn't happen in practice)
            percentages[i] = if percentage > u64::MAX as u128 {
                u64::MAX
            } else {
                percentage as u64
            };
        }

        Ok(percentages)
    }

    /// Compute the marginal price for a given outcome.
    /// This represents the cost per token based on the current reserve-to-supply ratio.
    /// Returns a u64 scaled by 1e9 (i.e., price of 1.0 = 1_000_000_000).
    ///
    /// Formula: price = reserve_i / supply_i
    ///
    /// This gives each outcome an independent price that reflects its own liquidity,
    /// minimizing cross-impact from other outcomes.
    ///
    /// For example:
    /// - If reserve = 100M and supply = 100M tokens, price = 1.0 (1_000_000_000)
    /// - If reserve = 200M and supply = 100M tokens, price = 2.0 (2_000_000_000)
    pub fn outcome_price(&self, outcome_index: usize) -> Result<u64> {
        let n = self.num_outcomes as usize;
        check_condition!(n <= MAX_OUTCOMES, InvalidOutcomeIndex);
        check_condition!(outcome_index < n, InvalidOutcomeIndex);

        let reserve = self.reserves[outcome_index] as u128;
        let supply = self.supplies[outcome_index] as u128;

        // Handle edge case: if supply is zero, return 0
        if supply == 0 {
            return Ok(0);
        }

        // Compute price: (reserve / supply) * 1e9
        // This gives the average cost per token in lamports, scaled by 1e9
        let price = reserve
            .checked_mul(D9_U128)
            .ok_or(error!(ErrorCode::MathOverflow))?
            .checked_div(supply)
            .ok_or(error!(ErrorCode::MathOverflow))?;

        // Clamp to u64::MAX if somehow exceeds (shouldn't happen in practice)
        if price > u64::MAX as u128 {
            Ok(u64::MAX)
        } else {
            Ok(price as u64)
        }
    }
}
