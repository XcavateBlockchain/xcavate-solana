use anchor_lang::prelude::*;

use crate::constants::{BOOKING_SEED, CONFIG_SEED, DELIVERER_SEED, MODULE_SEED};
use crate::error::EducationError;
use crate::pricing::fee_ceil;
use crate::state::{Booking, Config, Deliverer, Module};

use xcavate_roles::state::{Role, RoleAccount};

/// Claim a booking to deliver it. ModuleDeliverer-only; the caller must be
/// registered with enough deposit to back another concurrent claim, and may not
/// claim a booking made by their own school account.
#[derive(Accounts)]
#[instruction(module_id: u64, booking_id: u64)]
pub struct ClaimBooking<'info> {
    pub lecturer: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [MODULE_SEED, &module_id.to_le_bytes()],
        bump = module.bump,
    )]
    pub module: Box<Account<'info, Module>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            lecturer.key().as_ref(),
            &[Role::ModuleDeliverer.seed_byte()],
        ],
        bump = lecturer_role.bump,
        seeds::program = xcavate_roles::ID,
        constraint = lecturer_role.is_compliant() @ EducationError::NotCompliant,
    )]
    pub lecturer_role: Box<Account<'info, RoleAccount>>,

    #[account(
        mut,
        seeds = [DELIVERER_SEED, lecturer.key().as_ref()],
        bump = deliverer.bump,
    )]
    pub deliverer: Box<Account<'info, Deliverer>>,

    #[account(
        mut,
        seeds = [BOOKING_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        bump = booking.bump,
    )]
    pub booking: Box<Account<'info, Booking>>,
}

pub fn claim_booking_handler(
    ctx: Context<ClaimBooking>,
    _module_id: u64,
    _booking_id: u64,
) -> Result<()> {
    require!(
        ctx.accounts.booking.lecturer.is_none(),
        EducationError::LecturerAlreadySet
    );
    require!(
        ctx.accounts.booking.school != ctx.accounts.lecturer.key(),
        EducationError::SchoolCannotClaimOwnBooking
    );

    // Each concurrent claim must be backed by one strike's worth of deposit.
    let slash_per_strike = u64::try_from(fee_ceil(
        ctx.accounts.config.deliverer_deposit as u128,
        ctx.accounts.config.strike_slash_bps,
    )?)
    .map_err(|_| EducationError::Overflow)?;
    let concurrent_claims = ctx
        .accounts
        .deliverer
        .active_claims
        .checked_add(1)
        .ok_or(EducationError::Overflow)?;
    let required_deposit = slash_per_strike
        .checked_mul(concurrent_claims as u64)
        .ok_or(EducationError::Overflow)?;
    require!(
        ctx.accounts.deliverer.deposit >= required_deposit,
        EducationError::InsufficientDepositToClaim
    );

    require!(
        ctx.accounts.module.student_allocation >= 1,
        EducationError::NotEnoughTokenAvailable
    );

    let clock = Clock::get()?;

    ctx.accounts.module.student_allocation = ctx
        .accounts
        .module
        .student_allocation
        .checked_sub(1)
        .ok_or(EducationError::Underflow)?;

    let booking = &mut ctx.accounts.booking;
    booking.lecturer = Some(ctx.accounts.lecturer.key());
    booking.claimed_at = Some(clock.unix_timestamp);

    ctx.accounts.deliverer.active_claims = concurrent_claims;

    emit!(BookingClaimed {
        module_id: booking.module_id,
        booking_id: booking.booking_id,
        lecturer: ctx.accounts.lecturer.key(),
        claimed_at: clock.unix_timestamp,
    });
    Ok(())
}

#[event]
pub struct BookingClaimed {
    pub module_id: u64,
    pub booking_id: u64,
    pub lecturer: Pubkey,
    pub claimed_at: i64,
}
