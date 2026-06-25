use anchor_lang::prelude::*;

use crate::constants::CONFIG_SEED;
use crate::error::RolesError;
use crate::state::Config;

/// Creates the singleton config and sets the sudo authority to the signer.
///
/// First caller becomes the sudo authority, so run this in the same script
/// that deploys the program — otherwise someone could claim it in between.
/// Before mainnet, bind it to the program's upgrade authority by loading the
/// `ProgramData` account and checking `upgrade_authority_address`.
#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + Config::INIT_SPACE,
        seeds = [CONFIG_SEED],
        bump,
    )]
    pub config: Account<'info, Config>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<InitializeConfig>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.authority = ctx.accounts.authority.key();
    config.bump = ctx.bumps.config;

    emit!(ConfigInitialized { authority: config.authority });
    Ok(())
}

/// Rotates the sudo authority. Only the current authority may call this.
/// Gives a recovery path if the sudo key must change or is compromised.
#[derive(Accounts)]
pub struct UpdateAuthority<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = authority @ RolesError::NotAuthority,
    )]
    pub config: Account<'info, Config>,
}

pub fn update_authority_handler(
    ctx: Context<UpdateAuthority>,
    new_authority: Pubkey,
) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let old_authority = config.authority;
    config.authority = new_authority;

    emit!(AuthorityUpdated { old_authority, new_authority });
    Ok(())
}

#[event]
pub struct ConfigInitialized {
    pub authority: Pubkey,
}

#[event]
pub struct AuthorityUpdated {
    pub old_authority: Pubkey,
    pub new_authority: Pubkey,
}
