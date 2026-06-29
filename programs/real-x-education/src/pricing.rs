//! Module pricing math.
//!
//! A module's base price is quoted in whole currency units. Payment happens in a
//! stablecoin whose smallest unit depends on its decimals, so prices are scaled
//! up by `10^decimals` and the per-party fees are applied in basis points at
//! that resolution. Fees round up (ceil) so the protocol never under-charges.

use crate::error::EducationError;
use anchor_lang::prelude::*;

/// `price_units * 10^decimals`, the base price in the asset's smallest unit.
pub fn scale_to_asset(price_units: u64, decimals: u8) -> Result<u128> {
    let multiplier = 10u128
        .checked_pow(decimals as u32)
        .ok_or(EducationError::Overflow)?;
    (price_units as u128)
        .checked_mul(multiplier)
        .ok_or(EducationError::Overflow.into())
}

/// `ceil(amount * bps / 10_000)`.
pub fn fee_ceil(amount: u128, bps: u16) -> Result<u128> {
    if bps == 0 {
        return Ok(0);
    }
    let numerator = amount
        .checked_mul(bps as u128)
        .ok_or(EducationError::Overflow)?
        .checked_add(9_999)
        .ok_or(EducationError::Overflow)?;
    Ok(numerator / 10_000)
}

/// `floor(amount * bps / 10_000)`. Used to scale a payout by a student score so
/// the sponsor is never over-charged.
pub fn bps_floor(amount: u128, bps: u16) -> Result<u128> {
    Ok(amount
        .checked_mul(bps as u128)
        .ok_or(EducationError::Overflow)?
        / 10_000)
}

/// The four fee parts of a base price, in the asset's smallest unit.
pub struct FeeParts {
    pub content_creator: u128,
    pub regional_operator: u128,
    pub protocol: u128,
    pub dbs: u128,
}

impl FeeParts {
    pub fn total(&self) -> Result<u128> {
        self.content_creator
            .checked_add(self.regional_operator)
            .and_then(|v| v.checked_add(self.protocol))
            .and_then(|v| v.checked_add(self.dbs))
            .ok_or(EducationError::Overflow.into())
    }
}

/// Compute the fee parts for a base price already scaled to the asset.
pub fn fee_parts(
    base_scaled: u128,
    content_creator_bps: u16,
    regional_operator_bps: u16,
    protocol_bps: u16,
    dbs_bps: u16,
) -> Result<FeeParts> {
    Ok(FeeParts {
        content_creator: fee_ceil(base_scaled, content_creator_bps)?,
        regional_operator: fee_ceil(base_scaled, regional_operator_bps)?,
        protocol: fee_ceil(base_scaled, protocol_bps)?,
        dbs: fee_ceil(base_scaled, dbs_bps)?,
    })
}

/// Total price for one module token: base price plus all fees, in asset units.
pub fn price_per_token(
    price_units: u64,
    decimals: u8,
    content_creator_bps: u16,
    regional_operator_bps: u16,
    protocol_bps: u16,
    dbs_bps: u16,
) -> Result<u64> {
    let base = scale_to_asset(price_units, decimals)?;
    let parts = fee_parts(
        base,
        content_creator_bps,
        regional_operator_bps,
        protocol_bps,
        dbs_bps,
    )?;
    let total = base
        .checked_add(parts.total()?)
        .ok_or(EducationError::Overflow)?;
    u64::try_from(total).map_err(|_| EducationError::Overflow.into())
}
