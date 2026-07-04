use anchor_lang::prelude::*;

use crate::constants::{ADMIN_SEED, CONFIG_SEED, ROLE_SEED};
use crate::error::RolesError;
use crate::state::{AccessPermission, Admin, Config, Role, RoleAccount};

/// Grant a role to a user. The assignment starts compliant. Admin-only.
#[derive(Accounts)]
#[instruction(role: Role)]
pub struct AssignRole<'info> {
    #[account(mut)]
    pub admin_signer: Signer<'info>,

    /// Proves `admin_signer` is a registered admin (PDA must exist).
    #[account(
        seeds = [ADMIN_SEED, admin_signer.key().as_ref()],
        bump = admin.bump,
    )]
    pub admin: Account<'info, Admin>,

    /// CHECK: the user receiving the role; used as identity / PDA seed only.
    pub user: UncheckedAccount<'info>,

    #[account(
        init,
        payer = admin_signer,
        space = 8 + RoleAccount::INIT_SPACE,
        seeds = [ROLE_SEED, user.key().as_ref(), &[role.seed_byte()]],
        bump,
    )]
    pub role_account: Account<'info, RoleAccount>,

    pub system_program: Program<'info, System>,
}

pub fn assign_role_handler(ctx: Context<AssignRole>, role: Role) -> Result<()> {
    let role_account = &mut ctx.accounts.role_account;
    role_account.user = ctx.accounts.user.key();
    role_account.role = role;
    role_account.permission = AccessPermission::Compliant;
    role_account.bump = ctx.bumps.role_account;

    emit!(RoleAssigned {
        user: role_account.user,
        role
    });
    Ok(())
}

/// Revoke a role entirely and refund the account rent to the admin. Admin-only.
#[derive(Accounts)]
#[instruction(role: Role)]
pub struct RemoveRole<'info> {
    #[account(mut)]
    pub admin_signer: Signer<'info>,

    #[account(
        seeds = [ADMIN_SEED, admin_signer.key().as_ref()],
        bump = admin.bump,
    )]
    pub admin: Account<'info, Admin>,

    /// CHECK: the user losing the role; used as PDA seed only.
    pub user: UncheckedAccount<'info>,

    #[account(
        mut,
        close = admin_signer,
        seeds = [ROLE_SEED, user.key().as_ref(), &[role.seed_byte()]],
        bump = role_account.bump,
    )]
    pub role_account: Account<'info, RoleAccount>,
}

pub fn remove_role_handler(ctx: Context<RemoveRole>, role: Role) -> Result<()> {
    emit!(RoleRemoved {
        user: ctx.accounts.user.key(),
        role
    });
    Ok(())
}

/// Give up one's own role and refund the account rent to the authority. Holder-signed.
#[derive(Accounts)]
#[instruction(role: Role)]
pub struct RenounceRole<'info> {
    pub user: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,

    /// CHECK: rent destination, fixed to the configured authority.
    #[account(mut, address = config.authority)]
    pub authority: UncheckedAccount<'info>,

    #[account(
        mut,
        close = authority,
        seeds = [ROLE_SEED, user.key().as_ref(), &[role.seed_byte()]],
        bump = role_account.bump,
    )]
    pub role_account: Account<'info, RoleAccount>,
}

pub fn renounce_role_handler(ctx: Context<RenounceRole>, role: Role) -> Result<()> {
    emit!(RoleRemoved {
        user: ctx.accounts.user.key(),
        role
    });
    Ok(())
}

/// Update a user's compliance status for a role. Admin-only.
#[derive(Accounts)]
#[instruction(role: Role)]
pub struct SetPermission<'info> {
    pub admin_signer: Signer<'info>,

    #[account(
        seeds = [ADMIN_SEED, admin_signer.key().as_ref()],
        bump = admin.bump,
    )]
    pub admin: Account<'info, Admin>,

    /// CHECK: the user whose permission changes; used as PDA seed only.
    pub user: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [ROLE_SEED, user.key().as_ref(), &[role.seed_byte()]],
        bump = role_account.bump,
    )]
    pub role_account: Account<'info, RoleAccount>,
}

pub fn set_permission_handler(
    ctx: Context<SetPermission>,
    _role: Role,
    permission: AccessPermission,
) -> Result<()> {
    let role_account = &mut ctx.accounts.role_account;
    require!(
        role_account.permission != permission,
        RolesError::PermissionAlreadySet
    );

    role_account.permission = permission;
    emit!(PermissionUpdated {
        user: role_account.user,
        role: role_account.role,
        permission,
    });
    Ok(())
}

#[event]
pub struct RoleAssigned {
    pub user: Pubkey,
    pub role: Role,
}

#[event]
pub struct RoleRemoved {
    pub user: Pubkey,
    pub role: Role,
}

#[event]
pub struct PermissionUpdated {
    pub user: Pubkey,
    pub role: Role,
    pub permission: AccessPermission,
}
