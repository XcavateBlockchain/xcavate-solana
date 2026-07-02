use anchor_lang::prelude::*;

/// All roles recognised across the Xcavate protocol.
///
/// A role is app-level authorization, separate from KYC/compliance. Compliance
/// lives in [`AccessPermission`], and will eventually be driven by SAS
/// attestations rather than a manually set flag.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// Manages a region and oversees its educational operations.
    RegionalOperator,
    /// Creates educational content/modules.
    ModuleCreator,
    /// Funds the delivery of educational modules.
    ModuleSponsor,
    /// Books a sponsored module for delivery to students.
    ModuleBooker,
    /// Delivers the educational module (lecturer / teacher).
    ModuleDeliverer,
    /// AI agent that evaluates impact / scores delivery.
    ModuleAIAgent,
}

impl Role {
    /// Stable one-byte tag for PDA seeds. Explicit on purpose so the derivation
    /// doesn't shift if the enum is ever reordered.
    pub fn seed_byte(&self) -> u8 {
        match self {
            Role::RegionalOperator => 0,
            Role::ModuleCreator => 1,
            Role::ModuleSponsor => 2,
            Role::ModuleBooker => 3,
            Role::ModuleDeliverer => 4,
            Role::ModuleAIAgent => 5,
        }
    }
}

/// Compliance status for a (user, role) assignment, set by an admin after
/// off-chain KYC/AML. The education programs gate on role existence only and
/// do not read this flag; it is bookkeeping for market-side programs that
/// need a softer switch than removing the role outright. Later it'll be
/// driven by a SAS attestation instead of a manual toggle.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccessPermission {
    /// Passed KYC/AML, so role-specific actions are allowed.
    Compliant,
    /// Revoked, so role-specific actions are blocked.
    Revoked,
}

/// Singleton config holding the sudo authority that manages admins.
#[account]
#[derive(InitSpace)]
pub struct Config {
    /// Sudo authority allowed to add/remove admins.
    pub authority: Pubkey,
    pub bump: u8,
}

/// Marks an address as a whitelist admin.
#[account]
#[derive(InitSpace)]
pub struct Admin {
    pub admin: Pubkey,
    pub bump: u8,
}

/// One (user, role) assignment together with its compliance status.
#[account]
#[derive(InitSpace)]
pub struct RoleAccount {
    pub user: Pubkey,
    pub role: Role,
    pub permission: AccessPermission,
    pub bump: u8,
}

impl RoleAccount {
    /// Whether this assignment is currently active (KYC-compliant).
    pub fn is_compliant(&self) -> bool {
        self.permission == AccessPermission::Compliant
    }
}
