use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, REGION_SEED, REGION_STATE_SEED, VAULT_SEED};
use crate::error::RegionsError;
use crate::state::{Config, Region, RegionState, RegionStatus};
use crate::vault::{lock_to_vault, release_from_vault};

use xcavate_roles::state::{Role, RoleAccount};

/// Bid to become a region's operator. RegionalOperator-only. The bid is locked
/// as XCAV in the vault; the previously outbid bidder (if any) is refunded in
/// the same instruction.
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct BidOnRegion<'info> {
    #[account(mut)]
    pub bidder: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            bidder.key().as_ref(),
            &[Role::RegionalOperator.seed_byte()],
        ],
        bump = operator_role.bump,
        seeds::program = xcavate_roles::ID,
        constraint = operator_role.is_compliant() @ RegionsError::NotCompliant,
    )]
    pub operator_role: Box<Account<'info, RoleAccount>>,

    /// The XCAV mint (for `transfer_checked`).
    #[account(address = config.xcav_mint @ RegionsError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The bidder's XCAV account the bid is pulled from.
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = bidder,
    )]
    pub bidder_token: Box<InterfaceAccount<'info, TokenAccount>>,

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
        seeds = [REGION_STATE_SEED, &region_id.to_le_bytes()],
        bump = region_state.bump,
    )]
    pub region_state: Box<Account<'info, RegionState>>,

    /// The outbid bidder's XCAV account to refund; required (and validated in
    /// the handler) only when there already is a highest bidder.
    #[account(mut)]
    pub previous_bidder_token: Option<Box<InterfaceAccount<'info, TokenAccount>>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn bid_on_region_handler(ctx: Context<BidOnRegion>, _region_id: u16, amount: u64) -> Result<()> {
    require!(
        ctx.accounts.region_state.status == RegionStatus::Auctioning,
        RegionsError::NotAuctioning
    );
    let now = Clock::get()?.unix_timestamp;
    require!(now < ctx.accounts.region_state.auction_expiry, RegionsError::AuctionEnded);

    let bidder_key = ctx.accounts.bidder.key();
    require!(
        ctx.accounts.region_state.highest_bidder != Some(bidder_key),
        RegionsError::AlreadyHighestBidder
    );

    // Validate the bid and figure out who to refund.
    let refund: u64 = match ctx.accounts.region_state.highest_bidder {
        Some(prev) => {
            require!(amount > ctx.accounts.region_state.collateral, RegionsError::BidTooLow);
            let previous = ctx
                .accounts
                .previous_bidder_token
                .as_ref()
                .ok_or(RegionsError::MissingPreviousBidder)?;
            require!(previous.owner == prev, RegionsError::WrongPreviousBidder);
            require!(previous.mint == ctx.accounts.config.xcav_mint, RegionsError::InvalidMint);
            ctx.accounts.region_state.collateral
        }
        None => {
            require!(amount >= ctx.accounts.region_state.collateral, RegionsError::BidBelowMinimum);
            0
        }
    };

    let decimals = ctx.accounts.xcav_mint.decimals;
    let config_bump = ctx.accounts.config.bump;

    // Lock the new bid in the vault, then refund the outbid bidder from it.
    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.bidder_token.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.bidder.to_account_info(),
        amount,
        decimals,
    )?;

    if refund > 0 {
        let previous = ctx.accounts.previous_bidder_token.as_ref().unwrap();
        release_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &previous.to_account_info(),
            &ctx.accounts.config.to_account_info(),
            config_bump,
            refund,
            decimals,
        )?;
    }

    let region_state = &mut ctx.accounts.region_state;
    region_state.highest_bidder = Some(bidder_key);
    region_state.collateral = amount;

    emit!(BidPlaced { region_id: region_state.region_id, bidder: bidder_key, amount });
    Ok(())
}

/// Create the region once its auction has ended. Callable by the winning
/// bidder; their locked bid (already in the vault) becomes the region's
/// collateral.
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct CreateNewRegion<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,

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

pub fn create_new_region_handler(ctx: Context<CreateNewRegion>, region_id: u16) -> Result<()> {
    require!(
        ctx.accounts.region_state.status == RegionStatus::Auctioning,
        RegionsError::NotAuctioning
    );
    let now = Clock::get()?.unix_timestamp;
    require!(now >= ctx.accounts.region_state.auction_expiry, RegionsError::AuctionNotFinished);
    require!(
        ctx.accounts.region_state.highest_bidder == Some(ctx.accounts.creator.key()),
        RegionsError::NotWinner
    );

    // The winning bid already sits in the vault; the region just records it as
    // its collateral (the vault keeps custody until the operator is replaced).
    let collateral = ctx.accounts.region_state.collateral;

    let next_owner_change = now
        .checked_add(ctx.accounts.config.owner_change_period)
        .ok_or(RegionsError::Overflow)?;

    let region = &mut ctx.accounts.region;
    region.region_id = region_id;
    region.owner = ctx.accounts.creator.key();
    region.collateral = collateral;
    region.active_strikes = 0;
    region.next_owner_change = next_owner_change;
    region.bump = ctx.bumps.region;

    emit!(RegionCreated { region_id, owner: region.owner, collateral });
    Ok(())
}

#[event]
pub struct BidPlaced {
    pub region_id: u16,
    pub bidder: Pubkey,
    pub amount: u64,
}

#[event]
pub struct RegionCreated {
    pub region_id: u16,
    pub owner: Pubkey,
    pub collateral: u64,
}
