use anchor_lang::prelude::*;

#[error_code]
pub enum RolesError {
    /// The signer is not the configured sudo authority.
    #[msg("Signer is not the sudo authority")]
    NotAuthority,
    /// The permission is already set to the requested value.
    #[msg("Permission is already set to this value")]
    PermissionAlreadySet,
    /// The new authority cannot be the zero address.
    #[msg("Invalid authority address")]
    InvalidAuthority,
    /// The signer is not the program's upgrade authority.
    #[msg("Signer is not the program upgrade authority")]
    NotUpgradeAuthority,
}
