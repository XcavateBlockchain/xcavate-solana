use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, REGION_SEED, REGION_STATE_SEED, VAULT_SEED};
use crate::error::RegionsError;
use crate::state::{operator_bond, Config, Region, RegionState, RegionStatus};
use crate::vault::{lock_to_vault, release_from_vault};

use xcavate_roles::state::{Role, RoleAccount};

/// Claim a region whose proposal passed, creating it. Only the proposer may
/// claim it, and only while they still hold the RegionalOperator role (re-checked
/// here so a proposer whose role was revoked can't take the seat). The bond they
/// locked when proposing is already sitting in the vault and becomes the
/// region's collateral.
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct CreateRegion<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            creator.key().as_ref(),
            &[Role::RegionalOperator.seed_byte()],
        ],
        bump = creator_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub creator_role: Box<Account<'info, RoleAccount>>,

    #[account(
        mut,
        close = creator,
        seeds = [REGION_STATE_SEED, &region_id.to_le_bytes()],
        bump = region_state.bump,
    )]
    pub region_state: Account<'info, RegionState>,

    #[account(
        init,
        payer = creator,
        space = 8 + Region::INIT_SPACE,
        seeds = [REGION_SEED, &region_id.to_le_bytes()],
        bump,
    )]
    pub region: Account<'info, Region>,

    pub system_program: Program<'info, System>,
}

pub fn create_region_handler(ctx: Context<CreateRegion>, region_id: u16) -> Result<()> {
    require!(
        ctx.accounts.region_state.status == RegionStatus::Passed,
        RegionsError::RegionNotPassed
    );
    // Only the proposer may claim the region they proposed.
    require!(
        ctx.accounts.region_state.proposer == ctx.accounts.creator.key(),
        RegionsError::NotProposer
    );

    let now = Clock::get()?.unix_timestamp;
    // The claim window closes at `claim_deadline`; past it the state is only
    // clearable (the bond is refunded), so the seat can't be taken late.
    require!(
        now < ctx.accounts.region_state.claim_deadline,
        RegionsError::ClaimWindowClosed
    );
    let next_owner_change = now
        .checked_add(ctx.accounts.config.owner_change_period)
        .ok_or(RegionsError::Overflow)?;
    // The bond is already in the vault (locked when proposing); it just becomes
    // the region's recorded collateral.
    let collateral = ctx.accounts.region_state.deposit;

    let region = &mut ctx.accounts.region;
    region.region_id = region_id;
    region.owner = ctx.accounts.creator.key();
    region.collateral = collateral;
    region.next_owner_change = next_owner_change;
    region.bump = ctx.bumps.region;

    emit!(RegionCreated {
        region_id,
        owner: region.owner,
        collateral,
    });
    Ok(())
}

