use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, MODULE_SEED, SPONSORSHIP_SEED, SPONSOR_ESCROW_SEED};
use crate::error::EducationError;
use crate::pricing::price_per_token;
use crate::state::{Config, Module, Sponsorship};
use crate::vault::{close_vault_account, lock_to_vault, release_from_vault};

use xcavate_roles::state::{Role, RoleAccount};

/// Sponsor a module: lock the full price (base + fees) per token in escrow and
/// move that many tokens from the creator's allocation to a school allocation.
/// ModuleSponsor-only; the payment asset must be one the protocol accepts.
#[derive(Accounts)]
#[instruction(module_id: u64, token_amount: u64)]
pub struct SponsorModule<'info> {
    #[account(mut)]
    pub sponsor: Signer<'info>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [MODULE_SEED, &module_id.to_le_bytes()],
        bump = module.bump,
    )]
    pub module: Box<Account<'info, Module>>,

    /// The caller's ModuleSponsor role.
    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            sponsor.key().as_ref(),
            &[Role::ModuleSponsor.seed_byte()],
        ],
        bump = sponsor_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub sponsor_role: Box<Account<'info, RoleAccount>>,

    /// The stablecoin the sponsor pays in; must be an accepted asset.
    pub payment_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The sponsor's account the payment is pulled from.
    #[account(
        mut,
        token::mint = payment_mint,
        token::authority = sponsor,
    )]
    pub sponsor_payment: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        init,
        payer = sponsor,
        space = 8 + Sponsorship::INIT_SPACE,
        seeds = [SPONSORSHIP_SEED, &module_id.to_le_bytes(), &config.next_sponsor_id.to_le_bytes()],
        bump,
    )]
    pub sponsorship: Box<Account<'info, Sponsorship>>,

    /// The sponsorship's payment escrow, owned by the config PDA.
    #[account(
        init,
        payer = sponsor,
        seeds = [SPONSOR_ESCROW_SEED, &module_id.to_le_bytes(), &config.next_sponsor_id.to_le_bytes()],
        bump,
        token::mint = payment_mint,
        token::authority = config,
        token::token_program = token_program,
    )]
    pub sponsor_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn sponsor_module_handler(
    ctx: Context<SponsorModule>,
    module_id: u64,
    token_amount: u64,
) -> Result<()> {
    require!(token_amount > 0, EducationError::AmountCannotBeZero);
    require!(
        ctx.accounts
            .config
            .accepts(&ctx.accounts.payment_mint.key()),
        EducationError::PaymentAssetNotSupported
    );
    crate::mint_guard::require_supported_mint(&ctx.accounts.payment_mint.to_account_info())?;
    require!(
        token_amount <= ctx.accounts.module.sponsor_allocation,
        EducationError::NotEnoughTokenAvailable
    );

    let clock = Clock::get()?;
    let sponsor_id = ctx.accounts.config.next_sponsor_id;
    let decimals = ctx.accounts.payment_mint.decimals;

    let per_token = price_per_token(
        ctx.accounts.module.price,
        decimals,
        ctx.accounts.module.content_creator_bps,
        ctx.accounts.module.regional_operator_bps,
        ctx.accounts.module.protocol_bps,
        ctx.accounts.module.dbs_bps,
    )?;
    let total = per_token
        .checked_mul(token_amount)
        .ok_or(EducationError::Overflow)?;

    // Lock the sponsor's payment in the escrow.
    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.sponsor_payment.to_account_info(),
        &ctx.accounts.payment_mint.to_account_info(),
        &ctx.accounts.sponsor_escrow.to_account_info(),
        &ctx.accounts.sponsor.to_account_info(),
        total,
        decimals,
    )?;

    let module = &mut ctx.accounts.module;
    module.sponsor_allocation = module
        .sponsor_allocation
        .checked_sub(token_amount)
        .ok_or(EducationError::Underflow)?;
    module.school_allocation = module
        .school_allocation
        .checked_add(token_amount)
        .ok_or(EducationError::Overflow)?;

    let sponsorship = &mut ctx.accounts.sponsorship;
    sponsorship.module_id = module_id;
    sponsorship.sponsor_id = sponsor_id;
    sponsorship.sponsor = ctx.accounts.sponsor.key();
    sponsorship.payment_asset = ctx.accounts.payment_mint.key();
    sponsorship.amount = token_amount;
    sponsorship.active_bookings = 0;
    sponsorship.price_per_token = per_token;
    sponsorship.sponsored_at = clock.unix_timestamp;
    sponsorship.bump = ctx.bumps.sponsorship;

    ctx.accounts.config.next_sponsor_id =
        sponsor_id.checked_add(1).ok_or(EducationError::Overflow)?;

    emit!(ModuleSponsored {
        module_id,
        sponsor_id,
        sponsor: sponsorship.sponsor,
        token_amount,
        sponsored_at: sponsorship.sponsored_at,
    });
    Ok(())
}

