use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{
    BOOKING_SEED, BOOK_ESCROW_SEED, CANCELLATION_SEED, CANCEL_COUNTER_SEED, CONFIG_SEED,
    DELIVERER_SEED, MODULE_SEED, SPONSORSHIP_SEED, SPONSOR_ESCROW_SEED, VAULT_SEED,
};
use crate::error::EducationError;
use crate::pricing::fee_ceil;
use crate::state::{
    Booking, Cancellation, CancellationCounter, Config, Deliverer, Module, Sponsorship,
};
use crate::vault::{close_vault_account, release_from_vault};

use xcavate_roles::state::{Role, RoleAccount};

/// Cancel a booking. ModuleBooker-only and limited to the booking's own school.
/// Returns the token and its escrowed payment to the sponsor, refunds the
/// school's deposit (or slashes it once the school has cancelled too often), and
/// rolls back any lecturer claim.
#[derive(Accounts)]
#[instruction(module_id: u64, booking_id: u64)]
pub struct CancelBooking<'info> {
    #[account(mut)]
    pub school: Signer<'info>,

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
            school.key().as_ref(),
            &[Role::ModuleBooker.seed_byte()],
        ],
        bump = school_role.bump,
        seeds::program = xcavate_roles::ID,
        constraint = school_role.is_compliant() @ EducationError::NotCompliant,
    )]
    pub school_role: Box<Account<'info, RoleAccount>>,

    #[account(
        mut,
        close = school,
        seeds = [BOOKING_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        bump = booking.bump,
        constraint = booking.school == school.key() @ EducationError::NoPermission,
    )]
    pub booking: Box<Account<'info, Booking>>,

    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// Receives the school's deposit if it is slashed.
    #[account(mut, address = config.treasury @ EducationError::InvalidTreasury)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    /// Receives the school's deposit if it is refunded.
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = school,
    )]
    pub school_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = school,
        space = 8 + CancellationCounter::INIT_SPACE,
        seeds = [CANCEL_COUNTER_SEED, school.key().as_ref()],
        bump,
    )]
    pub counter: Box<Account<'info, CancellationCounter>>,

    #[account(
        init,
        payer = school,
        space = 8 + Cancellation::INIT_SPACE,
        seeds = [CANCELLATION_SEED, school.key().as_ref(), &booking_id.to_le_bytes()],
        bump,
    )]
    pub cancellation: Box<Account<'info, Cancellation>>,

    #[account(
        mut,
        seeds = [SPONSORSHIP_SEED, &module_id.to_le_bytes(), &booking.sponsor_id.to_le_bytes()],
        bump = sponsorship.bump,
    )]
    pub sponsorship: Box<Account<'info, Sponsorship>>,

    #[account(address = booking.payment_asset @ EducationError::InvalidMint)]
    pub payment_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [SPONSOR_ESCROW_SEED, &module_id.to_le_bytes(), &booking.sponsor_id.to_le_bytes()],
        bump,
        token::mint = payment_mint,
        token::authority = config,
    )]
    pub sponsor_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [BOOK_ESCROW_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        bump,
        token::mint = payment_mint,
        token::authority = config,
    )]
    pub booking_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    /// Required only when the booking was already claimed; rolls back the
    /// lecturer's active-claim count.
    #[account(mut)]
    pub deliverer: Option<Box<Account<'info, Deliverer>>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn cancel_booking_handler(
    ctx: Context<CancelBooking>,
    module_id: u64,
    booking_id: u64,
) -> Result<()> {
    let clock = Clock::get()?;
    let config_bump = ctx.accounts.config.bump;
    let per_token = ctx.accounts.booking.price_per_token;
    let deposit = ctx.accounts.booking.deposit;
    let lecturer = ctx.accounts.booking.lecturer;

    let count = ctx
        .accounts
        .counter
        .count
        .checked_add(1)
        .ok_or(EducationError::Overflow)?;

    // Return this token's payment from the booking escrow to the sponsor escrow.
    release_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.booking_escrow.to_account_info(),
        &ctx.accounts.payment_mint.to_account_info(),
        &ctx.accounts.sponsor_escrow.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        config_bump,
        per_token,
        ctx.accounts.payment_mint.decimals,
    )?;

    // Refund or slash the school's deposit depending on its cancellation count.
    let slashed = count >= ctx.accounts.config.max_cancellations;
    let destination = if slashed {
        ctx.accounts.treasury.to_account_info()
    } else {
        ctx.accounts.school_xcav.to_account_info()
    };
    release_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &destination,
        &ctx.accounts.config.to_account_info(),
        config_bump,
        deposit,
        ctx.accounts.xcav_mint.decimals,
    )?;

    // The token becomes bookable from the sponsor again, and this booking no
    // longer counts against the sponsorship.
    ctx.accounts.sponsorship.amount = ctx
        .accounts
        .sponsorship
        .amount
        .checked_add(1)
        .ok_or(EducationError::Overflow)?;
    ctx.accounts.sponsorship.active_bookings = ctx
        .accounts
        .sponsorship
        .active_bookings
        .checked_sub(1)
        .ok_or(EducationError::Underflow)?;
    ctx.accounts.module.school_allocation = ctx
        .accounts
        .module
        .school_allocation
        .checked_add(1)
        .ok_or(EducationError::Overflow)?;

    if let Some(lect) = lecturer {
        let deliverer = ctx
            .accounts
            .deliverer
            .as_mut()
            .ok_or(EducationError::MissingAccount)?;
        require!(deliverer.deliverer == lect, EducationError::WrongPayoutRecipient);
        deliverer.active_claims = deliverer
            .active_claims
            .checked_sub(1)
            .ok_or(EducationError::Underflow)?;
    } else {
        ctx.accounts.module.student_allocation = ctx
            .accounts
            .module
            .student_allocation
            .checked_sub(1)
            .ok_or(EducationError::Underflow)?;
    }

    let counter = &mut ctx.accounts.counter;
    counter.school = ctx.accounts.school.key();
    counter.count = count;
    counter.bump = ctx.bumps.counter;

    let cancellation = &mut ctx.accounts.cancellation;
    cancellation.school = ctx.accounts.school.key();
    cancellation.booking_id = booking_id;
    cancellation.module_id = module_id;
    cancellation.created_at = clock.unix_timestamp;
    cancellation.bump = ctx.bumps.cancellation;

    // The booking escrow has been drained back to the sponsor, so close it and
    // return its rent to the school along with the closed booking record.
    close_vault_account(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.booking_escrow.to_account_info(),
        &ctx.accounts.school.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        config_bump,
    )?;

    emit!(BookingCancelled {
        module_id,
        booking_id,
        school: ctx.accounts.school.key(),
        cancellation_count: count,
        deposit_slashed: slashed,
    });
    Ok(())
}

