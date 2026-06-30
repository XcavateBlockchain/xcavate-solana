use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{
    CONFIG_SEED, MODULE_MINT_SEED, MODULE_PROPOSAL_SEED, MODULE_SEED, MODULE_VAULT_SEED,
    PROPOSAL_ESCROW_SEED, PROPOSAL_VOTE_SEED, SPONSORSHIP_SEED, SPONSOR_ESCROW_SEED, VAULT_SEED,
};
use crate::error::EducationError;
use crate::minting::{lock_module_deposit, mint_full_supply, write_module, ModuleTerms};
use crate::pricing::price_per_token;
use crate::state::{
    Config, Module, ModuleProposal, ModuleProposalVote, ModuleVote, ProposalStatus, Sponsorship,
};
use crate::vault::{close_vault_account, lock_to_vault, release_from_vault};

use education_regions::state::Region;
use xcavate_roles::state::{Role, RoleAccount};

/// Roles allowed to open a proposal: creators, sponsors, schools, and lecturers.
fn is_proposer_role(role: Role) -> bool {
    matches!(
        role,
        Role::ModuleCreator | Role::ModuleSponsor | Role::ModuleBooker | Role::ModuleDeliverer
    )
}

/// Snapshot the config's pricing onto a fresh proposal so the module is later
/// built on the terms it was voted on.
#[allow(clippy::too_many_arguments)]
fn init_proposal_common(
    proposal: &mut ModuleProposal,
    config: &Config,
    proposal_id: u64,
    proposer: Pubkey,
    role: Role,
    region: u16,
    module_amount: u64,
    metadata: String,
    now: i64,
    bump: u8,
) {
    proposal.proposal_id = proposal_id;
    proposal.proposer = proposer;
    proposal.proposer_role = role;
    proposal.region = region;
    proposal.status = ProposalStatus::Voting;
    proposal.created_at = now;
    proposal.expiry = now.saturating_add(config.voting_period);
    proposal.deposit = config.proposal_deposit;
    proposal.yes_power = 0;
    proposal.no_power = 0;
    proposal.abstain_power = 0;
    proposal.module_amount = module_amount;
    proposal.price = config.module_price;
    proposal.content_creator_bps = config.content_creator_bps;
    proposal.regional_operator_bps = config.regional_operator_bps;
    proposal.protocol_bps = config.protocol_bps;
    proposal.dbs_bps = config.dbs_bps;
    proposal.claimant = None;
    proposal.claim_bond = 0;
    proposal.upload_deadline = 0;
    proposal.claimant_failures = 0;
    proposal.banned = Vec::new();
    proposal.build_deadline = 0;
    proposal.payment_asset = Pubkey::default();
    proposal.pre_sponsor_amount = 0;
    proposal.pre_sponsor_price_per_token = 0;
    proposal.metadata = metadata;
    proposal.content_uri = String::new();
    proposal.bump = bump;
}

// ============================ propose ============================

/// Open a proposal to create a module. The proposer must hold the role they
/// propose under: creator, sponsor, school, or lecturer. The proposer's XCAV
/// stake is locked in the vault, returned when the proposal passes and slashed
/// when it fails.
#[derive(Accounts)]
#[instruction(role: Role, region: u16)]
pub struct CreateModuleProposal<'info> {
    #[account(mut)]
    pub proposer: Signer<'info>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = proposer,
    )]
    pub proposer_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The proposer's role, of whichever kind they're proposing under.
    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            proposer.key().as_ref(),
            &[role.seed_byte()],
        ],
        bump = proposer_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub proposer_role: Box<Account<'info, RoleAccount>>,

    /// The region the module is scoped to; loading it proves it exists.
    #[account(
        seeds = [education_regions::REGION_SEED, &region.to_le_bytes()],
        bump = region_account.bump,
        seeds::program = education_regions::ID,
    )]
    pub region_account: Box<Account<'info, Region>>,

    #[account(
        init,
        payer = proposer,
        space = 8 + ModuleProposal::INIT_SPACE,
        seeds = [MODULE_PROPOSAL_SEED, config.next_proposal_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn create_module_proposal_handler(
    ctx: Context<CreateModuleProposal>,
    role: Role,
    region: u16,
    module_amount: u64,
    metadata: String,
) -> Result<()> {
    require!(is_proposer_role(role), EducationError::InvalidProposalRole);
    require!(module_amount > 0, EducationError::AmountCannotBeZero);
    require!(
        module_amount <= ctx.accounts.config.max_module_tokens,
        EducationError::TooManyTokens
    );
    require!(
        metadata.len() <= ModuleProposal::METADATA_MAX_LEN,
        EducationError::InvalidConfig
    );

    let now = Clock::get()?.unix_timestamp;
    let proposal_id = ctx.accounts.config.next_proposal_id;
    let deposit = ctx.accounts.config.proposal_deposit;

    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.proposer_xcav.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.proposer.to_account_info(),
        deposit,
        ctx.accounts.xcav_mint.decimals,
    )?;

    init_proposal_common(
        &mut ctx.accounts.proposal,
        &ctx.accounts.config,
        proposal_id,
        ctx.accounts.proposer.key(),
        role,
        region,
        module_amount,
        metadata,
        now,
        ctx.bumps.proposal,
    );

    ctx.accounts.config.next_proposal_id =
        proposal_id.checked_add(1).ok_or(EducationError::Overflow)?;

    emit!(ModuleProposalOpened {
        proposal_id,
        proposer: ctx.accounts.proposer.key(),
        role,
        region,
        expiry: ctx.accounts.proposal.expiry,
        pre_sponsored: false,
    });
    Ok(())
}

