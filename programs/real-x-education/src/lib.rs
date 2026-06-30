pub mod constants;
pub mod error;
pub mod instructions;
pub mod minting;
pub mod pricing;
pub mod state;
pub mod vault;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("5AeeFej1ZeiRGxVQ8TFCw9hYnAGk3KHqEGje8Lk6qz7A");

/// Learning modules. A creator fractionalizes a module into tokens, sponsors
/// fund its delivery, schools book it and lecturers deliver it. Once an AI agent
/// scores the result, the payment is split between the creator, the region's
/// operator, the protocol and the lecturer.
#[program]
pub mod real_x_education {
    use super::*;

    /// Initialize the singleton config and open the XCAV escrow vault.
    pub fn initialize_config(ctx: Context<InitializeConfig>, params: ConfigParams) -> Result<()> {
        initialize::handler(ctx, params)
    }

    /// Update the protocol parameters. Authority-only.
    pub fn update_config(ctx: Context<UpdateConfig>, params: ConfigParams) -> Result<()> {
        initialize::update_config_handler(ctx, params)
    }

    /// Rotate the config authority. Current-authority-only.
    pub fn update_authority(ctx: Context<UpdateAuthority>, new_authority: Pubkey) -> Result<()> {
        initialize::update_authority_handler(ctx, new_authority)
    }

    /// Create a learning module and fractionalize it into tokens.
    /// ModuleCreator-only; locks a deposit.
    pub fn create_module(
        ctx: Context<CreateModule>,
        region: u16,
        module_amount: u64,
        metadata: String,
    ) -> Result<()> {
        create_module::create_module_handler(ctx, region, module_amount, metadata)
    }

    /// Sponsor a module: lock payment in escrow and reserve tokens for schools.
    /// ModuleSponsor-only.
    pub fn sponsor_module(
        ctx: Context<SponsorModule>,
        module_id: u64,
        token_amount: u64,
    ) -> Result<()> {
        sponsor::sponsor_module_handler(ctx, module_id, token_amount)
    }

    /// Reclaim unbooked sponsored tokens once the sponsorship window passes.
    /// ModuleSponsor-only.
    pub fn reclaim_sponsorship(
        ctx: Context<ReclaimSponsorship>,
        module_id: u64,
        sponsor_id: u64,
        amount: u64,
    ) -> Result<()> {
        sponsor::reclaim_sponsorship_handler(ctx, module_id, sponsor_id, amount)
    }

    /// Close a fully-spent sponsorship and reclaim its rent. ModuleSponsor-only.
    pub fn close_sponsorship(
        ctx: Context<CloseSponsorship>,
        module_id: u64,
        sponsor_id: u64,
    ) -> Result<()> {
        sponsor::close_sponsorship_handler(ctx, module_id, sponsor_id)
    }

    /// Burn tokens from a module's unsponsored allocation. ModuleCreator-only.
    pub fn burn_unsponsored_token(
        ctx: Context<BurnUnsponsored>,
        module_id: u64,
        amount: u64,
    ) -> Result<()> {
        burn::burn_unsponsored_handler(ctx, module_id, amount)
    }

    /// Remove a fully-retired module and refund its deposit. ModuleCreator-only.
    pub fn remove_module(ctx: Context<RemoveModule>, module_id: u64) -> Result<()> {
        burn::remove_module_handler(ctx, module_id)
    }

    /// Book one token of a sponsored module. ModuleBooker-only; locks a deposit.
    pub fn book_module(
        ctx: Context<BookModule>,
        module_id: u64,
        sponsor_id: u64,
        metadata: String,
    ) -> Result<()> {
        booking::book_module_handler(ctx, module_id, sponsor_id, metadata)
    }

    /// Release the deposit and close a scored booking. ModuleBooker-only.
    pub fn finish_booking_process(
        ctx: Context<FinishBooking>,
        module_id: u64,
        booking_id: u64,
    ) -> Result<()> {
        booking::finish_booking_handler(ctx, module_id, booking_id)
    }

    /// Claim a booking to deliver it. ModuleDeliverer-only.
    pub fn claim_booking(
        ctx: Context<ClaimBooking>,
        module_id: u64,
        booking_id: u64,
    ) -> Result<()> {
        claim::claim_booking_handler(ctx, module_id, booking_id)
    }

    /// Submit a student score and settle payment. ModuleAIAgent-only.
    pub fn submit_impact_score(
        ctx: Context<SubmitImpactScore>,
        module_id: u64,
        booking_id: u64,
        score: u16,
    ) -> Result<()> {
        score::submit_impact_score_handler(ctx, module_id, booking_id, score)
    }

    /// Issue a non-transferable credential for a scored booking.
    /// ModuleAIAgent-only.
    pub fn mint_credential(
        ctx: Context<MintCredential>,
        module_id: u64,
        booking_id: u64,
        kind: CredentialKind,
        recipient: Pubkey,
        uri: String,
    ) -> Result<()> {
        credential::mint_credential_handler(ctx, module_id, booking_id, kind, recipient, uri)
    }

    /// Cancel a booking. ModuleBooker-only.
    pub fn cancel_booking(
        ctx: Context<CancelBooking>,
        module_id: u64,
        booking_id: u64,
    ) -> Result<()> {
        cancel::cancel_booking_handler(ctx, module_id, booking_id)
    }

    /// Clear an aged-out cancellation record. ModuleBooker-only.
    pub fn clear_old_cancellation(
        ctx: Context<ClearOldCancellation>,
        booking_id: u64,
    ) -> Result<()> {
        cancel::clear_old_cancellation_handler(ctx, booking_id)
    }

