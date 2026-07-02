use anchor_lang::prelude::*;

/// PDA seed for the singleton config.
#[constant]
pub const CONFIG_SEED: &[u8] = b"config";

/// PDA seed for a created region.
#[constant]
pub const REGION_SEED: &[u8] = b"region";

/// PDA seed for a region proposal.
#[constant]
pub const PROPOSAL_SEED: &[u8] = b"proposal";

/// PDA seed for a region's lifecycle state (enforces one in-flight at a time).
#[constant]
pub const REGION_STATE_SEED: &[u8] = b"region_state";

/// PDA seed for a single voter's vote on a proposal.
#[constant]
pub const VOTE_SEED: &[u8] = b"vote";

/// PDA seed for the protocol's XCAV escrow vault (holds all staked governance
/// tokens: proposal deposits, vote locks, and auction bids).
#[constant]
pub const VAULT_SEED: &[u8] = b"vault";

/// PDA seed for an open proposal to remove a region's operator (one per region).
#[constant]
pub const REMOVAL_PROPOSAL_SEED: &[u8] = b"removal_proposal";

/// PDA seed for a single voter's vote on a removal proposal.
#[constant]
pub const REMOVAL_VOTE_SEED: &[u8] = b"removal_vote";

/// PDA seed for a region's replacement auction (one per region).
#[constant]
pub const REPLACEMENT_AUCTION_SEED: &[u8] = b"replacement_auction";

/// The protocol treasury token account (program owned, shared with real-x).
#[constant]
pub const TREASURY_SEED: &[u8] = b"treasury";