/// Open a proposal as a sponsor and pre-fund it. On top of the XCAV stake, the
/// sponsor locks the payment for `pre_sponsor_amount` tokens up front; once the
/// module is created that lock is converted into a real sponsorship in the
/// sponsor's name.
#[derive(Accounts)]
#[instruction(region: u16)]
pub struct CreateSponsorProposal<'info> {
    #[account(mut)]
    pub proposer: Signer<'info>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = proposer,
    )]
    pub proposer_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            proposer.key().as_ref(),
            &[Role::ModuleSponsor.seed_byte()],
        ],
        bump = sponsor_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub sponsor_role: Box<Account<'info, RoleAccount>>,

    #[account(
        seeds = [education_regions::REGION_SEED, &region.to_le_bytes()],
        bump = region_account.bump,
        seeds::program = education_regions::ID,
    )]
    pub region_account: Box<Account<'info, Region>>,

    /// The stablecoin the sponsor pre-funds in; must be an accepted asset.
    pub payment_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = payment_mint,
        token::authority = proposer,
    )]
    pub proposer_payment: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        init,
        payer = proposer,
        space = 8 + ModuleProposal::INIT_SPACE,
        seeds = [MODULE_PROPOSAL_SEED, config.next_proposal_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    /// Holds the pre-sponsorship payment until the module is created or the
    /// proposal is rejected.
    #[account(
        init,
        payer = proposer,
        seeds = [PROPOSAL_ESCROW_SEED, config.next_proposal_id.to_le_bytes().as_ref()],
        bump,
        token::mint = payment_mint,
        token::authority = config,
        token::token_program = token_program,
    )]
    pub proposal_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn create_sponsor_proposal_handler(
    ctx: Context<CreateSponsorProposal>,
    region: u16,
    module_amount: u64,
    metadata: String,
) -> Result<()> {
    require!(module_amount > 0, EducationError::AmountCannotBeZero);
    require!(
        module_amount <= ctx.accounts.config.max_module_tokens,
        EducationError::TooManyTokens
    );
    require!(
        metadata.len() <= ModuleProposal::METADATA_MAX_LEN,
        EducationError::InvalidConfig
    );
    require!(
        ctx.accounts
            .config
            .accepts(&ctx.accounts.payment_mint.key()),
        EducationError::PaymentAssetNotSupported
    );

    let pre_sponsor_amount = ctx.accounts.config.pre_sponsor_amount;
    require!(pre_sponsor_amount > 0, EducationError::InvalidConfig);
    require!(
        pre_sponsor_amount <= module_amount,
        EducationError::NotEnoughTokenAvailable
    );

    let now = Clock::get()?.unix_timestamp;
    let proposal_id = ctx.accounts.config.next_proposal_id;
    let deposit = ctx.accounts.config.proposal_deposit;
    let decimals = ctx.accounts.payment_mint.decimals;

    // Lock the proposer's XCAV stake.
    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.proposer_xcav.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.proposer.to_account_info(),
        deposit,
        ctx.accounts.xcav_mint.decimals,
    )?;

    // Lock the pre-sponsorship payment, priced on the snapshotted terms.
    let per_token = price_per_token(
        ctx.accounts.config.module_price,
        decimals,
        ctx.accounts.config.content_creator_bps,
        ctx.accounts.config.regional_operator_bps,
        ctx.accounts.config.protocol_bps,
        ctx.accounts.config.dbs_bps,
    )?;
    let pre_sponsor_total = per_token
        .checked_mul(pre_sponsor_amount)
        .ok_or(EducationError::Overflow)?;
    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.proposer_payment.to_account_info(),
        &ctx.accounts.payment_mint.to_account_info(),
        &ctx.accounts.proposal_escrow.to_account_info(),
        &ctx.accounts.proposer.to_account_info(),
        pre_sponsor_total,
        decimals,
    )?;

    init_proposal_common(
        &mut ctx.accounts.proposal,
        &ctx.accounts.config,
        proposal_id,
        ctx.accounts.proposer.key(),
        Role::ModuleSponsor,
        region,
        module_amount,
        metadata,
        now,
        ctx.bumps.proposal,
    );
    let proposal = &mut ctx.accounts.proposal;
    proposal.payment_asset = ctx.accounts.payment_mint.key();
    proposal.pre_sponsor_amount = pre_sponsor_amount;
    proposal.pre_sponsor_price_per_token = per_token;

    ctx.accounts.config.next_proposal_id =
        proposal_id.checked_add(1).ok_or(EducationError::Overflow)?;

    emit!(ModuleProposalOpened {
        proposal_id,
        proposer: ctx.accounts.proposer.key(),
        role: Role::ModuleSponsor,
        region,
        expiry: proposal.expiry,
        pre_sponsored: true,
    });
    Ok(())
}

// ============================ vote ============================

