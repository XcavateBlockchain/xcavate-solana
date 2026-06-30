//! Minimal happy-path coverage of the module lifecycle.
//!
//! XCAV (deposits) and a USDC-style payment asset are classic SPL tokens; the
//! mints and each participant's token account are seeded directly with
//! `set_account`. The roles program is loaded so the role gates run for real,
//! and a created `Region` is seeded directly so we don't have to drive the whole
//! regions governance flow here. Broader negative-case tests come later.

use anchor_lang::solana_program::clock::Clock;
use anchor_lang::solana_program::program_option::COption;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_lang::{
    prelude::Pubkey, solana_program::instruction::Instruction, AccountDeserialize, AccountSerialize,
    InstructionData, ToAccountMetas,
};
use anchor_spl::token::spl_token::state::{Account as SplAccount, AccountState, Mint as SplMint};
use anchor_spl::token::ID as TOKEN_PROGRAM_ID;
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

use education_regions::state::Region;
use real_x_education::instructions::ConfigParams;
use real_x_education::state::{
    Booking, Config, Deliverer, Module, ModuleProposal, ModuleVote, ProposalStatus, Sponsorship,
};
use real_x_education::{
    BOOKING_SEED, BOOK_ESCROW_SEED, CONFIG_SEED, DELIVERER_SEED, MODULE_MINT_SEED,
    MODULE_PROPOSAL_SEED, MODULE_SEED, MODULE_VAULT_SEED, PROPOSAL_ESCROW_SEED, PROPOSAL_VOTE_SEED,
    SPONSORSHIP_SEED, SPONSOR_ESCROW_SEED, VAULT_SEED,
};
use xcavate_roles::state::Role;

const SYS: Pubkey = anchor_lang::system_program::ID;
const DECIMALS: u8 = 6;
const FUND: u64 = 1_000_000_000_000;

const MODULE_DEPOSIT: u64 = 1_000_000_000;
const BOOKING_DEPOSIT: u64 = 500_000_000;
const DELIVERER_DEPOSIT: u64 = 2_000_000_000;
const MODULE_PRICE: u64 = 100; // whole units
const PER_TOKEN: u64 = 140_000_000; // base 100e6 + 40% fees, at 6 decimals

fn eid() -> Pubkey {
    real_x_education::id()
}
fn roles_id() -> Pubkey {
    xcavate_roles::id()
}
fn regions_id() -> Pubkey {
    education_regions::id()
}

// --- real-x-education PDAs ---
fn config_pda() -> Pubkey {
    Pubkey::find_program_address(&[CONFIG_SEED], &eid()).0
}
fn vault() -> Pubkey {
    Pubkey::find_program_address(&[VAULT_SEED], &eid()).0
}
fn module_pda(id: u64) -> Pubkey {
    Pubkey::find_program_address(&[MODULE_SEED, &id.to_le_bytes()], &eid()).0
}
fn module_mint_pda(id: u64) -> Pubkey {
    Pubkey::find_program_address(&[MODULE_MINT_SEED, &id.to_le_bytes()], &eid()).0
}
fn module_vault_pda(id: u64) -> Pubkey {
    Pubkey::find_program_address(&[MODULE_VAULT_SEED, &id.to_le_bytes()], &eid()).0
}
fn sponsorship_pda(module_id: u64, sponsor_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[SPONSORSHIP_SEED, &module_id.to_le_bytes(), &sponsor_id.to_le_bytes()],
        &eid(),
    )
    .0
}
fn sponsor_escrow_pda(module_id: u64, sponsor_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[SPONSOR_ESCROW_SEED, &module_id.to_le_bytes(), &sponsor_id.to_le_bytes()],
        &eid(),
    )
    .0
}
fn booking_pda(module_id: u64, booking_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[BOOKING_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        &eid(),
    )
    .0
}
fn book_escrow_pda(module_id: u64, booking_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[BOOK_ESCROW_SEED, &module_id.to_le_bytes(), &booking_id.to_le_bytes()],
        &eid(),
    )
    .0
}
fn deliverer_pda(who: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[DELIVERER_SEED, who.as_ref()], &eid()).0
}

