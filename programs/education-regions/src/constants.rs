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
