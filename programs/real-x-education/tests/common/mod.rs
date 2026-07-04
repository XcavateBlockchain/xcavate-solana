//! Shared test scaffolding: the `World` harness, instruction builders, PDA and
//! token helpers, and the send/assert helpers (`ok`, `err`).
//!
//! XCAV (deposits) and the payment assets are classic SPL tokens; the mints and
//! each participant's token account are seeded directly with `set_account`. The
//! roles program is loaded so the role gates run for real, and a created
//! `Region` is seeded directly so we don't have to drive the regions governance
//! flow here. Each test file pulls this in with `mod common; use common::*;`.
//!
//! Each test file is its own binary that uses a subset of this, so unused
//! helpers and re-exports are expected.
#![allow(dead_code, unused_imports)]

use anchor_lang::solana_program::program_option::COption;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_lang::{
    solana_program::instruction::Instruction, AccountSerialize, InstructionData, ToAccountMetas,
};
use anchor_spl::token::spl_token::state::{Account as SplAccount, AccountState, Mint as SplMint};
use anchor_spl::token::ID as TOKEN_PROGRAM_ID;
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use solana_account::Account;
use solana_message::{Message, VersionedMessage};
use solana_transaction::versioned::VersionedTransaction;

use real_x_education::{
    BOOKING_SEED, BOOK_ESCROW_SEED, CANCELLATION_SEED, CANCEL_COUNTER_SEED, CONFIG_SEED,
    CREDENTIAL_SEED, DELIVERER_SEED, MODULE_MINT_SEED, MODULE_PROPOSAL_SEED, MODULE_SEED,
    MODULE_VAULT_SEED, PROPOSAL_ESCROW_SEED, PROPOSAL_VOTE_SEED, SPONSORSHIP_SEED,
    SPONSOR_ESCROW_SEED, VAULT_SEED,
};

// Re-exported so each test file gets them through `use common::*`.
pub use anchor_lang::prelude::Pubkey;
pub use anchor_lang::solana_program::clock::Clock;
pub use anchor_lang::AccountDeserialize;
pub use education_regions::state::Region;
pub use litesvm::LiteSVM;
pub use real_x_education::instructions::ConfigParams;
pub use real_x_education::state::{
    Booking, Config, Credential, CredentialKind, Deliverer, Module, ModuleProposal, ModuleVote,
    ProposalStatus, Sponsorship,
};
pub use solana_keypair::Keypair;
pub use solana_signer::Signer;
pub use xcavate_roles::state::Role;

pub const SYS: Pubkey = anchor_lang::system_program::ID;
pub const DECIMALS: u8 = 6;
pub const FUND: u64 = 1_000_000_000_000;

pub const MODULE_DEPOSIT: u64 = 1_000_000_000;
pub const BOOKING_DEPOSIT: u64 = 500_000_000;
pub const DELIVERER_DEPOSIT: u64 = 2_000_000_000;
pub const MODULE_PRICE: u64 = 100; // whole units
pub const PER_TOKEN: u64 = 125_000_000; // base 100e6 + 25% fees, at 6 decimals

// A second payment asset with different decimals, for multi-asset coverage.
pub const GBP_DECIMALS: u8 = 2;
pub const PER_TOKEN_GBP: u64 = 12_500; // base 10_000 + 25% fees, at 2 decimals

pub fn eid() -> Pubkey {
    real_x_education::id()
}
pub fn roles_id() -> Pubkey {
    xcavate_roles::id()
}
// The protocol-wide treasury: the regions program's treasury vault.
pub fn treasury_pda() -> Pubkey {
    Pubkey::find_program_address(&[education_regions::TREASURY_SEED], &regions_id()).0
}

pub fn treasury_balance(svm: &LiteSVM) -> u64 {
    spl_amount(svm, &treasury_pda())
}

// Seed the shared treasury token account (owned by the regions config PDA).
pub fn set_treasury(svm: &mut LiteSVM) {
    let regions_config =
        Pubkey::find_program_address(&[education_regions::CONFIG_SEED], &regions_id()).0;
    let acc = SplAccount {
        mint: xcav_mint(),
        owner: regions_config,
        amount: 0,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0u8; SplAccount::LEN];
    acc.pack_into_slice(&mut data);
    svm.set_account(
        treasury_pda(),
        Account {
            lamports: 100_000_000,
            data,
            owner: TOKEN_PROGRAM_ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

pub fn regions_id() -> Pubkey {
    education_regions::id()
}

// --- real-x-education PDAs ---
pub fn config_pda() -> Pubkey {
    Pubkey::find_program_address(&[CONFIG_SEED], &eid()).0
}
pub fn vault() -> Pubkey {
    Pubkey::find_program_address(&[VAULT_SEED], &eid()).0
}
pub fn module_pda(id: u64) -> Pubkey {
    Pubkey::find_program_address(&[MODULE_SEED, &id.to_le_bytes()], &eid()).0
}
pub fn module_mint_pda(id: u64) -> Pubkey {
    Pubkey::find_program_address(&[MODULE_MINT_SEED, &id.to_le_bytes()], &eid()).0
}
pub fn module_vault_pda(id: u64) -> Pubkey {
    Pubkey::find_program_address(&[MODULE_VAULT_SEED, &id.to_le_bytes()], &eid()).0
}
pub fn sponsorship_pda(module_id: u64, sponsor_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            SPONSORSHIP_SEED,
            &module_id.to_le_bytes(),
            &sponsor_id.to_le_bytes(),
        ],
        &eid(),
    )
    .0
}
pub fn sponsor_escrow_pda(module_id: u64, sponsor_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            SPONSOR_ESCROW_SEED,
            &module_id.to_le_bytes(),
            &sponsor_id.to_le_bytes(),
        ],
        &eid(),
    )
    .0
}
pub fn booking_pda(module_id: u64, booking_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            BOOKING_SEED,
            &module_id.to_le_bytes(),
            &booking_id.to_le_bytes(),
        ],
        &eid(),
    )
    .0
}
pub fn book_escrow_pda(module_id: u64, booking_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            BOOK_ESCROW_SEED,
            &module_id.to_le_bytes(),
            &booking_id.to_le_bytes(),
        ],
        &eid(),
    )
    .0
}
pub fn deliverer_pda(who: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[DELIVERER_SEED, who.as_ref()], &eid()).0
}
pub fn counter_pda(school: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[CANCEL_COUNTER_SEED, school.as_ref()], &eid()).0
}
pub fn cancellation_pda(school: &Pubkey, booking_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            CANCELLATION_SEED,
            school.as_ref(),
            &booking_id.to_le_bytes(),
        ],
        &eid(),
    )
    .0
}
pub fn credential_pda(booking_id: u64, kind: CredentialKind, recipient: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            CREDENTIAL_SEED,
            &booking_id.to_le_bytes(),
            &[kind.seed_byte()],
            recipient.as_ref(),
        ],
        &eid(),
    )
    .0
}