/// Vote on an open proposal. Anyone may vote, no role or KYC required. The
/// voting power is locked as XCAV and returned when the voter unlocks after the
/// proposal closes. Voting again replaces the prior vote.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct VoteOnProposal<'info> {
    #[account(mut)]
    pub voter: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = voter,
    )]
    pub voter_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

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
        seeds = [MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    // A per-voter record keyed by the voter, fully overwritten each time, so an
    // upsert is safe: nobody else can target it and there's no state to reset.
    #[account(
        init_if_needed,
        payer = voter,
        space = 8 + ModuleProposalVote::INIT_SPACE,
        seeds = [PROPOSAL_VOTE_SEED, &proposal_id.to_le_bytes(), voter.key().as_ref()],
        bump,
    )]
    pub vote_record: Box<Account<'info, ModuleProposalVote>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn vote_on_proposal_handler(
    ctx: Context<VoteOnProposal>,
    proposal_id: u64,
    vote: ModuleVote,
    amount: u64,
) -> Result<()> {
    require!(
        amount >= ctx.accounts.config.minimum_voting_amount,
        EducationError::BelowMinimumVotingAmount
    );
    require!(
        ctx.accounts.proposal.status == ProposalStatus::Voting,
        EducationError::InvalidProposalState
    );
    let now = Clock::get()?.unix_timestamp;
    require!(
        now < ctx.accounts.proposal.expiry,
        EducationError::ProposalExpired
    );

    let decimals = ctx.accounts.xcav_mint.decimals;
    let config_bump = ctx.accounts.config.bump;

    // Undo and refund a prior vote before locking the new one.
    if ctx.accounts.vote_record.voter != Pubkey::default() {
        let old = ctx.accounts.vote_record.power;
        let old_vote = ctx.accounts.vote_record.vote;
        sub_power(&mut ctx.accounts.proposal, old_vote, old);
        if old > 0 {
            release_from_vault(
                &ctx.accounts.token_program.to_account_info(),
                &ctx.accounts.vault.to_account_info(),
                &ctx.accounts.xcav_mint.to_account_info(),
                &ctx.accounts.voter_xcav.to_account_info(),
                &ctx.accounts.config.to_account_info(),
                config_bump,
                old,
                decimals,
            )?;
        }
    }

    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.voter_xcav.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.voter.to_account_info(),
        amount,
        decimals,
    )?;
    add_power(&mut ctx.accounts.proposal, vote, amount)?;

    let expiry = ctx.accounts.proposal.expiry;
    let vote_record = &mut ctx.accounts.vote_record;
    vote_record.proposal_id = proposal_id;
    vote_record.voter = ctx.accounts.voter.key();
    vote_record.vote = vote;
    vote_record.power = amount;
    vote_record.expiry = expiry;
    vote_record.bump = ctx.bumps.vote_record;

    emit!(ProposalVoted {
        proposal_id,
        voter: vote_record.voter,
        vote,
        power: amount,
    });
    Ok(())
}

fn add_power(p: &mut ModuleProposal, vote: ModuleVote, amount: u64) -> Result<()> {
    let slot = match vote {
        ModuleVote::Yes => &mut p.yes_power,
        ModuleVote::No => &mut p.no_power,
        ModuleVote::Abstain => &mut p.abstain_power,
    };
    *slot = slot.checked_add(amount).ok_or(EducationError::Overflow)?;
    Ok(())
}

fn sub_power(p: &mut ModuleProposal, vote: ModuleVote, amount: u64) {
    let slot = match vote {
        ModuleVote::Yes => &mut p.yes_power,
        ModuleVote::No => &mut p.no_power,
        ModuleVote::Abstain => &mut p.abstain_power,
    };
    *slot = slot.saturating_sub(amount);
}

// ============================ finalize ============================

/// Finalize a proposal once voting closes. Permissionless. If it passed
/// (threshold + quorum) the proposal becomes claimable and the proposer's stake
/// is returned; otherwise the stake is slashed to the treasury and the proposal
/// is rejected. A sponsor's pre-sponsorship lock is untouched here and reclaimed
/// separately if the proposal was rejected.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct FinalizeProposal<'info> {
    #[account(mut)]
    pub cranker: Signer<'info>,

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
        seeds = [MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        bump = proposal.bump,
        has_one = proposer @ EducationError::NoPermission,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    /// CHECK: the proposal's proposer (enforced by `has_one`).
    pub proposer: UncheckedAccount<'info>,

    /// The proposer's XCAV account; receives the returned stake if it passed.
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = proposer,
    )]
    pub proposer_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The treasury; receives the slashed stake if it failed.
    #[account(mut, address = config.treasury @ EducationError::InvalidTreasury)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn finalize_proposal_handler(ctx: Context<FinalizeProposal>, proposal_id: u64) -> Result<()> {
    require!(
        ctx.accounts.proposal.status == ProposalStatus::Voting,
        EducationError::InvalidProposalState
    );
    let now = Clock::get()?.unix_timestamp;
    require!(
        now >= ctx.accounts.proposal.expiry,
        EducationError::VotingStillOngoing
    );

    let p = &ctx.accounts.proposal;
    let yes = p.yes_power;
    let no = p.no_power;
    let total = yes
        .checked_add(no)
        .and_then(|v| v.checked_add(p.abstain_power))
        .ok_or(EducationError::Overflow)?;
    let approval_base = yes.checked_add(no).ok_or(EducationError::Overflow)?;
    let threshold_bps = ctx.accounts.config.threshold_bps as u128;
    let meets_threshold = total != 0
        && (yes as u128).saturating_mul(10_000)
            >= (approval_base as u128).saturating_mul(threshold_bps);
    let meets_quorum = total >= ctx.accounts.config.quorum;
    let deposit = p.deposit;
    let decimals = ctx.accounts.xcav_mint.decimals;
    let config_bump = ctx.accounts.config.bump;

    let passed = meets_threshold && meets_quorum;
    let destination = if passed {
        ctx.accounts.proposer_xcav.to_account_info()
    } else {
        ctx.accounts.treasury.to_account_info()
    };
    release_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &destination,
        &ctx.accounts.config.to_account_info(),
        config_bump,
        deposit,
        decimals,
    )?;

    ctx.accounts.proposal.status = if passed {
        ProposalStatus::Claimable
    } else {
        ProposalStatus::Rejected
    };
    // A passed proposal must be built within the claim period; once that passes
    // anyone can expire it so its rent and any pre-sponsorship are recoverable.
    if passed {
        ctx.accounts.proposal.build_deadline =
            now.saturating_add(ctx.accounts.config.claim_period);
    }

    emit!(ProposalFinalized {
        proposal_id,
        passed,
        yes_power: yes,
        no_power: no,
    });
    Ok(())
}

