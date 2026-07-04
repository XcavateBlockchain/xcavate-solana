use anchor_lang::prelude::*;

use xcavate_roles::state::Role;

/// Singleton config holding every protocol parameter and the authority.
///
/// Deposits, proposal stakes, and votes are denominated in XCAV and escrowed in
/// the vault.
/// Module payments are made in one of the `accepted_assets` (stablecoin) mints;
/// prices are quoted in whole currency units and scaled to each asset's decimals
/// at the point of payment.
#[account]
#[derive(InitSpace)]
pub struct Config {
    /// Authority allowed to update parameters.
    pub authority: Pubkey,
    /// The XCAV mint staked for deposits.
    pub xcav_mint: Pubkey,
    /// XCAV token account that receives slashed deposits.
    pub treasury: Pubkey,
    /// Owner of the protocol's fee token accounts (one per accepted asset).
    pub protocol_authority: Pubkey,

    /// XCAV locked when a creator registers a module.
    pub module_deposit: u64,
    /// XCAV locked when a school books a module.
    pub booking_deposit: u64,
    /// XCAV locked when a lecturer registers to deliver modules.
    pub deliverer_deposit: u64,

    /// Base module price, in whole currency units (scaled to asset decimals).
    pub module_price: u64,
    /// Maximum number of tokens a module can be fractionalized into.
    pub max_module_tokens: u64,

    /// Fee shares, in basis points of the base price.
    pub content_creator_bps: u16,
    pub regional_operator_bps: u16,
    pub protocol_bps: u16,
    pub dbs_bps: u16,

    /// Minimum student score (bps) for a delivery to pay out and count.
    pub min_impact_score_bps: u16,

    /// Seconds a sponsorship is locked before the sponsor can reclaim it.
    pub sponsorship_window: i64,
    /// Window (seconds) over which a school's cancellations are counted.
    pub cancellation_window: i64,
    /// Grace period (seconds) after a booking's scheduled delivery before a
    /// no-show claim can be expired (striking the absent lecturer).
    pub no_show_grace: i64,

    /// School cancellations within the window before its deposit is slashed.
    pub max_cancellations: u32,
    /// Lecturer strikes before its deposit is slashed.
    pub max_strikes: u8,
    /// Share (bps) of the deliverer deposit slashed per strike / claim.
    pub strike_slash_bps: u16,
    /// Successful deliveries that retire one strike.
    pub deliveries_per_strike_reduction: u32,

    /// XCAV locked when opening a module proposal, returned if it passes and
    /// slashed if it fails.
    pub proposal_deposit: u64,
    /// Minimum XCAV a voter must lock to cast a vote on a proposal.
    pub minimum_voting_amount: u64,
    /// Seconds a proposal stays open for voting.
    pub voting_period: i64,
    /// Approval threshold in basis points (yes / (yes + no)).
    pub threshold_bps: u16,
    /// Minimum total voting power for a proposal to count.
    pub quorum: u64,
    /// Tokens auto-sponsored for the proposer of a sponsor-opened proposal once
    /// its module is created. Zero disables the auto-sponsorship.
    pub pre_sponsor_amount: u64,
    /// Seconds a passed proposal has to be built before it can be expired.
    pub claim_period: i64,
    /// Seconds a creator has to upload after reserving a proposal, before the
    /// reservation can be released and its bond slashed.
    pub upload_period: i64,

    /// Accepted payment mints (stablecoins). A zeroed key is an unused slot.
    pub accepted_assets: [Pubkey; 3],

    /// Monotonic id counters.
    pub next_module_id: u64,
    pub next_sponsor_id: u64,
    pub next_booking_id: u64,
    pub next_proposal_id: u64,

    pub bump: u8,
}

impl Config {
    /// Whether `mint` is one of the configured payment assets.
    pub fn accepts(&self, mint: &Pubkey) -> bool {
        self.accepted_assets.contains(mint)
    }
}

/// A learning module: a fractionalized SPL token plus the pricing and allocation
/// bookkeeping the rest of the lifecycle reads. The module's NFT-style identity
/// is this account itself; soulbound credentials are minted later on delivery.
///
/// Tokens flow `sponsor_allocation -> school_allocation -> student_allocation`
/// as the module is sponsored, booked, and claimed, and are burned on delivery.
#[account]
#[derive(InitSpace)]
pub struct Module {
    pub module_id: u64,
    pub creator: Pubkey,
    pub region: u16,
    /// The fractionalized token mint (PDA, mint authority = config).
    pub mint: Pubkey,
    /// XCAV held from the creator, returned when the module is removed.
    pub deposit: u64,
    pub total_token_amount: u64,
    /// Tokens still held by the creator, available for sponsors to buy.
    pub sponsor_allocation: u64,
    /// Tokens held by sponsors, awaiting schools to book them.
    pub school_allocation: u64,
    /// Tokens booked by schools, awaiting a lecturer to deliver them.
    pub student_allocation: u64,
    /// Base price, in whole currency units, snapshotted from config.
    pub price: u64,
    pub content_creator_bps: u16,
    pub regional_operator_bps: u16,
    pub protocol_bps: u16,
    pub dbs_bps: u16,
    pub created_at: i64,
    #[max_len(200)]
    pub metadata: String,
    pub bump: u8,
}

