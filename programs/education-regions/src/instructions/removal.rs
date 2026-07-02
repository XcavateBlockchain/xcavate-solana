use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{
    CONFIG_SEED, REGION_SEED, REMOVAL_PROPOSAL_SEED, REMOVAL_VOTE_SEED, VAULT_SEED,
};
use crate::error::RegionsError;
use crate::state::{Config, Region, RemovalProposal, RemovalVoteRecord, Vote};
use crate::vault::{lock_to_vault, release_from_vault};

/// Propose removing a region's operator. Anyone may open one; the proposer locks
/// a dispute deposit in the vault. Only one removal proposal can be open per
/// region at a time (the PDA is seeded by `region_id`).
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct ProposeRemoveOperator<'info> {
    #[account(mut)]
    pub proposer: Signer<'info>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    /// The XCAV mint (for `transfer_checked`).
    #[account(address = config.xcav_mint @ RegionsError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The proposer's XCAV account the deposit is pulled from.
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = proposer,
    )]
    pub proposer_token: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The protocol's XCAV escrow vault.
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The region whose operator is being challenged; loading it proves it exists.
    #[account(
        seeds = [REGION_SEED, &region_id.to_le_bytes()],
        bump = region.bump,
    )]
    pub region: Box<Account<'info, Region>>,

    #[account(
        init,
        payer = proposer,
        space = 8 + RemovalProposal::INIT_SPACE,
        seeds = [REMOVAL_PROPOSAL_SEED, &region_id.to_le_bytes()],
        bump,
    )]
    pub removal_proposal: Box<Account<'info, RemovalProposal>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn propose_remove_operator_handler(
    ctx: Context<ProposeRemoveOperator>,
    region_id: u16,
) -> Result<()> {
    let clock = Clock::get()?;
    let proposal_id = ctx.accounts.config.proposal_counter;
    let deposit = ctx.accounts.config.removal_deposit;
    let voting_period = ctx.accounts.config.removal_voting_period;

    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.proposer_token.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.proposer.to_account_info(),
        deposit,
        ctx.accounts.xcav_mint.decimals,
    )?;

    let expiry = clock.unix_timestamp.saturating_add(voting_period);

    let proposal = &mut ctx.accounts.removal_proposal;
    proposal.proposal_id = proposal_id;
    proposal.region_id = region_id;
    proposal.proposer = ctx.accounts.proposer.key();
    proposal.created_at = clock.unix_timestamp;
    proposal.expiry = expiry;
    proposal.deposit = deposit;
    proposal.yes_power = 0;
    proposal.no_power = 0;
    proposal.abstain_power = 0;
    proposal.bump = ctx.bumps.removal_proposal;

    ctx.accounts.config.proposal_counter =
        proposal_id.checked_add(1).ok_or(RegionsError::Overflow)?;

    emit!(RemoveOperatorProposed {
        region_id,
        proposal_id,
        proposer: ctx.accounts.proposer.key(),
        expiry,
    });
    Ok(())
}

