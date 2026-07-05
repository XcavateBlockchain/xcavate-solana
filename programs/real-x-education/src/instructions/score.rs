use anchor_lang::prelude::*;
use anchor_spl::token_interface::{burn, Burn, Mint, TokenAccount, TokenInterface};

use crate::constants::{
    BOOKING_SEED, BOOK_ESCROW_SEED, CONFIG_SEED, DELIVERER_SEED, MODULE_SEED, MODULE_VAULT_SEED,
};
use crate::error::EducationError;
use crate::pricing::{bps_floor, fee_parts, scale_to_asset};
use crate::state::{Booking, Config, Deliverer, Module};
use crate::vault::release_from_vault;

use education_regions::state::Region;
use xcavate_roles::state::{Role, RoleAccount};

/// Propose a student score for a delivered booking. ModuleAIAgent-only. This
/// doesn't move any money: it records the score and opens the dispute window
/// during which the school or lecturer can contest it. Payment is released later
/// by `finalize_score`.
#[derive(Accounts)]
#[instruction(module_id: u64, booking_id: u64)]
pub struct SubmitImpactScore<'info> {
    pub agent: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            agent.key().as_ref(),
            &[Role::ModuleAIAgent.seed_byte()],
        ],
        bump = agent_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub agent_role: Box<Account<'info, RoleAccount>>,

    #[account(
        mut,
        seeds = [BOOKING_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        bump = booking.bump,
    )]
    pub booking: Box<Account<'info, Booking>>,
}

pub fn submit_impact_score_handler(
    ctx: Context<SubmitImpactScore>,
    module_id: u64,
    booking_id: u64,
    score: u16,
) -> Result<()> {
    require!(score <= 10_000, EducationError::InvalidScore);
    let lecturer = ctx
        .accounts
        .booking
        .lecturer
        .ok_or(EducationError::NoLecturerSet)?;
    require!(
        ctx.accounts.booking.score.is_none(),
        EducationError::ScoreAlreadySet
    );
    // A score can't be proposed before the session was scheduled to be delivered.
    let now = Clock::get()?.unix_timestamp;
    require!(
        now >= ctx.accounts.booking.delivery_at,
        EducationError::DeliveryNotReached
    );

    ctx.accounts.booking.score = Some(score);
    ctx.accounts.booking.score_at = Some(now);

    emit!(ImpactScoreSubmitted {
        module_id,
        booking_id,
        lecturer,
        score,
    });
    Ok(())
}

/// Dispute a proposed score with an amended one. Callable by the booking's
/// school or lecturer, once per booking, while the dispute window is open. The
/// amendment only takes effect if the counterparty accepts it.
#[derive(Accounts)]
#[instruction(module_id: u64, booking_id: u64)]
pub struct DisputeScore<'info> {
    pub party: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [BOOKING_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        bump = booking.bump,
    )]
    pub booking: Box<Account<'info, Booking>>,
}

pub fn dispute_score_handler(
    ctx: Context<DisputeScore>,
    module_id: u64,
    booking_id: u64,
    proposed_score: u16,
) -> Result<()> {
    require!(proposed_score <= 10_000, EducationError::InvalidScore);
    require!(
        !ctx.accounts.booking.settled,
        EducationError::AlreadySettled
    );
    let score_at = ctx
        .accounts
        .booking
        .score_at
        .ok_or(EducationError::NoTestResultsSubmitted)?;
    require!(
        ctx.accounts.booking.disputer.is_none(),
        EducationError::DisputeAlreadyRaised
    );

    let signer = ctx.accounts.party.key();
    let is_party =
        signer == ctx.accounts.booking.school || Some(signer) == ctx.accounts.booking.lecturer;
    require!(is_party, EducationError::NoPermission);

    let now = Clock::get()?.unix_timestamp;
    let deadline = score_at.saturating_add(ctx.accounts.config.dispute_window);
    require!(now < deadline, EducationError::DisputeWindowClosed);

    ctx.accounts.booking.disputer = Some(signer);
    ctx.accounts.booking.proposed_score = Some(proposed_score);

    emit!(ScoreDisputed {
        module_id,
        booking_id,
        disputer: signer,
        proposed_score,
    });
    Ok(())
}

