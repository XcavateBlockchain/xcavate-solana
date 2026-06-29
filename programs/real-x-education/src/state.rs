use anchor_lang::prelude::*;

/// Singleton config holding every protocol parameter and the authority.
///
/// Deposits are denominated in XCAV and escrowed in the vault.
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

    /// School cancellations within the window before its deposit is slashed.
    pub max_cancellations: u32,
    /// Lecturer strikes before its deposit is slashed.
    pub max_strikes: u8,
    /// Share (bps) of the deliverer deposit slashed per strike / claim.
    pub strike_slash_bps: u16,
    /// Successful deliveries that retire one strike.
    pub deliveries_per_strike_reduction: u32,

    /// Accepted payment mints (stablecoins). A zeroed key is an unused slot.
    pub accepted_assets: [Pubkey; 3],

    /// Monotonic id counters.
    pub next_module_id: u64,
    pub next_sponsor_id: u64,
    pub next_booking_id: u64,

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
    pub issued_at: i64,
    #[max_len(200)]
    pub uri: String,
    pub bump: u8,
}

impl Credential {
    pub const URI_MAX_LEN: usize = 200;
}
