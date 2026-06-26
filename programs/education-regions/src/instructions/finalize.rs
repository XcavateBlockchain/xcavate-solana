use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, PROPOSAL_SEED, REGION_STATE_SEED, VAULT_SEED};
use crate::error::RegionsError;
use crate::state::{Config, RegionProposal, RegionState, RegionStatus};
use crate::vault::release_from_vault;

/// Finalize an expired proposal. Permissionless — anyone can crank it once the
/// voting window closes. If it passed (threshold + quorum) the region moves to
/// an auction and the proposer's deposit is returned; otherwise the deposit is
/// slashed to the treasury and the region is marked rejected.
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct FinalizeRegionProposal<'info> {
    /// Pays the transaction fee; no authority required.
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
        seeds = [REGION_STATE_SEED, &region_id.to_le_bytes()],
        bump = region_state.bump,
    )]
    pub region_state: Box<Account<'info, RegionState>>,

    #[account(
        mut,
        close = proposer,
        has_one = proposer,
        seeds = [PROPOSAL_SEED, &region_state.proposal_id.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Box<Account<'info, RegionProposal>>,

    /// CHECK: the proposal's proposer (enforced by `has_one`); receives the
    /// closed proposal's rent.
    #[account(mut)]
    pub proposer: UncheckedAccount<'info>,

    /// The proposer's XCAV account; receives the returned deposit if it passed.
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = proposer,
    )]
    pub proposer_token: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The configured treasury XCAV account; receives a slashed deposit.
    #[account(mut, address = config.treasury @ RegionsError::InvalidTreasury)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn finalize_region_proposal_handler(
    ctx: Context<FinalizeRegionProposal>,
    region_id: u16,
) -> Result<()> {
    require!(
        ctx.accounts.region_state.status == RegionStatus::Proposing,
        RegionsError::NotProposing
    );

    let now = Clock::get()?.unix_timestamp;
    require!(now >= ctx.accounts.proposal.expiry, RegionsError::VotingStillOngoing);

    let proposal = &ctx.accounts.proposal;
    let yes = proposal.yes_power;
    let no = proposal.no_power;
    let abstain = proposal.abstain_power;
    let total = yes
        .checked_add(no)
        .and_then(|v| v.checked_add(abstain))
        .ok_or(RegionsError::Overflow)?;
    let approval_base = yes.checked_add(no).ok_or(RegionsError::Overflow)?;

    let threshold_bps = ctx.accounts.config.threshold_bps as u128;
    let meets_threshold = total != 0
        && (yes as u128).saturating_mul(10_000) >= (approval_base as u128).saturating_mul(threshold_bps);
    let meets_quorum = total >= ctx.accounts.config.quorum;
    let proposal_id = proposal.proposal_id;
    let deposit = proposal.deposit;

    let decimals = ctx.accounts.xcav_mint.decimals;
    let config_bump = ctx.accounts.config.bump;

    if meets_threshold && meets_quorum {
        // Passed: return the proposer's deposit and open the region for bidding.
        // (The proposal account also closes, returning its rent to the proposer.)
        release_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &ctx.accounts.proposer_token.to_account_info(),
            &ctx.accounts.config.to_account_info(),
            config_bump,
            deposit,
            decimals,
        )?;

        let region_state = &mut ctx.accounts.region_state;
        region_state.status = RegionStatus::Auctioning;
        region_state.highest_bidder = None;
        region_state.collateral = ctx.accounts.config.minimum_region_deposit;
        region_state.auction_expiry = now
            .checked_add(ctx.accounts.config.auction_period)
            .ok_or(RegionsError::Overflow)?;

        emit!(RegionAuctionStarted { region_id, proposal_id });
    } else {
        // Rejected: slash the deposit to the treasury. (The proposal account
        // still closes, returning its rent to the proposer.)
        release_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &ctx.accounts.treasury.to_account_info(),
            &ctx.accounts.config.to_account_info(),
            config_bump,
            deposit,
            decimals,
        )?;

        ctx.accounts.region_state.status = RegionStatus::Rejected;
        emit!(RegionProposalRejected { region_id, proposal_id, slashed: deposit });
    }
    Ok(())
}

#[event]
pub struct RegionAuctionStarted {
    pub region_id: u16,
    pub proposal_id: u64,
}

#[event]
pub struct RegionProposalRejected {
    pub region_id: u16,
    pub proposal_id: u64,
    pub slashed: u64,
}