/// Decide a pending dispute. Callable by the counterparty: whichever of the
/// school and lecturer didn't raise it. Accepting adopts the amended score;
/// rejecting leaves the agent's score in place. Either way the dispute closes.
#[derive(Accounts)]
#[instruction(module_id: u64, booking_id: u64)]
pub struct ResolveDispute<'info> {
    pub counterparty: Signer<'info>,

    #[account(
        mut,
        seeds = [BOOKING_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        bump = booking.bump,
    )]
    pub booking: Box<Account<'info, Booking>>,
}

pub fn resolve_dispute_handler(
    ctx: Context<ResolveDispute>,
    module_id: u64,
    booking_id: u64,
    accept: bool,
) -> Result<()> {
    require!(
        !ctx.accounts.booking.settled,
        EducationError::AlreadySettled
    );
    let proposed = ctx
        .accounts
        .booking
        .proposed_score
        .ok_or(EducationError::NoDispute)?;
    let disputer = ctx
        .accounts
        .booking
        .disputer
        .ok_or(EducationError::NoDispute)?;
    let lecturer = ctx
        .accounts
        .booking
        .lecturer
        .ok_or(EducationError::NoLecturerSet)?;

    // The counterparty is the party on the other side of whoever disputed.
    let counterparty = if disputer == ctx.accounts.booking.school {
        lecturer
    } else {
        ctx.accounts.booking.school
    };
    require!(
        ctx.accounts.counterparty.key() == counterparty,
        EducationError::NoPermission
    );

    if accept {
        ctx.accounts.booking.score = Some(proposed);
    }
    ctx.accounts.booking.proposed_score = None;

    emit!(DisputeResolved {
        module_id,
        booking_id,
        accepted: accept,
        score: ctx.accounts.booking.score.unwrap_or(proposed),
    });
    Ok(())
}

