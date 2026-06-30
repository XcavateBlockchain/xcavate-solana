use anchor_lang::prelude::*;

#[error_code]
pub enum EducationError {
    /// The signer is not the configured authority.
    #[msg("Signer is not the authority")]
    NotAuthority,
    /// The supplied parameters are invalid.
    #[msg("Invalid configuration parameters")]
    InvalidConfig,
    /// The supplied mint is not the configured XCAV mint.
    #[msg("Invalid XCAV mint")]
    InvalidMint,
    /// The treasury account does not match the configured treasury.
    #[msg("Invalid treasury account")]
    InvalidTreasury,
    /// The caller's role is not KYC-compliant.
    #[msg("Caller's role is not compliant")]
    NotCompliant,
    /// The region id is not one of the recognised regions.
    #[msg("Unknown region")]
    RegionUnknown,
    /// A token amount was zero where a positive value is required.
    #[msg("Amount cannot be zero")]
    AmountCannotBeZero,
    /// A module would be fractionalized into more tokens than allowed.
    #[msg("Too many tokens requested")]
    TooManyTokens,
    /// Arithmetic overflow.
    #[msg("Arithmetic overflow")]
    Overflow,
    /// Arithmetic underflow.
    #[msg("Arithmetic underflow")]
    Underflow,
    /// The payment asset is not one of the accepted assets.
    #[msg("Payment asset is not supported")]
    PaymentAssetNotSupported,
    /// The referenced module does not exist.
    #[msg("Module is not available")]
    ModuleNotAvailable,
    /// Not enough tokens are available in the requested allocation.
    #[msg("Not enough tokens available")]
    NotEnoughTokenAvailable,
    /// The caller is not permitted to act on this record.
    #[msg("Caller has no permission")]
    NoPermission,
    /// The sponsorship window has not yet elapsed.
    #[msg("Sponsorship window has not expired")]
    SponsorshipWindowNotExpired,
    /// Cannot burn more tokens than remain in the sponsor allocation.
    #[msg("Cannot burn more than the available allocation")]
    CannotBurnMoreThanAvailable,
    /// The sponsor has no tokens left to book.
    #[msg("Sponsor has no funded tokens available")]
    NoFundedModulesFromSponsor,
    /// The referenced booking does not exist.
    #[msg("Booking is not available")]
    NoBookingAvailable,
    /// The booking already has a lecturer.
    #[msg("Booking already has a lecturer")]
    LecturerAlreadySet,
    /// A school cannot deliver its own booking.
    #[msg("School cannot claim its own booking")]
    SchoolCannotClaimOwnBooking,
    /// The lecturer is not registered as a deliverer.
    #[msg("Module deliverer is not registered")]
    ModuleDelivererNotRegistered,
    /// The deliverer's deposit is too low for another concurrent claim.
    #[msg("Insufficient deposit to claim")]
    InsufficientDepositToClaim,
    /// The booking has no lecturer set.
    #[msg("Booking has no lecturer")]
    NoLecturerSet,
    /// The booking already has a score.
    #[msg("Score already submitted")]
    ScoreAlreadySet,
    /// The score is out of the valid 0..=10000 bps range.
    #[msg("Score is out of range")]
    InvalidScore,
    /// No score has been submitted yet.
    #[msg("No score submitted")]
    NoTestResultsSubmitted,
    /// A payout account does not belong to its expected owner.
    #[msg("Wrong payout recipient")]
    WrongPayoutRecipient,
    /// The deliverer still has active claims.
    #[msg("Module deliverer is still active")]
    ModuleDelivererStillActive,
    /// The cancellation record is not yet old enough to clear.
    #[msg("Cancellation is not yet clearable")]
    CancellationNotClearable,
    /// A required optional account was not supplied.
    #[msg("A required account was not provided")]
    MissingAccount,
    /// A module cannot be removed while tokens remain in circulation.
    #[msg("Cannot remove a module with active tokens")]
    CannotRemoveModuleWithActiveTokens,
    /// The proposer's role is not allowed to open proposals.
    #[msg("Role cannot open proposals")]
    InvalidProposalRole,
    /// The vote lock is below the configured minimum.
    #[msg("Vote is below the minimum voting amount")]
    BelowMinimumVotingAmount,
    /// The proposal's voting window has already closed.
    #[msg("Proposal voting has ended")]
    ProposalExpired,
    /// The proposal's voting window is still open.
    #[msg("Proposal voting is still ongoing")]
    VotingStillOngoing,
    /// The proposal is not in the state this action requires.
    #[msg("Proposal is in the wrong state")]
    InvalidProposalState,
    /// Only the proposing creator may build a creator-opened proposal.
    #[msg("Only the proposer may build this proposal")]
    NotProposalCreator,
    /// The creator was banned from this proposal after failing review twice.
    #[msg("Creator is banned from this proposal")]
    CreatorBanned,
    /// The proposal cannot be cleared yet.
    #[msg("Proposal is not clearable")]
    ProposalNotClearable,
    /// The sponsorship still has tokens left to book.
    #[msg("Sponsorship is not yet empty")]
    SponsorshipNotEmpty,
    /// The proposal's build deadline has not yet passed.
    #[msg("Proposal build deadline has not passed")]
    BuildDeadlineNotReached,
    /// The reservation's upload deadline has not yet passed.
    #[msg("Upload deadline has not passed")]
    UploadDeadlineNotReached,
}