// ============================ claim ============================

/// Claim a passed proposal to reserve the right to build it, locking a bond so
/// the reservation can't be held for free. The caller must be a ModuleCreator.
/// A creator-opened proposal can only be built by its proposer;
/// otherwise any creator may claim it, unless they've already been banned from
/// it for failing review twice. While a claimant has a retry pending, only they
/// can re-claim.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct ClaimProposal<'info> {
    pub creator: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The creator's XCAV account the bond is pulled from.
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = creator,
    )]
    pub creator_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            creator.key().as_ref(),
            &[Role::ModuleCreator.seed_byte()],
        ],
        bump = creator_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub creator_role: Box<Account<'info, RoleAccount>>,

    #[account(
        mut,
        seeds = [MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn claim_proposal_handler(ctx: Context<ClaimProposal>, _proposal_id: u64) -> Result<()> {
    require!(
        ctx.accounts.proposal.status == ProposalStatus::Claimable,
        EducationError::InvalidProposalState
    );

    let creator = ctx.accounts.creator.key();
    let bond = ctx.accounts.config.module_deposit;
    let upload_period = ctx.accounts.config.upload_period;
    let now = Clock::get()?.unix_timestamp;

    {
        let proposal = &ctx.accounts.proposal;
        // A creator-opened proposal is reserved for its proposer.
        if proposal.proposer_role == Role::ModuleCreator {
            require!(proposal.proposer == creator, EducationError::NotProposalCreator);
        }
        match proposal.claimant {
            // A retry is pending: only the same claimant may pick it back up.
            Some(pending) => require!(pending == creator, EducationError::NotProposalCreator),
            // A fresh claim: the creator must not have been banned from it.
            None => require!(!proposal.banned.contains(&creator), EducationError::CreatorBanned),
        }
    }

    // Lock the build bond before recording the reservation.
    lock_module_deposit(
        &ctx.accounts.token_program,
        &ctx.accounts.creator_xcav,
        &ctx.accounts.xcav_mint,
        &ctx.accounts.vault,
        &ctx.accounts.creator,
        bond,
    )?;

    let proposal = &mut ctx.accounts.proposal;
    proposal.claimant = Some(creator);
    proposal.claim_bond = bond;
    proposal.upload_deadline = now.saturating_add(upload_period);
    proposal.status = ProposalStatus::Claimed;

    emit!(ProposalClaimed {
        proposal_id: proposal.proposal_id,
        creator,
    });
    Ok(())
}

// ============================ upload ============================

/// Upload the content for a reserved proposal and send it for review. Only the
/// reserving claimant may upload. The deposit keeps riding with the proposal:
/// it's returned only when the finished module is removed, and forfeit if the
/// build fails or stalls, so there's no token movement here.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct UploadProposal<'info> {
    pub claimant: Signer<'info>,

    #[account(
        mut,
        seeds = [MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,
}

pub fn upload_proposal_handler(
    ctx: Context<UploadProposal>,
    _proposal_id: u64,
    content_uri: String,
) -> Result<()> {
    require!(
        content_uri.len() <= ModuleProposal::URI_MAX_LEN,
        EducationError::InvalidConfig
    );
    require!(
        ctx.accounts.proposal.status == ProposalStatus::Claimed,
        EducationError::InvalidProposalState
    );
    require!(
        ctx.accounts.proposal.claimant == Some(ctx.accounts.claimant.key()),
        EducationError::NotProposalCreator
    );

    let proposal = &mut ctx.accounts.proposal;
    // No longer waiting on an upload; the review is what moves it next.
    proposal.upload_deadline = 0;
    proposal.content_uri = content_uri;
    proposal.status = ProposalStatus::UnderReview;

    emit!(ProposalUploaded {
        proposal_id: proposal.proposal_id,
        claimant: ctx.accounts.claimant.key(),
    });
    Ok(())
}

/// Release a reservation whose upload deadline passed, slashing the bond to the
/// treasury and reopening the proposal so another creator can build it.
/// Permissionless.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct ReleaseClaim<'info> {
    pub cranker: Signer<'info>,

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

    /// The treasury; receives the slashed bond.
    #[account(mut, address = config.treasury @ EducationError::InvalidTreasury)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn release_claim_handler(ctx: Context<ReleaseClaim>, proposal_id: u64) -> Result<()> {
    require!(
        ctx.accounts.proposal.status == ProposalStatus::Claimed,
        EducationError::InvalidProposalState
    );
    let now = Clock::get()?.unix_timestamp;
    require!(
        now >= ctx.accounts.proposal.upload_deadline,
        EducationError::UploadDeadlineNotReached
    );

    let bond = ctx.accounts.proposal.claim_bond;
    let config_bump = ctx.accounts.config.bump;
    let decimals = ctx.accounts.xcav_mint.decimals;

    // Slash the abandoned reservation's bond to the treasury.
    if bond > 0 {
        release_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &ctx.accounts.treasury.to_account_info(),
            &ctx.accounts.config.to_account_info(),
            config_bump,
            bond,
            decimals,
        )?;
    }

    let proposal = &mut ctx.accounts.proposal;
    proposal.claimant = None;
    proposal.claim_bond = 0;
    proposal.upload_deadline = 0;
    proposal.status = ProposalStatus::Claimable;

    emit!(ProposalClaimReleased {
        proposal_id,
        bond,
    });
    Ok(())
}

