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
    /// The region is not currently auctioning.
    #[msg("Region is not auctioning")]
    NotAuctioning,
    /// The auction has already ended.
    #[msg("Auction has ended")]
    AuctionEnded,
    /// The auction has not ended yet.
    #[msg("Auction has not finished")]
    AuctionNotFinished,
    /// The bid does not beat the current highest bid.
    #[msg("Bid does not beat the current bid")]
    BidTooLow,
    /// The first bid is below the minimum collateral.
    #[msg("Bid is below the minimum")]
    BidBelowMinimum,
    /// The passed previous bidder does not match the current highest bidder.
    #[msg("Wrong previous bidder account")]
    WrongPreviousBidder,
    /// The previous bidder account is required to refund the outbid bidder.
    #[msg("Previous bidder account is required")]
    MissingPreviousBidder,
    /// The caller did not win the auction.
    #[msg("Caller is not the winning bidder")]
    NotWinner,
    /// The region state is not in a clearable (rejected/empty) state.
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