/// Clear one of a school's cancellation records once it has aged out of the
/// window, decrementing the rolling counter.
#[derive(Accounts)]
#[instruction(booking_id: u64)]
pub struct ClearOldCancellation<'info> {
    #[account(mut)]
    pub school: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            school.key().as_ref(),
            &[Role::ModuleBooker.seed_byte()],
        ],
        bump = school_role.bump,
        seeds::program = xcavate_roles::ID,
        constraint = school_role.is_compliant() @ EducationError::NotCompliant,
    )]
    pub school_role: Box<Account<'info, RoleAccount>>,

    #[account(
        mut,
        seeds = [CANCEL_COUNTER_SEED, school.key().as_ref()],
        bump = counter.bump,
    )]
    pub counter: Box<Account<'info, CancellationCounter>>,

    #[account(
        mut,
        close = school,
        seeds = [CANCELLATION_SEED, school.key().as_ref(), &booking_id.to_le_bytes()],
        bump = cancellation.bump,
    )]
    pub cancellation: Box<Account<'info, Cancellation>>,
}

pub fn clear_old_cancellation_handler(
    ctx: Context<ClearOldCancellation>,
    _booking_id: u64,
) -> Result<()> {
    let clock = Clock::get()?;
    let age = clock
        .unix_timestamp
        .saturating_sub(ctx.accounts.cancellation.created_at);
    require!(
        age > ctx.accounts.config.cancellation_window,
        EducationError::CancellationNotClearable
    );

    let counter = &mut ctx.accounts.counter;
    counter.count = counter.count.saturating_sub(1);

    emit!(OldCancellationCleared {
        school: ctx.accounts.school.key(),
        remaining: counter.count,
    });
    Ok(())
}

