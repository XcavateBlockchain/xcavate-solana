use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, VAULT_SEED};
use crate::error::EducationError;
use crate::state::Config;

/// The protocol-wide treasury: the regions program's treasury vault, so every
/// slash across the protocol lands in one pot. Transfers in need no authority;
/// only the regions program can pay out.
pub fn protocol_treasury() -> Pubkey {
    Pubkey::find_program_address(&[education_regions::TREASURY_SEED], &education_regions::ID).0
}

/// Protocol parameters. The treasury is supplied as a validated XCAV token
/// account so it can't be pointed at the wrong mint.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct ConfigParams {
    pub module_deposit: u64,
    pub booking_deposit: u64,
    pub deliverer_deposit: u64,
    pub module_price: u64,
    pub max_module_tokens: u64,
    pub content_creator_bps: u16,
    pub regional_operator_bps: u16,
    pub protocol_bps: u16,
    pub dbs_bps: u16,
    pub min_impact_score_bps: u16,
    pub sponsorship_window: i64,
    pub cancellation_window: i64,
    pub max_cancellations: u32,
    pub max_strikes: u8,
    pub strike_slash_bps: u16,
    pub deliveries_per_strike_reduction: u32,
    pub proposal_deposit: u64,
    pub minimum_voting_amount: u64,
    pub voting_period: i64,
    pub threshold_bps: u16,
    pub quorum: u64,
    pub pre_sponsor_amount: u64,
    pub claim_period: i64,
    pub upload_period: i64,
    pub accepted_assets: [Pubkey; 3],
}

impl ConfigParams {
    /// Catch broken parameters at the point they're set: a fee split over 100%,
    /// a slash rate over 100%, or non-positive windows would all silently break
    /// later instructions.
    fn validate(&self) -> Result<()> {
        let fee_total = (self.content_creator_bps as u32)
            + (self.regional_operator_bps as u32)
            + (self.protocol_bps as u32)
            + (self.dbs_bps as u32);
        require!(fee_total <= 10_000, EducationError::InvalidConfig);
        require!(
            self.strike_slash_bps <= 10_000
                && self.min_impact_score_bps <= 10_000
                && self.threshold_bps <= 10_000,
            EducationError::InvalidConfig
        );
        require!(
            self.sponsorship_window > 0
                && self.cancellation_window > 0
                && self.voting_period > 0
                && self.claim_period > 0
                && self.upload_period > 0,
            EducationError::InvalidConfig
        );
        require!(self.module_price > 0, EducationError::InvalidConfig);
        require!(self.max_module_tokens > 0, EducationError::InvalidConfig);
        require!(
            self.module_deposit > 0
                && self.booking_deposit > 0
                && self.deliverer_deposit > 0
                && self.proposal_deposit > 0,
            EducationError::InvalidConfig
        );
        Ok(())
    }

    fn apply(&self, config: &mut Config) {
        config.module_deposit = self.module_deposit;
        config.booking_deposit = self.booking_deposit;
        config.deliverer_deposit = self.deliverer_deposit;
        config.module_price = self.module_price;
        config.max_module_tokens = self.max_module_tokens;
        config.content_creator_bps = self.content_creator_bps;
        config.regional_operator_bps = self.regional_operator_bps;
        config.protocol_bps = self.protocol_bps;
        config.dbs_bps = self.dbs_bps;
        config.min_impact_score_bps = self.min_impact_score_bps;
        config.sponsorship_window = self.sponsorship_window;
        config.cancellation_window = self.cancellation_window;
        config.max_cancellations = self.max_cancellations;
        config.max_strikes = self.max_strikes;
        config.strike_slash_bps = self.strike_slash_bps;
        config.deliveries_per_strike_reduction = self.deliveries_per_strike_reduction;
        config.proposal_deposit = self.proposal_deposit;
        config.minimum_voting_amount = self.minimum_voting_amount;
        config.voting_period = self.voting_period;
        config.threshold_bps = self.threshold_bps;
        config.quorum = self.quorum;
        config.pre_sponsor_amount = self.pre_sponsor_amount;
        config.claim_period = self.claim_period;
        config.upload_period = self.upload_period;
        config.accepted_assets = self.accepted_assets;
    }
}