/// Finalize a scored booking and settle payment. Permissionless once the dispute
/// window has lapsed; the school or lecturer can bring it forward by calling it
/// themselves, and a concluded dispute settles immediately. Splits the escrowed
/// payment between the content creator, the region's operator, the protocol, and
/// the lecturer in proportion to the score (nothing pays out below the minimum),
/// refunds the remainder to the sponsor, and burns the delivered token.
#[derive(Accounts)]
#[instruction(module_id: u64, booking_id: u64)]
pub struct FinalizeScore<'info> {
    pub cranker: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        seeds = [MODULE_SEED, &module_id.to_le_bytes()],
        bump = module.bump,
    )]
    pub module: Box<Account<'info, Module>>,

    #[account(
        mut,
        seeds = [BOOKING_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        bump = booking.bump,
    )]
    pub booking: Box<Account<'info, Booking>>,

    /// The module's region, read for the regional operator's identity.
    #[account(
        seeds = [education_regions::REGION_SEED, &module.region.to_le_bytes()],
        bump = region_account.bump,
        seeds::program = education_regions::ID,
    )]
    pub region_account: Box<Account<'info, Region>>,

    #[account(address = booking.payment_asset @ EducationError::InvalidMint)]
    pub payment_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(mut, address = module.mint @ EducationError::InvalidMint)]
    pub module_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [MODULE_VAULT_SEED, &module_id.to_le_bytes()],
        bump,
        token::mint = module_mint,
        token::authority = config,
    )]
    pub module_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [BOOK_ESCROW_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        bump,
        token::mint = payment_mint,
        token::authority = config,
    )]
    pub booking_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = payment_mint,
        token::authority = module.creator,
    )]
    pub creator_payment: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = payment_mint,
        token::authority = region_account.owner,
    )]
    pub regional_operator_payment: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = payment_mint,
        token::authority = config.protocol_authority,
    )]
    pub protocol_payment: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The lecturer's payout account; its owner is checked against the booking.
    #[account(mut, token::mint = payment_mint)]
    pub lecturer_payment: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The lecturer's delivery registration (strike / claim bookkeeping).
    #[account(
        mut,
        seeds = [DELIVERER_SEED, lecturer_payment.owner.as_ref()],
        bump = deliverer.bump,
    )]
    pub deliverer: Box<Account<'info, Deliverer>>,

    #[account(
        mut,
        token::mint = payment_mint,
        token::authority = booking.sponsor,
    )]
    pub sponsor_payment: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn finalize_score_handler(
    ctx: Context<FinalizeScore>,
    module_id: u64,
    booking_id: u64,
) -> Result<()> {
    require!(
        !ctx.accounts.booking.settled,
        EducationError::AlreadySettled
    );
    let lecturer = ctx
        .accounts
        .booking
        .lecturer
        .ok_or(EducationError::NoLecturerSet)?;
    let score = ctx
        .accounts
        .booking
        .score
        .ok_or(EducationError::NoTestResultsSubmitted)?;
    let score_at = ctx
        .accounts
        .booking
        .score_at
        .ok_or(EducationError::NoTestResultsSubmitted)?;
    require!(
        ctx.accounts.lecturer_payment.owner == lecturer,
        EducationError::WrongPayoutRecipient
    );

    let now = Clock::get()?.unix_timestamp;
    let window_over = now >= score_at.saturating_add(ctx.accounts.config.dispute_window);

    // An amendment awaiting the counterparty only lapses once the window is over;
    // until then it has to be resolved rather than finalized around.
    if ctx.accounts.booking.proposed_score.is_some() {
        require!(window_over, EducationError::DisputePending);
        ctx.accounts.booking.proposed_score = None;
    }

    // Before the window closes, settlement can only be brought forward by one of
    // the two parties (their signature is their consent) or once a raised dispute
    // has already been decided.
    let signer = ctx.accounts.cranker.key();
    let is_party = signer == ctx.accounts.booking.school || signer == lecturer;
    let dispute_concluded = ctx.accounts.booking.disputer.is_some();
    require!(
        window_over || is_party || dispute_concluded,
        EducationError::DisputeWindowOpen
    );

    let decimals = ctx.accounts.payment_mint.decimals;
    let total_module_price = ctx.accounts.booking.price_per_token as u128;

    let base_scaled = scale_to_asset(ctx.accounts.module.price, decimals)?;
    let parts = fee_parts(
        base_scaled,
        ctx.accounts.module.content_creator_bps,
        ctx.accounts.module.regional_operator_bps,
        ctx.accounts.module.protocol_bps,
        ctx.accounts.module.dbs_bps,
    )?;

    let (cc_pay, ro_pay, proto_pay, lecturer_pay) =
        if score >= ctx.accounts.config.min_impact_score_bps {
            let cc = bps_floor(parts.content_creator, score)?;
            let ro = bps_floor(parts.regional_operator, score)?;
            let proto = bps_floor(parts.protocol, score)?;
            let dbs = bps_floor(parts.dbs, score)?;
            let lecturer_full = bps_floor(base_scaled, score)?
                .checked_add(dbs)
                .ok_or(EducationError::Overflow)?;

            let others = cc
                .checked_add(ro)
                .and_then(|v| v.checked_add(proto))
                .ok_or(EducationError::Overflow)?;
            let total_pay = lecturer_full
                .checked_add(others)
                .ok_or(EducationError::Overflow)?;

            // Never pay out more than was escrowed for this token.
            let lecturer_pay = if total_pay <= total_module_price {
                lecturer_full
            } else {
                total_module_price
                    .checked_sub(others)
                    .ok_or(EducationError::Underflow)?
            };

            let deliverer = &mut ctx.accounts.deliverer;
            deliverer.successful_deliveries = deliverer
                .successful_deliveries
                .checked_add(1)
                .ok_or(EducationError::Overflow)?;
            let reduction = ctx.accounts.config.deliveries_per_strike_reduction;
            if reduction > 0 && deliverer.successful_deliveries % reduction == 0 {
                deliverer.active_strikes = deliverer.active_strikes.saturating_sub(1);
                emit!(StrikeReduced {
                    lecturer,
                    new_strikes: deliverer.active_strikes,
                });
            }

            (cc, ro, proto, lecturer_pay)
        } else {
            (0u128, 0u128, 0u128, 0u128)
        };

    // The lecturer's claim concludes whether or not it paid out.
    ctx.accounts.deliverer.active_claims = ctx
        .accounts
        .deliverer
        .active_claims
        .checked_sub(1)
        .ok_or(EducationError::Underflow)?;

    let paid_out = cc_pay
        .checked_add(ro_pay)
        .and_then(|v| v.checked_add(proto_pay))
        .and_then(|v| v.checked_add(lecturer_pay))
        .ok_or(EducationError::Overflow)?;
    let refund = total_module_price
        .checked_sub(paid_out)
        .ok_or(EducationError::Underflow)?;

    let config_bump = ctx.accounts.config.bump;
    let token_program = ctx.accounts.token_program.to_account_info();
    let escrow = ctx.accounts.booking_escrow.to_account_info();
    let mint = ctx.accounts.payment_mint.to_account_info();
    let config_ai = ctx.accounts.config.to_account_info();
    pay_out(
        &token_program,
        &escrow,
        &mint,
        &ctx.accounts.creator_payment.to_account_info(),
        &config_ai,
        cc_pay,
        decimals,
        config_bump,
    )?;
    pay_out(
        &token_program,
        &escrow,
        &mint,
        &ctx.accounts.regional_operator_payment.to_account_info(),
        &config_ai,
        ro_pay,
        decimals,
        config_bump,
    )?;
    pay_out(
        &token_program,
        &escrow,
        &mint,
        &ctx.accounts.protocol_payment.to_account_info(),
        &config_ai,
        proto_pay,
        decimals,
        config_bump,
    )?;
    pay_out(
        &token_program,
        &escrow,
        &mint,
        &ctx.accounts.lecturer_payment.to_account_info(),
        &config_ai,
        lecturer_pay,
        decimals,
        config_bump,
    )?;
    pay_out(
        &token_program,
        &escrow,
        &mint,
        &ctx.accounts.sponsor_payment.to_account_info(),
        &config_ai,
        refund,
        decimals,
        config_bump,
    )?;

    // Burn the delivered module token.
    let bump = [config_bump];
    let signer_seeds: &[&[u8]] = &[CONFIG_SEED, &bump];
    burn(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Burn {
                mint: ctx.accounts.module_mint.to_account_info(),
                from: ctx.accounts.module_vault.to_account_info(),
                authority: ctx.accounts.config.to_account_info(),
            },
            &[signer_seeds],
        ),
        1,
    )?;

    ctx.accounts.booking.settled = true;

    emit!(ScoreFinalized {
        module_id,
        booking_id,
        lecturer,
        score,
        lecturer_pay: u64::try_from(lecturer_pay).map_err(|_| EducationError::Overflow)?,
    });
    Ok(())
}