/// Cancel a claimed booking. ModuleDeliverer-only and limited to the lecturer
/// who claimed it. Adds a strike (slashing the deposit once the strike ceiling
/// is hit) and frees the token to be claimed again.
#[derive(Accounts)]
#[instruction(module_id: u64, booking_id: u64)]
pub struct CancelClaim<'info> {
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

    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(mut, address = config.treasury @ EducationError::InvalidTreasury)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn cancel_claim_handler(
    ctx: Context<CancelClaim>,
    module_id: u64,
    booking_id: u64,
) -> Result<()> {
    require!(
        ctx.accounts.booking.lecturer == Some(ctx.accounts.lecturer.key()),
        EducationError::NoPermission
    );
    // A delivery that has already been scored is settled: its claim was released
    // and its token burned when the score landed. Cancelling it here would
    // double-count the released claim and free a token that no longer exists.
    require!(
        ctx.accounts.booking.score.is_none(),
        EducationError::ScoreAlreadySet
    );

    let strikes = ctx
        .accounts
        .deliverer
        .active_strikes
        .checked_add(1)
        .ok_or(EducationError::Overflow)?;

    if strikes >= ctx.accounts.config.max_strikes {
        let slash_full = u64::try_from(fee_ceil(
            ctx.accounts.config.deliverer_deposit as u128,
            ctx.accounts.config.strike_slash_bps,
        )?)
        .map_err(|_| EducationError::Overflow)?;
        let slash = slash_full.min(ctx.accounts.deliverer.deposit);
        if slash > 0 {
            release_from_vault(
                &ctx.accounts.token_program.to_account_info(),
                &ctx.accounts.vault.to_account_info(),
                &ctx.accounts.xcav_mint.to_account_info(),
                &ctx.accounts.treasury.to_account_info(),
                &ctx.accounts.config.to_account_info(),
                ctx.accounts.config.bump,
                slash,
                ctx.accounts.xcav_mint.decimals,
            )?;
            ctx.accounts.deliverer.deposit = ctx
                .accounts
                .deliverer
                .deposit
                .checked_sub(slash)
                .ok_or(EducationError::Underflow)?;
        }
    }

    let booking = &mut ctx.accounts.booking;
    booking.lecturer = None;
    booking.claimed_at = None;

    let deliverer = &mut ctx.accounts.deliverer;
    deliverer.active_strikes = strikes;
    deliverer.active_claims = deliverer
        .active_claims
        .checked_sub(1)
        .ok_or(EducationError::Underflow)?;

    ctx.accounts.module.student_allocation = ctx
        .accounts
        .module
        .student_allocation
        .checked_add(1)
        .ok_or(EducationError::Overflow)?;

    emit!(ClaimCancelled {
        module_id,
        booking_id,
        lecturer: ctx.accounts.lecturer.key(),
        active_strikes: strikes,
    });
    Ok(())
}

#[event]
pub struct BookingCancelled {
    pub module_id: u64,
    pub booking_id: u64,
    pub school: Pubkey,
    pub cancellation_count: u32,
    pub deposit_slashed: bool,
}

#[event]
pub struct OldCancellationCleared {
    pub school: Pubkey,
    pub remaining: u32,
}

#[event]
pub struct ClaimCancelled {
    pub module_id: u64,
    pub booking_id: u64,
    pub lecturer: Pubkey,
    pub active_strikes: u8,
}
