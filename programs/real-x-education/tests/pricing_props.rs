//! Property tests for the module pricing math.
//!
//! These check the invariants the payment split relies on: fees round up but
//! never by more than a unit, score scaling never over-charges, and the total
//! price is exactly the base plus every fee part with nothing lost or created.

use proptest::prelude::*;
use real_x_education::pricing::{
    bps_floor, fee_ceil, fee_parts, price_per_token, scale_to_asset,
};

/// Exact `floor(a * bps / 10_000)` in u128, used as the reference.
fn ref_floor(a: u128, bps: u16) -> u128 {
    a * bps as u128 / 10_000
}

/// Exact `ceil(a * bps / 10_000)` in u128, used as the reference.
fn ref_ceil(a: u128, bps: u16) -> u128 {
    if bps == 0 {
        return 0;
    }
    (a * bps as u128 + 9_999) / 10_000
}

proptest! {
    /// Scaling by decimals is just multiplication by a power of ten, and it
    /// never overflows u128 for any realistic decimals count.
    #[test]
    fn scale_to_asset_is_power_of_ten(price in any::<u64>(), decimals in 0u8..=18) {
        let expected = price as u128 * 10u128.pow(decimals as u32);
        prop_assert_eq!(scale_to_asset(price, decimals).unwrap(), expected);
    }

    /// A fee of zero basis points is always zero.
    #[test]
    fn fee_ceil_zero_bps_is_zero(a in 0u128..=1_000_000_000_000_000_000u128) {
        prop_assert_eq!(fee_ceil(a, 0).unwrap(), 0);
    }

    /// `fee_ceil` equals the exact rounded-up fraction.
    #[test]
    fn fee_ceil_matches_reference(
        a in 0u128..=1_000_000_000_000_000_000u128,
        bps in 0u16..=10_000,
    ) {
        prop_assert_eq!(fee_ceil(a, bps).unwrap(), ref_ceil(a, bps));
    }

    /// Rounding up sits at or one unit above the exact floor, never further.
    #[test]
    fn fee_ceil_within_one_of_floor(
        a in 0u128..=1_000_000_000_000_000_000u128,
        bps in 0u16..=10_000,
    ) {
        let floor = ref_floor(a, bps);
        let ceil = fee_ceil(a, bps).unwrap();
        prop_assert!(ceil >= floor);
        prop_assert!(ceil - floor <= 1);
    }

    /// A larger fee rate never produces a smaller fee.
    #[test]
    fn fee_ceil_is_monotonic(
        a in 0u128..=1_000_000_000_000_000_000u128,
        b1 in 0u16..=10_000,
        b2 in 0u16..=10_000,
    ) {
        let (lo, hi) = if b1 <= b2 { (b1, b2) } else { (b2, b1) };
        prop_assert!(fee_ceil(a, lo).unwrap() <= fee_ceil(a, hi).unwrap());
    }

    /// `bps_floor` equals the exact rounded-down fraction.
    #[test]
    fn bps_floor_matches_reference(
        a in 0u128..=1_000_000_000_000_000_000u128,
        bps in 0u16..=10_000,
    ) {
        prop_assert_eq!(bps_floor(a, bps).unwrap(), ref_floor(a, bps));
    }

    /// Scaling by a score in basis points never exceeds the input, so a payout
    /// scaled by a score can never over-charge.
    #[test]
    fn bps_floor_never_exceeds_input(
        a in 0u128..=1_000_000_000_000_000_000u128,
        bps in 0u16..=10_000,
    ) {
        prop_assert!(bps_floor(a, bps).unwrap() <= a);
    }

    /// Full score returns the whole amount; zero score returns nothing.
    #[test]
    fn bps_floor_endpoints(a in 0u128..=1_000_000_000_000_000_000u128) {
        prop_assert_eq!(bps_floor(a, 0).unwrap(), 0);
        prop_assert_eq!(bps_floor(a, 10_000).unwrap(), a);
    }

    /// A higher score never scales a payout down.
    #[test]
    fn bps_floor_is_monotonic(
        a in 0u128..=1_000_000_000_000_000_000u128,
        b1 in 0u16..=10_000,
        b2 in 0u16..=10_000,
    ) {
        let (lo, hi) = if b1 <= b2 { (b1, b2) } else { (b2, b1) };
        prop_assert!(bps_floor(a, lo).unwrap() <= bps_floor(a, hi).unwrap());
    }

    /// The total escrowed price is exactly the base plus every fee part, and
    /// each part is bounded by the total. This is the invariant the score
    /// settlement depends on to conserve money.
    #[test]
    fn price_per_token_is_base_plus_fees(
        price in 0u64..=1_000_000,
        decimals in 0u8..=9,
        cc in 0u16..=2_500,
        ro in 0u16..=2_500,
        proto in 0u16..=2_500,
        dbs in 0u16..=2_500,
    ) {
        let base = scale_to_asset(price, decimals).unwrap();
        let parts = fee_parts(base, cc, ro, proto, dbs).unwrap();
        let total = price_per_token(price, decimals, cc, ro, proto, dbs).unwrap() as u128;

        prop_assert_eq!(total, base + parts.total().unwrap());
        prop_assert!(base <= total);
        prop_assert!(parts.content_creator <= total);
        prop_assert!(parts.regional_operator <= total);
        prop_assert!(parts.protocol <= total);
        prop_assert!(parts.dbs <= total);
    }

    /// On any input the price computation returns a value or a clean error; it
    /// never wraps or panics.
    #[test]
    fn price_per_token_never_wraps(
        price in any::<u64>(),
        decimals in 0u8..=40,
        cc in 0u16..=10_000,
        ro in 0u16..=10_000,
        proto in 0u16..=10_000,
        dbs in 0u16..=10_000,
    ) {
        let _ = price_per_token(price, decimals, cc, ro, proto, dbs);
    }
}