impl Module {
    /// Must match the `#[max_len]` on `metadata`.
    pub const METADATA_MAX_LEN: usize = 200;
}

/// A sponsor's funding of a module. The sponsor locks the full price (base +
/// fees) per token in the sponsor escrow; `amount` is how many of those tokens
/// are still available for schools to book. When a school books, the per-token
/// payment moves to that booking's escrow; on reclaim it returns to the sponsor.
#[account]
#[derive(InitSpace)]
pub struct Sponsorship {
    pub module_id: u64,
    pub sponsor_id: u64,
    pub sponsor: Pubkey,
    /// The stablecoin mint the sponsor paid in.
    pub payment_asset: Pubkey,
    /// Tokens still bookable from this sponsorship.
    pub amount: u64,
    /// Bookings made from this sponsorship that haven't settled yet. A booking
    /// can be cancelled (returning its payment to this escrow) until it's
    /// finished, so the escrow must stay open while this is non-zero.
    pub active_bookings: u32,
    /// Full price (base + fees) locked per token, in `payment_asset` units.
    pub price_per_token: u64,
    pub sponsored_at: i64,
    pub bump: u8,
}

/// A school's booking of one module token. The per-token payment sits in the
/// booking escrow until a score is submitted (paid out and refunded) or the
/// booking is cancelled (returned to the sponsor).
#[account]
#[derive(InitSpace)]
pub struct Booking {
    pub module_id: u64,
    pub booking_id: u64,
    pub sponsor_id: u64,
    pub sponsor: Pubkey,
    pub school: Pubkey,
    /// The lecturer delivering it, once claimed.
    pub lecturer: Option<Pubkey>,
    pub payment_asset: Pubkey,
    /// Payment escrowed for this booking, in `payment_asset` units.
    pub price_per_token: u64,
    /// Student score in basis points, once submitted.
    pub score: Option<u16>,
    /// XCAV deposit held from the school.
    pub deposit: u64,
    pub booked_at: i64,
    /// When the session is scheduled to be delivered. Set by the school at
    /// booking time; a score can't be submitted before it, and a no-show claim
    /// can be expired once it plus the grace period has passed.
    pub delivery_at: i64,
    pub claimed_at: Option<i64>,
    #[max_len(200)]
    pub metadata: String,
    pub bump: u8,
}

impl Booking {
    pub const METADATA_MAX_LEN: usize = 200;
}

/// A lecturer's registration to deliver modules. The deposit (in the vault)
/// backs concurrent claims and is slashed on strikes.
#[account]
#[derive(InitSpace)]
pub struct Deliverer {
    pub deliverer: Pubkey,
    pub deposit: u64,
    pub active_claims: u32,
    pub active_strikes: u8,
    pub successful_deliveries: u32,
    pub bump: u8,
}

/// A school's rolling count of recent cancellations; slashing kicks in once it
/// reaches the configured maximum.
#[account]
#[derive(InitSpace)]
pub struct CancellationCounter {
    pub school: Pubkey,
    pub count: u32,
    pub bump: u8,
}

/// One recorded cancellation, kept until it ages out of the window.
#[account]
#[derive(InitSpace)]
pub struct Cancellation {
    pub school: Pubkey,
    pub booking_id: u64,
    pub module_id: u64,
    pub created_at: i64,
    pub bump: u8,
}

/// How a voter sided on a module proposal.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModuleVote {
    Yes,
    No,
    Abstain,
}

/// Where a module proposal is in its lifecycle.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProposalStatus {
    /// Open for voting.
    Voting,
    /// Passed the vote; a creator can now claim it to reserve the build.
    Claimable,
    /// Reserved by a creator who has locked the build bond but not yet uploaded
    /// the content. Releases back to claimable (slashing the bond) if they miss
    /// the upload deadline.
    Claimed,
    /// Content uploaded; waiting on the AI agent's review.
    UnderReview,
    /// The review passed; the claimant can mint the module.
    Approved,
    /// The module has been created (terminal).
    Created,
    /// The vote failed (terminal; lingers until cleared).
    Rejected,
}