// --- roles PDAs ---
fn roles_config() -> Pubkey {
    Pubkey::find_program_address(&[xcavate_roles::CONFIG_SEED], &roles_id()).0
}
fn admin_pda(who: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[xcavate_roles::ADMIN_SEED, who.as_ref()], &roles_id()).0
}
fn role_pda(user: &Pubkey, role: Role) -> Pubkey {
    Pubkey::find_program_address(
        &[xcavate_roles::ROLE_SEED, user.as_ref(), &[role.seed_byte()]],
        &roles_id(),
    )
    .0
}
fn region_pda(region_id: u16) -> Pubkey {
    Pubkey::find_program_address(
        &[education_regions::REGION_SEED, &region_id.to_le_bytes()],
        &regions_id(),
    )
    .0
}
fn proposal_pda(proposal_id: u64) -> Pubkey {
    Pubkey::find_program_address(&[MODULE_PROPOSAL_SEED, &proposal_id.to_le_bytes()], &eid()).0
}
fn proposal_vote_pda(proposal_id: u64, voter: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[PROPOSAL_VOTE_SEED, &proposal_id.to_le_bytes(), voter.as_ref()],
        &eid(),
    )
    .0
}
fn proposal_escrow_pda(proposal_id: u64) -> Pubkey {
    Pubkey::find_program_address(&[PROPOSAL_ESCROW_SEED, &proposal_id.to_le_bytes()], &eid()).0
}

// --- mints / token accounts ---
fn xcav_mint() -> Pubkey {
    Pubkey::new_from_array([7u8; 32])
}
fn usdc_mint() -> Pubkey {
    Pubkey::new_from_array([9u8; 32])
}
fn token_acc(mint: &Pubkey, owner: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"tok", mint.as_ref(), owner.as_ref()], &eid()).0
}

fn set_mint(svm: &mut LiteSVM, mint: Pubkey) {
    let m = SplMint {
        mint_authority: COption::None,
        supply: 0,
        decimals: DECIMALS,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    let mut data = vec![0u8; SplMint::LEN];
    m.pack_into_slice(&mut data);
    svm.set_account(
        mint,
        Account { lamports: 100_000_000, data, owner: TOKEN_PROGRAM_ID, executable: false, rent_epoch: 0 },
    )
    .unwrap();
}

fn give(svm: &mut LiteSVM, mint: &Pubkey, owner: &Pubkey, amount: u64) {
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
        Account { lamports: 100_000_000, data, owner: TOKEN_PROGRAM_ID, executable: false, rent_epoch: 0 },
    )
    .unwrap();
}

fn balance(svm: &LiteSVM, mint: &Pubkey, owner: &Pubkey) -> u64 {
    let acc = svm.get_account(&token_acc(mint, owner)).unwrap();
    SplAccount::unpack(&acc.data).unwrap().amount
}

fn spl_amount(svm: &LiteSVM, addr: &Pubkey) -> u64 {
    let acc = svm.get_account(addr).unwrap();
    SplAccount::unpack(&acc.data).unwrap().amount
}

