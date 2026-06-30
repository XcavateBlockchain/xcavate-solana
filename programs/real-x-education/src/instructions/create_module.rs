use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, MODULE_MINT_SEED, MODULE_SEED, MODULE_VAULT_SEED, VAULT_SEED};
use crate::error::EducationError;
use crate::minting::{lock_module_deposit, mint_full_supply, write_module, ModuleTerms};
use crate::state::{Config, Module};

use education_regions::state::Region;
use xcavate_roles::state::{Role, RoleAccount};

/// Create a learning module. The caller must be a ModuleCreator and
/// the region must already exist. A fresh token mint is fractionalized into
/// `module_amount` shares held in the module vault, all credited to the creator
/// as the initial sponsor allocation, and the creator's XCAV deposit is locked
/// in the vault.
#[derive(Accounts)]
#[instruction(region: u16, module_amount: u64, metadata: String)]
pub struct CreateModule<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    /// The XCAV mint (deposit is pulled from the creator in XCAV).
    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The creator's XCAV account the deposit is pulled from.
    #[account(
        mut,
        token::mint = config.xcav_mint,
        token::authority = creator,
    )]
    pub creator_xcav: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The protocol's XCAV escrow vault.
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
        token::mint = config.xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The caller's ModuleCreator role, owned by the roles program.
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

    /// The region the module is scoped to; must already be created. Loading it
    /// from the regions program is what proves the region exists.
    #[account(
        seeds = [education_regions::REGION_SEED, &region.to_le_bytes()],
        bump = region_account.bump,
        seeds::program = education_regions::ID,
    )]
    pub region_account: Box<Account<'info, Region>>,

    #[account(
        init,
        payer = creator,
        space = 8 + Module::INIT_SPACE,
        seeds = [MODULE_SEED, config.next_module_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub module: Box<Account<'info, Module>>,

    /// The fractionalized token mint, owned (mint authority) by the config PDA.
    #[account(
        init,
        payer = creator,
        seeds = [MODULE_MINT_SEED, config.next_module_id.to_le_bytes().as_ref()],
        bump,
        mint::decimals = 0,
        mint::authority = config,
        mint::token_program = token_program,
    )]
    pub module_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The module's token vault; receives and custodies the full supply.
    #[account(
        init,
        payer = creator,
        seeds = [MODULE_VAULT_SEED, config.next_module_id.to_le_bytes().as_ref()],
        bump,
        token::mint = module_mint,
        token::authority = config,
        token::token_program = token_program,
    )]
    pub module_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn create_module_handler(
    ctx: Context<CreateModule>,
    region: u16,
    module_amount: u64,
    metadata: String,
) -> Result<()> {
    require!(module_amount > 0, EducationError::AmountCannotBeZero);
    require!(
        module_amount <= ctx.accounts.config.max_module_tokens,
        EducationError::TooManyTokens
    );
    require!(
        metadata.len() <= Module::METADATA_MAX_LEN,
        EducationError::InvalidConfig
    );

    let config = &ctx.accounts.config;
    let module_id = config.next_module_id;
    let deposit = config.module_deposit;
    let config_bump = config.bump;
    let now = Clock::get()?.unix_timestamp;
    // Gather the creation terms from config up front, before it's bumped below.
    let terms = ModuleTerms {
        module_id,
        creator: ctx.accounts.creator.key(),
        region,
        mint: ctx.accounts.module_mint.key(),
        deposit,
        module_amount,
        school_allocated: 0,
        price: config.module_price,
        content_creator_bps: config.content_creator_bps,
        regional_operator_bps: config.regional_operator_bps,
        protocol_bps: config.protocol_bps,
        dbs_bps: config.dbs_bps,
        created_at: now,
        metadata: &metadata,
        bump: ctx.bumps.module,
    };

    lock_module_deposit(
        &ctx.accounts.token_program,
        &ctx.accounts.creator_xcav,
        &ctx.accounts.xcav_mint,
        &ctx.accounts.vault,
        &ctx.accounts.creator,
        deposit,
    )?;
    // The config PDA is the mint authority; the creator's whole supply starts as
    // their sponsor allocation.
    mint_full_supply(
        &ctx.accounts.token_program,
        &ctx.accounts.module_mint,
        &ctx.accounts.module_vault,
        &ctx.accounts.config.to_account_info(),
        config_bump,
        module_amount,
    )?;

    write_module(&mut ctx.accounts.module, &terms);
    ctx.accounts.config.next_module_id =
        module_id.checked_add(1).ok_or(EducationError::Overflow)?;

    emit!(LearningModuleCreated {
        module_id,
        creator: ctx.accounts.creator.key(),
        region,
        mint: ctx.accounts.module_mint.key(),
        token_amount: module_amount,
        metadata,
        created_at: now,
    });
    Ok(())
}

#[event]
pub struct LearningModuleCreated {
    pub module_id: u64,
    pub creator: Pubkey,
    pub region: u16,
    pub mint: Pubkey,
    pub token_amount: u64,
    pub metadata: String,
    pub created_at: i64,
}
