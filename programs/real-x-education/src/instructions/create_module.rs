use anchor_lang::prelude::*;
use anchor_spl::token_interface::{mint_to, Mint, MintTo, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, MODULE_MINT_SEED, MODULE_SEED, MODULE_VAULT_SEED, VAULT_SEED};
use crate::error::EducationError;
use crate::state::{Config, Module};
use crate::vault::lock_to_vault;

use education_regions::state::Region;
use xcavate_roles::state::{Role, RoleAccount};

/// Create a learning module. The caller must be a compliant ModuleCreator and
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
        constraint = creator_role.is_compliant() @ EducationError::NotCompliant,
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

    let clock = Clock::get()?;
    let module_id = ctx.accounts.config.next_module_id;
    let deposit = ctx.accounts.config.module_deposit;
    let config_bump = ctx.accounts.config.bump;

    // Lock the creator's XCAV deposit in the vault.
    lock_to_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.creator_xcav.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.creator.to_account_info(),
        deposit,
        ctx.accounts.xcav_mint.decimals,
    )?;

    // Mint the full supply into the module vault. The config PDA is the mint
    // authority, and the creator's share is tracked as the sponsor allocation.
    let bump = [config_bump];
    let signer_seeds: &[&[u8]] = &[CONFIG_SEED, &bump];
    mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            MintTo {
                mint: ctx.accounts.module_mint.to_account_info(),
                to: ctx.accounts.module_vault.to_account_info(),
                authority: ctx.accounts.config.to_account_info(),
            },
            &[signer_seeds],
        ),
        module_amount,
    )?;

    let config = &mut ctx.accounts.config;
    let module = &mut ctx.accounts.module;
    module.module_id = module_id;
    module.creator = ctx.accounts.creator.key();
    module.region = region;
    module.mint = ctx.accounts.module_mint.key();
    module.deposit = deposit;
    module.total_token_amount = module_amount;
    module.sponsor_allocation = module_amount;
    module.school_allocation = 0;
    module.student_allocation = 0;
    module.price = config.module_price;
    module.content_creator_bps = config.content_creator_bps;
    module.regional_operator_bps = config.regional_operator_bps;
    module.protocol_bps = config.protocol_bps;
    module.dbs_bps = config.dbs_bps;
    module.created_at = clock.unix_timestamp;
    module.metadata = metadata.clone();
    module.bump = ctx.bumps.module;

    config.next_module_id = module_id.checked_add(1).ok_or(EducationError::Overflow)?;

    emit!(LearningModuleCreated {
        module_id,
        creator: module.creator,
        region,
        mint: module.mint,
        token_amount: module_amount,
        metadata,
        created_at: module.created_at,
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