// --- send helpers ---
fn process(
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

fn ok(svm: &mut LiteSVM, ix: Instruction, payer: &Keypair, signers: &[&Keypair]) {
    if let Err(failed) = process(svm, ix, payer, signers) {
        panic!("expected success, failed with: {:?}\n{}", failed.err, failed.meta.logs.join("\n"));
    }
}

/// Run an instruction expecting it to fail, and assert the logs name `code`.
fn err(svm: &mut LiteSVM, ix: Instruction, payer: &Keypair, signers: &[&Keypair], code: &str) {
    match process(svm, ix, payer, signers) {
        Ok(_) => panic!("expected failure with {code}, but it succeeded"),
        Err(failed) => {
            let logs = failed.meta.logs.join("\n");
            assert!(logs.contains(code), "expected error {code}, got logs:\n{logs}");
        }
    }
}

fn funded(svm: &mut LiteSVM) -> Keypair {
    let kp = Keypair::new();
    svm.airdrop(&kp.pubkey(), 100_000_000_000).unwrap();
    kp
}

/// SOL-funded keypair holding XCAV and USDC, with both token accounts seeded.
fn actor(svm: &mut LiteSVM) -> Keypair {
    let kp = funded(svm);
    give(svm, &xcav_mint(), &kp.pubkey(), FUND);
    give(svm, &usdc_mint(), &kp.pubkey(), FUND);
    kp
}

// --- roles ix builders ---
fn roles_init_ix(authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        roles_id(),
        &xcavate_roles::instruction::InitializeConfig {}.data(),
        xcavate_roles::accounts::InitializeConfig { authority: *authority, config: roles_config(), system_program: SYS }
            .to_account_metas(None),
    )
}
fn roles_add_admin_ix(authority: &Pubkey, new_admin: &Pubkey) -> Instruction {
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
fn roles_assign_ix(admin: &Pubkey, user: &Pubkey, role: Role) -> Instruction {
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
fn default_params() -> ConfigParams {
    ConfigParams {
        module_deposit: MODULE_DEPOSIT,
        booking_deposit: BOOKING_DEPOSIT,
        deliverer_deposit: DELIVERER_DEPOSIT,
        module_price: MODULE_PRICE,
        max_module_tokens: 1_000,
        content_creator_bps: 2_000,
        regional_operator_bps: 1_000,
        protocol_bps: 500,
        dbs_bps: 500,
        min_impact_score_bps: 5_000,
        sponsorship_window: 1_000,
        cancellation_window: 1_000,
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
        accepted_assets: [usdc_mint(), Pubkey::default(), Pubkey::default()],
    }
}

fn edu_init_ix(authority: &Pubkey, protocol: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::InitializeConfig { params: default_params() }.data(),
        real_x_education::accounts::InitializeConfig {
            authority: *authority,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            treasury: token_acc(&xcav_mint(), authority),
            protocol_authority: *protocol,
            vault: vault(),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

fn create_module_ix(creator: &Pubkey, region: u16, module_id: u64, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::CreateModule { region, module_amount: amount, metadata: "ipfs://m".to_string() }
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

fn sponsor_ix(sponsor: &Pubkey, module_id: u64, sponsor_id: u64, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::SponsorModule { module_id, token_amount: amount }.data(),
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

fn book_ix(school: &Pubkey, module_id: u64, sponsor_id: u64, booking_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::BookModule { module_id, sponsor_id, metadata: "ipfs://b".to_string() }.data(),
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

fn register_deliverer_ix(lecturer: &Pubkey) -> Instruction {
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

fn claim_ix(lecturer: &Pubkey, module_id: u64, booking_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ClaimBooking { module_id, booking_id }.data(),
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

fn cancel_claim_ix(lecturer: &Pubkey, authority: &Pubkey, module_id: u64, booking_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::CancelClaim { module_id, booking_id }.data(),
        real_x_education::accounts::CancelClaim {
            lecturer: *lecturer,
            config: config_pda(),
            module: module_pda(module_id),
            lecturer_role: role_pda(lecturer, Role::ModuleDeliverer),
            deliverer: deliverer_pda(lecturer),
            booking: booking_pda(module_id, booking_id),
            xcav_mint: xcav_mint(),
            vault: vault(),
            treasury: token_acc(&xcav_mint(), authority),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

#[allow(clippy::too_many_arguments)]
fn submit_score_ix(
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
        &real_x_education::instruction::SubmitImpactScore { module_id, booking_id, score }.data(),
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

fn finish_ix(school: &Pubkey, module_id: u64, booking_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::FinishBookingProcess { module_id, booking_id }.data(),
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

fn reclaim_sponsorship_ix(sponsor: &Pubkey, module_id: u64, sponsor_id: u64, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ReclaimSponsorship { module_id, sponsor_id, amount }.data(),
        real_x_education::accounts::ReclaimSponsorship {
            sponsor: *sponsor,
            config: config_pda(),
            module: module_pda(module_id),
            sponsor_role: role_pda(sponsor, Role::ModuleSponsor),
            sponsorship: sponsorship_pda(module_id, sponsor_id),
            payment_mint: usdc_mint(),
            sponsor_escrow: sponsor_escrow_pda(module_id, sponsor_id),
            sponsor_payment: token_acc(&usdc_mint(), sponsor),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

fn close_sponsorship_ix(sponsor: &Pubkey, module_id: u64, sponsor_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::CloseSponsorship { module_id, sponsor_id }.data(),
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
fn create_proposal_ix(
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

fn vote_ix(voter: &Pubkey, proposal_id: u64, vote: ModuleVote, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::VoteOnProposal { proposal_id, vote, amount }.data(),
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

fn finalize_proposal_ix(cranker: &Pubkey, proposal_id: u64, proposer: &Pubkey, authority: &Pubkey) -> Instruction {
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
            treasury: token_acc(&xcav_mint(), authority),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

fn claim_proposal_ix(creator: &Pubkey, proposal_id: u64) -> Instruction {
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

fn upload_proposal_ix(claimant: &Pubkey, proposal_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::UploadProposal { proposal_id, content_uri: "ipfs://c".to_string() }.data(),
        real_x_education::accounts::UploadProposal {
            claimant: *claimant,
            proposal: proposal_pda(proposal_id),
        }
        .to_account_metas(None),
    )
}

fn release_claim_ix(cranker: &Pubkey, proposal_id: u64, authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ReleaseClaim { proposal_id }.data(),
        real_x_education::accounts::ReleaseClaim {
            cranker: *cranker,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            treasury: token_acc(&xcav_mint(), authority),
            proposal: proposal_pda(proposal_id),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

fn review_proposal_ix(agent: &Pubkey, proposal_id: u64, passed: bool, authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ReviewProposal { proposal_id, passed }.data(),
        real_x_education::accounts::ReviewProposal {
            agent: *agent,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            treasury: token_acc(&xcav_mint(), authority),
            agent_role: role_pda(agent, Role::ModuleAIAgent),
            proposal: proposal_pda(proposal_id),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

fn mint_proposed_ix(creator: &Pubkey, proposal_id: u64, module_id: u64) -> Instruction {
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

fn create_sponsor_proposal_ix(proposer: &Pubkey, region: u16, proposal_id: u64, amount: u64) -> Instruction {
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
fn mint_sponsored_ix(
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

fn unlock_vote_ix(voter: &Pubkey, proposal_id: u64) -> Instruction {
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

fn expire_proposal_ix(cranker: &Pubkey, proposal_id: u64, authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        eid(),
        &real_x_education::instruction::ExpireProposal { proposal_id }.data(),
        real_x_education::accounts::ExpireProposal {
            cranker: *cranker,
            config: config_pda(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            treasury: token_acc(&xcav_mint(), authority),
            proposal: proposal_pda(proposal_id),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

fn reclaim_pre_sponsor_ix(sponsor: &Pubkey, proposal_id: u64) -> Instruction {
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
fn warp_past_voting(svm: &mut LiteSVM) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp += 2_000;
    svm.set_sysvar(&clock);
}

// --- world ---
struct World {
    svm: LiteSVM,
    admin: Keypair,
    authority: Keypair,
    protocol: Keypair,
    operator: Keypair,
}

/// Loads the three programs, seeds mints, a created region, and the education
/// config. The region operator is seeded directly into a `Region` account.
fn setup() -> World {
    let mut svm = LiteSVM::new();
    svm.add_program(roles_id(), include_bytes!("../../../target/deploy/xcavate_roles.so")).unwrap();
    svm.add_program(regions_id(), include_bytes!("../../../target/deploy/education_regions.so")).unwrap();
    svm.add_program(eid(), include_bytes!("../../../target/deploy/real_x_education.so")).unwrap();
    set_mint(&mut svm, xcav_mint());
    set_mint(&mut svm, usdc_mint());

    let authority = funded(&mut svm);
    give(&mut svm, &xcav_mint(), &authority.pubkey(), 0); // XCAV treasury
    ok(&mut svm, roles_init_ix(&authority.pubkey()), &authority, &[&authority]);

    let admin = funded(&mut svm);
    ok(&mut svm, roles_add_admin_ix(&authority.pubkey(), &admin.pubkey()), &authority, &[&authority]);

    // Region operator: seed a created Region directly + give them a USDC account.
    let operator = actor(&mut svm);
    let (region_addr, bump) = Pubkey::find_program_address(
        &[education_regions::REGION_SEED, &1u16.to_le_bytes()],
        &regions_id(),
    );
    let region = Region { region_id: 1, owner: operator.pubkey(), collateral: 0, active_strikes: 0, next_owner_change: 0, bump };
    let mut data = Vec::new();
    region.try_serialize(&mut data).unwrap();
    svm.set_account(
        region_addr,
        Account { lamports: 10_000_000, data, owner: regions_id(), executable: false, rent_epoch: 0 },
    )
    .unwrap();

    let protocol = actor(&mut svm);
    ok(&mut svm, edu_init_ix(&authority.pubkey(), &protocol.pubkey()), &authority, &[&authority]);

    World { svm, admin, authority, protocol, operator }
}

fn with_role(w: &mut World, role: Role) -> Keypair {
    let kp = actor(&mut w.svm);
    let admin = w.admin.insecure_clone();
    ok(&mut w.svm, roles_assign_ix(&admin.pubkey(), &kp.pubkey(), role), &admin, &[&admin]);
    kp
}

fn config(svm: &LiteSVM) -> Config {
    Config::try_deserialize(&mut &svm.get_account(&config_pda()).unwrap().data[..]).unwrap()
}
fn module_of(svm: &LiteSVM, id: u64) -> Module {
    Module::try_deserialize(&mut &svm.get_account(&module_pda(id)).unwrap().data[..]).unwrap()
}
fn deliverer_of(svm: &LiteSVM, who: &Pubkey) -> Deliverer {
    Deliverer::try_deserialize(&mut &svm.get_account(&deliverer_pda(who)).unwrap().data[..]).unwrap()
}
fn proposal_of(svm: &LiteSVM, id: u64) -> ModuleProposal {
    ModuleProposal::try_deserialize(&mut &svm.get_account(&proposal_pda(id)).unwrap().data[..]).unwrap()
}

// ============================ tests ============================

#[test]
fn initialize_config_works() {
    let w = setup();
    let cfg = config(&w.svm);
    assert_eq!(cfg.authority, w.authority.pubkey());
    assert_eq!(cfg.xcav_mint, xcav_mint());
    assert_eq!(cfg.protocol_authority, w.protocol.pubkey());
    assert_eq!(cfg.module_price, MODULE_PRICE);
    assert_eq!(cfg.accepted_assets[0], usdc_mint());
    assert_eq!(cfg.next_module_id, 0);
}

#[test]
fn create_module_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let before = balance(&w.svm, &xcav_mint(), &creator.pubkey());

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);

    let m = module_of(&w.svm, 0);
    assert_eq!(m.creator, creator.pubkey());
    assert_eq!(m.region, 1);
    assert_eq!(m.total_token_amount, 10);
    assert_eq!(m.sponsor_allocation, 10);
    assert_eq!(m.school_allocation, 0);
    assert_eq!(m.price, MODULE_PRICE);
    // Deposit locked in the vault; full supply minted into the module vault.
    assert_eq!(before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    assert_eq!(spl_amount(&w.svm, &vault()), MODULE_DEPOSIT);
    assert_eq!(spl_amount(&w.svm, &module_vault_pda(0)), 10);
    assert_eq!(config(&w.svm).next_module_id, 1);
}

#[test]
fn sponsor_module_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);

    let before = balance(&w.svm, &usdc_mint(), &sponsor.pubkey());
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 5), &sponsor, &[&sponsor]);

    let sp = Sponsorship::try_deserialize(
        &mut &w.svm.get_account(&sponsorship_pda(0, 0)).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(sp.amount, 5);
    assert_eq!(sp.price_per_token, PER_TOKEN);
    // Payment for 5 tokens escrowed.
    assert_eq!(before - balance(&w.svm, &usdc_mint(), &sponsor.pubkey()), 5 * PER_TOKEN);
    assert_eq!(spl_amount(&w.svm, &sponsor_escrow_pda(0, 0)), 5 * PER_TOKEN);
    let m = module_of(&w.svm, 0);
    assert_eq!(m.sponsor_allocation, 5);
    assert_eq!(m.school_allocation, 5);
}

#[test]
fn full_flow_through_score_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let operator = w.operator.pubkey();
    let protocol = w.protocol.pubkey();

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 5), &sponsor, &[&sponsor]);
    ok(&mut w.svm, book_ix(&school.pubkey(), 0, 0, 0), &school, &[&school]);

    // Booking escrow funded; school deposit locked.
    assert_eq!(spl_amount(&w.svm, &book_escrow_pda(0, 0)), PER_TOKEN);
    let m = module_of(&w.svm, 0);
    assert_eq!(m.school_allocation, 4);
    assert_eq!(m.student_allocation, 1);

    ok(&mut w.svm, register_deliverer_ix(&lecturer.pubkey()), &lecturer, &[&lecturer]);
    ok(&mut w.svm, claim_ix(&lecturer.pubkey(), 0, 0), &lecturer, &[&lecturer]);

    let cre_before = balance(&w.svm, &usdc_mint(), &creator.pubkey());
    let op_before = balance(&w.svm, &usdc_mint(), &operator);
    let proto_before = balance(&w.svm, &usdc_mint(), &protocol);
    let lec_before = balance(&w.svm, &usdc_mint(), &lecturer.pubkey());

    ok(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 10_000, 1, &creator.pubkey(), &operator, &protocol, &lecturer.pubkey(), &sponsor.pubkey()),
        &agent,
        &[&agent],
    );

    // Full score: creator 20%, operator 10%, protocol 5%, lecturer base+dbs.
    assert_eq!(balance(&w.svm, &usdc_mint(), &creator.pubkey()) - cre_before, 20_000_000);
    assert_eq!(balance(&w.svm, &usdc_mint(), &operator) - op_before, 10_000_000);
    assert_eq!(balance(&w.svm, &usdc_mint(), &protocol) - proto_before, 5_000_000);
    assert_eq!(balance(&w.svm, &usdc_mint(), &lecturer.pubkey()) - lec_before, 105_000_000);
    // Escrow drained, delivered token burned.
    assert_eq!(spl_amount(&w.svm, &book_escrow_pda(0, 0)), 0);
    assert_eq!(spl_amount(&w.svm, &module_vault_pda(0)), 9);

    let b = Booking::try_deserialize(&mut &w.svm.get_account(&booking_pda(0, 0)).unwrap().data[..]).unwrap();
    assert_eq!(b.score, Some(10_000));

    // School reclaims its deposit and the booking closes.
    let school_before = balance(&w.svm, &xcav_mint(), &school.pubkey());
    ok(&mut w.svm, finish_ix(&school.pubkey(), 0, 0), &school, &[&school]);
    assert_eq!(balance(&w.svm, &xcav_mint(), &school.pubkey()) - school_before, BOOKING_DEPOSIT);
    assert!(w.svm.get_account(&booking_pda(0, 0)).map(|a| a.data.is_empty()).unwrap_or(true));
}

#[test]
fn cancel_claim_rejected_after_score() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let operator = w.operator.pubkey();
    let protocol = w.protocol.pubkey();
    let authority = w.authority.pubkey();

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 5), &sponsor, &[&sponsor]);
    ok(&mut w.svm, book_ix(&school.pubkey(), 0, 0, 0), &school, &[&school]);
    ok(&mut w.svm, register_deliverer_ix(&lecturer.pubkey()), &lecturer, &[&lecturer]);
    ok(&mut w.svm, claim_ix(&lecturer.pubkey(), 0, 0), &lecturer, &[&lecturer]);
    ok(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 10_000, 1, &creator.pubkey(), &operator, &protocol, &lecturer.pubkey(), &sponsor.pubkey()),
        &agent,
        &[&agent],
    );

    // The booking is settled: cancelling the (already scored) claim must be
    // rejected so the released claim can't be double-counted.
    let claims_before = deliverer_of(&w.svm, &lecturer.pubkey()).active_claims;
    err(
        &mut w.svm,
        cancel_claim_ix(&lecturer.pubkey(), &authority, 0, 0),
        &lecturer,
        &[&lecturer],
        "ScoreAlreadySet",
    );
    assert_eq!(deliverer_of(&w.svm, &lecturer.pubkey()).active_claims, claims_before);
}

#[test]
fn close_sponsorship_blocked_while_booking_active() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    // Sponsor exactly one token, then book it: amount hits 0 but the booking is
    // still cancellable, so the escrow must not be closeable yet.
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 1), &sponsor, &[&sponsor]);
    ok(&mut w.svm, book_ix(&school.pubkey(), 0, 0, 0), &school, &[&school]);

    let sp = Sponsorship::try_deserialize(
        &mut &w.svm.get_account(&sponsorship_pda(0, 0)).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(sp.amount, 0);
    assert_eq!(sp.active_bookings, 1);

    err(
        &mut w.svm,
        close_sponsorship_ix(&sponsor.pubkey(), 0, 0),
        &sponsor,
        &[&sponsor],
        "SponsorshipNotEmpty",
    );
    // The escrow is still alive to refund a cancellation into.
    assert!(w.svm.get_account(&sponsor_escrow_pda(0, 0)).map(|a| !a.data.is_empty()).unwrap_or(false));
}

#[test]
fn close_sponsorship_after_reclaim_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 1), &sponsor, &[&sponsor]);

    // Nothing booked: reclaim after the window empties the sponsorship.
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, reclaim_sponsorship_ix(&sponsor.pubkey(), 0, 0, 1), &sponsor, &[&sponsor]);

    let rent_before = w.svm.get_account(&sponsor.pubkey()).map(|a| a.lamports).unwrap_or(0);
    ok(&mut w.svm, close_sponsorship_ix(&sponsor.pubkey(), 0, 0), &sponsor, &[&sponsor]);

    // Both the record and the escrow are gone, and their rent came back.
    assert!(w.svm.get_account(&sponsorship_pda(0, 0)).map(|a| a.data.is_empty()).unwrap_or(true));
    assert!(w.svm.get_account(&sponsor_escrow_pda(0, 0)).map(|a| a.data.is_empty()).unwrap_or(true));
    assert!(w.svm.get_account(&sponsor.pubkey()).map(|a| a.lamports).unwrap_or(0) > rent_before);
}

#[test]
fn proposal_to_module_flow_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);
    let authority = w.authority.pubkey();

    // A creator opens a proposal, staking the proposal deposit.
    let staked_before = balance(&w.svm, &xcav_mint(), &creator.pubkey());
    ok(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 10), &creator, &[&creator]);
    assert_eq!(staked_before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Voting);
    assert_eq!(config(&w.svm).next_proposal_id, 1);

    // A single voter clears the quorum and threshold with a yes vote.
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    assert_eq!(proposal_of(&w.svm, 0).yes_power, 10_000);

    // After voting closes, anyone can finalize; the stake comes back.
    warp_past_voting(&mut w.svm);
    let refund_before = balance(&w.svm, &xcav_mint(), &creator.pubkey());
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &creator.pubkey(), &authority), &cranker, &[&cranker]);
    assert_eq!(balance(&w.svm, &xcav_mint(), &creator.pubkey()) - refund_before, MODULE_DEPOSIT);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimable);

    // The proposing creator reserves the build (locking the deposit), uploads
    // the content, the agent passes review, and the module mints. The deposit
    // stays locked the whole way and becomes the module's deposit.
    let deposit_before = balance(&w.svm, &xcav_mint(), &creator.pubkey());
    ok(&mut w.svm, claim_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimed);
    assert_eq!(deposit_before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    ok(&mut w.svm, upload_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::UnderReview);
    assert_eq!(deposit_before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    ok(&mut w.svm, review_proposal_ix(&agent.pubkey(), 0, true, &authority), &agent, &[&agent]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Approved);

    ok(&mut w.svm, mint_proposed_ix(&creator.pubkey(), 0, 0), &creator, &[&creator]);
    let m = module_of(&w.svm, 0);
    assert_eq!(m.creator, creator.pubkey());
    assert_eq!(m.deposit, MODULE_DEPOSIT);
    assert_eq!(m.total_token_amount, 10);
    assert_eq!(m.sponsor_allocation, 10);
    assert_eq!(spl_amount(&w.svm, &module_vault_pda(0)), 10);
    // The deposit is still locked, now as the module's deposit.
    assert_eq!(deposit_before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    assert_eq!(config(&w.svm).next_module_id, 1);
    // The proposal record is closed once the module is created.
    assert!(w.svm.get_account(&proposal_pda(0)).map(|a| a.data.is_empty()).unwrap_or(true));

    // The voter unlocks the XCAV they locked.
    let unlock_before = balance(&w.svm, &xcav_mint(), &voter.pubkey());
    ok(&mut w.svm, unlock_vote_ix(&voter.pubkey(), 0), &voter, &[&voter]);
    assert_eq!(balance(&w.svm, &xcav_mint(), &voter.pubkey()) - unlock_before, 10_000);
}

#[test]
fn sponsor_proposal_pre_sponsors_on_mint() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);
    let authority = w.authority.pubkey();

    // A sponsor opens a proposal, locking the stake plus the pre-sponsorship
    // payment for two tokens (config.pre_sponsor_amount).
    let usdc_before = balance(&w.svm, &usdc_mint(), &sponsor.pubkey());
    ok(&mut w.svm, create_sponsor_proposal_ix(&sponsor.pubkey(), 1, 0, 10), &sponsor, &[&sponsor]);
    assert_eq!(usdc_before - balance(&w.svm, &usdc_mint(), &sponsor.pubkey()), 2 * PER_TOKEN);
    assert_eq!(spl_amount(&w.svm, &proposal_escrow_pda(0)), 2 * PER_TOKEN);
    assert_eq!(proposal_of(&w.svm, 0).pre_sponsor_amount, 2);

    // Pass the vote and finalize.
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &sponsor.pubkey(), &authority), &cranker, &[&cranker]);

    // A creator reserves, uploads, and builds it; on mint the pre-sponsorship
    // converts into a real sponsorship in the sponsor's name.
    ok(&mut w.svm, claim_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    ok(&mut w.svm, upload_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    ok(&mut w.svm, review_proposal_ix(&agent.pubkey(), 0, true, &authority), &agent, &[&agent]);
    ok(&mut w.svm, mint_sponsored_ix(&creator.pubkey(), &sponsor.pubkey(), 0, 0, 0), &creator, &[&creator]);

    let m = module_of(&w.svm, 0);
    assert_eq!(m.creator, creator.pubkey());
    assert_eq!(m.total_token_amount, 10);
    assert_eq!(m.sponsor_allocation, 8);
    assert_eq!(m.school_allocation, 2);

    let sp = Sponsorship::try_deserialize(
        &mut &w.svm.get_account(&sponsorship_pda(0, 0)).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(sp.sponsor, sponsor.pubkey());
    assert_eq!(sp.amount, 2);
    assert_eq!(sp.price_per_token, PER_TOKEN);
    // Funds moved from the pre-sponsor escrow into the sponsorship escrow.
    assert_eq!(spl_amount(&w.svm, &sponsor_escrow_pda(0, 0)), 2 * PER_TOKEN);
    assert!(w.svm.get_account(&proposal_escrow_pda(0)).map(|a| a.data.is_empty()).unwrap_or(true));
    assert!(w.svm.get_account(&proposal_pda(0)).map(|a| a.data.is_empty()).unwrap_or(true));
    assert_eq!(config(&w.svm).next_sponsor_id, 1);
}

#[test]
fn unbuilt_sponsor_proposal_expires_and_refunds() {
    let mut w = setup();
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);
    let authority = w.authority.pubkey();

    // A sponsor proposal passes the vote but nobody ever builds the module.
    let usdc_before = balance(&w.svm, &usdc_mint(), &sponsor.pubkey());
    ok(&mut w.svm, create_sponsor_proposal_ix(&sponsor.pubkey(), 1, 0, 10), &sponsor, &[&sponsor]);
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &sponsor.pubkey(), &authority), &cranker, &[&cranker]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimable);

    // Before the build deadline passes the proposal can't be expired.
    err(&mut w.svm, expire_proposal_ix(&cranker.pubkey(), 0, &authority), &cranker, &[&cranker], "BuildDeadlineNotReached");

    // Once it does, anyone can expire it.
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, expire_proposal_ix(&cranker.pubkey(), 0, &authority), &cranker, &[&cranker]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Rejected);

    // The sponsor reclaims the pre-sponsorship payment in full and the records
    // are closed.
    ok(&mut w.svm, reclaim_pre_sponsor_ix(&sponsor.pubkey(), 0), &sponsor, &[&sponsor]);
    assert_eq!(balance(&w.svm, &usdc_mint(), &sponsor.pubkey()), usdc_before);
    assert!(w.svm.get_account(&proposal_escrow_pda(0)).map(|a| a.data.is_empty()).unwrap_or(true));
    assert!(w.svm.get_account(&proposal_pda(0)).map(|a| a.data.is_empty()).unwrap_or(true));
}

#[test]
fn abandoned_reservation_slashes_bond_and_reopens() {
    let mut w = setup();
    let school = with_role(&mut w, Role::ModuleBooker);
    let creator1 = with_role(&mut w, Role::ModuleCreator);
    let creator2 = with_role(&mut w, Role::ModuleCreator);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);
    let authority = w.authority.pubkey();

    // A school opens a proposal that passes, so any creator may build it.
    ok(&mut w.svm, create_proposal_ix(&school.pubkey(), Role::ModuleBooker, 1, 0, 10), &school, &[&school]);
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &school.pubkey(), &authority), &cranker, &[&cranker]);

    // The first creator reserves the build, locking the bond.
    let before = balance(&w.svm, &xcav_mint(), &creator1.pubkey());
    ok(&mut w.svm, claim_proposal_ix(&creator1.pubkey(), 0), &creator1, &[&creator1]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimed);
    assert_eq!(before - balance(&w.svm, &xcav_mint(), &creator1.pubkey()), MODULE_DEPOSIT);

    // The reservation can't be released until the upload deadline passes.
    err(&mut w.svm, release_claim_ix(&cranker.pubkey(), 0, &authority), &cranker, &[&cranker], "UploadDeadlineNotReached");

    // After it lapses, anyone can release it: the bond is slashed to the
    // treasury and the proposal reopens.
    let treasury_before = balance(&w.svm, &xcav_mint(), &authority);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, release_claim_ix(&cranker.pubkey(), 0, &authority), &cranker, &[&cranker]);
    assert_eq!(balance(&w.svm, &xcav_mint(), &authority) - treasury_before, MODULE_DEPOSIT);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimable);
    assert_eq!(proposal_of(&w.svm, 0).claimant, None);

    // A different creator can now reserve it.
    ok(&mut w.svm, claim_proposal_ix(&creator2.pubkey(), 0), &creator2, &[&creator2]);
    assert_eq!(proposal_of(&w.svm, 0).claimant, Some(creator2.pubkey()));
}

