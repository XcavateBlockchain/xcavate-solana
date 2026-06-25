use anchor_lang::prelude::*;

/// PDA seed for the singleton program config (sudo authority).
#[constant]
pub const CONFIG_SEED: &[u8] = b"config";

/// PDA seed for an admin marker account.
#[constant]
pub const ADMIN_SEED: &[u8] = b"admin";

/// PDA seed for a per-(user, role) assignment account.
#[constant]
pub const ROLE_SEED: &[u8] = b"role";
