use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{
    BOOKING_SEED, BOOK_ESCROW_SEED, CONFIG_SEED, MODULE_SEED, SPONSORSHIP_SEED,
    SPONSOR_ESCROW_SEED, VAULT_SEED,
};
use crate::error::EducationError;
use crate::state::{Booking, Config, Module, Sponsorship};
use crate::vault::{close_vault_account, lock_to_vault, release_from_vault};

use xcavate_roles::state::{Role, RoleAccount};

/// Book one token of a sponsored module. ModuleBooker-only; locks the school's
/// XCAV deposit and moves the per-token payment from the sponsor escrow into a
/// per-booking escrow.
#[derive(Accounts)]
#[instruction(module_id: u64, sponsor_id: u64)]
pub struct BookModule<'info> {
    #[account(mut)]
    pub school: Signer<'info>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
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
    )]
    pub school_role: Box<Account<'info, RoleAccount>>,

    /// XCAV for the booking deposit.
    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = school,
    )]
    pub school_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [SPONSORSHIP_SEED, &module_id.to_le_bytes(), &sponsor_id.to_le_bytes()],
        bump = sponsorship.bump,
    )]
    pub sponsorship: Box<Account<'info, Sponsorship>>,

    #[account(address = sponsorship.payment_asset @ EducationError::InvalidMint)]
    pub payment_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [SPONSOR_ESCROW_SEED, &module_id.to_le_bytes(), &sponsor_id.to_le_bytes()],
        bump,
        token::mint = payment_mint,
        token::authority = config,
    )]
    pub sponsor_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        init,
        payer = school,
        space = 8 + Booking::INIT_SPACE,
        seeds = [BOOKING_SEED, &module_id.to_le_bytes(), &config.next_booking_id.to_le_bytes()],
        bump,
    )]
    pub booking: Box<Account<'info, Booking>>,

    #[account(
        init,
        payer = school,
        seeds = [BOOK_ESCROW_SEED, &module_id.to_le_bytes(), &config.next_booking_id.to_le_bytes()],
        bump,
        token::mint = payment_mint,
        token::authority = config,
        token::token_program = token_program,
    )]
    pub booking_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn book_module_handler(
    ctx: Context<BookModule>,
    module_id: u64,
    sponsor_id: u64,
    delivery_at: i64,
    metadata: String,
) -> Result<()> {
    require!(
        metadata.len() <= Booking::METADATA_MAX_LEN,
        EducationError::InvalidConfig
    );
    require!(
        ctx.accounts.module.school_allocation >= 1,
        EducationError::NotEnoughTokenAvailable
    );
    require!(
        ctx.accounts.sponsorship.amount > 0,
        EducationError::NoFundedModulesFromSponsor
    );

    let clock = Clock::get()?;
    // The scheduled delivery must be a present-or-future time, since scoring and
    // no-show expiry both key off it.
    require!(
        delivery_at >= clock.unix_timestamp,
        EducationError::DeliveryNotReached
    );
    let booking_id = ctx.accounts.config.next_booking_id;
    let deposit = ctx.accounts.config.booking_deposit;
    let per_token = ctx.accounts.sponsorship.price_per_token;
    let config_bump = ctx.accounts.config.bump;
    let decimals = ctx.accounts.payment_mint.decimals;

    // Lock the school's XCAV booking deposit.
    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.school_xcav.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.school.to_account_info(),
        deposit,
        ctx.accounts.xcav_mint.decimals,
    )?;

    // Move this token's payment from the sponsor escrow into the booking escrow.
    release_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.sponsor_escrow.to_account_info(),
        &ctx.accounts.payment_mint.to_account_info(),
        &ctx.accounts.booking_escrow.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        config_bump,
        per_token,
        decimals,
    )?;

    ctx.accounts.sponsorship.amount = ctx
        .accounts
        .sponsorship
        .amount
        .checked_sub(1)
        .ok_or(EducationError::Underflow)?;
    ctx.accounts.sponsorship.active_bookings = ctx
        .accounts
        .sponsorship
        .active_bookings
        .checked_add(1)
        .ok_or(EducationError::Overflow)?;

    let module = &mut ctx.accounts.module;
    module.school_allocation = module
        .school_allocation
        .checked_sub(1)
        .ok_or(EducationError::Underflow)?;
    module.student_allocation = module
        .student_allocation
        .checked_add(1)
        .ok_or(EducationError::Overflow)?;

    let sponsor = ctx.accounts.sponsorship.sponsor;
    let payment_asset = ctx.accounts.sponsorship.payment_asset;
    let booking = &mut ctx.accounts.booking;
    booking.module_id = module_id;
    booking.booking_id = booking_id;
    booking.sponsor_id = sponsor_id;
    booking.sponsor = sponsor;
    booking.school = ctx.accounts.school.key();
    booking.lecturer = None;
    booking.payment_asset = payment_asset;
    booking.price_per_token = per_token;
    booking.score = None;
    booking.deposit = deposit;
    booking.booked_at = clock.unix_timestamp;
    booking.delivery_at = delivery_at;
    booking.claimed_at = None;
    booking.metadata = metadata;
    booking.bump = ctx.bumps.booking;

    ctx.accounts.config.next_booking_id =
        booking_id.checked_add(1).ok_or(EducationError::Overflow)?;

    emit!(ModuleBooked {
        module_id,
        sponsor_id,
        booking_id,
        sponsor,
        school: booking.school,
        booked_at: booking.booked_at,
    });
    Ok(())
}

