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

/// Submit a student score for a delivered booking and settle payment.
/// ModuleAIAgent-only. Splits the escrowed payment between the content creator,
/// the region's operator, the protocol, and the lecturer in proportion to the
/// score (nothing pays out below the minimum), refunds the remainder to the
/// sponsor, and burns the delivered token.
#[derive(Accounts)]
#[instruction(module_id: u64, booking_id: u64)]
pub struct SubmitImpactScore<'info> {
    pub agent: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        seeds = [MODULE_SEED, &module_id.to_le_bytes()],
        bump = module.bump,
    )]
    pub module: Box<Account<'info, Module>>,

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
    require!(
        ctx.accounts.lecturer_payment.owner == lecturer,
        EducationError::WrongPayoutRecipient
    );
    // A score can't be settled before the session was scheduled to be delivered.
    require!(
        Clock::get()?.unix_timestamp >= ctx.accounts.booking.delivery_at,
        EducationError::DeliveryNotReached
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

    ctx.accounts.booking.score = Some(score);

    emit!(ImpactScoreSubmitted {
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
    pub lecturer_pay: u64,
}
