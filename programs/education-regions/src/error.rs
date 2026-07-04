use anchor_lang::prelude::*;

#[error_code]
pub enum RegionsError {
    /// The signer is not the configured authority.
    #[msg("Signer is not the authority")]
    NotAuthority,
    /// The supplied governance parameters are invalid.
    #[msg("Invalid governance parameters")]
    InvalidConfig,
    /// The supplied mint is not the configured XCAV mint.
    #[msg("Invalid XCAV mint")]
    InvalidMint,
    /// The region id is not one of the recognised regions.
    #[msg("Unknown region")]
    InvalidRegion,
    /// A region with this id already exists.
    #[msg("Region already created")]
    RegionAlreadyCreated,
    /// The voting amount is below the configured minimum.
    #[msg("Voting amount is below the minimum")]
    BelowMinimumVotingAmount,
    /// The proposal's voting window has closed.
    #[msg("Voting window has closed")]
    ProposalExpired,
    /// The proposal is not in the voting phase.
    #[msg("Region is not in the proposing phase")]
    NotProposing,
    /// The voting window has not yet closed.
    #[msg("Voting is still ongoing")]
    VotingStillOngoing,
    /// The treasury account does not match the configured treasury.
    #[msg("Invalid treasury account")]
    InvalidTreasury,
    /// The XCAV supply is too small to produce a positive operator bond.
    #[msg("XCAV supply too small to bond")]
    BondTooSmall,
    /// The region proposal has not passed, so it can't be claimed.
    #[msg("Region proposal has not passed")]
    RegionNotPassed,
    /// The caller is not the proposer of this region.
    #[msg("Caller is not the region proposer")]
    NotProposer,
    /// The current operator cannot claim their own open seat.
    #[msg("Operator cannot claim their own seat")]
    SelfClaimNotAllowed,
    /// The region's operator changed since the removal was opened.
    #[msg("Region operator changed since removal opened")]
    RemovalTargetChanged,
    /// The region state is not in a clearable (rejected/stale-passed) state.
    #[msg("Region state is not clearable")]
    NotClearable,
    /// The caller is not the region's operator.
    #[msg("Caller is not the region operator")]
    NotRegionOwner,
    /// The region's operator cannot be changed yet.
    #[msg("Region operator cannot be changed yet")]
    RegionOwnerCantBeChanged,
    /// An earlier (or equal) owner change is already scheduled.
    #[msg("An owner change is already scheduled")]
    OwnerChangeAlreadyScheduled,
    /// Arithmetic overflow.
    #[msg("Arithmetic overflow")]
    Overflow,
    /// A payout recipient's token account is required on this path.
    #[msg("Recipient token account is required")]
    MissingRecipientToken,
    /// The mint carries a token extension the escrow accounting cannot support.
    #[msg("Unsupported token extension on mint")]
    UnsupportedMintExtension,
    /// The signer is not the program's upgrade authority.
    #[msg("Signer is not the program upgrade authority")]
    NotUpgradeAuthority,
}
