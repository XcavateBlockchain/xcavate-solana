use anchor_lang::prelude::*;

use crate::constants::{ADMIN_SEED, CONFIG_SEED};
use crate::error::RolesError;
use crate::state::{Admin, Config};

/// Register a new whitelist admin. Sudo-only.
#[derive(Accounts)]
pub struct AddAdmin<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = authority @ RolesError::NotAuthority,
    )]
    pub config: Account<'info, Config>,

    /// CHECK: only used as the admin identity / PDA seed; no data read.
    pub new_admin: UncheckedAccount<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + Admin::INIT_SPACE,
        seeds = [ADMIN_SEED, new_admin.key().as_ref()],
        bump,
    )]
    pub admin: Account<'info, Admin>,

    pub system_program: Program<'info, System>,
}

pub fn add_admin_handler(ctx: Context<AddAdmin>) -> Result<()> {
    let admin = &mut ctx.accounts.admin;
    admin.admin = ctx.accounts.new_admin.key();
    admin.bump = ctx.bumps.admin;

    emit!(AdminRegistered { admin: admin.admin });
    Ok(())
}

/// Remove a whitelist admin and refund their account rent. Sudo-only.
#[derive(Accounts)]
pub struct RemoveAdmin<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = authority @ RolesError::NotAuthority,
    )]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        close = authority,
        seeds = [ADMIN_SEED, admin.admin.as_ref()],
        bump = admin.bump,
    )]
    pub admin: Account<'info, Admin>,
}

pub fn remove_admin_handler(ctx: Context<RemoveAdmin>) -> Result<()> {
    emit!(AdminRemoved { admin: ctx.accounts.admin.admin });
    Ok(())
}

#[event]
pub struct AdminRegistered {
    pub admin: Pubkey,
}

#[event]
pub struct AdminRemoved {
    pub admin: Pubkey,
}