    /// Cancel a claimed booking, taking a strike. ModuleDeliverer-only.
    pub fn cancel_claim(
        ctx: Context<CancelClaim>,
        module_id: u64,
        booking_id: u64,
    ) -> Result<()> {
        cancel::cancel_claim_handler(ctx, module_id, booking_id)
    }

    /// Register as a module deliverer (or top up the deposit).
    /// ModuleDeliverer-only.
    pub fn register_module_deliverer(ctx: Context<RegisterDeliverer>) -> Result<()> {
        deliverer::register_deliverer_handler(ctx)
    }

    /// Unregister as a module deliverer and withdraw the deposit.
    /// ModuleDeliverer-only.
    pub fn unregister_module_deliverer(ctx: Context<UnregisterDeliverer>) -> Result<()> {
        deliverer::unregister_deliverer_handler(ctx)
    }

    /// Open a proposal to create a module under one of the participant roles.
    pub fn create_module_proposal(
        ctx: Context<CreateModuleProposal>,
        role: xcavate_roles::state::Role,
        region: u16,
        module_amount: u64,
        metadata: String,
    ) -> Result<()> {
        proposal::create_module_proposal_handler(ctx, role, region, module_amount, metadata)
    }

    /// Open a sponsor proposal and pre-fund its auto-sponsorship.
    /// ModuleSponsor-only.
    pub fn create_sponsor_proposal(
        ctx: Context<CreateSponsorProposal>,
        region: u16,
        module_amount: u64,
        metadata: String,
    ) -> Result<()> {
        proposal::create_sponsor_proposal_handler(ctx, region, module_amount, metadata)
    }

    /// Vote on an open proposal by locking XCAV. No role or KYC required.
    pub fn vote_on_proposal(
        ctx: Context<VoteOnProposal>,
        proposal_id: u64,
        vote: ModuleVote,
        amount: u64,
    ) -> Result<()> {
        proposal::vote_on_proposal_handler(ctx, proposal_id, vote, amount)
    }

    /// Finalize a proposal once voting closes. Permissionless.
    pub fn finalize_proposal(ctx: Context<FinalizeProposal>, proposal_id: u64) -> Result<()> {
        proposal::finalize_proposal_handler(ctx, proposal_id)
    }

    /// Reserve a passed proposal to build it, locking a bond. ModuleCreator-only.
    pub fn claim_proposal(ctx: Context<ClaimProposal>, proposal_id: u64) -> Result<()> {
        proposal::claim_proposal_handler(ctx, proposal_id)
    }

    /// Upload the content for a reserved proposal and send it for review,
    /// refunding the bond. ModuleCreator-only (the claimant).
    pub fn upload_proposal(
        ctx: Context<UploadProposal>,
        proposal_id: u64,
        content_uri: String,
    ) -> Result<()> {
        proposal::upload_proposal_handler(ctx, proposal_id, content_uri)
    }

    /// Release a reservation whose upload deadline passed, slashing the bond.
    /// Permissionless.
    pub fn release_claim(ctx: Context<ReleaseClaim>, proposal_id: u64) -> Result<()> {
        proposal::release_claim_handler(ctx, proposal_id)
    }

    /// Record the AI agent's review of a claimed proposal. ModuleAIAgent-only.
    pub fn review_proposal(
        ctx: Context<ReviewProposal>,
        proposal_id: u64,
        passed: bool,
    ) -> Result<()> {
        proposal::review_proposal_handler(ctx, proposal_id, passed)
    }

    /// Mint the module for an approved proposal with no pre-sponsorship.
    /// ModuleCreator-only (the claimant).
    pub fn mint_proposed_module(
        ctx: Context<MintProposedModule>,
        proposal_id: u64,
    ) -> Result<()> {
        proposal::mint_proposed_module_handler(ctx, proposal_id)
    }

    /// Mint the module for an approved sponsor proposal and convert the
    /// pre-sponsorship. ModuleCreator-only (the claimant).
    pub fn mint_sponsored_module(
        ctx: Context<MintSponsoredModule>,
        proposal_id: u64,
    ) -> Result<()> {
        proposal::mint_sponsored_module_handler(ctx, proposal_id)
    }

    /// Reclaim a voter's locked XCAV once the proposal's voting window ended.
    pub fn unlock_proposal_vote(
        ctx: Context<UnlockProposalVote>,
        proposal_id: u64,
    ) -> Result<()> {
        proposal::unlock_proposal_vote_handler(ctx, proposal_id)
    }

    /// Refund a rejected sponsor proposal's pre-sponsorship. ModuleSponsor-only.
    pub fn reclaim_pre_sponsor(
        ctx: Context<ReclaimPreSponsor>,
        proposal_id: u64,
    ) -> Result<()> {
        proposal::reclaim_pre_sponsor_handler(ctx, proposal_id)
    }

    /// Reject a passed proposal that wasn't built before its deadline.
    /// Permissionless.
    pub fn expire_proposal(ctx: Context<ExpireProposal>, proposal_id: u64) -> Result<()> {
        proposal::expire_proposal_handler(ctx, proposal_id)
    }

    /// Close a rejected proposal and reclaim its rent. Permissionless.
    pub fn clear_proposal(ctx: Context<ClearProposal>, proposal_id: u64) -> Result<()> {
        proposal::clear_proposal_handler(ctx, proposal_id)
    }
}