// --- roles PDAs ---
pub fn roles_config() -> Pubkey {
    Pubkey::find_program_address(&[xcavate_roles::CONFIG_SEED], &roles_id()).0
}
pub fn admin_pda(who: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[xcavate_roles::ADMIN_SEED, who.as_ref()], &roles_id()).0
}
pub fn role_pda(user: &Pubkey, role: Role) -> Pubkey {
    Pubkey::find_program_address(
        &[xcavate_roles::ROLE_SEED, user.as_ref(), &[role.seed_byte()]],
        &roles_id(),
    )
    .0
}
pub fn region_pda(region_id: u16) -> Pubkey {
    Pubkey::find_program_address(
        &[education_regions::REGION_SEED, &region_id.to_le_bytes()],
        &regions_id(),
    )
    .0
}
pub fn regions_config() -> Pubkey {
    Pubkey::find_program_address(&[education_regions::CONFIG_SEED], &regions_id()).0
}
pub fn regions_vault() -> Pubkey {
    Pubkey::find_program_address(&[education_regions::VAULT_SEED], &regions_id()).0
}
pub fn region_state_pda(region_id: u16) -> Pubkey {
    Pubkey::find_program_address(
        &[
            education_regions::REGION_STATE_SEED,
            &region_id.to_le_bytes(),
        ],
        &regions_id(),
    )
    .0
}
pub fn region_proposal_pda(proposal_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[education_regions::PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        &regions_id(),
    )
    .0
}
pub fn region_vote_pda(proposal_id: u64, voter: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            education_regions::VOTE_SEED,
            &proposal_id.to_le_bytes(),
            voter.as_ref(),
        ],
        &regions_id(),
    )
    .0
}
pub fn proposal_pda(proposal_id: u64) -> Pubkey {
    Pubkey::find_program_address(&[MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()], &eid()).0
}
pub fn proposal_vote_pda(proposal_id: u64, voter: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            PROPOSAL_VOTE_SEED,
            &proposal_id.to_le_bytes(),
            voter.as_ref(),
        ],
        &eid(),
    )
    .0
}
pub fn proposal_escrow_pda(proposal_id: u64) -> Pubkey {
    Pubkey::find_program_address(&[PROPOSAL_ESCROW_SEED, &proposal_id.to_le_bytes()], &eid()).0
}

// --- mints / token accounts ---
pub fn xcav_mint() -> Pubkey {
    Pubkey::new_from_array([7u8; 32])
}
pub fn usdc_mint() -> Pubkey {
    Pubkey::new_from_array([9u8; 32])
}
pub fn gbp_mint() -> Pubkey {
    Pubkey::new_from_array([11u8; 32])
}
pub fn token_acc(mint: &Pubkey, owner: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"tok", mint.as_ref(), owner.as_ref()], &eid()).0
}

pub fn set_mint(svm: &mut LiteSVM, mint: Pubkey) {
    set_mint_dec(svm, mint, DECIMALS);
}