/// Vote on an open removal proposal. Anyone may vote; the voting power is locked
/// as XCAV in the vault and returned when they unlock after the proposal ends.
/// Voting again replaces the prior vote.
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct VoteOnRemoval<'info> {
    #[account(mut)]
    pub voter: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

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
        mut,
        seeds = [REMOVAL_PROPOSAL_SEED, &region_id.to_le_bytes()],
        bump = removal_proposal.bump,
    )]
    pub removal_proposal: Box<Account<'info, RemovalProposal>>,

    // init_if_needed upserts a per-voter record: the PDA is seeded by the voter,
    // so only they can target it, and the handler fully overwrites it.
    #[account(
        init_if_needed,
        payer = voter,
        space = 8 + RemovalVoteRecord::INIT_SPACE,
        seeds = [REMOVAL_VOTE_SEED, &removal_proposal.proposal_id.to_le_bytes(), voter.key().as_ref()],
        bump,
    )]
    pub vote_record: Box<Account<'info, RemovalVoteRecord>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn vote_on_removal_handler(
    ctx: Context<VoteOnRemoval>,
    region_id: u16,
    vote: Vote,
    amount: u64,
) -> Result<()> {
    require!(
        amount >= ctx.accounts.config.minimum_voting_amount,
        RegionsError::BelowMinimumVotingAmount
    );

    let now = Clock::get()?.unix_timestamp;
    require!(now < ctx.accounts.removal_proposal.expiry, RegionsError::ProposalExpired);

    let decimals = ctx.accounts.xcav_mint.decimals;
    let config_bump = ctx.accounts.config.bump;

    // If the voter already voted, undo the previous tally and refund the prior
    // lock from the vault before locking the new amount.
    if ctx.accounts.vote_record.voter != Pubkey::default() {
        let old = ctx.accounts.vote_record.power;
        let old_vote = ctx.accounts.vote_record.vote;
        sub_power(&mut ctx.accounts.removal_proposal, old_vote, old);
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

    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.voter_token.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.voter.to_account_info(),
        amount,
        decimals,
    )?;

    add_power(&mut ctx.accounts.removal_proposal, vote, amount)?;

    let proposal_id = ctx.accounts.removal_proposal.proposal_id;
    let expiry = ctx.accounts.removal_proposal.expiry;
    let vote_record = &mut ctx.accounts.vote_record;
    vote_record.proposal_id = proposal_id;
    vote_record.voter = ctx.accounts.voter.key();
    vote_record.region_id = region_id;
    vote_record.vote = vote;
    vote_record.power = amount;
    vote_record.expiry = expiry;
    vote_record.bump = ctx.bumps.vote_record;

    emit!(VotedOnRemoval {
        region_id,
        proposal_id,
        voter: vote_record.voter,
        vote,
        power: amount,
    });
    Ok(())
}

/// Finalize an expired removal proposal. Permissionless crank. If it passed
/// (threshold + quorum) the region's operator is slashed a strike worth of
/// collateral and, once enough strikes accrue, the seat is opened for a
/// replacement auction; the proposer's deposit is returned. Otherwise the
/// proposer's deposit is slashed to the treasury.
#[derive(Accounts)]
#[instruction(region_id: u16)]
pub struct FinalizeRemoval<'info> {
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
        close = proposer,
        has_one = proposer,
        seeds = [REMOVAL_PROPOSAL_SEED, &region_id.to_le_bytes()],
        bump = removal_proposal.bump,
    )]
    pub removal_proposal: Box<Account<'info, RemovalProposal>>,

    /// CHECK: the proposal's proposer (enforced by `has_one`); receives the
    /// closed proposal's rent.
    #[account(mut)]
    pub proposer: UncheckedAccount<'info>,

    /// The proposer's XCAV account; receives the returned deposit if the removal passed.
    /// Optional so a rejection (which slashes to the treasury) can still be
    /// cranked when the proposer has closed their token account.
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = proposer,
    )]
    pub proposer_token: Option<Box<InterfaceAccount<'info, TokenAccount>>>,

    /// The configured treasury XCAV account; receives slashed deposits/collateral.
    #[account(mut, address = config.treasury @ RegionsError::InvalidTreasury)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn finalize_removal_handler(ctx: Context<FinalizeRemoval>, region_id: u16) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(
        now >= ctx.accounts.removal_proposal.expiry,
        RegionsError::VotingStillOngoing
    );

    let proposal = &ctx.accounts.removal_proposal;
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
    let meets_quorum = total > ctx.accounts.config.quorum;
    let proposal_id = proposal.proposal_id;
    let deposit = proposal.deposit;

    let decimals = ctx.accounts.xcav_mint.decimals;
    let config_bump = ctx.accounts.config.bump;

    if meets_threshold && meets_quorum {
        // Slash a strike's worth of the operator's collateral to the treasury.
        let slash = ctx
            .accounts
            .config
            .slash_amount
            .min(ctx.accounts.region.collateral);
        if slash > 0 {
            release_from_vault(
                &ctx.accounts.token_program.to_account_info(),
                &ctx.accounts.vault.to_account_info(),
                &ctx.accounts.xcav_mint.to_account_info(),
                &ctx.accounts.treasury.to_account_info(),
                &ctx.accounts.config.to_account_info(),
                config_bump,
                slash,
                decimals,
            )?;
        }

        let allowed_strikes = ctx.accounts.config.allowed_strikes;
        let region = &mut ctx.accounts.region;
        region.collateral = region.collateral.saturating_sub(slash);
        region.active_strikes = region.active_strikes.saturating_add(1);
        let new_strikes = region.active_strikes;
        let new_collateral = region.collateral;
        // Once the strike ceiling is hit, open the seat for replacement now.
        if new_strikes >= allowed_strikes {
            region.next_owner_change = now;
        }

        // Return the proposer's deposit.
        let proposer_token = ctx
            .accounts
            .proposer_token
            .as_ref()
            .ok_or(RegionsError::MissingRecipientToken)?;
        release_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &proposer_token.to_account_info(),
            &ctx.accounts.config.to_account_info(),
            config_bump,
            deposit,
            decimals,
        )?;

        emit!(OperatorSlashed {
            region_id,
            proposal_id,
            slashed: slash,
            new_collateral,
            new_strikes,
        });
    } else {
        // Rejected: slash the proposer's deposit to the treasury.
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

        emit!(RemoveOperatorRejected { region_id, proposal_id, slashed: deposit });
    }
    Ok(())
}

