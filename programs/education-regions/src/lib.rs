pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;
pub mod vault;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("HnXYYBRgi453sKjjKwDpbMjJZxKvUEn4KPtPgnLKGkz7");

/// Region governance: regional operators propose regions, holders vote, and the
/// winning bidder becomes the region's operator. Education modules are scoped to
/// regions and pay out to the region's operator.
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

    /// Propose a new region. RegionalOperator-only; locks a deposit.
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

    /// Bid to operate a region whose proposal passed. RegionalOperator-only.
    pub fn bid_on_region(ctx: Context<BidOnRegion>, region_id: u16, amount: u64) -> Result<()> {
        auction::bid_on_region_handler(ctx, region_id, amount)
    }

    /// Create the region once its auction ends; callable by the winner.
    pub fn create_new_region(ctx: Context<CreateNewRegion>, region_id: u16) -> Result<()> {
        auction::create_new_region_handler(ctx, region_id)
    }

    /// Reclaim locked voting SOL after a proposal's voting window ends.
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

    /// Bid to take over a region whose operator seat is open. RegionalOperator-only.
    pub fn bid_on_replacement(
        ctx: Context<BidOnReplacement>,
        region_id: u16,
        amount: u64,
    ) -> Result<()> {
        replacement::bid_on_replacement_handler(ctx, region_id, amount)
    }

    /// Finalize a replacement auction once it ends (permissionless crank).
    pub fn finalize_replacement(ctx: Context<FinalizeReplacement>, region_id: u16) -> Result<()> {
        replacement::finalize_replacement_handler(ctx, region_id)
    }

    /// Schedule the caller's own departure as a region's operator.
    pub fn initiate_resignation(ctx: Context<InitiateResignation>, region_id: u16) -> Result<()> {
        replacement::initiate_resignation_handler(ctx, region_id)
    }
}
