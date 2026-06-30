//! Shared module-creation core.
//!
//! A module can be created two ways: directly by a creator, or by minting one
//! that's cleared community review. Both end the same way, so the deposit lock,
//! the supply mint, and writing the `Module` record live here and are driven by
//! a small `ModuleTerms` so each path only has to gather the values it creates
//! the module on.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{mint_to, Mint, MintTo, TokenAccount, TokenInterface};

use crate::constants::CONFIG_SEED;
use crate::state::Module;
use crate::vault::lock_to_vault;

/// The terms a module is created on, so the creation paths write the `Module`
/// identically.
pub struct ModuleTerms<'a> {
    pub module_id: u64,
    pub creator: Pubkey,
    pub region: u16,
    pub mint: Pubkey,
    pub deposit: u64,
    pub module_amount: u64,
    /// Tokens credited straight to the school allocation (a pre-sponsorship);
    /// zero for a plain creation.
    pub school_allocated: u64,
    pub price: u64,
    pub content_creator_bps: u16,
    pub regional_operator_bps: u16,
    pub protocol_bps: u16,
    pub dbs_bps: u16,
    pub created_at: i64,
    pub metadata: &'a str,
    pub bump: u8,
}

/// Lock the creator's module deposit in the vault.
pub fn lock_module_deposit<'info>(
    token_program: &Interface<'info, TokenInterface>,
    creator_xcav: &InterfaceAccount<'info, TokenAccount>,
    xcav_mint: &InterfaceAccount<'info, Mint>,
    vault: &InterfaceAccount<'info, TokenAccount>,
    creator: &Signer<'info>,
    deposit: u64,
) -> Result<()> {
    lock_to_vault(
        &token_program.to_account_info(),
        &creator_xcav.to_account_info(),
        &xcav_mint.to_account_info(),
        &vault.to_account_info(),
        &creator.to_account_info(),
        deposit,
        xcav_mint.decimals,
    )
}

/// Mint the full module supply into the module vault, signed by the config PDA.
pub fn mint_full_supply<'info>(
    token_program: &Interface<'info, TokenInterface>,
    module_mint: &InterfaceAccount<'info, Mint>,
    module_vault: &InterfaceAccount<'info, TokenAccount>,
    config: &AccountInfo<'info>,
    config_bump: u8,
    amount: u64,
) -> Result<()> {
    let bump = [config_bump];
    let signer_seeds: &[&[u8]] = &[CONFIG_SEED, &bump];
    mint_to(
        CpiContext::new_with_signer(
            token_program.key(),
            MintTo {
                mint: module_mint.to_account_info(),
                to: module_vault.to_account_info(),
                authority: config.clone(),
            },
            &[signer_seeds],
        ),
        amount,
    )
}

/// Populate a freshly created module from its terms. `school_allocated` is the
/// count pre-sponsored straight into the school allocation (zero for a plain
/// creation); `module_amount - school_allocated` always covers it because that's
/// enforced before either path reaches here.
pub fn write_module(module: &mut Module, terms: &ModuleTerms) {
    module.module_id = terms.module_id;
    module.creator = terms.creator;
    module.region = terms.region;
    module.mint = terms.mint;
    module.deposit = terms.deposit;
    module.total_token_amount = terms.module_amount;
    module.sponsor_allocation = terms.module_amount.saturating_sub(terms.school_allocated);
    module.school_allocation = terms.school_allocated;
    module.student_allocation = 0;
    module.price = terms.price;
    module.content_creator_bps = terms.content_creator_bps;
    module.regional_operator_bps = terms.regional_operator_bps;
    module.protocol_bps = terms.protocol_bps;
    module.dbs_bps = terms.dbs_bps;
    module.created_at = terms.created_at;
    module.metadata = terms.metadata.to_string();
    module.bump = terms.bump;
}