/// Creates the singleton config, sets the authority to the signer, and opens the
/// XCAV escrow vault. Only the program's upgrade authority can call this, so
/// the config can't be claimed by a front-runner between deploy and
/// initialization.
#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// This program's executable account, tying `program_data` to it.
    #[account(constraint = program.programdata_address()? == Some(program_data.key()) @ EducationError::NotUpgradeAuthority)]
    pub program: Program<'info, crate::program::RealXEducation>,

    /// The program's upgrade authority must be the initializing signer.
    #[account(constraint = program_data.upgrade_authority_address == Some(authority.key()) @ EducationError::NotUpgradeAuthority)]
    pub program_data: Account<'info, ProgramData>,

    #[account(
        init,
        payer = authority,
        space = 8 + Config::INIT_SPACE,
        seeds = [CONFIG_SEED],
        bump,
    )]
    pub config: Box<Account<'info, Config>>,

    /// The XCAV mint staked for deposits.
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The shared protocol treasury, pinned to the regions program's treasury
    /// vault; receives slashed deposits.
    #[account(
        token::mint = xcav_mint,
        address = protocol_treasury() @ EducationError::InvalidTreasury,
    )]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    /// Owner of the protocol's per-asset fee token accounts.
    /// CHECK: stored as a key; only used to validate fee recipients later.
    pub protocol_authority: UncheckedAccount<'info>,

    /// The protocol's XCAV escrow vault, owned by the config PDA.
    #[account(
        init,
        payer = authority,
        seeds = [VAULT_SEED],
        bump,
        token::mint = xcav_mint,
        token::authority = config,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<InitializeConfig>, params: ConfigParams) -> Result<()> {
    params.validate()?;
    crate::mint_guard::require_supported_mint(&ctx.accounts.xcav_mint.to_account_info())?;

    let config = &mut ctx.accounts.config;
    config.authority = ctx.accounts.authority.key();
    config.xcav_mint = ctx.accounts.xcav_mint.key();
    config.treasury = ctx.accounts.treasury.key();
    config.protocol_authority = ctx.accounts.protocol_authority.key();
    params.apply(config);
    config.next_module_id = 0;
    config.next_sponsor_id = 0;
    config.next_booking_id = 0;
    config.next_proposal_id = 0;
    config.bump = ctx.bumps.config;

    emit!(ConfigInitialized {
        authority: config.authority,
        xcav_mint: config.xcav_mint,
        treasury: config.treasury,
    });
    Ok(())
}

/// Update the protocol parameters. Authority-only. In-flight modules, bookings,
/// and proposals keep the values they were created with. The XCAV mint is fixed
/// at initialization.
#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = authority @ EducationError::NotAuthority,
    )]
    pub config: Box<Account<'info, Config>>,

    /// The configured XCAV mint (the treasury is validated against it).
    #[account(address = config.xcav_mint @ EducationError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The shared protocol treasury, pinned to the regions program's treasury
    /// vault; receives slashed deposits.
    #[account(
        token::mint = config.xcav_mint,
        address = protocol_treasury() @ EducationError::InvalidTreasury,
    )]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    /// Owner of the protocol's per-asset fee token accounts.
    /// CHECK: stored as a key; only used to validate fee recipients later.
    pub protocol_authority: UncheckedAccount<'info>,
}

pub fn update_config_handler(ctx: Context<UpdateConfig>, params: ConfigParams) -> Result<()> {
    params.validate()?;
    let treasury = ctx.accounts.treasury.key();
    let protocol_authority = ctx.accounts.protocol_authority.key();
    let config = &mut ctx.accounts.config;
    config.treasury = treasury;
    config.protocol_authority = protocol_authority;
    params.apply(config);

    emit!(ConfigUpdated { treasury });
    Ok(())
}

/// Rotate the authority. Only the current authority may call this.
#[derive(Accounts)]
pub struct UpdateAuthority<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = authority @ EducationError::NotAuthority,
    )]
    pub config: Account<'info, Config>,
}

pub fn update_authority_handler(
    ctx: Context<UpdateAuthority>,
    new_authority: Pubkey,
) -> Result<()> {
    require!(new_authority != Pubkey::default(), EducationError::InvalidConfig);
    let config = &mut ctx.accounts.config;
    let old_authority = config.authority;
    config.authority = new_authority;

    emit!(AuthorityUpdated { old_authority, new_authority });
    Ok(())
}

#[event]
pub struct ConfigInitialized {
    pub authority: Pubkey,
    pub xcav_mint: Pubkey,
    pub treasury: Pubkey,
}

#[event]
pub struct ConfigUpdated {
    pub treasury: Pubkey,
}

#[event]
pub struct AuthorityUpdated {
    pub old_authority: Pubkey,
    pub new_authority: Pubkey,
}
