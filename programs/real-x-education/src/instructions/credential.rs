use anchor_lang::prelude::*;

use crate::constants::{BOOKING_SEED, CONFIG_SEED, CREDENTIAL_SEED, MODULE_SEED};
use crate::error::EducationError;
use crate::state::{Booking, Config, Credential, CredentialKind, Module};

use xcavate_roles::state::{Role, RoleAccount};

/// Issue a non-transferable credential for a scored booking. ModuleAIAgent-only.
/// This handles the sponsor, school and lecturer attestations as well as the
/// per-student credentials; there's one record per (booking, kind, recipient).
#[derive(Accounts)]
#[instruction(module_id: u64, booking_id: u64, kind: CredentialKind, recipient: Pubkey)]
pub struct MintCredential<'info> {
    #[account(mut)]
    pub agent: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        seeds = [MODULE_SEED, &module_id.to_le_bytes()],
        bump = module.bump,
    )]
    pub module: Box<Account<'info, Module>>,

    #[account(
        seeds = [
            xcavate_roles::ROLE_SEED,
            agent.key().as_ref(),
            &[Role::ModuleAIAgent.seed_byte()],
        ],
        bump = agent_role.bump,
        seeds::program = xcavate_roles::ID,
        constraint = agent_role.is_compliant() @ EducationError::NotCompliant,
    )]
    pub agent_role: Box<Account<'info, RoleAccount>>,

    #[account(
        seeds = [BOOKING_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        bump = booking.bump,
    )]
    pub booking: Box<Account<'info, Booking>>,

    #[account(
        init,
        payer = agent,
        space = 8 + Credential::INIT_SPACE,
        seeds = [
            CREDENTIAL_SEED,
            &booking_id.to_le_bytes(),
            &[kind.seed_byte()],
            recipient.as_ref(),
        ],
        bump,
    )]
    pub credential: Box<Account<'info, Credential>>,

    pub system_program: Program<'info, System>,
}

pub fn mint_credential_handler(
    ctx: Context<MintCredential>,
    module_id: u64,
    booking_id: u64,
    kind: CredentialKind,
    recipient: Pubkey,
    uri: String,
) -> Result<()> {
    require!(
        uri.len() <= Credential::URI_MAX_LEN,
        EducationError::InvalidConfig
    );
    require!(
        ctx.accounts.booking.score.is_some(),
        EducationError::NoTestResultsSubmitted
    );

    // A student's credential carries their individual result; the sponsor,
    // school, and lecturer attestations don't record a score.
    let score = match kind {
        CredentialKind::Student => ctx.accounts.booking.score,
        _ => None,
    };

    let clock = Clock::get()?;
    let credential = &mut ctx.accounts.credential;
    credential.recipient = recipient;
    credential.module_id = module_id;
    credential.booking_id = booking_id;
    credential.kind = kind;
    credential.score = score;
    credential.issued_at = clock.unix_timestamp;
    credential.uri = uri;
    credential.bump = ctx.bumps.credential;

    emit!(CredentialIssued {
        module_id,
        booking_id,
        kind,
        recipient,
        score,
    });
    Ok(())
}

#[event]
pub struct CredentialIssued {
    pub module_id: u64,
    pub booking_id: u64,
    pub kind: CredentialKind,
    pub recipient: Pubkey,
    pub score: Option<u16>,
}
