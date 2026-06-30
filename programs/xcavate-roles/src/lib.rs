pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("EFiBfC2bPoY3gsHQQS2kNyb8XKQSUmeAwQ9jocHPPz6i");

/// Roles and compliance registry for the Xcavate protocol.
///
/// Tracks which addresses hold which roles and whether each assignment is
/// KYC-compliant. Other programs gate actions by loading the `RoleAccount`
/// PDA (`["role", user, role.seed_byte()]`), which requires the role to be
/// assigned.
#[program]
pub mod xcavate_roles {
    use super::*;

    /// Initialize the singleton config; sets sudo authority to the signer.
    pub fn initialize_config(ctx: Context<InitializeConfig>) -> Result<()> {
        initialize::handler(ctx)
    }

    /// Rotate the sudo authority. Current-authority-only.
    pub fn update_authority(
        ctx: Context<UpdateAuthority>,
        new_authority: Pubkey,
    ) -> Result<()> {
        initialize::update_authority_handler(ctx, new_authority)
    }

    /// Register a whitelist admin. Sudo-only.
    pub fn add_admin(ctx: Context<AddAdmin>) -> Result<()> {
        admin::add_admin_handler(ctx)
    }

    /// Remove a whitelist admin. Sudo-only.
    pub fn remove_admin(ctx: Context<RemoveAdmin>) -> Result<()> {
        admin::remove_admin_handler(ctx)
    }

    /// Assign a role to a user (default Compliant). Admin-only.
    pub fn assign_role(ctx: Context<AssignRole>, role: Role) -> Result<()> {
        role::assign_role_handler(ctx, role)
    }

    /// Remove a role from a user. Admin-only.
    pub fn remove_role(ctx: Context<RemoveRole>, role: Role) -> Result<()> {
        role::remove_role_handler(ctx, role)
    }

    /// Update a user's compliance status for a role. Admin-only.
    pub fn set_permission(
        ctx: Context<SetPermission>,
        role: Role,
        permission: AccessPermission,
    ) -> Result<()> {
        role::set_permission_handler(ctx, role, permission)
    }
}
