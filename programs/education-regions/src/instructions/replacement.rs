use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, REGION_SEED, REPLACEMENT_AUCTION_SEED, VAULT_SEED};
use crate::error::RegionsError;
use crate::state::{Config, Region, ReplacementAuction};
use crate::vault::{lock_to_vault, release_from_vault};

use xcavate_roles::state::{Role, RoleAccount};

/// Bid to take over a region whose operator seat is open (after the operator was
/// slashed past the strike ceiling, or their resignation notice elapsed).
/// RegionalOperator-only. The bid is locked as XCAV in the vault; an outbid
/// bidder is refunded in the same instruction. The leader may raise their own
/// bid, in which case only the difference is locked.
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct BidOnReplacement<'info> {
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

    /// The region whose seat is being contested; its `next_owner_change` gates
    /// whether the auction is allowed to run.
    #[account(
        seeds = [REGION_SEED, &region_id.to_le_bytes()],
        bump = region.bump,
    )]
    pub region: Box<Account<'info, Region>>,

    #[account(
        init_if_needed,
        payer = bidder,
        space = 8 + ReplacementAuction::INIT_SPACE,
        seeds = [REPLACEMENT_AUCTION_SEED, &region_id.to_le_bytes()],
        bump,
    )]
    pub auction: Box<Account<'info, ReplacementAuction>>,

    /// The outbid bidder's XCAV account to refund; required (and validated in
    /// the handler) only when there already is a highest bidder.
    #[account(mut, token::mint = config.xcav_mint)]
    pub previous_bidder_token: Option<Box<InterfaceAccount<'info, TokenAccount>>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn bid_on_replacement_handler(
    ctx: Context<BidOnReplacement>,
    region_id: u16,
    amount: u64,
) -> Result<()> {
    require!(amount > 0, RegionsError::BidBelowMinimum);

    let now = Clock::get()?.unix_timestamp;
    // The seat must be open: the region's owner-change lock has elapsed.
    require!(
        ctx.accounts.region.next_owner_change < now,
        RegionsError::RegionOwnerCantBeChanged
    );

    // A fresh auction (zero expiry) opens at the minimum collateral; otherwise it
    // must still be running.
    if ctx.accounts.auction.auction_expiry == 0 {
        let auction = &mut ctx.accounts.auction;
        auction.region_id = region_id;
        auction.highest_bidder = None;
        auction.collateral = ctx.accounts.config.minimum_region_deposit;
        auction.auction_expiry = now
            .checked_add(ctx.accounts.config.auction_period)
            .ok_or(RegionsError::Overflow)?;
        auction.bump = ctx.bumps.auction;
    } else {
        require!(now < ctx.accounts.auction.auction_expiry, RegionsError::AuctionEnded);
    }

    let bidder_key = ctx.accounts.bidder.key();

    // Work out how much extra XCAV to lock and who (if anyone) to refund.
    let (lock_amount, refund) = match ctx.accounts.auction.highest_bidder {
        Some(prev) if prev == bidder_key => {
            require!(amount > ctx.accounts.auction.collateral, RegionsError::BidTooLow);
            let extra = amount
                .checked_sub(ctx.accounts.auction.collateral)
                .ok_or(RegionsError::Overflow)?;
            (extra, 0)
        }
        Some(prev) => {
            require!(amount > ctx.accounts.auction.collateral, RegionsError::BidTooLow);
            let previous = ctx
                .accounts
                .previous_bidder_token
                .as_ref()
                .ok_or(RegionsError::MissingPreviousBidder)?;
            require!(previous.owner == prev, RegionsError::WrongPreviousBidder);
            (amount, ctx.accounts.auction.collateral)
        }
        None => {
            require!(amount >= ctx.accounts.auction.collateral, RegionsError::BidBelowMinimum);
            (amount, 0)
        }
    };

    let decimals = ctx.accounts.xcav_mint.decimals;
    let config_bump = ctx.accounts.config.bump;

    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.bidder_token.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.bidder.to_account_info(),
        lock_amount,
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

    let auction = &mut ctx.accounts.auction;
    auction.highest_bidder = Some(bidder_key);
    auction.collateral = amount;

    emit!(ReplacementBidPlaced { region_id, bidder: bidder_key, amount });
    Ok(())
}

/// Finalize a replacement auction once it has ended. Permissionless crank. If a
/// bidder won, the outgoing operator's collateral is returned and the winner is
/// installed with their bid as the new collateral; otherwise the auction is
/// simply cleared. The cranker reclaims the auction account's rent.
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct FinalizeReplacement<'info> {
    #[account(mut)]
    pub cranker: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    /// The XCAV mint (for `transfer_checked`).
    #[account(address = config.xcav_mint @ RegionsError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

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

    #[account(
        mut,
        close = cranker,
        seeds = [REPLACEMENT_AUCTION_SEED, &region_id.to_le_bytes()],
        bump = auction.bump,
    )]
    pub auction: Box<Account<'info, ReplacementAuction>>,

    /// The outgoing operator's XCAV account; receives their returned collateral
    /// when the seat changes hands. Optional so a no-bidder auction can be
    /// cranked without it; required whenever there is a winner.
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = region.owner,
    )]
    pub old_owner_token: Option<Box<InterfaceAccount<'info, TokenAccount>>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn finalize_replacement_handler(
    ctx: Context<FinalizeReplacement>,
    region_id: u16,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(
        now >= ctx.accounts.auction.auction_expiry,
        RegionsError::AuctionNotFinished
    );

    if let Some(new_owner) = ctx.accounts.auction.highest_bidder {
        let decimals = ctx.accounts.xcav_mint.decimals;
        let config_bump = ctx.accounts.config.bump;
        let outgoing_collateral = ctx.accounts.region.collateral;

        // Return the outgoing operator's collateral from the vault.
        let old_owner_token = ctx
            .accounts
            .old_owner_token
            .as_ref()
            .ok_or(RegionsError::MissingRecipientToken)?;
        release_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &old_owner_token.to_account_info(),
            &ctx.accounts.config.to_account_info(),
            config_bump,
            outgoing_collateral,
            decimals,
        )?;

        let next_owner_change = now
            .checked_add(ctx.accounts.config.owner_change_period)
            .ok_or(RegionsError::Overflow)?;
        let new_collateral = ctx.accounts.auction.collateral;

        let region = &mut ctx.accounts.region;
        region.owner = new_owner;
        region.collateral = new_collateral;
        region.active_strikes = 0;
        region.next_owner_change = next_owner_change;

        emit!(RegionOwnerChanged { region_id, new_owner, next_owner_change });
    }
    // `close = cranker` returns the auction account's rent.
    Ok(())
}

/// Schedule the caller's own departure as a region's operator. RegionalOperator
/// and current owner only. Sets the seat to open after the configured notice
/// period, allowing a replacement auction to start.
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
pub struct ReplacementBidPlaced {
    pub region_id: u16,
    pub bidder: Pubkey,
    pub amount: u64,
}

#[event]
pub struct RegionOwnerChanged {
    pub region_id: u16,
    pub new_owner: Pubkey,
    pub next_owner_change: i64,
}

#[event]
pub struct ResignationInitiated {
    pub region_id: u16,
    pub operator: Pubkey,
    pub next_owner_change: i64,
}