/// Claim a region whose operator seat is open: the term has elapsed, whether it
/// ran its course or was brought forward by a resignation notice. First-come:
/// RegionalOperator-only, no vote. The caller bonds 0.1% of the XCAV supply,
/// which becomes the new collateral, and the outgoing operator's collateral is
/// returned. The incumbent may claim their own seat to renew it, in which case
/// `old_owner_token` is omitted and only the difference between their existing
/// collateral and the current bond moves.
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct ClaimOpenRegion<'info> {
    #[account(mut)]
    pub new_operator: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            new_operator.key().as_ref(),
            &[Role::RegionalOperator.seed_byte()],
        ],
        bump = operator_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub operator_role: Box<Account<'info, RoleAccount>>,

    /// The XCAV mint (for `transfer_checked` and reading supply for the bond).
    #[account(address = config.xcav_mint @ RegionsError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The new operator's XCAV account the bond is pulled from.
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = new_operator,
    )]
    pub new_operator_token: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The protocol's XCAV escrow vault.
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
        seeds = [REGION_SEED, &region_id.to_le_bytes()],
        bump = region.bump,
    )]
    pub region: Box<Account<'info, Region>>,

    /// The outgoing operator's XCAV account; receives their returned collateral.
    /// Omitted when the incumbent renews their own seat (they are refunded
    /// through `new_operator_token` instead).
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = region.owner,
    )]
    pub old_owner_token: Option<Box<InterfaceAccount<'info, TokenAccount>>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn claim_open_region_handler(ctx: Context<ClaimOpenRegion>, _region_id: u16) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    // The seat must be open: the operator's term has elapsed.
    require!(
        ctx.accounts.region.next_owner_change < now,
        RegionsError::RegionOwnerCantBeChanged
    );

    let bond = operator_bond(ctx.accounts.xcav_mint.supply).ok_or(RegionsError::BondTooSmall)?;
    let decimals = ctx.accounts.xcav_mint.decimals;
    let config_bump = ctx.accounts.config.bump;
    let existing_collateral = ctx.accounts.region.collateral;
    let new_owner = ctx.accounts.new_operator.key();

    if new_owner == ctx.accounts.region.owner {
        // The incumbent renews their own seat: keep the collateral in place and
        // re-bond to the current amount, moving only the difference. There is no
        // separate outgoing account here, so `old_owner_token` is omitted.
        if bond > existing_collateral {
            lock_to_vault(
                &ctx.accounts.token_program.to_account_info(),
                &ctx.accounts.new_operator_token.to_account_info(),
                &ctx.accounts.xcav_mint.to_account_info(),
                &ctx.accounts.vault.to_account_info(),
                &ctx.accounts.new_operator.to_account_info(),
                bond - existing_collateral,
                decimals,
            )?;
        } else if existing_collateral > bond {
            release_from_vault(
                &ctx.accounts.token_program.to_account_info(),
                &ctx.accounts.vault.to_account_info(),
                &ctx.accounts.xcav_mint.to_account_info(),
                &ctx.accounts.new_operator_token.to_account_info(),
                &ctx.accounts.config.to_account_info(),
                config_bump,
                existing_collateral - bond,
                decimals,
            )?;
        }
    } else {
        // A different operator takes over: lock their full bond, then return the
        // outgoing operator's collateral to their account.
        let old_owner_token = ctx
            .accounts
            .old_owner_token
            .as_ref()
            .ok_or(RegionsError::MissingRecipientToken)?;
        lock_to_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.new_operator_token.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.new_operator.to_account_info(),
            bond,
            decimals,
        )?;
        release_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &old_owner_token.to_account_info(),
            &ctx.accounts.config.to_account_info(),
            config_bump,
            existing_collateral,
            decimals,
        )?;
    }

    let next_owner_change = now
        .checked_add(ctx.accounts.config.owner_change_period)
        .ok_or(RegionsError::Overflow)?;

    let region = &mut ctx.accounts.region;
    region.owner = new_owner;
    region.collateral = bond;
    region.next_owner_change = next_owner_change;

    emit!(RegionClaimed {
        region_id: region.region_id,
        new_owner,
        collateral: bond,
        next_owner_change,
    });
    Ok(())
}

/// Schedule the caller's own departure as a region's operator. RegionalOperator
/// and current owner only. Brings the seat open after the configured notice
/// period, allowing another operator to claim it.
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct InitiateResignation<'info> {
    pub operator: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            operator.key().as_ref(),
            &[Role::RegionalOperator.seed_byte()],
        ],
        bump = operator_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub operator_role: Box<Account<'info, RoleAccount>>,

    #[account(
        mut,
        seeds = [REGION_SEED, &region_id.to_le_bytes()],
        bump = region.bump,
    )]
    pub region: Box<Account<'info, Region>>,
}

pub fn initiate_resignation_handler(
    ctx: Context<InitiateResignation>,
    region_id: u16,
) -> Result<()> {
    require!(
        ctx.accounts.region.owner == ctx.accounts.operator.key(),
        RegionsError::NotRegionOwner
    );

    let now = Clock::get()?.unix_timestamp;
    let next_owner_change = now
        .checked_add(ctx.accounts.config.notice_period)
        .ok_or(RegionsError::Overflow)?;

    // Only allow bringing the change forward, never pushing it back or repeating.
    require!(
        ctx.accounts.region.next_owner_change > next_owner_change,
        RegionsError::OwnerChangeAlreadyScheduled
    );
    ctx.accounts.region.next_owner_change = next_owner_change;

    emit!(ResignationInitiated {
        region_id,
        operator: ctx.accounts.operator.key(),
        next_owner_change,
    });
    Ok(())
}

#[event]
pub struct RegionCreated {
    pub region_id: u16,
    pub owner: Pubkey,
    pub collateral: u64,
}

#[event]
pub struct RegionClaimed {
    pub region_id: u16,
    pub new_owner: Pubkey,
    pub collateral: u64,
    pub next_owner_change: i64,
}

#[event]
pub struct ResignationInitiated {
    pub region_id: u16,
    pub operator: Pubkey,
    pub next_owner_change: i64,
}
