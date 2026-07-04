use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, TREASURY_SEED, VAULT_SEED};
use crate::error::RegionsError;
use crate::state::Config;

/// Governance parameters for the regions program. The treasury is supplied as a
/// (validated) account rather than a raw key, so it can't be set to something
/// that isn't an XCAV token account.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct ConfigParams {
    pub minimum_voting_amount: u64,
    pub voting_period: i64,
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
            self.voting_period > 0 && self.owner_change_period > 0,
            RegionsError::InvalidConfig
        );
        require!(self.minimum_voting_amount > 0, RegionsError::InvalidConfig);
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
        config.minimum_voting_amount = self.minimum_voting_amount;
        config.voting_period = self.voting_period;
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

/// Creates the singleton config and sets the authority to the signer. Only
/// the program's upgrade authority can call this, so the config can't be
/// claimed by a front-runner between deploy and initialization.
#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// This program's executable account, tying `program_data` to it.
    #[account(constraint = program.programdata_address()? == Some(program_data.key()) @ RegionsError::NotUpgradeAuthority)]
    pub program: Program<'info, crate::program::EducationRegions>,

    /// The program's upgrade authority must be the initializing signer.
    #[account(constraint = program_data.upgrade_authority_address == Some(authority.key()) @ RegionsError::NotUpgradeAuthority)]
    pub program_data: Account<'info, ProgramData>,

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

    /// The protocol treasury: a program-owned XCAV account that collects
    /// slashes (from this program and real-x) and funds proposer rewards.
    /// Anyone can pay in; only this program can pay out.
    #[account(
        init,
        payer = authority,
        seeds = [TREASURY_SEED],
        bump,
        token::mint = xcav_mint,
        token::authority = config,
    )]
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
    crate::mint_guard::require_supported_mint(&ctx.accounts.xcav_mint.to_account_info())?;

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

/// Update the governance parameters. Authority-only. Each proposal snapshots
/// its deposit and expiry, but threshold, quorum and slash amount are read live
/// at finalization, so a change reaches proposals already in flight. The mint
/// and treasury are fixed at initialization.
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
}

pub fn update_config_handler(ctx: Context<UpdateConfig>, params: ConfigParams) -> Result<()> {
    params.validate()?;
    let config = &mut ctx.accounts.config;
    params.apply(config);

    emit!(ConfigUpdated {
        treasury: config.treasury
    });
    Ok(())
}

/// Move funds out of the protocol treasury. Authority-only; the destination
/// just has to be an XCAV account, so the authority decides where collected
/// slashes go.
#[derive(Accounts)]
pub struct WithdrawTreasury<'info> {
    pub authority: Signer<'info>,

    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = authority @ RegionsError::NotAuthority,
    )]
    pub config: Box<Account<'info, Config>>,

    #[account(address = config.xcav_mint @ RegionsError::InvalidMint)]
    pub xcav_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(mut, address = config.treasury @ RegionsError::InvalidTreasury)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(mut, token::mint = config.xcav_mint)]
    pub destination: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn withdraw_treasury_handler(ctx: Context<WithdrawTreasury>, amount: u64) -> Result<()> {
    require!(amount > 0, RegionsError::InvalidConfig);
    crate::vault::release_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.treasury.to_account_info(),
        &ctx.accounts.xcav_mint.to_account_info(),
        &ctx.accounts.destination.to_account_info(),
        &ctx.accounts.config.to_account_info(),
        ctx.accounts.config.bump,
        amount,
        ctx.accounts.xcav_mint.decimals,
    )?;

    emit!(TreasuryWithdrawn {
        destination: ctx.accounts.destination.key(),
        amount
    });
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
    require!(
        new_authority != Pubkey::default(),
        RegionsError::InvalidConfig
    );
    let config = &mut ctx.accounts.config;
    let old_authority = config.authority;
    config.authority = new_authority;

    emit!(AuthorityUpdated {
        old_authority,
        new_authority
    });
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
pub struct TreasuryWithdrawn {
    pub destination: Pubkey,
    pub amount: u64,
}

#[event]
pub struct AuthorityUpdated {
    pub old_authority: Pubkey,
    pub new_authority: Pubkey,
}
