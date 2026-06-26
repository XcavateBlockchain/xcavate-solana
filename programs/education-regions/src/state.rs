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
    pub fn code(&self) -> u16 {
        match self {
            RegionIdentifier::England => 1,
            RegionIdentifier::France => 2,
            RegionIdentifier::Japan => 3,
            RegionIdentifier::India => 4,
        }
    }

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