// ============================ review ============================

/// Record the AI agent's review of a claimed proposal. ModuleAIAgent-only. A
/// pass moves the proposal to approved so the claimant can mint it. A first fail
/// sends it back for the same creator to re-upload, with their deposit still
/// riding. On a second fail that deposit is slashed to the treasury and the
/// creator is banned and the proposal reopens to anyone else, or is rejected
/// outright once the ban list is full.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct ReviewProposal<'info> {
    pub agent: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The vault the failed claimant's deposit is slashed from.
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The treasury; receives a slashed deposit on a final failure.
    #[account(mut, address = config.treasury @ EducationError::InvalidTreasury)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

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
        seeds = [MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn review_proposal_handler(
    ctx: Context<ReviewProposal>,
    proposal_id: u64,
    passed: bool,
) -> Result<()> {
    require!(
        ctx.accounts.proposal.status == ProposalStatus::UnderReview,
        EducationError::InvalidProposalState
    );
    let claimant = ctx
        .accounts
        .proposal
        .claimant
        .ok_or(EducationError::NotProposalCreator)?;

    let mut banned = false;
    if passed {
        ctx.accounts.proposal.status = ProposalStatus::Approved;
    } else if ctx.accounts.proposal.claimant_failures.saturating_add(1) < 2 {
        // First failure: the same claimant keeps the slot and the riding deposit,
        // and gets a fresh window to re-upload.
        let now = Clock::get()?.unix_timestamp;
        let upload_period = ctx.accounts.config.upload_period;
        let proposal = &mut ctx.accounts.proposal;
        proposal.claimant_failures = proposal.claimant_failures.saturating_add(1);
        proposal.upload_deadline = now.saturating_add(upload_period);
        proposal.status = ProposalStatus::Claimed;
    } else {
        // Second failure: slash the deposit to the treasury and drop the
        // claimant. Ban them and reopen to others if there's room; once the ban
        // list is full the proposal has burned through too many creators, so
        // reject it instead of stalling.
        let bond = ctx.accounts.proposal.claim_bond;
        let config_bump = ctx.accounts.config.bump;
        let decimals = ctx.accounts.xcav_mint.decimals;
        if bond > 0 {
            release_from_vault(
                &ctx.accounts.token_program.to_account_info(),
                &ctx.accounts.vault.to_account_info(),
                &ctx.accounts.xcav_mint.to_account_info(),
                &ctx.accounts.treasury.to_account_info(),
                &ctx.accounts.config.to_account_info(),
                config_bump,
                bond,
                decimals,
            )?;
        }
        let proposal = &mut ctx.accounts.proposal;
        proposal.claim_bond = 0;
        proposal.claimant = None;
        proposal.claimant_failures = 0;
        proposal.upload_deadline = 0;
        if proposal.banned.len() < ModuleProposal::MAX_BANNED {
            proposal.banned.push(claimant);
            proposal.status = ProposalStatus::Claimable;
            banned = true;
        } else {
            proposal.status = ProposalStatus::Rejected;
        }
    }

    emit!(ProposalReviewed {
        proposal_id,
        claimant,
        passed,
        banned,
    });
    Ok(())
}

// ============================ mint ============================

/// Mint the module for an approved proposal that carries no pre-sponsorship. The
/// deposit locked at claim time becomes the module's deposit, the full supply is
/// minted into the module vault, and the proposal record is closed back to the
/// proposer.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct MintProposedModule<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        close = proposer,
        seeds = [MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        bump = proposal.bump,
        has_one = proposer @ EducationError::NoPermission,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    /// CHECK: the proposer (enforced by `has_one`); receives the proposal's rent.
    #[account(mut)]
    pub proposer: UncheckedAccount<'info>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            creator.key().as_ref(),
            &[Role::ModuleCreator.seed_byte()],
        ],
        bump = creator_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub creator_role: Box<Account<'info, RoleAccount>>,

    #[account(
        init,
        payer = creator,
        space = 8 + Module::INIT_SPACE,
        seeds = [MODULE_SEED, config.next_module_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub module: Box<Account<'info, Module>>,

    #[account(
        init,
        payer = creator,
        seeds = [MODULE_MINT_SEED, config.next_module_id.to_le_bytes().as_ref()],
        bump,
        mint::decimals = 0,
        mint::authority = config,
        mint::token_program = token_program,
    )]
    pub module_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        init,
        payer = creator,
        seeds = [MODULE_VAULT_SEED, config.next_module_id.to_le_bytes().as_ref()],
        bump,
        token::mint = module_mint,
        token::authority = config,
        token::token_program = token_program,
    )]
    pub module_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn mint_proposed_module_handler(
    ctx: Context<MintProposedModule>,
    _proposal_id: u64,
) -> Result<()> {
    require!(
        ctx.accounts.proposal.status == ProposalStatus::Approved,
        EducationError::InvalidProposalState
    );
    require!(
        ctx.accounts.proposal.claimant == Some(ctx.accounts.creator.key()),
        EducationError::NotProposalCreator
    );
    require!(
        ctx.accounts.proposal.pre_sponsor_amount == 0,
        EducationError::InvalidProposalState
    );

    let now = Clock::get()?.unix_timestamp;
    let module_id = ctx.accounts.config.next_module_id;
    // The deposit was already locked at claim time and rides on the proposal.
    let deposit = ctx.accounts.proposal.claim_bond;
    let config_bump = ctx.accounts.config.bump;
    let module_amount = ctx.accounts.proposal.module_amount;

    mint_full_supply(
        &ctx.accounts.token_program,
        &ctx.accounts.module_mint,
        &ctx.accounts.module_vault,
        &ctx.accounts.config.to_account_info(),
        config_bump,
        module_amount,
    )?;

    write_module(
        &mut ctx.accounts.module,
        &proposal_terms(
            &ctx.accounts.proposal,
            module_id,
            ctx.accounts.creator.key(),
            ctx.accounts.module_mint.key(),
            deposit,
            0,
            now,
            ctx.bumps.module,
        ),
    );
    ctx.accounts.config.next_module_id =
        module_id.checked_add(1).ok_or(EducationError::Overflow)?;
    ctx.accounts.proposal.status = ProposalStatus::Created;

    emit!(ProposedModuleMinted {
        proposal_id: ctx.accounts.proposal.proposal_id,
        module_id,
        creator: ctx.accounts.creator.key(),
        token_amount: module_amount,
    });
    Ok(())
}