/// Release `amount` of the escrowed payment to `to`, skipping zero moves.
#[allow(clippy::too_many_arguments)]
fn pay_out<'info>(
    token_program: &AccountInfo<'info>,
    escrow: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    config: &AccountInfo<'info>,
    amount: u128,
    decimals: u8,
    config_bump: u8,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let amount = u64::try_from(amount).map_err(|_| EducationError::Overflow)?;
    release_from_vault(
        token_program,
        escrow,
        mint,
        to,
        config,
        config_bump,
        amount,
        decimals,
    )
}

#[event]
pub struct StrikeReduced {
    pub lecturer: Pubkey,
    pub new_strikes: u8,
}

#[event]
pub struct ImpactScoreSubmitted {
    pub module_id: u64,
    pub booking_id: u64,
    pub lecturer: Pubkey,
    pub score: u16,
}

#[event]
pub struct ScoreDisputed {
    pub module_id: u64,
    pub booking_id: u64,
    pub disputer: Pubkey,
    pub proposed_score: u16,
}

#[event]
pub struct DisputeResolved {
    pub module_id: u64,
    pub booking_id: u64,
    pub accepted: bool,
    pub score: u16,
}

#[event]
pub struct ScoreFinalized {
    pub module_id: u64,
    pub booking_id: u64,
    pub lecturer: Pubkey,
    pub score: u16,
    pub lecturer_pay: u64,
}
