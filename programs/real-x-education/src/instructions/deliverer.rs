use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, DELIVERER_SEED, VAULT_SEED};
use crate::error::EducationError;
use crate::state::{Config, Deliverer};
use crate::vault::{lock_to_vault, release_from_vault};

use xcavate_roles::program::XcavateRoles;
use xcavate_roles::state::{Config as RolesConfig, Role, RoleAccount};

/// Register as a module deliverer, or top the deposit back up to the current
/// requirement. ModuleDeliverer-only; locks the deposit in the vault.
#[derive(Accounts)]
pub struct RegisterDeliverer<'info> {
    #[account(mut)]
    pub deliverer_signer: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            deliverer_signer.key().as_ref(),
            &[Role::ModuleDeliverer.seed_byte()],
        ],
        bump = deliverer_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub deliverer_role: Box<Account<'info, RoleAccount>>,

    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = deliverer_signer,
    )]
    pub deliverer_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = deliverer_signer,
        space = 8 + Deliverer::INIT_SPACE,
        seeds = [DELIVERER_SEED, deliverer_signer.key().as_ref()],
        bump,
    )]
    pub deliverer: Box<Account<'info, Deliverer>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn register_deliverer_handler(ctx: Context<RegisterDeliverer>) -> Result<()> {
    let required = ctx.accounts.config.deliverer_deposit;
    let was_new = ctx.accounts.deliverer.deliverer == Pubkey::default();

    if was_new {
        let deliverer = &mut ctx.accounts.deliverer;
        deliverer.deliverer = ctx.accounts.deliverer_signer.key();
        deliverer.deposit = 0;
        deliverer.active_claims = 0;
        deliverer.active_strikes = 0;
        deliverer.successful_deliveries = 0;
        deliverer.bump = ctx.bumps.deliverer;
    }

    let additional = required.saturating_sub(ctx.accounts.deliverer.deposit);
    if additional > 0 {
        lock_to_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.deliverer_xcav.to_account_info(),
            &ctx.accounts.xcav_mint.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.deliverer_signer.to_account_info(),
            additional,
            ctx.accounts.xcav_mint.decimals,
        )?;
        ctx.accounts.deliverer.deposit = required;
    }

    if was_new {
        emit!(DelivererRegistered {
            deliverer: ctx.accounts.deliverer_signer.key(),
            deposit: ctx.accounts.deliverer.deposit,
        });
    } else if additional > 0 {
        emit!(DelivererDepositIncreased {
            deliverer: ctx.accounts.deliverer_signer.key(),
            new_deposit: required,
        });
    }
    Ok(())
}

/// Unregister as a deliverer and withdraw the deposit. Requires no active
/// claims, and renounces the ModuleDeliverer role if it is still held, so
/// strikes can't be shed by cycling out and back in. The role account is
/// optional so a deliverer who already gave it up can still get their deposit.
#[derive(Accounts)]
pub struct UnregisterDeliverer<'info> {
    #[account(mut)]
    pub deliverer_signer: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [
            xcavate_roles::ROLE_SEED,
            deliverer_signer.key().as_ref(),
            &[Role::ModuleDeliverer.seed_byte()],
        ],
        bump = deliverer_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub deliverer_role: Option<Box<Account<'info, RoleAccount>>>,

    #[account(
        seeds = [xcavate_roles::CONFIG_SEED],
        bump = roles_config.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub roles_config: Box<Account<'info, RolesConfig>>,

    /// CHECK: rent destination for the renounced role account, pinned to the
    /// roles program's configured authority.
    #[account(mut, address = roles_config.authority)]
    pub roles_authority: UncheckedAccount<'info>,

    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = deliverer_signer,
    )]
    pub deliverer_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        close = deliverer_signer,
        seeds = [DELIVERER_SEED, deliverer_signer.key().as_ref()],
        bump = deliverer.bump,
    )]
    pub deliverer: Box<Account<'info, Deliverer>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub roles_program: Program<'info, XcavateRoles>,
}

pub fn unregister_deliverer_handler(ctx: Context<UnregisterDeliverer>) -> Result<()> {
    require!(
        ctx.accounts.deliverer.active_claims == 0,
        EducationError::ModuleDelivererStillActive
    );

    release_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.deliverer_xcav.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        ctx.accounts.config.bump,
        ctx.accounts.deliverer.deposit,
        ctx.accounts.xcav_mint.decimals,
    )?;

    // Renounce the role if it is still held. A deliverer who already gave it
    // up reaches here with no role account and just recovers their deposit;
    // they still need a fresh admin grant to register again.
    if let Some(deliverer_role) = ctx.accounts.deliverer_role.as_ref() {
        xcavate_roles::cpi::renounce_role(
            CpiContext::new(
                ctx.accounts.roles_program.key(),
                xcavate_roles::cpi::accounts::RenounceRole {
                    user: ctx.accounts.deliverer_signer.to_account_info(),
                    config: ctx.accounts.roles_config.to_account_info(),
                    authority: ctx.accounts.roles_authority.to_account_info(),
                    role_account: deliverer_role.to_account_info(),
                },
            ),
            Role::ModuleDeliverer,
        )?;
    }

    emit!(DelivererUnregistered {
        deliverer: ctx.accounts.deliverer_signer.key(),
    });
    Ok(())
}

#[event]
pub struct DelivererRegistered {
    pub deliverer: Pubkey,
    pub deposit: u64,
}

#[event]
pub struct DelivererDepositIncreased {
    pub deliverer: Pubkey,
    pub new_deposit: u64,
}

#[event]
pub struct DelivererUnregistered {
    pub deliverer: Pubkey,
}