/// Release the school's deposit and close a booking once it has been scored.
/// ModuleBooker-only and limited to the booking's own school.
#[derive(Accounts)]
#[instruction(module_id: u64, booking_id: u64)]
pub struct FinishBooking<'info> {
    #[account(mut)]
    pub school: Signer<'info>,

    // Gated by ownership, not by an active ModuleBooker role: the `booking.school`
    // constraint below already restricts this to the booking's own school, which
    // is all that's needed to release its deposit and settle the sponsorship.
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

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

    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = school,
    )]
    pub school_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        close = school,
        seeds = [BOOKING_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        bump = booking.bump,
        constraint = booking.school == school.key() @ EducationError::NoPermission,
    )]
    pub booking: Box<Account<'info, Booking>>,

    /// The booking's payment escrow, drained when the score landed. Closed here
    /// so its rent goes back to the school that opened it.
    #[account(
        mut,
        seeds = [BOOK_ESCROW_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        bump,
        token::authority = config,
    )]
    pub booking_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The sponsorship this booking drew from; its in-flight count drops as the
    /// booking settles here.
    #[account(
        mut,
        seeds = [SPONSORSHIP_SEED, &module_id.to_le_bytes(), &booking.sponsor_id.to_le_bytes()],
        bump = sponsorship.bump,
    )]
    pub sponsorship: Box<Account<'info, Sponsorship>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn finish_booking_handler(
    ctx: Context<FinishBooking>,
    _module_id: u64,
    _booking_id: u64,
) -> Result<()> {
    require!(
        ctx.accounts.booking.score.is_some(),
        EducationError::NoTestResultsSubmitted
    );

    ctx.accounts.sponsorship.active_bookings = ctx
        .accounts
        .sponsorship
        .active_bookings
        .checked_sub(1)
        .ok_or(EducationError::Underflow)?;

    release_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.school_xcav.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        ctx.accounts.config.bump,
        ctx.accounts.booking.deposit,
        ctx.accounts.xcav_mint.decimals,
    )?;

    // The escrow was drained by the payout, so close it to return its rent to the
    // school alongside the closed booking record, but only if it is actually
    // empty. A stray transfer into the account after settlement would make the SPL
    // close fail and revert the whole instruction, so any residual balance leaves
    // the escrow behind while the deposit release still stands.
    ctx.accounts.booking_escrow.reload()?;
    if ctx.accounts.booking_escrow.amount == 0 {
        close_vault_account(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.booking_escrow.to_account_info(),
            &ctx.accounts.school.to_account_info(),
            &ctx.accounts.config.to_account_info(),
            ctx.accounts.config.bump,
        )?;
    }

    emit!(BookingFinished {
        module_id: ctx.accounts.booking.module_id,
        booking_id: ctx.accounts.booking.booking_id,
        school: ctx.accounts.school.key(),
    });
    Ok(())
}

#[event]
pub struct ModuleBooked {
    pub module_id: u64,
    pub sponsor_id: u64,
    pub booking_id: u64,
    pub sponsor: Pubkey,
    pub school: Pubkey,
    pub booked_at: i64,
}

#[event]
pub struct BookingFinished {
    pub module_id: u64,
    pub booking_id: u64,
    pub school: Pubkey,
}