/// Mint the module for an approved sponsor proposal, converting the locked
/// pre-sponsorship into a real sponsorship in the proposer's name: the funds
/// move into a fresh sponsorship escrow and that many tokens move to the school
/// allocation, ready to be booked.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct MintSponsoredModule<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        close = proposer,
        seeds = [MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        bump = proposal.bump,
        has_one = proposer @ EducationError::NoPermission,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    /// CHECK: the proposer / sponsor (enforced by `has_one`); receives the
    /// proposal and escrow rent, and is recorded as the sponsorship's owner.
    #[account(mut)]
    pub proposer: UncheckedAccount<'info>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            creator.key().as_ref(),
            &[Role::ModuleCreator.seed_byte()],
        ],
        bump = creator_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub creator_role: Box<Account<'info, RoleAccount>>,

    #[account(
        init,
        payer = creator,
        space = 8 + Module::INIT_SPACE,
        seeds = [MODULE_SEED, config.next_module_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub module: Box<Account<'info, Module>>,

    #[account(
        init,
        payer = creator,
        seeds = [MODULE_MINT_SEED, config.next_module_id.to_le_bytes().as_ref()],
        bump,
        mint::decimals = 0,
        mint::authority = config,
        mint::token_program = token_program,
    )]
    pub module_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        init,
        payer = creator,
        seeds = [MODULE_VAULT_SEED, config.next_module_id.to_le_bytes().as_ref()],
        bump,
        token::mint = module_mint,
        token::authority = config,
        token::token_program = token_program,
    )]
    pub module_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The sponsor's pre-funded payment, in the proposal's asset.
    #[account(address = proposal.payment_asset @ EducationError::InvalidMint)]
    pub payment_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [PROPOSAL_ESCROW_SEED, &proposal_id.to_le_bytes()],
        bump,
        token::mint = payment_mint,
        token::authority = config,
    )]
    pub proposal_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        init,
        payer = creator,
        space = 8 + Sponsorship::INIT_SPACE,
        seeds = [SPONSORSHIP_SEED, config.next_module_id.to_le_bytes().as_ref(), config.next_sponsor_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub sponsorship: Box<Account<'info, Sponsorship>>,

    #[account(
        init,
        payer = creator,
        seeds = [SPONSOR_ESCROW_SEED, config.next_module_id.to_le_bytes().as_ref(), config.next_sponsor_id.to_le_bytes().as_ref()],
        bump,
        token::mint = payment_mint,
        token::authority = config,
        token::token_program = token_program,
    )]
    pub sponsor_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn mint_sponsored_module_handler(
    ctx: Context<MintSponsoredModule>,
    _proposal_id: u64,
) -> Result<()> {
    require!(
        ctx.accounts.proposal.status == ProposalStatus::Approved,
        EducationError::InvalidProposalState
    );
    require!(
        ctx.accounts.proposal.claimant == Some(ctx.accounts.creator.key()),
        EducationError::NotProposalCreator
    );
    require!(
        ctx.accounts.proposal.pre_sponsor_amount > 0,
        EducationError::InvalidProposalState
    );

    let now = Clock::get()?.unix_timestamp;
    let module_id = ctx.accounts.config.next_module_id;
    let sponsor_id = ctx.accounts.config.next_sponsor_id;
    // The deposit was already locked at claim time and rides on the proposal.
    let deposit = ctx.accounts.proposal.claim_bond;
    let config_bump = ctx.accounts.config.bump;
    let module_amount = ctx.accounts.proposal.module_amount;
    let pre_sponsor_amount = ctx.accounts.proposal.pre_sponsor_amount;
    let per_token = ctx.accounts.proposal.pre_sponsor_price_per_token;
    let locked = ctx
        .accounts
        .proposal
        .pre_sponsor_locked()
        .ok_or(EducationError::Overflow)?;
    let decimals = ctx.accounts.payment_mint.decimals;

    mint_full_supply(
        &ctx.accounts.token_program,
        &ctx.accounts.module_mint,
        &ctx.accounts.module_vault,
        &ctx.accounts.config.to_account_info(),
        config_bump,
        module_amount,
    )?;

    // Move the pre-funded payment into the sponsorship escrow.
    release_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.proposal_escrow.to_account_info(),
        &ctx.accounts.payment_mint.to_account_info(),
        &ctx.accounts.sponsor_escrow.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        config_bump,
        locked,
        decimals,
    )?;
    // The emptied pre-sponsor escrow can be closed back to the sponsor.
    close_vault_account(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.proposal_escrow.to_account_info(),
        &ctx.accounts.proposer.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        config_bump,
    )?;

    // Mint the module, then immediately move the pre-sponsored tokens from the
    // creator's allocation to the school allocation.
    write_module(
        &mut ctx.accounts.module,
        &proposal_terms(
            &ctx.accounts.proposal,
            module_id,
            ctx.accounts.creator.key(),
            ctx.accounts.module_mint.key(),
            deposit,
            pre_sponsor_amount,
            now,
            ctx.bumps.module,
        ),
    );

    let sponsorship = &mut ctx.accounts.sponsorship;
    sponsorship.module_id = module_id;
    sponsorship.sponsor_id = sponsor_id;
    sponsorship.sponsor = ctx.accounts.proposer.key();
    sponsorship.payment_asset = ctx.accounts.payment_mint.key();
    sponsorship.amount = pre_sponsor_amount;
    sponsorship.active_bookings = 0;
    sponsorship.price_per_token = per_token;
    sponsorship.sponsored_at = now;
    sponsorship.bump = ctx.bumps.sponsorship;

    ctx.accounts.config.next_module_id =
        module_id.checked_add(1).ok_or(EducationError::Overflow)?;
    ctx.accounts.config.next_sponsor_id =
        sponsor_id.checked_add(1).ok_or(EducationError::Overflow)?;
    ctx.accounts.proposal.status = ProposalStatus::Created;

    emit!(ProposedModuleMinted {
        proposal_id: ctx.accounts.proposal.proposal_id,
        module_id,
        creator: ctx.accounts.creator.key(),
        token_amount: module_amount,
    });
    Ok(())
}

