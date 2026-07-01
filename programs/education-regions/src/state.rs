use anchor_lang::prelude::*;

/// The recognised regions and their on-chain ids.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegionIdentifier {
    England,
    France,
    Japan,
    India,
}

impl RegionIdentifier {
    /// Returns the identifier for a raw id, or `None` if it isn't recognised.
    pub fn from_code(code: u16) -> Option<Self> {
        match code {
            1 => Some(RegionIdentifier::England),
            2 => Some(RegionIdentifier::France),
            3 => Some(RegionIdentifier::Japan),
            4 => Some(RegionIdentifier::India),
            _ => None,
        }
    }
}

/// How a voter can vote on a proposal.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Vote {
    Yes,
    No,
    Abstain,
}

/// Singleton config holding governance parameters and the authority.
#[account]
#[derive(InitSpace)]
pub struct Config {
    /// Authority allowed to update parameters.
    pub authority: Pubkey,
    /// The XCAV governance mint staked for proposals, votes, and bids.
    pub xcav_mint: Pubkey,
    /// XCAV token account that receives slashed deposits.
    pub treasury: Pubkey,
    /// XCAV locked when proposing a region.
    pub proposal_deposit: u64,
    /// Minimum XCAV a voter must lock to vote.
    pub minimum_voting_amount: u64,
    /// Minimum collateral (XCAV) to win a region auction.
    pub minimum_region_deposit: u64,
    /// Seconds a proposal stays open for voting.
    pub voting_period: i64,
    /// Seconds a region auction stays open.
    pub auction_period: i64,
    /// Minimum time between region-owner changes.
    pub owner_change_period: i64,
    /// Approval threshold in basis points (yes / (yes + no)).
    pub threshold_bps: u16,
    /// Minimum total voting power for a proposal to be valid.
    pub quorum: u64,
    /// XCAV locked when proposing to remove a region's operator.
    pub removal_deposit: u64,
    /// Seconds an operator-removal proposal stays open for voting.
    pub removal_voting_period: i64,
    /// XCAV slashed from a region's collateral per upheld strike.
    pub slash_amount: u64,
    /// Notice an operator must give before resigning (seconds).
    pub notice_period: i64,
    /// Strikes that, once reached, open a region to a replacement auction.
    pub allowed_strikes: u8,
    /// Monotonic id for the next proposal.
    pub proposal_counter: u64,
    pub bump: u8,
}

/// A created region and its operator.
#[account]
#[derive(InitSpace)]
pub struct Region {
    pub region_id: u16,
    pub owner: Pubkey,
    pub collateral: u64,
    pub active_strikes: u8,
    pub next_owner_change: i64,
    pub bump: u8,
}

/// An open proposal to create a region, with its running vote tally. The
/// proposer's `deposit` of XCAV is held in the vault and returned (or slashed)
/// at finalization.
#[account]
#[derive(InitSpace)]
pub struct RegionProposal {
    pub proposal_id: u64,
    pub proposer: Pubkey,
    pub region_id: u16,
    pub created_at: i64,
    pub expiry: i64,
    pub deposit: u64,
    pub yes_power: u64,
    pub no_power: u64,
    pub abstain_power: u64,
    pub bump: u8,
}

/// Where a region is in its lifecycle.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegionStatus {
    /// A proposal is open for voting.
    Proposing,
    /// The proposal passed; an auction for the operator slot is running.
    Auctioning,
    /// The proposal was rejected; the state lingers until cleared so the
    /// region can be proposed again.
    Rejected,
}

/// One account per region tracking its lifecycle. Created when a region is
/// proposed and kept (transitioning Proposing -> Auctioning) until the region
/// is created or the proposal is rejected. Having exactly one of these per
/// region is what enforces a single in-flight proposal/auction at a time.
#[account]
#[derive(InitSpace)]
pub struct RegionState {
    pub region_id: u16,
    pub status: RegionStatus,
    /// The proposal this region is voting on.
    pub proposal_id: u64,
    /// Highest bidder once `Auctioning`. Their bid of XCAV is held in the vault.
    pub highest_bidder: Option<Pubkey>,
    /// Current highest bid / minimum collateral during the auction.
    pub collateral: u64,
    /// When the auction ends (set on transition to `Auctioning`).
    pub auction_expiry: i64,
    pub bump: u8,
}

/// One voter's vote on a proposal. The locked voting power (XCAV) is held in the
/// vault and returned when the voter unlocks. `expiry` is copied from the
/// proposal so unlocking never needs the proposal account (which may already be
/// closed by finalization).
#[account]
#[derive(InitSpace)]
pub struct VoteRecord {
    pub proposal_id: u64,
    pub voter: Pubkey,
    pub region_id: u16,
    pub vote: Vote,
    pub power: u64,
    pub expiry: i64,
    pub bump: u8,
}

/// An open proposal to remove a region's operator, with its running vote tally.
/// One per region (the PDA is seeded by `region_id`), so a region can only have a
/// single removal vote in flight at a time. The proposer's `deposit` is held in
/// the vault and returned if the proposal passes, slashed otherwise.
#[account]
#[derive(InitSpace)]
pub struct RemovalProposal {
    pub proposal_id: u64,
    pub region_id: u16,
    pub proposer: Pubkey,
    pub created_at: i64,
    pub expiry: i64,
    pub deposit: u64,
    pub yes_power: u64,
    pub no_power: u64,
    pub abstain_power: u64,
    pub bump: u8,
}

/// One voter's vote on a removal proposal. Mirrors `VoteRecord` but is a distinct
/// account so removal votes never collide with region-proposal votes.
#[account]
#[derive(InitSpace)]
pub struct RemovalVoteRecord {
    pub proposal_id: u64,
    pub voter: Pubkey,
    pub region_id: u16,
    pub vote: Vote,
    pub power: u64,
    pub expiry: i64,
    pub bump: u8,
}

/// An auction to replace a region's operator once the seat is open (after a
/// successful removal or a resignation notice elapses). One per region. The
/// leading bid is held in the vault and becomes the new operator's collateral.
#[account]
#[derive(InitSpace)]
pub struct ReplacementAuction {
    pub region_id: u16,
    /// Highest bidder so far, if any. Their bid of XCAV is held in the vault.
    pub highest_bidder: Option<Pubkey>,
    /// Current highest bid / minimum collateral during the auction.
    pub collateral: u64,
    /// When the auction ends.
    pub auction_expiry: i64,
    pub bump: u8,
}
