pub mod constants;
pub mod error;
pub mod instructions;
pub mod mint_guard;
pub mod state;
pub mod vault;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("HnXYYBRgi453sKjjKwDpbMjJZxKvUEn4KPtPgnLKGkz7");

/// Region governance: a regional operator proposes a region by bonding XCAV,
/// holders vote, and on a pass the proposer claims it and becomes the operator.
/// Seats turn over by resignation or removal, after which any operator can claim
/// the open region by bonding. Education modules are scoped to regions and pay
/// out to the region's operator.
#[program]
pub mod education_regions {
    use super::*;

    /// Initialize the singleton config with governance parameters.
    pub fn initialize_config(ctx: Context<InitializeConfig>, params: ConfigParams) -> Result<()> {
        initialize::handler(ctx, params)
    }

    /// Update the governance parameters. Authority-only.
    pub fn update_config(ctx: Context<UpdateConfig>, params: ConfigParams) -> Result<()> {
        initialize::update_config_handler(ctx, params)
    }

    /// Rotate the config authority. Current-authority-only.
    pub fn update_authority(ctx: Context<UpdateAuthority>, new_authority: Pubkey) -> Result<()> {
        initialize::update_authority_handler(ctx, new_authority)
    }

    /// Move funds out of the protocol treasury. Authority-only.
    pub fn withdraw_treasury(ctx: Context<WithdrawTreasury>, amount: u64) -> Result<()> {
        initialize::withdraw_treasury_handler(ctx, amount)
    }

    /// Propose a new region. RegionalOperator-only; bonds 0.1% of XCAV supply.
    pub fn propose_new_region(ctx: Context<ProposeNewRegion>, region_id: u16) -> Result<()> {
        propose::propose_new_region_handler(ctx, region_id)
    }

    /// Vote on an open proposal. Anyone may vote; the amount is locked.
    pub fn vote_on_region_proposal(
        ctx: Context<VoteOnRegionProposal>,
        region_id: u16,
        vote: Vote,
        amount: u64,
    ) -> Result<()> {
        vote::vote_on_region_proposal_handler(ctx, region_id, vote, amount)
    }

    /// Finalize an expired proposal (permissionless crank).
    pub fn finalize_region_proposal(
        ctx: Context<FinalizeRegionProposal>,
        region_id: u16,
    ) -> Result<()> {
        finalize::finalize_region_proposal_handler(ctx, region_id)
    }

    /// Claim a region whose proposal passed, creating it. Proposer-only.
    pub fn create_region(ctx: Context<CreateRegion>, region_id: u16) -> Result<()> {
        create::create_region_handler(ctx, region_id)
    }

    /// Claim an existing region whose seat is open, bonding 0.1% of XCAV supply.
    /// First-come; RegionalOperator-only.
    pub fn claim_open_region(ctx: Context<ClaimOpenRegion>, region_id: u16) -> Result<()> {
        create::claim_open_region_handler(ctx, region_id)
    }

    /// Reclaim locked voting XCAV after a proposal's voting window ends.
    pub fn unlock_voting_token(ctx: Context<UnlockVotingToken>, proposal_id: u64) -> Result<()> {
        cleanup::unlock_voting_token_handler(ctx, proposal_id)
    }

    /// Close a rejected/empty region state so the region can be proposed again.
    pub fn clear_region_state(ctx: Context<ClearRegionState>, region_id: u16) -> Result<()> {
        cleanup::clear_region_state_handler(ctx, region_id)
    }

    /// Propose removing a region's operator. Locks a dispute deposit.
    pub fn propose_remove_operator(
        ctx: Context<ProposeRemoveOperator>,
        region_id: u16,
    ) -> Result<()> {
        removal::propose_remove_operator_handler(ctx, region_id)
    }

    /// Vote on an open operator-removal proposal. Anyone may vote.
    pub fn vote_on_removal(
        ctx: Context<VoteOnRemoval>,
        region_id: u16,
        vote: Vote,
        amount: u64,
    ) -> Result<()> {
        removal::vote_on_removal_handler(ctx, region_id, vote, amount)
    }

    /// Finalize an expired removal proposal (permissionless crank).
    pub fn finalize_removal(ctx: Context<FinalizeRemoval>, region_id: u16) -> Result<()> {
        removal::finalize_removal_handler(ctx, region_id)
    }

    /// Reclaim locked voting tokens after a removal proposal's window ends.
    pub fn unlock_removal_vote(ctx: Context<UnlockRemovalVote>, proposal_id: u64) -> Result<()> {
        removal::unlock_removal_vote_handler(ctx, proposal_id)
    }

    /// Schedule the caller's own departure as a region's operator.
    pub fn initiate_resignation(ctx: Context<InitiateResignation>, region_id: u16) -> Result<()> {
        create::initiate_resignation_handler(ctx, region_id)
    }
}