/// A community proposal to open a learning module. Anyone holding one of the
/// participant roles can propose; anyone at all can vote by locking XCAV. Once a
/// proposal passes, a creator claims it, uploads the content, and an AI agent
/// reviews it before the module is minted.
///
/// The pricing (`price` and the fee splits) is snapshotted at proposal time so
/// the module is created on the terms it was voted on, even if config changes in
/// the meantime.
#[account]
#[derive(InitSpace)]
pub struct ModuleProposal {
    pub proposal_id: u64,
    pub proposer: Pubkey,
    /// The role the proposer opened this under (one of creator, sponsor, school,
    /// or lecturer). A creator-opened proposal can only be built by that creator.
    pub proposer_role: Role,
    pub region: u16,
    pub status: ProposalStatus,
    pub created_at: i64,
    /// When voting closes.
    pub expiry: i64,
    /// The proposer's XCAV stake held in the vault.
    pub deposit: u64,

    pub yes_power: u64,
    pub no_power: u64,
    pub abstain_power: u64,

    /// Tokens the module will be fractionalized into once created.
    pub module_amount: u64,
    /// Snapshot of the base price and fee splits at proposal time.
    pub price: u64,
    pub content_creator_bps: u16,
    pub regional_operator_bps: u16,
    pub protocol_bps: u16,
    pub dbs_bps: u16,

    /// The creator currently building it (reserved, under review, or pending a
    /// retry).
    pub claimant: Option<Pubkey>,
    /// XCAV bond the current claimant locked when reserving the build, refunded
    /// when they upload and slashed if they let the upload deadline pass. Zero
    /// when no reservation is held.
    pub claim_bond: u64,
    /// When the current reservation must have uploaded by; after this anyone can
    /// release the claim and slash the bond.
    pub upload_deadline: i64,
    /// AI-review failures by the current claimant (banned at two).
    pub claimant_failures: u8,
    /// Creators banned from this proposal after failing review twice.
    #[max_len(8)]
    pub banned: Vec<Pubkey>,
    /// Deadline for a passed proposal to be built. If the module isn't created
    /// by then the proposal can be expired and rejected so its rent and any
    /// pre-sponsorship are recoverable. Zero until the proposal passes.
    pub build_deadline: i64,

    /// For a sponsor-opened proposal: the asset and per-token price of the
    /// auto-sponsorship locked up front, converted to a real sponsorship when
    /// the module is created.
    pub payment_asset: Pubkey,
    pub pre_sponsor_amount: u64,
    pub pre_sponsor_price_per_token: u64,

    #[max_len(200)]
    pub metadata: String,
    /// The content uri uploaded by the claimant for review.
    #[max_len(200)]
    pub content_uri: String,
    pub bump: u8,
}

impl ModuleProposal {
    pub const METADATA_MAX_LEN: usize = 200;
    pub const URI_MAX_LEN: usize = 200;
    pub const MAX_BANNED: usize = 8;

    /// Total XCAV the locked pre-sponsorship escrow should hold.
    pub fn pre_sponsor_locked(&self) -> Option<u64> {
        self.pre_sponsor_price_per_token
            .checked_mul(self.pre_sponsor_amount)
    }
}

/// One voter's locked vote on a proposal. The XCAV stake sits in the vault and
/// is returned when the voter unlocks. `expiry` is copied from the proposal so
/// unlocking never needs the proposal account.
#[account]
#[derive(InitSpace)]
pub struct ModuleProposalVote {
    pub proposal_id: u64,
    pub voter: Pubkey,
    pub vote: ModuleVote,
    pub power: u64,
    pub expiry: i64,
    pub bump: u8,
}

/// Who a soulbound credential is issued to.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug)]
pub enum CredentialKind {
    Sponsor,
    School,
    Lecturer,
    Student,
}

impl CredentialKind {
    pub fn seed_byte(&self) -> u8 {
        match self {
            CredentialKind::Sponsor => 0,
            CredentialKind::School => 1,
            CredentialKind::Lecturer => 2,
            CredentialKind::Student => 3,
        }
    }
}

/// A non-transferable credential issued to a participant once a booking is
/// scored. Owned by the program (so it can't be moved), it records who earned
/// it and links back to the module and booking, with an off-chain metadata uri.
#[account]
#[derive(InitSpace)]
pub struct Credential {
    pub recipient: Pubkey,
    pub module_id: u64,
    pub booking_id: u64,
    pub kind: CredentialKind,
    /// The booking's impact score, recorded on a student's credential so it
    /// carries their individual result. `None` for the other credential kinds.
    pub score: Option<u16>,
    pub issued_at: i64,
    #[max_len(200)]
    pub uri: String,
    pub bump: u8,
}

impl Credential {
    pub const URI_MAX_LEN: usize = 200;
}