/// Gather a proposal's snapshotted terms into `ModuleTerms` for minting. The
/// creator is the claimant who actually built it (not necessarily the proposer),
/// since that's who earns the content-creator share.
#[allow(clippy::too_many_arguments)]
fn proposal_terms<'a>(
    proposal: &'a ModuleProposal,
    module_id: u64,
    creator: Pubkey,
    mint: Pubkey,
    deposit: u64,
    school_allocated: u64,
    created_at: i64,
    bump: u8,
) -> ModuleTerms<'a> {
    ModuleTerms {
        module_id,
        creator,
        region: proposal.region,
        mint,
        deposit,
        module_amount: proposal.module_amount,
        school_allocated,
        price: proposal.price,
        content_creator_bps: proposal.content_creator_bps,
        regional_operator_bps: proposal.regional_operator_bps,
        protocol_bps: proposal.protocol_bps,
        dbs_bps: proposal.dbs_bps,
        created_at,
        metadata: &proposal.metadata,
        bump,
    }
}

// ============================ cleanup ============================

/// Reclaim the XCAV a voter locked, once the proposal's voting window ended. The
/// stored `expiry` means this never needs the proposal account.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct UnlockProposalVote<'info> {
    #[account(mut)]
    pub voter: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = voter,
    )]
    pub voter_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

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
        seeds = [PROPOSAL_VOTE_SEED, &proposal_id.to_le_bytes(), voter.key().as_ref()],
        bump = vote_record.bump,
    )]
    pub vote_record: Box<Account<'info, ModuleProposalVote>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn unlock_proposal_vote_handler(
    ctx: Context<UnlockProposalVote>,
    _proposal_id: u64,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(
        now >= ctx.accounts.vote_record.expiry,
        EducationError::VotingStillOngoing
    );

    let power = ctx.accounts.vote_record.power;
    if power > 0 {
        release_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &ctx.accounts.voter_xcav.to_account_info(),
            &ctx.accounts.config.to_account_info(),
            ctx.accounts.config.bump,
            power,
            ctx.accounts.xcav_mint.decimals,
        )?;
    }

    emit!(ProposalVoteUnlocked {
        proposal_id: ctx.accounts.vote_record.proposal_id,
        voter: ctx.accounts.voter.key(),
        power,
    });
    Ok(())
}

/// Expire a passed proposal that wasn't built in time, rejecting it so its rent
/// and any pre-sponsorship can be recovered. Permissionless. Covers every way a
/// build can stall: nobody claims it, the claimant abandons it, the review never
/// lands, or the claimant never mints after it's approved. Any deposit still
/// riding on the proposal is slashed to the treasury.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct ExpireProposal<'info> {
    pub cranker: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The vault a riding deposit is slashed from.
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The treasury; receives a slashed deposit.
    #[account(mut, address = config.treasury @ EducationError::InvalidTreasury)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn expire_proposal_handler(ctx: Context<ExpireProposal>, proposal_id: u64) -> Result<()> {
    // Only a passed-but-unbuilt proposal can be expired; voting, created, and
    // already-rejected proposals are left alone.
    require!(
        matches!(
            ctx.accounts.proposal.status,
            ProposalStatus::Claimable
                | ProposalStatus::Claimed
                | ProposalStatus::UnderReview
                | ProposalStatus::Approved
        ),
        EducationError::InvalidProposalState
    );
    let now = Clock::get()?.unix_timestamp;
    require!(
        now >= ctx.accounts.proposal.build_deadline,
        EducationError::BuildDeadlineNotReached
    );

    // Slash whatever deposit a claimant still has riding on the build.
    let bond = ctx.accounts.proposal.claim_bond;
    let config_bump = ctx.accounts.config.bump;
    let decimals = ctx.accounts.xcav_mint.decimals;
    if bond > 0 {
        release_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &ctx.accounts.treasury.to_account_info(),
            &ctx.accounts.config.to_account_info(),
            config_bump,
            bond,
            decimals,
        )?;
    }

    let proposal = &mut ctx.accounts.proposal;
    proposal.claim_bond = 0;
    proposal.claimant = None;
    proposal.upload_deadline = 0;
    proposal.status = ProposalStatus::Rejected;

    emit!(ProposalExpired { proposal_id });
    Ok(())
}

