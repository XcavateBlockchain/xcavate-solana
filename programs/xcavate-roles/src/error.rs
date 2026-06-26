use anchor_lang::prelude::*;

#[error_code]
pub enum RolesError {
    /// The signer is not the configured sudo authority.
    #[msg("Signer is not the sudo authority")]
    NotAuthority,
    /// The permission is already set to the requested value.
    #[msg("Permission is already set to this value")]
    PermissionAlreadySet,
}