/// Reclaim the XCAV a voter locked on a removal proposal, once its voting window
/// has ended. The record's stored `expiry` means this never needs the proposal
/// account (which finalization may already have closed).
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct UnlockRemovalVote<'info> {
    #[account(mut)]
    pub voter: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    /// The XCAV mint (for `transfer_checked`).
    #[account(address = config.xcav_mint @ RegionsError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The voter's XCAV account the locked power is returned to.
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
        mut,
        close = voter,
        seeds = [REMOVAL_VOTE_SEED, &proposal_id.to_le_bytes(), voter.key().as_ref()],
        bump = vote_record.bump,
    )]
    pub vote_record: Box<Account<'info, RemovalVoteRecord>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn unlock_removal_vote_handler(ctx: Context<UnlockRemovalVote>, _proposal_id: u64) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(now >= ctx.accounts.vote_record.expiry, RegionsError::VotingStillOngoing);

    let power = ctx.accounts.vote_record.power;
    if power > 0 {
        release_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &ctx.accounts.voter_token.to_account_info(),
            &ctx.accounts.config.to_account_info(),
            ctx.accounts.config.bump,
            power,
            ctx.accounts.xcav_mint.decimals,
        )?;
    }

    emit!(RemovalVoteUnlocked {
        proposal_id: ctx.accounts.vote_record.proposal_id,
        voter: ctx.accounts.voter.key(),
        power,
    });
    // `close = voter` returns the record's rent.
    Ok(())
}

fn add_power(p: &mut RemovalProposal, vote: Vote, amount: u64) -> Result<()> {
    let slot = match vote {
        Vote::Yes => &mut p.yes_power,
        Vote::No => &mut p.no_power,
        Vote::Abstain => &mut p.abstain_power,
    };
    *slot = slot.checked_add(amount).ok_or(RegionsError::Overflow)?;
    Ok(())
}

fn sub_power(p: &mut RemovalProposal, vote: Vote, amount: u64) {
    let slot = match vote {
        Vote::Yes => &mut p.yes_power,
        Vote::No => &mut p.no_power,
        Vote::Abstain => &mut p.abstain_power,
    };
    *slot = slot.saturating_sub(amount);
}

#[event]
pub struct RemoveOperatorProposed {
    pub region_id: u16,
    pub proposal_id: u64,
    pub proposer: Pubkey,
    pub expiry: i64,
}

#[event]
pub struct VotedOnRemoval {
    pub region_id: u16,
    pub proposal_id: u64,
    pub voter: Pubkey,
    pub vote: Vote,
    pub power: u64,
}

#[event]
pub struct OperatorSlashed {
    pub region_id: u16,
    pub proposal_id: u64,
    pub slashed: u64,
    pub new_collateral: u64,
    pub new_strikes: u8,
}

#[event]
pub struct RemoveOperatorRejected {
    pub region_id: u16,
    pub proposal_id: u64,
    pub slashed: u64,
}

#[event]
pub struct RemovalVoteUnlocked {
    pub proposal_id: u64,
    pub voter: Pubkey,
    pub power: u64,
}
