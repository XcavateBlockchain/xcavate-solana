use anchor_lang::prelude::*;

/// PDA seed for the singleton config.
#[constant]
pub const CONFIG_SEED: &[u8] = b"config";

/// PDA seed for the protocol's XCAV escrow vault. Holds every staked deposit:
/// the module, booking, and deliverer collateral.
#[constant]
pub const VAULT_SEED: &[u8] = b"vault";

/// PDA seed for a learning module's canonical record.
#[constant]
pub const MODULE_SEED: &[u8] = b"module";

/// PDA seed for a module's fractionalized token mint.
#[constant]
pub const MODULE_MINT_SEED: &[u8] = b"module_mint";

/// PDA seed for a module's token vault. Holds the entire fractionalized supply;
/// sponsor/school/student ownership is tracked as counters on the module rather
/// than by moving tokens between participant wallets.
#[constant]
pub const MODULE_VAULT_SEED: &[u8] = b"module_vault";

/// PDA seed for a sponsorship record.
#[constant]
pub const SPONSORSHIP_SEED: &[u8] = b"sponsorship";

/// PDA seed for a sponsorship's payment escrow (holds the sponsor's locked
/// stablecoin until tokens are delivered, reclaimed, or booked).
#[constant]
pub const SPONSOR_ESCROW_SEED: &[u8] = b"sponsor_escrow";

/// PDA seed for a booking record.
#[constant]
pub const BOOKING_SEED: &[u8] = b"booking";

/// PDA seed for a booking's payment escrow. The per-token payment moves here
/// when a school books and is paid out (or refunded) when the score lands.
#[constant]
pub const BOOK_ESCROW_SEED: &[u8] = b"book_escrow";

/// PDA seed for a lecturer's delivery registration.
#[constant]
pub const DELIVERER_SEED: &[u8] = b"deliverer";

/// PDA seed for a school's rolling cancellation counter.
#[constant]
pub const CANCEL_COUNTER_SEED: &[u8] = b"cancel_counter";

/// PDA seed for a single school cancellation record.
#[constant]
pub const CANCELLATION_SEED: &[u8] = b"cancellation";

/// PDA seed for a non-transferable credential record issued on delivery.
#[constant]
pub const CREDENTIAL_SEED: &[u8] = b"credential";
