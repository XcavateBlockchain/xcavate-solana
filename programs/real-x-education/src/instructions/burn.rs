use anchor_lang::prelude::*;
use anchor_spl::token_interface::{burn, Burn, Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, MODULE_SEED, MODULE_VAULT_SEED, VAULT_SEED};
use crate::error::EducationError;
use crate::state::{Config, Module};
use crate::vault::{close_vault_account, release_from_vault};

use xcavate_roles::state::{Role, RoleAccount};

/// Burn tokens from a module's unsponsored allocation. ModuleCreator-only and
/// limited to the creator's own module; permanently shrinks the supply.
#[derive(Accounts)]
#[instruction(module_id: u64)]
pub struct BurnUnsponsored<'info> {
    pub creator: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [MODULE_SEED, &module_id.to_le_bytes()],
        bump = module.bump,
        constraint = module.creator == creator.key() @ EducationError::NoPermission,
    )]
    pub module: Box<Account<'info, Module>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            creator.key().as_ref(),
            &[Role::ModuleCreator.seed_byte()],
        ],
        bump = creator_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub creator_role: Box<Account<'info, RoleAccount>>,

    #[account(mut, address = module.mint @ EducationError::InvalidMint)]
    pub module_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [MODULE_VAULT_SEED, &module_id.to_le_bytes()],
        bump,
        token::mint = module_mint,
        token::authority = config,
    )]
    pub module_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn burn_unsponsored_handler(
    ctx: Context<BurnUnsponsored>,
    _module_id: u64,
    amount: u64,
) -> Result<()> {
    require!(amount > 0, EducationError::AmountCannotBeZero);
    require!(
        amount <= ctx.accounts.module.sponsor_allocation,
        EducationError::CannotBurnMoreThanAvailable
    );

    let bump = [ctx.accounts.config.bump];
    let signer_seeds: &[&[u8]] = &[CONFIG_SEED, &bump];
    burn(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Burn {
                mint: ctx.accounts.module_mint.to_account_info(),
                from: ctx.accounts.module_vault.to_account_info(),
                authority: ctx.accounts.config.to_account_info(),
            },
            &[signer_seeds],
        ),
        amount,
    )?;

    let module = &mut ctx.accounts.module;
    module.sponsor_allocation = module
        .sponsor_allocation
        .checked_sub(amount)
        .ok_or(EducationError::Underflow)?;

    emit!(UnsponsoredTokensBurned {
        module_id: module.module_id,
        creator: module.creator,
        amount,
        remaining_allocation: module.sponsor_allocation,
    });
    Ok(())
}

#[event]
pub struct UnsponsoredTokensBurned {
    pub module_id: u64,
    pub creator: Pubkey,
    pub amount: u64,
    pub remaining_allocation: u64,
}

/// Remove a fully-retired module and refund the creator's deposit.
/// ModuleCreator-only and limited to the creator's own module; only once every
/// token has been burned (the vault is empty). The emptied vault is closed back
/// to the creator; the zero-supply mint is left in place.
#[derive(Accounts)]
#[instruction(module_id: u64)]
pub struct RemoveModule<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        close = creator,
        seeds = [MODULE_SEED, &module_id.to_le_bytes()],
        bump = module.bump,
        constraint = module.creator == creator.key() @ EducationError::NoPermission,
    )]
    pub module: Box<Account<'info, Module>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            creator.key().as_ref(),
            &[Role::ModuleCreator.seed_byte()],
        ],
        bump = creator_role.bump,
        seeds::program = xcavate_roles::ID,
    )]
    pub creator_role: Box<Account<'info, RoleAccount>>,

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
        token::authority = creator,
    )]
    pub creator_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [MODULE_VAULT_SEED, &module_id.to_le_bytes()],
        bump,
        token::mint = module.mint,
        token::authority = config,
    )]
    pub module_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn remove_module_handler(ctx: Context<RemoveModule>, _module_id: u64) -> Result<()> {
    require!(
        ctx.accounts.module_vault.amount == 0,
        EducationError::CannotRemoveModuleWithActiveTokens
    );

    release_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.creator_xcav.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        ctx.accounts.config.bump,
        ctx.accounts.module.deposit,
        ctx.accounts.xcav_mint.decimals,
    )?;

    // The vault is empty, so close it and return its rent to the creator. The
    // zero-supply mint can't be closed under the classic token program, so it
    // stays in place.
    close_vault_account(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.module_vault.to_account_info(),
        &ctx.accounts.creator.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        ctx.accounts.config.bump,
    )?;

    emit!(ModuleRemoved {
        module_id: ctx.accounts.module.module_id,
        creator: ctx.accounts.creator.key(),
    });
    Ok(())
}

#[event]
pub struct ModuleRemoved {
    pub module_id: u64,
    pub creator: Pubkey,
}