pub fn set_mint_dec(svm: &mut LiteSVM, mint: Pubkey, decimals: u8) {
    let m = SplMint {
        mint_authority: COption::None,
        // Only the XCAV supply is read (by the regions operator bond, 0.1% of
        // supply); this value makes that bond 1e9. Harmless for the other mints.
        supply: 1_000_000_000_000,
        decimals,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    let mut data = vec![0u8; SplMint::LEN];
    m.pack_into_slice(&mut data);
    svm.set_account(
        mint,
        Account {
            lamports: 100_000_000,
            data,
            owner: TOKEN_PROGRAM_ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

pub fn give(svm: &mut LiteSVM, mint: &Pubkey, owner: &Pubkey, amount: u64) {
    let acc = SplAccount {
        mint: *mint,
        owner: *owner,
        amount,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0u8; SplAccount::LEN];
    acc.pack_into_slice(&mut data);
    svm.set_account(
        token_acc(mint, owner),
        Account {
            lamports: 100_000_000,
            data,
            owner: TOKEN_PROGRAM_ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

pub fn balance(svm: &LiteSVM, mint: &Pubkey, owner: &Pubkey) -> u64 {
    let acc = svm.get_account(&token_acc(mint, owner)).unwrap();
    SplAccount::unpack(&acc.data).unwrap().amount
}

pub fn spl_amount(svm: &LiteSVM, addr: &Pubkey) -> u64 {
    let acc = svm.get_account(addr).unwrap();
    SplAccount::unpack(&acc.data).unwrap().amount
}

// --- send helpers ---
pub fn process(
    svm: &mut LiteSVM,
    ix: Instruction,
    payer: &Keypair,
    signers: &[&Keypair],
) -> Result<TransactionMetadata, FailedTransactionMetadata> {
    svm.expire_blockhash();
    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx)
}

pub fn ok(svm: &mut LiteSVM, ix: Instruction, payer: &Keypair, signers: &[&Keypair]) {
    if let Err(failed) = process(svm, ix, payer, signers) {
        panic!(
            "expected success, failed with: {:?}\n{}",
            failed.err,
            failed.meta.logs.join("\n")
        );
    }
}

/// Run an instruction expecting it to fail, and assert the logs name `code`.
pub fn err(svm: &mut LiteSVM, ix: Instruction, payer: &Keypair, signers: &[&Keypair], code: &str) {
    match process(svm, ix, payer, signers) {
        Ok(_) => panic!("expected failure with {code}, but it succeeded"),
        Err(failed) => {
            let logs = failed.meta.logs.join("\n");
            assert!(
                logs.contains(code),
                "expected error {code}, got logs:\n{logs}"
            );
        }
    }
}

/// Send an instruction expecting success and return the compute units it burned.
pub fn send_cu(svm: &mut LiteSVM, ix: Instruction, payer: &Keypair, signers: &[&Keypair]) -> u64 {
    match process(svm, ix, payer, signers) {
        Ok(meta) => meta.compute_units_consumed,
        Err(failed) => {
            panic!(
                "expected success, failed with: {:?}\n{}",
                failed.err,
                failed.meta.logs.join("\n")
            )
        }
    }
}

pub fn funded(svm: &mut LiteSVM) -> Keypair {
    let kp = Keypair::new();
    svm.airdrop(&kp.pubkey(), 100_000_000_000).unwrap();
    kp
}

/// SOL-funded keypair holding XCAV and USDC, with both token accounts seeded.
pub fn actor(svm: &mut LiteSVM) -> Keypair {
    let kp = funded(svm);
    give(svm, &xcav_mint(), &kp.pubkey(), FUND);
    give(svm, &usdc_mint(), &kp.pubkey(), FUND);
    give(svm, &gbp_mint(), &kp.pubkey(), FUND);
    kp
}

// --- roles ix builders ---
pub fn roles_init_ix(authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        roles_id(),
        &xcavate_roles::instruction::InitializeConfig {}.data(),
        xcavate_roles::accounts::InitializeConfig {
            authority: *authority,
            program: roles_id(),
            program_data: program_data_pda(&roles_id()),
            config: roles_config(),
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}
pub fn roles_add_admin_ix(authority: &Pubkey, new_admin: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        roles_id(),
        &xcavate_roles::instruction::AddAdmin {}.data(),
        xcavate_roles::accounts::AddAdmin {
            authority: *authority,
            config: roles_config(),
            new_admin: *new_admin,
            admin: admin_pda(new_admin),
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}
pub fn roles_remove_ix(admin: &Pubkey, user: &Pubkey, role: Role) -> Instruction {
    Instruction::new_with_bytes(
        roles_id(),
        &xcavate_roles::instruction::RemoveRole { role }.data(),
        xcavate_roles::accounts::RemoveRole {
            admin_signer: *admin,
            admin: admin_pda(admin),
            user: *user,
            role_account: role_pda(user, role),
        }
        .to_account_metas(None),
    )
}

pub fn roles_assign_ix(admin: &Pubkey, user: &Pubkey, role: Role) -> Instruction {
    Instruction::new_with_bytes(
        roles_id(),
        &xcavate_roles::instruction::AssignRole { role }.data(),
        xcavate_roles::accounts::AssignRole {
            admin_signer: *admin,
            admin: admin_pda(admin),
            user: *user,
            role_account: role_pda(user, role),
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

// --- education ix builders ---
pub fn default_params() -> ConfigParams {
    ConfigParams {
        module_deposit: MODULE_DEPOSIT,
        booking_deposit: BOOKING_DEPOSIT,
        deliverer_deposit: DELIVERER_DEPOSIT,
        module_price: MODULE_PRICE,
        max_module_tokens: 1_000,
        content_creator_bps: 830,
        regional_operator_bps: 830,
        protocol_bps: 500,
        dbs_bps: 340,
        min_impact_score_bps: 5_000,
        sponsorship_window: 1_000,
        cancellation_window: 1_000,
        no_show_grace: 1_000,
        max_cancellations: 3,
        max_strikes: 3,
        strike_slash_bps: 1_000,
        deliveries_per_strike_reduction: 5,
        proposal_deposit: MODULE_DEPOSIT,
        minimum_voting_amount: 1_000,
        voting_period: 1_000,
        threshold_bps: 5_000,
        quorum: 10_000,
        pre_sponsor_amount: 2,
        claim_period: 1_000,
        upload_period: 1_000,
        accepted_assets: [usdc_mint(), gbp_mint(), Pubkey::default()],
    }
}

// The programdata account the upgradeable loader keeps beside each program.
pub fn program_data_pda(program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[program_id.as_ref()],
        &anchor_lang::solana_program::bpf_loader_upgradeable::ID,
    )
    .0
}

// Point a deployed program's upgrade authority at `authority` so the
// authority-bound initialize passes. The loader metadata is 4 bytes of enum
// tag, 8 of slot, then an optional pubkey.
pub fn bind_upgrade_authority(svm: &mut LiteSVM, program_id: &Pubkey, authority: &Pubkey) {
    let pd = program_data_pda(program_id);
    let mut acc = svm.get_account(&pd).unwrap();
    acc.data[12] = 1;
    acc.data[13..45].copy_from_slice(authority.as_ref());
    svm.set_account(pd, acc).unwrap();
}

pub fn edu_init_ix(authority: &Pubkey, protocol: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::InitializeConfig {
            params: default_params(),
        }
        .data(),
        real_x_education::accounts::InitializeConfig {
            authority: *authority,
            program: eid(),
            program_data: program_data_pda(&eid()),
            config: config_pda(),
            xcav_mint: xcav_mint(),
            treasury: treasury_pda(),
            protocol_authority: *protocol,
            vault: vault(),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn create_module_ix(creator: &Pubkey, region: u16, module_id: u64, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::CreateModule {
            region,
            module_amount: amount,
            metadata: "ipfs://m".to_string(),
        }
        .data(),
        real_x_education::accounts::CreateModule {
            creator: *creator,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            creator_xcav: token_acc(&xcav_mint(), creator),
            vault: vault(),
            creator_role: role_pda(creator, Role::ModuleCreator),
            region_account: region_pda(region),
            module: module_pda(module_id),
            module_mint: module_mint_pda(module_id),
            module_vault: module_vault_pda(module_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn sponsor_ix(sponsor: &Pubkey, module_id: u64, sponsor_id: u64, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::SponsorModule {
            module_id,
            token_amount: amount,
        }
        .data(),
        real_x_education::accounts::SponsorModule {
            sponsor: *sponsor,
            config: config_pda(),
            module: module_pda(module_id),
            sponsor_role: role_pda(sponsor, Role::ModuleSponsor),
            payment_mint: usdc_mint(),
            sponsor_payment: token_acc(&usdc_mint(), sponsor),
            sponsorship: sponsorship_pda(module_id, sponsor_id),
            sponsor_escrow: sponsor_escrow_pda(module_id, sponsor_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

/// Far-future delivery time for bookings that are never scored, so the
/// delivery/score gate never blocks booking-mechanics tests.
pub const NEVER_DELIVER: i64 = i64::MAX;

/// The current on-chain timestamp; use as `delivery_at` when a test books and
/// then scores at the same clock.
pub fn now_ts(svm: &LiteSVM) -> i64 {
    svm.get_sysvar::<Clock>().unix_timestamp
}

pub fn book_ix(school: &Pubkey, module_id: u64, sponsor_id: u64, booking_id: u64) -> Instruction {
    book_ix_at(school, module_id, sponsor_id, booking_id, NEVER_DELIVER)
}

pub fn book_ix_at(
    school: &Pubkey,
    module_id: u64,
    sponsor_id: u64,
    booking_id: u64,
    delivery_at: i64,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::BookModule {
            module_id,
            sponsor_id,
            delivery_at,
            metadata: "ipfs://b".to_string(),
        }
        .data(),
        real_x_education::accounts::BookModule {
            school: *school,
            config: config_pda(),
            module: module_pda(module_id),
            school_role: role_pda(school, Role::ModuleBooker),
            xcav_mint: xcav_mint(),
            school_xcav: token_acc(&xcav_mint(), school),
            vault: vault(),
            sponsorship: sponsorship_pda(module_id, sponsor_id),
            payment_mint: usdc_mint(),
            sponsor_escrow: sponsor_escrow_pda(module_id, sponsor_id),
            booking: booking_pda(module_id, booking_id),
            booking_escrow: book_escrow_pda(module_id, booking_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn register_deliverer_ix(lecturer: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::RegisterModuleDeliverer {}.data(),
        real_x_education::accounts::RegisterDeliverer {
            deliverer_signer: *lecturer,
            config: config_pda(),
            deliverer_role: role_pda(lecturer, Role::ModuleDeliverer),
            xcav_mint: xcav_mint(),
            deliverer_xcav: token_acc(&xcav_mint(), lecturer),
            vault: vault(),
            deliverer: deliverer_pda(lecturer),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn claim_ix(lecturer: &Pubkey, module_id: u64, booking_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ClaimBooking {
            module_id,
            booking_id,
        }
        .data(),
        real_x_education::accounts::ClaimBooking {
            lecturer: *lecturer,
            config: config_pda(),
            module: module_pda(module_id),
            lecturer_role: role_pda(lecturer, Role::ModuleDeliverer),
            deliverer: deliverer_pda(lecturer),
            booking: booking_pda(module_id, booking_id),
        }
        .to_account_metas(None),
    )
}

pub fn cancel_claim_ix(lecturer: &Pubkey, module_id: u64, booking_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::CancelClaim {
            module_id,
            booking_id,
        }
        .data(),
        real_x_education::accounts::CancelClaim {
            lecturer: *lecturer,
            config: config_pda(),
            module: module_pda(module_id),
            lecturer_role: role_pda(lecturer, Role::ModuleDeliverer),
            deliverer: deliverer_pda(lecturer),
            booking: booking_pda(module_id, booking_id),
            xcav_mint: xcav_mint(),
            vault: vault(),
            treasury: treasury_pda(),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

/// Permissionless no-show expiry. `lecturer` is the claimant being struck.
pub fn expire_claim_ix(
    cranker: &Pubkey,
    lecturer: &Pubkey,
    module_id: u64,
    booking_id: u64,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ExpireClaim {
            module_id,
            booking_id,
        }
        .data(),
        real_x_education::accounts::ExpireClaim {
            cranker: *cranker,
            config: config_pda(),
            module: module_pda(module_id),
            lecturer: *lecturer,
            deliverer: deliverer_pda(lecturer),
            booking: booking_pda(module_id, booking_id),
            xcav_mint: xcav_mint(),
            vault: vault(),
            treasury: treasury_pda(),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn submit_score_ix(
    agent: &Pubkey,
    module_id: u64,
    booking_id: u64,
    score: u16,
    region: u16,
    creator: &Pubkey,
    operator: &Pubkey,
    protocol: &Pubkey,
    lecturer: &Pubkey,
    sponsor: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::SubmitImpactScore {
            module_id,
            booking_id,
            score,
        }
        .data(),
        real_x_education::accounts::SubmitImpactScore {
            agent: *agent,
            config: config_pda(),
            module: module_pda(module_id),
            agent_role: role_pda(agent, Role::ModuleAIAgent),
            booking: booking_pda(module_id, booking_id),
            region_account: region_pda(region),
            payment_mint: usdc_mint(),
            module_mint: module_mint_pda(module_id),
            module_vault: module_vault_pda(module_id),
            booking_escrow: book_escrow_pda(module_id, booking_id),
            creator_payment: token_acc(&usdc_mint(), creator),
            regional_operator_payment: token_acc(&usdc_mint(), operator),
            protocol_payment: token_acc(&usdc_mint(), protocol),
            lecturer_payment: token_acc(&usdc_mint(), lecturer),
            deliverer: deliverer_pda(lecturer),
            sponsor_payment: token_acc(&usdc_mint(), sponsor),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn finish_ix(school: &Pubkey, module_id: u64, booking_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::FinishBookingProcess {
            module_id,
            booking_id,
        }
        .data(),
        real_x_education::accounts::FinishBooking {
            school: *school,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            school_xcav: token_acc(&xcav_mint(), school),
            booking: booking_pda(module_id, booking_id),
            booking_escrow: book_escrow_pda(module_id, booking_id),
            sponsorship: sponsorship_pda(module_id, 0),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn reclaim_sponsorship_ix(
    sponsor: &Pubkey,
    module_id: u64,
    sponsor_id: u64,
    amount: u64,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ReclaimSponsorship {
            module_id,
            sponsor_id,
            amount,
        }
        .data(),
        real_x_education::accounts::ReclaimSponsorship {
            sponsor: *sponsor,
            config: config_pda(),
            module: module_pda(module_id),
            sponsorship: sponsorship_pda(module_id, sponsor_id),
            payment_mint: usdc_mint(),
            sponsor_escrow: sponsor_escrow_pda(module_id, sponsor_id),
            sponsor_payment: token_acc(&usdc_mint(), sponsor),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn close_sponsorship_ix(sponsor: &Pubkey, module_id: u64, sponsor_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::CloseSponsorship {
            module_id,
            sponsor_id,
        }
        .data(),
        real_x_education::accounts::CloseSponsorship {
            sponsor: *sponsor,
            config: config_pda(),
            sponsorship: sponsorship_pda(module_id, sponsor_id),
            sponsor_escrow: sponsor_escrow_pda(module_id, sponsor_id),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

// --- governance ix builders ---
pub fn create_proposal_ix(
    proposer: &Pubkey,
    role: Role,
    region: u16,
    proposal_id: u64,
    amount: u64,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::CreateModuleProposal {
            role,
            region,
            module_amount: amount,
            metadata: "ipfs://p".to_string(),
        }
        .data(),
        real_x_education::accounts::CreateModuleProposal {
            proposer: *proposer,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            proposer_xcav: token_acc(&xcav_mint(), proposer),
            vault: vault(),
            proposer_role: role_pda(proposer, role),
            region_account: region_pda(region),
            proposal: proposal_pda(proposal_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn vote_ix(voter: &Pubkey, proposal_id: u64, vote: ModuleVote, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::VoteOnProposal {
            proposal_id,
            vote,
            amount,
        }
        .data(),
        real_x_education::accounts::VoteOnProposal {
            voter: *voter,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            voter_xcav: token_acc(&xcav_mint(), voter),
            vault: vault(),
            proposal: proposal_pda(proposal_id),
            vote_record: proposal_vote_pda(proposal_id, voter),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn finalize_proposal_ix(cranker: &Pubkey, proposal_id: u64, proposer: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::FinalizeProposal { proposal_id }.data(),
        real_x_education::accounts::FinalizeProposal {
            cranker: *cranker,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            proposal: proposal_pda(proposal_id),
            proposer: *proposer,
            proposer_xcav: token_acc(&xcav_mint(), proposer),
            treasury: treasury_pda(),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn claim_proposal_ix(creator: &Pubkey, proposal_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ClaimProposal { proposal_id }.data(),
        real_x_education::accounts::ClaimProposal {
            creator: *creator,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            creator_xcav: token_acc(&xcav_mint(), creator),
            vault: vault(),
            creator_role: role_pda(creator, Role::ModuleCreator),
            proposal: proposal_pda(proposal_id),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn upload_proposal_ix(claimant: &Pubkey, proposal_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::UploadProposal {
            proposal_id,
            content_uri: "ipfs://c".to_string(),
        }
        .data(),
        real_x_education::accounts::UploadProposal {
            claimant: *claimant,
            proposal: proposal_pda(proposal_id),
        }
        .to_account_metas(None),
    )
}

pub fn release_claim_ix(cranker: &Pubkey, proposal_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ReleaseClaim { proposal_id }.data(),
        real_x_education::accounts::ReleaseClaim {
            cranker: *cranker,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            treasury: treasury_pda(),
            proposal: proposal_pda(proposal_id),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn review_proposal_ix(agent: &Pubkey, proposal_id: u64, passed: bool) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ReviewProposal {
            proposal_id,
            passed,
        }
        .data(),
        real_x_education::accounts::ReviewProposal {
            agent: *agent,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            treasury: treasury_pda(),
            agent_role: role_pda(agent, Role::ModuleAIAgent),
            proposal: proposal_pda(proposal_id),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn mint_proposed_ix(creator: &Pubkey, proposal_id: u64, module_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::MintProposedModule { proposal_id }.data(),
        real_x_education::accounts::MintProposedModule {
            creator: *creator,
            config: config_pda(),
            proposal: proposal_pda(proposal_id),
            proposer: *creator,
            creator_role: role_pda(creator, Role::ModuleCreator),
            module: module_pda(module_id),
            module_mint: module_mint_pda(module_id),
            module_vault: module_vault_pda(module_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn create_sponsor_proposal_ix(
    proposer: &Pubkey,
    region: u16,
    proposal_id: u64,
    amount: u64,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::CreateSponsorProposal {
            region,
            module_amount: amount,
            metadata: "ipfs://sp".to_string(),
        }
        .data(),
        real_x_education::accounts::CreateSponsorProposal {
            proposer: *proposer,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            proposer_xcav: token_acc(&xcav_mint(), proposer),
            vault: vault(),
            sponsor_role: role_pda(proposer, Role::ModuleSponsor),
            region_account: region_pda(region),
            payment_mint: usdc_mint(),
            proposer_payment: token_acc(&usdc_mint(), proposer),
            proposal: proposal_pda(proposal_id),
            proposal_escrow: proposal_escrow_pda(proposal_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn mint_sponsored_ix(
    creator: &Pubkey,
    proposer: &Pubkey,
    proposal_id: u64,
    module_id: u64,
    sponsor_id: u64,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::MintSponsoredModule { proposal_id }.data(),
        real_x_education::accounts::MintSponsoredModule {
            creator: *creator,
            config: config_pda(),
            proposal: proposal_pda(proposal_id),
            proposer: *proposer,
            creator_role: role_pda(creator, Role::ModuleCreator),
            module: module_pda(module_id),
            module_mint: module_mint_pda(module_id),
            module_vault: module_vault_pda(module_id),
            payment_mint: usdc_mint(),
            proposal_escrow: proposal_escrow_pda(proposal_id),
            sponsorship: sponsorship_pda(module_id, sponsor_id),
            sponsor_escrow: sponsor_escrow_pda(module_id, sponsor_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn unlock_vote_ix(voter: &Pubkey, proposal_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::UnlockProposalVote { proposal_id }.data(),
        real_x_education::accounts::UnlockProposalVote {
            voter: *voter,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            voter_xcav: token_acc(&xcav_mint(), voter),
            vault: vault(),
            vote_record: proposal_vote_pda(proposal_id, voter),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn expire_proposal_ix(cranker: &Pubkey, proposal_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ExpireProposal { proposal_id }.data(),
        real_x_education::accounts::ExpireProposal {
            cranker: *cranker,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            treasury: treasury_pda(),
            proposal: proposal_pda(proposal_id),
            claimant_xcav: None,
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

/// `expire_proposal` supplying the claimant's XCAV account, so an `UnderReview`
/// proposal's bond is refunded to them rather than slashed.
pub fn expire_proposal_refund_ix(
    cranker: &Pubkey,
    proposal_id: u64,
    claimant: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ExpireProposal { proposal_id }.data(),
        real_x_education::accounts::ExpireProposal {
            cranker: *cranker,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            treasury: treasury_pda(),
            proposal: proposal_pda(proposal_id),
            claimant_xcav: Some(token_acc(&xcav_mint(), claimant)),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn reclaim_pre_sponsor_ix(sponsor: &Pubkey, proposal_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ReclaimPreSponsor { proposal_id }.data(),
        real_x_education::accounts::ReclaimPreSponsor {
            sponsor: *sponsor,
            config: config_pda(),
            proposal: proposal_pda(proposal_id),
            payment_mint: usdc_mint(),
            sponsor_payment: token_acc(&usdc_mint(), sponsor),
            proposal_escrow: proposal_escrow_pda(proposal_id),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

/// Push the clock past the proposal voting window so finalize can run.
pub fn warp_past_voting(svm: &mut LiteSVM) {
    warp(svm, 2_000);
}

/// Advance the clock by `secs` seconds.
pub fn warp(svm: &mut LiteSVM, secs: i64) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp += secs;
    svm.set_sysvar(&clock);
}

// --- world ---
pub struct World {
    pub svm: LiteSVM,
    pub admin: Keypair,
    pub authority: Keypair,
    pub protocol: Keypair,
    pub operator: Keypair,
}

// --- regions governance builders (for the cross-program e2e) ---

pub fn regions_default_params() -> education_regions::instructions::ConfigParams {
    education_regions::instructions::ConfigParams {
        minimum_voting_amount: 100_000_000,
        voting_period: 1_000,
        owner_change_period: 10_000,
        threshold_bps: 5_000,
        quorum: 100_000_000,
        removal_deposit: 1_000_000_000,
        removal_voting_period: 1_000,
        slash_amount: 100_000_000,
        notice_period: 5_000,
        allowed_strikes: 3,
    }
}

pub fn regions_init_ix(authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        regions_id(),
        &education_regions::instruction::InitializeConfig {
            params: regions_default_params(),
        }
        .data(),
        education_regions::accounts::InitializeConfig {
            authority: *authority,
            program: regions_id(),
            program_data: program_data_pda(&regions_id()),
            config: regions_config(),
            xcav_mint: xcav_mint(),
            treasury: treasury_pda(),
            vault: regions_vault(),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn region_propose_ix(proposer: &Pubkey, region_id: u16, proposal_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        regions_id(),
        &education_regions::instruction::ProposeNewRegion { region_id }.data(),
        education_regions::accounts::ProposeNewRegion {
            proposer: *proposer,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            proposer_token: token_acc(&xcav_mint(), proposer),
            vault: regions_vault(),
            operator_role: role_pda(proposer, Role::RegionalOperator),
            region: region_pda(region_id),
            region_state: region_state_pda(region_id),
            proposal: region_proposal_pda(proposal_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn region_vote_ix(
    voter: &Pubkey,
    region_id: u16,
    proposal_id: u64,
    amount: u64,
) -> Instruction {
    Instruction::new_with_bytes(
        regions_id(),
        &education_regions::instruction::VoteOnRegionProposal {
            region_id,
            vote: education_regions::state::Vote::Yes,
            amount,
        }
        .data(),
        education_regions::accounts::VoteOnRegionProposal {
            voter: *voter,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            voter_token: token_acc(&xcav_mint(), voter),
            vault: regions_vault(),
            region_state: region_state_pda(region_id),
            proposal: region_proposal_pda(proposal_id),
            vote_record: region_vote_pda(proposal_id, voter),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn region_finalize_ix(
    cranker: &Pubkey,
    region_id: u16,
    proposal_id: u64,
    proposer: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        regions_id(),
        &education_regions::instruction::FinalizeRegionProposal { region_id }.data(),
        education_regions::accounts::FinalizeRegionProposal {
            cranker: *cranker,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            vault: regions_vault(),
            region_state: region_state_pda(region_id),
            proposal: region_proposal_pda(proposal_id),
            proposer: *proposer,
            proposer_token: Some(token_acc(&xcav_mint(), proposer)),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn region_create_ix(creator: &Pubkey, region_id: u16) -> Instruction {
    Instruction::new_with_bytes(
        regions_id(),
        &education_regions::instruction::CreateRegion { region_id }.data(),
        education_regions::accounts::CreateRegion {
            creator: *creator,
            config: regions_config(),
            creator_role: role_pda(creator, Role::RegionalOperator),
            region_state: region_state_pda(region_id),
            region: region_pda(region_id),
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

/// Schedule the current operator's departure, opening the seat after the notice period.
pub fn resign_ix(operator: &Pubkey, region_id: u16) -> Instruction {
    Instruction::new_with_bytes(
        regions_id(),
        &education_regions::instruction::InitiateResignation { region_id }.data(),
        education_regions::accounts::InitiateResignation {
            operator: *operator,
            config: regions_config(),
            operator_role: role_pda(operator, Role::RegionalOperator),
            region: region_pda(region_id),
        }
        .to_account_metas(None),
    )
}

/// Claim an existing region whose seat is open, bonding 0.1% of XCAV supply and
/// refunding the outgoing operator's collateral.
pub fn region_claim_open_ix(
    new_operator: &Pubkey,
    region_id: u16,
    old_owner: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        regions_id(),
        &education_regions::instruction::ClaimOpenRegion { region_id }.data(),
        education_regions::accounts::ClaimOpenRegion {
            new_operator: *new_operator,
            config: regions_config(),
            operator_role: role_pda(new_operator, Role::RegionalOperator),
            xcav_mint: xcav_mint(),
            new_operator_token: token_acc(&xcav_mint(), new_operator),
            vault: regions_vault(),
            region: region_pda(region_id),
            old_owner_token: token_acc(&xcav_mint(), old_owner),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

/// Drive the regions governance flow end to end so `region_id` is created by
/// the regions program (not seeded), owned by `operator`: propose (bonding),
/// pass the vote, finalize, and claim the passed region.
pub fn govern_region(w: &mut World, operator: &Keypair, region_id: u16) {
    let admin = w.admin.insecure_clone();
    ok(
        &mut w.svm,
        roles_assign_ix(&admin.pubkey(), &operator.pubkey(), Role::RegionalOperator),
        &admin,
        &[&admin],
    );

    let proposal_id = regions_proposal_counter(&w.svm);
    ok(
        &mut w.svm,
        region_propose_ix(&operator.pubkey(), region_id, proposal_id),
        operator,
        &[operator],
    );

    let voter = actor(&mut w.svm);
    ok(
        &mut w.svm,
        region_vote_ix(&voter.pubkey(), region_id, proposal_id, 200_000_000),
        &voter,
        &[&voter],
    );

    warp(&mut w.svm, 2_000); // past the voting window
    let cranker = funded(&mut w.svm);
    ok(
        &mut w.svm,
        region_finalize_ix(
            &cranker.pubkey(),
            region_id,
            proposal_id,
            &operator.pubkey(),
        ),
        &cranker,
        &[&cranker],
    );

    // The proposer claims the passed region, becoming its operator.
    ok(
        &mut w.svm,
        region_create_ix(&operator.pubkey(), region_id),
        operator,
        &[operator],
    );
}

pub fn regions_proposal_counter(svm: &LiteSVM) -> u64 {
    let acc = svm.get_account(&regions_config()).unwrap();
    let cfg = education_regions::state::Config::try_deserialize(&mut &acc.data[..]).unwrap();
    cfg.proposal_counter
}

/// Like `setup`, but initializes the regions config (creating a real treasury
/// and vault) instead of seeding a Region, so a test can drive the governance
/// flow to create one. The returned `operator` holds no role yet.
pub fn setup_governed() -> World {
    let mut svm = LiteSVM::new();
    svm.add_program(
        roles_id(),
        include_bytes!("../../../../target/deploy/xcavate_roles.so"),
    )
    .unwrap();
    svm.add_program(
        regions_id(),
        include_bytes!("../../../../target/deploy/education_regions.so"),
    )
    .unwrap();
    svm.add_program(
        eid(),
        include_bytes!("../../../../target/deploy/real_x_education.so"),
    )
    .unwrap();
    set_mint(&mut svm, xcav_mint());
    set_mint(&mut svm, usdc_mint());
    set_mint_dec(&mut svm, gbp_mint(), GBP_DECIMALS);

    let authority = funded(&mut svm);
    bind_upgrade_authority(&mut svm, &roles_id(), &authority.pubkey());
    bind_upgrade_authority(&mut svm, &regions_id(), &authority.pubkey());
    bind_upgrade_authority(&mut svm, &eid(), &authority.pubkey());
    ok(
        &mut svm,
        roles_init_ix(&authority.pubkey()),
        &authority,
        &[&authority],
    );

    let admin = funded(&mut svm);
    ok(
        &mut svm,
        roles_add_admin_ix(&authority.pubkey(), &admin.pubkey()),
        &authority,
        &[&authority],
    );

    // Regions init creates the shared treasury and the regions vault for real.
    ok(
        &mut svm,
        regions_init_ix(&authority.pubkey()),
        &authority,
        &[&authority],
    );

    let operator = actor(&mut svm);
    let protocol = actor(&mut svm);
    ok(
        &mut svm,
        edu_init_ix(&authority.pubkey(), &protocol.pubkey()),
        &authority,
        &[&authority],
    );

    World {
        svm,
        admin,
        authority,
        protocol,
        operator,
    }
}

/// Loads the three programs, seeds mints, a created region, and the education
/// config. The region operator is seeded directly into a `Region` account.
pub fn setup() -> World {
    let mut svm = LiteSVM::new();
    svm.add_program(
        roles_id(),
        include_bytes!("../../../../target/deploy/xcavate_roles.so"),
    )
    .unwrap();
    svm.add_program(
        regions_id(),
        include_bytes!("../../../../target/deploy/education_regions.so"),
    )
    .unwrap();
    svm.add_program(
        eid(),
        include_bytes!("../../../../target/deploy/real_x_education.so"),
    )
    .unwrap();
    set_mint(&mut svm, xcav_mint());
    set_mint(&mut svm, usdc_mint());
    set_mint_dec(&mut svm, gbp_mint(), GBP_DECIMALS);

    let authority = funded(&mut svm);
    bind_upgrade_authority(&mut svm, &roles_id(), &authority.pubkey());
    bind_upgrade_authority(&mut svm, &eid(), &authority.pubkey());
    // The shared protocol treasury lives at the regions program's treasury
    // PDA; seed an empty XCAV account there.
    set_treasury(&mut svm);
    ok(
        &mut svm,
        roles_init_ix(&authority.pubkey()),
        &authority,
        &[&authority],
    );

    let admin = funded(&mut svm);
    ok(
        &mut svm,
        roles_add_admin_ix(&authority.pubkey(), &admin.pubkey()),
        &authority,
        &[&authority],
    );

    // Region operator: seed a created Region directly + give them a USDC account.
    let operator = actor(&mut svm);
    let (region_addr, bump) = Pubkey::find_program_address(
        &[education_regions::REGION_SEED, &1u16.to_le_bytes()],
        &regions_id(),
    );
    let region = Region {
        region_id: 1,
        owner: operator.pubkey(),
        collateral: 0,
        active_strikes: 0,
        next_owner_change: 0,
        bump,
    };
    let mut data = Vec::new();
    region.try_serialize(&mut data).unwrap();
    svm.set_account(
        region_addr,
        Account {
            lamports: 10_000_000,
            data,
            owner: regions_id(),
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();

    let protocol = actor(&mut svm);
    ok(
        &mut svm,
        edu_init_ix(&authority.pubkey(), &protocol.pubkey()),
        &authority,
        &[&authority],
    );

    World {
        svm,
        admin,
        authority,
        protocol,
        operator,
    }
}

pub fn with_role(w: &mut World, role: Role) -> Keypair {
    let kp = actor(&mut w.svm);
    let admin = w.admin.insecure_clone();
    ok(
        &mut w.svm,
        roles_assign_ix(&admin.pubkey(), &kp.pubkey(), role),
        &admin,
        &[&admin],
    );
    kp
}

pub fn config(svm: &LiteSVM) -> Config {
    Config::try_deserialize(&mut &svm.get_account(&config_pda()).unwrap().data[..]).unwrap()
}
pub fn module_of(svm: &LiteSVM, id: u64) -> Module {
    Module::try_deserialize(&mut &svm.get_account(&module_pda(id)).unwrap().data[..]).unwrap()
}
pub fn deliverer_of(svm: &LiteSVM, who: &Pubkey) -> Deliverer {
    Deliverer::try_deserialize(&mut &svm.get_account(&deliverer_pda(who)).unwrap().data[..])
        .unwrap()
}
pub fn proposal_of(svm: &LiteSVM, id: u64) -> ModuleProposal {
    ModuleProposal::try_deserialize(&mut &svm.get_account(&proposal_pda(id)).unwrap().data[..])
        .unwrap()
}
pub fn booking_of(svm: &LiteSVM, module_id: u64, booking_id: u64) -> Booking {
    Booking::try_deserialize(
        &mut &svm
            .get_account(&booking_pda(module_id, booking_id))
            .unwrap()
            .data[..],
    )
    .unwrap()
}
pub fn sponsorship_of(svm: &LiteSVM, module_id: u64, sponsor_id: u64) -> Sponsorship {
    Sponsorship::try_deserialize(
        &mut &svm
            .get_account(&sponsorship_pda(module_id, sponsor_id))
            .unwrap()
            .data[..],
    )
    .unwrap()
}
pub fn credential_of(
    svm: &LiteSVM,
    booking_id: u64,
    kind: CredentialKind,
    recipient: &Pubkey,
) -> Credential {
    Credential::try_deserialize(
        &mut &svm
            .get_account(&credential_pda(booking_id, kind, recipient))
            .unwrap()
            .data[..],
    )
    .unwrap()
}
/// Whether an account has been closed (gone, or zero-length data).
pub fn closed(svm: &LiteSVM, addr: &Pubkey) -> bool {
    svm.get_account(addr)
        .map(|a| a.data.is_empty())
        .unwrap_or(true)
}

// --- extra ix builders ---
pub fn update_config_ix(
    authority: &Pubkey,
    protocol: &Pubkey,
    params: ConfigParams,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::UpdateConfig { params }.data(),
        real_x_education::accounts::UpdateConfig {
            authority: *authority,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            treasury: treasury_pda(),
            protocol_authority: *protocol,
        }
        .to_account_metas(None),
    )
}

pub fn update_authority_ix(authority: &Pubkey, new_authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::UpdateAuthority {
            new_authority: *new_authority,
        }
        .data(),
        real_x_education::accounts::UpdateAuthority {
            authority: *authority,
            config: config_pda(),
        }
        .to_account_metas(None),
    )
}

pub fn burn_ix(creator: &Pubkey, module_id: u64, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::BurnUnsponsoredToken { module_id, amount }.data(),
        real_x_education::accounts::BurnUnsponsored {
            creator: *creator,
            config: config_pda(),
            module: module_pda(module_id),
            creator_role: role_pda(creator, Role::ModuleCreator),
            module_mint: module_mint_pda(module_id),
            module_vault: module_vault_pda(module_id),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn remove_module_ix(creator: &Pubkey, module_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::RemoveModule { module_id }.data(),
        real_x_education::accounts::RemoveModule {
            creator: *creator,
            config: config_pda(),
            module: module_pda(module_id),
            creator_role: role_pda(creator, Role::ModuleCreator),
            xcav_mint: xcav_mint(),
            vault: vault(),
            creator_xcav: token_acc(&xcav_mint(), creator),
            module_vault: module_vault_pda(module_id),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn cancel_booking_ix(
    school: &Pubkey,
    module_id: u64,
    sponsor_id: u64,
    booking_id: u64,
    deliverer: Option<&Pubkey>,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::CancelBooking {
            module_id,
            booking_id,
        }
        .data(),
        real_x_education::accounts::CancelBooking {
            school: *school,
            config: config_pda(),
            module: module_pda(module_id),
            booking: booking_pda(module_id, booking_id),
            xcav_mint: xcav_mint(),
            vault: vault(),
            treasury: treasury_pda(),
            school_xcav: token_acc(&xcav_mint(), school),
            counter: counter_pda(school),
            cancellation: cancellation_pda(school, booking_id),
            sponsorship: sponsorship_pda(module_id, sponsor_id),
            payment_mint: usdc_mint(),
            sponsor_escrow: sponsor_escrow_pda(module_id, sponsor_id),
            booking_escrow: book_escrow_pda(module_id, booking_id),
            deliverer: deliverer.map(deliverer_pda),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn clear_old_cancellation_ix(school: &Pubkey, booking_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ClearOldCancellation { booking_id }.data(),
        real_x_education::accounts::ClearOldCancellation {
            school: *school,
            config: config_pda(),
            counter: counter_pda(school),
            cancellation: cancellation_pda(school, booking_id),
        }
        .to_account_metas(None),
    )
}

// The role account is always the canonical PDA; the handler renounces it only
// while live, so an already-closed role is simply passed through.
pub fn unregister_deliverer_ix(lecturer: &Pubkey, roles_authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::UnregisterModuleDeliverer {}.data(),
        real_x_education::accounts::UnregisterDeliverer {
            deliverer_signer: *lecturer,
            config: config_pda(),
            deliverer_role: role_pda(lecturer, Role::ModuleDeliverer),
            roles_config: roles_config(),
            roles_authority: *roles_authority,
            xcav_mint: xcav_mint(),
            vault: vault(),
            deliverer_xcav: token_acc(&xcav_mint(), lecturer),
            deliverer: deliverer_pda(lecturer),
            token_program: TOKEN_PROGRAM_ID,
            roles_program: roles_id(),
        }
        .to_account_metas(None),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn mint_credential_ix(
    agent: &Pubkey,
    module_id: u64,
    booking_id: u64,
    kind: CredentialKind,
    recipient: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::MintCredential {
            module_id,
            booking_id,
            kind,
            recipient: *recipient,
            uri: "ipfs://cred".to_string(),
        }
        .data(),
        real_x_education::accounts::MintCredential {
            agent: *agent,
            config: config_pda(),
            module: module_pda(module_id),
            agent_role: role_pda(agent, Role::ModuleAIAgent),
            booking: booking_pda(module_id, booking_id),
            credential: credential_pda(booking_id, kind, recipient),
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn clear_proposal_ix(cranker: &Pubkey, proposal_id: u64, proposer: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ClearProposal { proposal_id }.data(),
        real_x_education::accounts::ClearProposal {
            cranker: *cranker,
            proposal: proposal_pda(proposal_id),
            proposer: *proposer,
        }
        .to_account_metas(None),
    )
}

/// Sponsor a module paying in an arbitrary accepted asset.
pub fn sponsor_asset_ix(
    sponsor: &Pubkey,
    module_id: u64,
    sponsor_id: u64,
    amount: u64,
    mint: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::SponsorModule {
            module_id,
            token_amount: amount,
        }
        .data(),
        real_x_education::accounts::SponsorModule {
            sponsor: *sponsor,
            config: config_pda(),
            module: module_pda(module_id),
            sponsor_role: role_pda(sponsor, Role::ModuleSponsor),
            payment_mint: *mint,
            sponsor_payment: token_acc(mint, sponsor),
            sponsorship: sponsorship_pda(module_id, sponsor_id),
            sponsor_escrow: sponsor_escrow_pda(module_id, sponsor_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

/// Book a module whose sponsorship was paid in an arbitrary accepted asset.
pub fn book_asset_ix(
    school: &Pubkey,
    module_id: u64,
    sponsor_id: u64,
    booking_id: u64,
    delivery_at: i64,
    mint: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::BookModule {
            module_id,
            sponsor_id,
            delivery_at,
            metadata: "ipfs://b".to_string(),
        }
        .data(),
        real_x_education::accounts::BookModule {
            school: *school,
            config: config_pda(),
            module: module_pda(module_id),
            school_role: role_pda(school, Role::ModuleBooker),
            xcav_mint: xcav_mint(),
            school_xcav: token_acc(&xcav_mint(), school),
            vault: vault(),
            sponsorship: sponsorship_pda(module_id, sponsor_id),
            payment_mint: *mint,
            sponsor_escrow: sponsor_escrow_pda(module_id, sponsor_id),
            booking: booking_pda(module_id, booking_id),
            booking_escrow: book_escrow_pda(module_id, booking_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

/// Submit a score for a booking settled in an arbitrary accepted asset.
#[allow(clippy::too_many_arguments)]
pub fn submit_score_asset_ix(
    agent: &Pubkey,
    module_id: u64,
    booking_id: u64,
    score: u16,
    region: u16,
    creator: &Pubkey,
    operator: &Pubkey,
    protocol: &Pubkey,
    lecturer: &Pubkey,
    sponsor: &Pubkey,
    mint: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::SubmitImpactScore {
            module_id,
            booking_id,
            score,
        }
        .data(),
        real_x_education::accounts::SubmitImpactScore {
            agent: *agent,
            config: config_pda(),
            module: module_pda(module_id),
            agent_role: role_pda(agent, Role::ModuleAIAgent),
            booking: booking_pda(module_id, booking_id),
            region_account: region_pda(region),
            payment_mint: *mint,
            module_mint: module_mint_pda(module_id),
            module_vault: module_vault_pda(module_id),
            booking_escrow: book_escrow_pda(module_id, booking_id),
            creator_payment: token_acc(mint, creator),
            regional_operator_payment: token_acc(mint, operator),
            protocol_payment: token_acc(mint, protocol),
            lecturer_payment: token_acc(mint, lecturer),
            deliverer: deliverer_pda(lecturer),
            sponsor_payment: token_acc(mint, sponsor),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}