#[test]
fn second_review_fail_slashes_deposit_and_bans() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);
    let authority = w.authority.pubkey();

    ok(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &creator.pubkey(), &authority), &cranker, &[&cranker]);

    // Claim locks the deposit; it rides through the first failed review.
    let before = balance(&w.svm, &xcav_mint(), &creator.pubkey());
    ok(&mut w.svm, claim_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    assert_eq!(before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    ok(&mut w.svm, upload_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    ok(&mut w.svm, review_proposal_ix(&agent.pubkey(), 0, false, &authority), &agent, &[&agent]);
    // First fail: back to reserved with the deposit still locked, no slash.
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimed);
    assert_eq!(before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);

    // Re-upload and fail again: the deposit is slashed and the creator banned.
    let treasury_before = balance(&w.svm, &xcav_mint(), &authority);
    ok(&mut w.svm, upload_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    ok(&mut w.svm, review_proposal_ix(&agent.pubkey(), 0, false, &authority), &agent, &[&agent]);
    assert_eq!(balance(&w.svm, &xcav_mint(), &authority) - treasury_before, MODULE_DEPOSIT);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimable);
    assert_eq!(proposal_of(&w.svm, 0).claimant, None);
    assert!(proposal_of(&w.svm, 0).banned.contains(&creator.pubkey()));

    // The banned creator can't re-claim.
    err(&mut w.svm, claim_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator], "CreatorBanned");
}
