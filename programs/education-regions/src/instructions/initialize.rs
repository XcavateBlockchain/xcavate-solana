use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, VAULT_SEED};
use crate::error::RegionsError;
use crate::state::Config;

/// Governance parameters for the regions program. The treasury is supplied as a
/// (validated) account rather than a raw key, so it can't be set to something
/// that isn't an XCAV token account.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct ConfigParams {
    pub proposal_deposit: u64,
    pub minimum_voting_amount: u64,
    pub minimum_region_deposit: u64,
    pub voting_period: i64,
    pub auction_period: i64,
    pub owner_change_period: i64,
    pub threshold_bps: u16,
    pub quorum: u64,
    pub removal_deposit: u64,
    pub removal_voting_period: i64,
    pub slash_amount: u64,
    pub notice_period: i64,
    pub allowed_strikes: u8,
}

impl ConfigParams {
    /// Reject obviously broken parameters up front. A threshold above 100% or a
    /// zero quorum/period would silently make every future proposal unwinnable,
    /// so it's worth catching at the point the authority sets them.
    fn validate(&self) -> Result<()> {
        require!(
            self.threshold_bps > 0 && self.threshold_bps <= 10_000,
            RegionsError::InvalidConfig
        );
        require!(self.quorum > 0, RegionsError::InvalidConfig);
        require!(
            self.voting_period > 0 && self.auction_period > 0 && self.owner_change_period > 0,
            RegionsError::InvalidConfig
        );
        require!(
            self.minimum_voting_amount > 0 && self.minimum_region_deposit > 0,
            RegionsError::InvalidConfig
        );
        require!(self.proposal_deposit > 0, RegionsError::InvalidConfig);
        require!(
            self.removal_deposit > 0 && self.slash_amount > 0,
            RegionsError::InvalidConfig
        );
        require!(
            self.removal_voting_period > 0 && self.notice_period > 0,
            RegionsError::InvalidConfig
        );
        require!(self.allowed_strikes > 0, RegionsError::InvalidConfig);
        Ok(())
    }

    fn apply(&self, config: &mut Config) {
        config.proposal_deposit = self.proposal_deposit;
        config.minimum_voting_amount = self.minimum_voting_amount;
        config.minimum_region_deposit = self.minimum_region_deposit;
        config.voting_period = self.voting_period;
        config.auction_period = self.auction_period;
        config.owner_change_period = self.owner_change_period;
        config.threshold_bps = self.threshold_bps;
        config.quorum = self.quorum;
        config.removal_deposit = self.removal_deposit;
        config.removal_voting_period = self.removal_voting_period;
        config.slash_amount = self.slash_amount;
        config.notice_period = self.notice_period;
        config.allowed_strikes = self.allowed_strikes;
    }
}

/// Creates the singleton config and sets the authority to the signer.
///
/// First caller becomes the authority, so run this in the same script that
/// deploys the program, otherwise someone could claim it in between. Before
/// mainnet, bind it to the program's upgrade authority by loading the
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

    /// The XCAV governance mint the protocol stakes.
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The treasury that receives slashed deposits; must be an XCAV account.
    #[account(token::mint = xcav_mint)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

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

    let config = &mut ctx.accounts.config;
    config.authority = ctx.accounts.authority.key();
    config.xcav_mint = ctx.accounts.xcav_mint.key();
    config.treasury = ctx.accounts.treasury.key();
    params.apply(config);
    config.proposal_counter = 0;
    config.bump = ctx.bumps.config;

    emit!(ConfigInitialized {
        authority: config.authority,
        xcav_mint: config.xcav_mint,
        treasury: config.treasury,
    });
    Ok(())
}

/// Update the governance parameters. Authority-only. Open proposals and auctions
/// keep the values they were created with; only future ones see the change. The
/// mint is fixed at initialization and can't be changed here.
#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = authority @ RegionsError::NotAuthority,
    )]
    pub config: Box<Account<'info, Config>>,

    /// The configured XCAV mint (the treasury is validated against it).
    #[account(address = config.xcav_mint @ RegionsError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The treasury that receives slashed deposits; must be an XCAV account.
    #[account(token::mint = config.xcav_mint)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,
}

pub fn update_config_handler(ctx: Context<UpdateConfig>, params: ConfigParams) -> Result<()> {
    params.validate()?;
    let treasury = ctx.accounts.treasury.key();
    let config = &mut ctx.accounts.config;
    config.treasury = treasury;
    params.apply(config);

    emit!(ConfigUpdated { treasury });
    Ok(())
}

/// Rotate the authority. Only the current authority may call this; gives a
/// recovery path if the key must change or is compromised.
#[derive(Accounts)]
pub struct UpdateAuthority<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = authority @ RegionsError::NotAuthority,
    )]
    pub config: Account<'info, Config>,
}

pub fn update_authority_handler(
    ctx: Context<UpdateAuthority>,
    new_authority: Pubkey,
) -> Result<()> {
    require!(new_authority != Pubkey::default(), RegionsError::InvalidConfig);
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
