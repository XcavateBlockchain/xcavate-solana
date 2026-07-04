use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, PROPOSAL_SEED, REGION_STATE_SEED, VAULT_SEED, VOTE_SEED};
use crate::error::RegionsError;
use crate::state::{Config, RegionProposal, RegionState, Vote, VoteRecord};
use crate::vault::{lock_to_vault, release_from_vault};

/// Vote on an open region proposal. Anyone may vote (no role/KYC). The voting
/// power is locked as XCAV in the vault and returned when they unlock after the
/// proposal ends. Voting again replaces the prior vote.
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct VoteOnRegionProposal<'info> {
    #[account(mut)]
    pub voter: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,

    /// The XCAV mint (for `transfer_checked`).
    #[account(address = config.xcav_mint @ RegionsError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The voter's XCAV account the lock is pulled from / refunded to.
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = voter,
    )]
    pub voter_token: Box<InterfaceAccount<'info, TokenAccount>>,

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
        seeds = [REGION_STATE_SEED, &region_id.to_le_bytes()],
        bump = region_state.bump,
    )]
    pub region_state: Account<'info, RegionState>,

    #[account(
        mut,
        seeds = [PROPOSAL_SEED, &region_state.proposal_id.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Account<'info, RegionProposal>,

    // init_if_needed is the natural fit for an upsert of a per-voter record: the
    // PDA is seeded by the voter, so only they can target it, and the handler
    // fully overwrites it, so there is no state an attacker could reset.
    #[account(
        init_if_needed,
        payer = voter,
        space = 8 + VoteRecord::INIT_SPACE,
        seeds = [VOTE_SEED, &region_state.proposal_id.to_le_bytes(), voter.key().as_ref()],
        bump,
    )]
    pub vote_record: Account<'info, VoteRecord>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn vote_on_region_proposal_handler(
    ctx: Context<VoteOnRegionProposal>,
    region_id: u16,
    vote: Vote,
    amount: u64,
) -> Result<()> {
    require!(
        amount >= ctx.accounts.config.minimum_voting_amount,
        RegionsError::BelowMinimumVotingAmount
    );

    let now = Clock::get()?.unix_timestamp;
    require!(
        now < ctx.accounts.proposal.expiry,
        RegionsError::ProposalExpired
    );

    let decimals = ctx.accounts.xcav_mint.decimals;
    let config_bump = ctx.accounts.config.bump;

    // If the voter already voted, undo the previous tally and refund the prior
    // lock from the vault before locking the new amount.
    if ctx.accounts.vote_record.voter != Pubkey::default() {
        let old = ctx.accounts.vote_record.power;
        let old_vote = ctx.accounts.vote_record.vote;
        sub_power(&mut ctx.accounts.proposal, old_vote, old);
        if old > 0 {
            release_from_vault(
                &ctx.accounts.token_program.to_account_info(),
                &ctx.accounts.vault.to_account_info(),
                &ctx.accounts.xcav_mint.to_account_info(),
                &ctx.accounts.voter_token.to_account_info(),
                &ctx.accounts.config.to_account_info(),
                config_bump,
                old,
                decimals,
            )?;
        }
    }

    // Lock the new amount of XCAV in the vault.
    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.voter_token.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.voter.to_account_info(),
        amount,
        decimals,
    )?;

    add_power(&mut ctx.accounts.proposal, vote, amount)?;

    let proposal_id = ctx.accounts.proposal.proposal_id;
    let expiry = ctx.accounts.proposal.expiry;
    let vote_record = &mut ctx.accounts.vote_record;
    vote_record.proposal_id = proposal_id;
    vote_record.voter = ctx.accounts.voter.key();
    vote_record.region_id = region_id;
    vote_record.vote = vote;
    vote_record.power = amount;
    vote_record.expiry = expiry;
    vote_record.bump = ctx.bumps.vote_record;

    emit!(VotedOnRegionProposal {
        region_id,
        proposal_id,
        voter: vote_record.voter,
        vote,
        power: amount,
    });
    Ok(())
}

fn add_power(p: &mut RegionProposal, vote: Vote, amount: u64) -> Result<()> {
    let slot = match vote {
        Vote::Yes => &mut p.yes_power,
        Vote::No => &mut p.no_power,
        Vote::Abstain => &mut p.abstain_power,
    };
    *slot = slot.checked_add(amount).ok_or(RegionsError::Overflow)?;
    Ok(())
}

fn sub_power(p: &mut RegionProposal, vote: Vote, amount: u64) {
    let slot = match vote {
        Vote::Yes => &mut p.yes_power,
        Vote::No => &mut p.no_power,
        Vote::Abstain => &mut p.abstain_power,
    };
    *slot = slot.saturating_sub(amount);
}

#[event]
pub struct VotedOnRegionProposal {
    pub region_id: u16,
    pub proposal_id: u64,
    pub voter: Pubkey,
    pub vote: Vote,
    pub power: u64,
}