/// Refund a rejected sponsor proposal's pre-sponsorship and close it out. The
/// sponsor gets back the locked payment plus the escrow and proposal rent.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct ReclaimPreSponsor<'info> {
    #[account(mut)]
    pub sponsor: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        close = sponsor,
        seeds = [MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        bump = proposal.bump,
        constraint = proposal.proposer == sponsor.key() @ EducationError::NoPermission,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    #[account(address = proposal.payment_asset @ EducationError::InvalidMint)]
    pub payment_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = payment_mint,
        token::authority = sponsor,
    )]
    pub sponsor_payment: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [PROPOSAL_ESCROW_SEED, &proposal_id.to_le_bytes()],
        bump,
        token::mint = payment_mint,
        token::authority = config,
    )]
    pub proposal_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn reclaim_pre_sponsor_handler(
    ctx: Context<ReclaimPreSponsor>,
    _proposal_id: u64,
) -> Result<()> {
    require!(
        ctx.accounts.proposal.status == ProposalStatus::Rejected,
        EducationError::InvalidProposalState
    );
    require!(
        ctx.accounts.proposal.pre_sponsor_amount > 0,
        EducationError::InvalidProposalState
    );

    let locked = ctx
        .accounts
        .proposal
        .pre_sponsor_locked()
        .ok_or(EducationError::Overflow)?;
    let config_bump = ctx.accounts.config.bump;

    release_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.proposal_escrow.to_account_info(),
        &ctx.accounts.payment_mint.to_account_info(),
        &ctx.accounts.sponsor_payment.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        config_bump,
        locked,
        ctx.accounts.payment_mint.decimals,
    )?;
    close_vault_account(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.proposal_escrow.to_account_info(),
        &ctx.accounts.sponsor.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        config_bump,
    )?;

    emit!(PreSponsorReclaimed {
        proposal_id: ctx.accounts.proposal.proposal_id,
        sponsor: ctx.accounts.sponsor.key(),
        refunded: locked,
    });
    Ok(())
}

/// Close a rejected proposal that has nothing left to settle, returning its rent
/// to the proposer. Permissionless. Sponsor proposals must reclaim their
/// pre-sponsorship first.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct ClearProposal<'info> {
    #[account(mut)]
    pub cranker: Signer<'info>,

    #[account(
        mut,
        close = proposer,
        seeds = [MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        bump = proposal.bump,
        has_one = proposer @ EducationError::NoPermission,
    )]
    pub proposal: Box<Account<'info, ModuleProposal>>,

    /// CHECK: the proposer (enforced by `has_one`); receives the rent.
    #[account(mut)]
    pub proposer: UncheckedAccount<'info>,
}

pub fn clear_proposal_handler(ctx: Context<ClearProposal>, proposal_id: u64) -> Result<()> {
    require!(
        ctx.accounts.proposal.status == ProposalStatus::Rejected,
        EducationError::ProposalNotClearable
    );
    // A pre-sponsorship still locked must be reclaimed before the record goes.
    require!(
        ctx.accounts.proposal.pre_sponsor_amount == 0,
        EducationError::ProposalNotClearable
    );

    emit!(ProposalCleared { proposal_id });
    Ok(())
}

// ============================ events ============================

#[event]
pub struct ModuleProposalOpened {
    pub proposal_id: u64,
    pub proposer: Pubkey,
    pub role: Role,
    pub region: u16,
    pub expiry: i64,
    pub pre_sponsored: bool,
}

#[event]
pub struct ProposalVoted {
    pub proposal_id: u64,
    pub voter: Pubkey,
    pub vote: ModuleVote,
    pub power: u64,
}

#[event]
pub struct ProposalFinalized {
    pub proposal_id: u64,
    pub passed: bool,
    pub yes_power: u64,
    pub no_power: u64,
}

#[event]
pub struct ProposalClaimed {
    pub proposal_id: u64,
    pub creator: Pubkey,
}

#[event]
pub struct ProposalUploaded {
    pub proposal_id: u64,
    pub claimant: Pubkey,
}

#[event]
pub struct ProposalClaimReleased {
    pub proposal_id: u64,
    pub bond: u64,
}

#[event]
pub struct ProposalReviewed {
    pub proposal_id: u64,
    pub claimant: Pubkey,
    pub passed: bool,
    pub banned: bool,
}

#[event]
pub struct ProposedModuleMinted {
    pub proposal_id: u64,
    pub module_id: u64,
    pub creator: Pubkey,
    pub token_amount: u64,
}

#[event]
pub struct ProposalVoteUnlocked {
    pub proposal_id: u64,
    pub voter: Pubkey,
    pub power: u64,
}

#[event]
pub struct PreSponsorReclaimed {
    pub proposal_id: u64,
    pub sponsor: Pubkey,
    pub refunded: u64,
}

#[event]
pub struct ProposalExpired {
    pub proposal_id: u64,
}

#[event]
pub struct ProposalCleared {
    pub proposal_id: u64,
}