/// Reclaim tokens a sponsor funded but that no school booked, once the
/// sponsorship window has passed. Refunds the locked payment for those tokens
/// and returns them to the creator's allocation.
#[derive(Accounts)]
#[instruction(module_id: u64, sponsor_id: u64)]
pub struct ReclaimSponsorship<'info> {
    #[account(mut)]
    pub sponsor: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [MODULE_SEED, &module_id.to_le_bytes()],
        bump = module.bump,
    )]
    pub module: Box<Account<'info, Module>>,

    // Gated by ownership, not by an active ModuleSponsor role: the
    // `sponsorship.sponsor` constraint below already restricts this to the
    // sponsor who funded it, which is all that's needed to refund their own
    // unbooked escrow.
    #[account(
        mut,
        seeds = [SPONSORSHIP_SEED, &module_id.to_le_bytes(), &sponsor_id.to_le_bytes()],
        bump = sponsorship.bump,
        constraint = sponsorship.sponsor == sponsor.key() @ EducationError::NoPermission,
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

    /// Where the refund is sent.
    #[account(
        mut,
        token::mint = payment_mint,
        token::authority = sponsor,
    )]
    pub sponsor_payment: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn reclaim_sponsorship_handler(
    ctx: Context<ReclaimSponsorship>,
    _module_id: u64,
    _sponsor_id: u64,
    amount: u64,
) -> Result<()> {
    require!(amount > 0, EducationError::AmountCannotBeZero);
    require!(
        amount <= ctx.accounts.sponsorship.amount,
        EducationError::NotEnoughTokenAvailable
    );

    let clock = Clock::get()?;
    let deadline = ctx
        .accounts
        .sponsorship
        .sponsored_at
        .saturating_add(ctx.accounts.config.sponsorship_window);
    require!(
        clock.unix_timestamp > deadline,
        EducationError::SponsorshipWindowNotExpired
    );

    let refund = ctx
        .accounts
        .sponsorship
        .price_per_token
        .checked_mul(amount)
        .ok_or(EducationError::Overflow)?;

    release_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.sponsor_escrow.to_account_info(),
        &ctx.accounts.payment_mint.to_account_info(),
        &ctx.accounts.sponsor_payment.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        ctx.accounts.config.bump,
        refund,
        ctx.accounts.payment_mint.decimals,
    )?;

    let module = &mut ctx.accounts.module;
    module.school_allocation = module
        .school_allocation
        .checked_sub(amount)
        .ok_or(EducationError::Underflow)?;
    module.sponsor_allocation = module
        .sponsor_allocation
        .checked_add(amount)
        .ok_or(EducationError::Overflow)?;

    let sponsorship = &mut ctx.accounts.sponsorship;
    sponsorship.amount = sponsorship
        .amount
        .checked_sub(amount)
        .ok_or(EducationError::Underflow)?;

    emit!(SponsorshipReclaimed {
        module_id: sponsorship.module_id,
        sponsor_id: sponsorship.sponsor_id,
        sponsor: sponsorship.sponsor,
        amount,
        refunded: refund,
    });
    Ok(())
}

#[event]
pub struct ModuleSponsored {
    pub module_id: u64,
    pub sponsor_id: u64,
    pub sponsor: Pubkey,
    pub token_amount: u64,
    pub sponsored_at: i64,
}

#[event]
pub struct SponsorshipReclaimed {
    pub module_id: u64,
    pub sponsor_id: u64,
    pub sponsor: Pubkey,
    pub amount: u64,
    pub refunded: u64,
}

/// Close a fully-spent sponsorship and reclaim its rent. Only once every funded
/// token has been booked or reclaimed (`amount == 0`) and every booking made
/// from it has settled (`active_bookings == 0`): an unsettled booking could
/// still be cancelled, which would need this escrow to refund into.
#[derive(Accounts)]
#[instruction(module_id: u64, sponsor_id: u64)]
pub struct CloseSponsorship<'info> {
    #[account(mut)]
    pub sponsor: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        close = sponsor,
        seeds = [SPONSORSHIP_SEED, &module_id.to_le_bytes(), &sponsor_id.to_le_bytes()],
        bump = sponsorship.bump,
        constraint = sponsorship.sponsor == sponsor.key() @ EducationError::NoPermission,
    )]
    pub sponsorship: Box<Account<'info, Sponsorship>>,

    #[account(
        mut,
        seeds = [SPONSOR_ESCROW_SEED, &module_id.to_le_bytes(), &sponsor_id.to_le_bytes()],
        bump,
        token::authority = config,
    )]
    pub sponsor_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn close_sponsorship_handler(
    ctx: Context<CloseSponsorship>,
    _module_id: u64,
    _sponsor_id: u64,
) -> Result<()> {
    require!(
        ctx.accounts.sponsorship.amount == 0 && ctx.accounts.sponsorship.active_bookings == 0,
        EducationError::SponsorshipNotEmpty
    );

    close_vault_account(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.sponsor_escrow.to_account_info(),
        &ctx.accounts.sponsor.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        ctx.accounts.config.bump,
    )?;

    emit!(SponsorshipClosed {
        module_id: ctx.accounts.sponsorship.module_id,
        sponsor_id: ctx.accounts.sponsorship.sponsor_id,
        sponsor: ctx.accounts.sponsor.key(),
    });
    Ok(())
}

#[event]
pub struct SponsorshipClosed {
    pub module_id: u64,
    pub sponsor_id: u64,
    pub sponsor: Pubkey,
}
