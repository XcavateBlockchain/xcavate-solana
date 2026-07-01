//! Shared test scaffolding for region governance: PDA and token helpers,
//! instruction builders, the send/assert helpers (`ok`, `fails_with`), and the
//! `setup` / `reach_*` drivers.
//!
//! Governance stakes are XCAV (a classic SPL token) held in the program vault.
//! LiteSVM loads the SPL Token program by default, and the XCAV mint and each
//! participant's token account are seeded directly with `set_account`. The roles
//! program is loaded alongside regions so the cross-program RegionalOperator gate
//! is exercised for real. Each test file pulls this in with
//! `mod common; use common::*;`.
//!
//! Each test file is its own binary that uses a subset of this, so unused
//! helpers are expected.
#![allow(dead_code, unused_imports)]

pub use anchor_lang::prelude::Pubkey;
pub use anchor_lang::solana_program::clock::Clock;
pub use anchor_lang::AccountDeserialize;
pub use education_regions::state::{
    Config as RegionsConfig, RegionProposal, RegionState, RegionStatus, Vote, VoteRecord,
};
pub use litesvm::LiteSVM;
pub use solana_keypair::Keypair;
pub use solana_signer::Signer;

use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_lang::solana_program::program_option::COption;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_spl::token::spl_token::state::{Account as SplAccount, AccountState, Mint as SplMint};
use anchor_spl::token::ID as TOKEN_PROGRAM_ID;
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use solana_account::Account;
use solana_message::{Message, VersionedMessage};
use solana_transaction::versioned::VersionedTransaction;

use education_regions::instructions::ConfigParams;
use education_regions::{CONFIG_SEED, PROPOSAL_SEED, REGION_SEED, REGION_STATE_SEED, VAULT_SEED, VOTE_SEED};
use xcavate_roles::state::Role;

pub const SYS: Pubkey = anchor_lang::system_program::ID;
pub const DEPOSIT: u64 = 1_000_000_000;
pub const DECIMALS: u8 = 6;
pub const FUND_XCAV: u64 = 100_000_000_000;

// --- ids / PDAs ---

pub fn rid() -> Pubkey {
    education_regions::id()
}
pub fn roles_id() -> Pubkey {
    xcavate_roles::id()
}

pub fn regions_config() -> Pubkey {
    Pubkey::find_program_address(&[CONFIG_SEED], &rid()).0
}
pub fn vault() -> Pubkey {
    Pubkey::find_program_address(&[VAULT_SEED], &rid()).0
}
pub fn region_state(region_id: u16) -> Pubkey {
    Pubkey::find_program_address(&[REGION_STATE_SEED, &region_id.to_le_bytes()], &rid()).0
}
pub fn proposal_pda(proposal_id: u64) -> Pubkey {
    Pubkey::find_program_address(&[PROPOSAL_SEED, &proposal_id.to_le_bytes()], &rid()).0
}
pub fn vote_record_pda(proposal_id: u64, voter: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[VOTE_SEED, &proposal_id.to_le_bytes(), voter.as_ref()],
        &rid(),
    )
    .0
}
pub fn region_pda(region_id: u16) -> Pubkey {
    Pubkey::find_program_address(&[REGION_SEED, &region_id.to_le_bytes()], &rid()).0
}

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

// --- XCAV mint / token accounts (seeded directly) ---

/// Fixed address for the test XCAV mint.
pub fn xcav_mint() -> Pubkey {
    Pubkey::new_from_array([7u8; 32])
}

/// Deterministic XCAV token account for an owner. Not a real ATA; the program
/// only checks the mint and authority, so any token account works.
pub fn token_acc(owner: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"xcav_token", owner.as_ref()], &rid()).0
}

pub fn set_mint(svm: &mut LiteSVM) {
    let mint = SplMint {
        mint_authority: COption::None,
        supply: 0,
        decimals: DECIMALS,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    let mut data = vec![0u8; SplMint::LEN];
    mint.pack_into_slice(&mut data);
    svm.set_account(
        xcav_mint(),
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

pub fn give_xcav(svm: &mut LiteSVM, owner: &Pubkey, amount: u64) {
    let acc = SplAccount {
        mint: xcav_mint(),
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
        token_acc(owner),
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

pub fn xcav_balance(svm: &LiteSVM, owner: &Pubkey) -> u64 {
    let acc = svm.get_account(&token_acc(owner)).unwrap();
    SplAccount::unpack(&acc.data).unwrap().amount
}

pub fn vault_balance(svm: &LiteSVM) -> u64 {
    let acc = svm.get_account(&vault()).unwrap();
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
        panic!("expected success, failed with: {:?}", failed.err);
    }
}

pub fn fails_with(svm: &mut LiteSVM, ix: Instruction, payer: &Keypair, signers: &[&Keypair], expected: &str) {
    match process(svm, ix, payer, signers) {
        Ok(_) => panic!("expected failure `{expected}`, but it succeeded"),
        Err(failed) => {
            let detail = format!("{:?}\n{}", failed.err, failed.meta.logs.join("\n"));
            assert!(detail.contains(expected), "expected `{expected}`, got:\n{detail}");
        }
    }
}

/// A SOL-funded keypair (for fees + account rent).
pub fn funded(svm: &mut LiteSVM) -> Keypair {
    let kp = Keypair::new();
    svm.airdrop(&kp.pubkey(), 100_000_000_000).unwrap();
    kp
}

/// A SOL-funded keypair that also holds XCAV.
pub fn actor(svm: &mut LiteSVM) -> Keypair {
    let kp = funded(svm);
    give_xcav(svm, &kp.pubkey(), FUND_XCAV);
    kp
}

// --- roles instruction builders ---

pub fn roles_init_ix(authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        roles_id(),
        &xcavate_roles::instruction::InitializeConfig {}.data(),
        xcavate_roles::accounts::InitializeConfig {
            authority: *authority,
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

// --- regions instruction builders ---

pub fn default_params() -> ConfigParams {
    ConfigParams {
        proposal_deposit: DEPOSIT,
        minimum_voting_amount: 100_000_000,
        minimum_region_deposit: 500_000_000,
        voting_period: 1_000,
        auction_period: 1_000,
        owner_change_period: 10_000,
        threshold_bps: 5_000,
        quorum: 100_000_000,
        removal_deposit: DEPOSIT,
        removal_voting_period: 1_000,
        slash_amount: 100_000_000,
        notice_period: 5_000,
        allowed_strikes: 3,
    }
}

pub fn regions_init_ix(authority: &Pubkey) -> Instruction {
    regions_init_ix_with(authority, default_params())
}

// The authority's own XCAV account doubles as the treasury in tests.
pub fn regions_init_ix_with(authority: &Pubkey, params: ConfigParams) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::InitializeConfig { params }.data(),
        education_regions::accounts::InitializeConfig {
            authority: *authority,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            treasury: token_acc(authority),
            vault: vault(),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn update_config_ix(authority: &Pubkey, treasury_owner: &Pubkey, params: ConfigParams) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::UpdateConfig { params }.data(),
        education_regions::accounts::UpdateConfig {
            authority: *authority,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            treasury: token_acc(treasury_owner),
        }
        .to_account_metas(None),
    )
}

pub fn update_authority_ix(authority: &Pubkey, new_authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::UpdateAuthority { new_authority: *new_authority }.data(),
        education_regions::accounts::UpdateAuthority {
            authority: *authority,
            config: regions_config(),
        }
        .to_account_metas(None),
    )
}

pub fn propose_ix(proposer: &Pubkey, region_id: u16, proposal_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::ProposeNewRegion { region_id }.data(),
        education_regions::accounts::ProposeNewRegion {
            proposer: *proposer,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            proposer_token: token_acc(proposer),
            vault: vault(),
            operator_role: role_pda(proposer, Role::RegionalOperator),
            region: region_pda(region_id),
            region_state: region_state(region_id),
            proposal: proposal_pda(proposal_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn vote_ix(voter: &Pubkey, region_id: u16, proposal_id: u64, vote: Vote, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::VoteOnRegionProposal { region_id, vote, amount }.data(),
        education_regions::accounts::VoteOnRegionProposal {
            voter: *voter,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            voter_token: token_acc(voter),
            vault: vault(),
            region_state: region_state(region_id),
            proposal: proposal_pda(proposal_id),
            vote_record: vote_record_pda(proposal_id, voter),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn finalize_ix(cranker: &Pubkey, region_id: u16, proposal_id: u64, proposer: &Pubkey, treasury_owner: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::FinalizeRegionProposal { region_id }.data(),
        education_regions::accounts::FinalizeRegionProposal {
            cranker: *cranker,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            region_state: region_state(region_id),
            proposal: proposal_pda(proposal_id),
            proposer: *proposer,
            proposer_token: token_acc(proposer),
            treasury: token_acc(treasury_owner),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn bid_ix(bidder: &Pubkey, region_id: u16, amount: u64, previous: Option<Pubkey>) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::BidOnRegion { region_id, amount }.data(),
        education_regions::accounts::BidOnRegion {
            bidder: *bidder,
            config: regions_config(),
            operator_role: role_pda(bidder, Role::RegionalOperator),
            xcav_mint: xcav_mint(),
            bidder_token: token_acc(bidder),
            vault: vault(),
            region_state: region_state(region_id),
            previous_bidder_token: previous.as_ref().map(token_acc),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn create_region_ix(creator: &Pubkey, region_id: u16) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::CreateNewRegion { region_id }.data(),
        education_regions::accounts::CreateNewRegion {
            creator: *creator,
            config: regions_config(),
            region_state: region_state(region_id),
            region: region_pda(region_id),
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn unlock_ix(voter: &Pubkey, proposal_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::UnlockVotingToken { proposal_id }.data(),
        education_regions::accounts::UnlockVotingToken {
            voter: *voter,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            voter_token: token_acc(voter),
            vault: vault(),
            vote_record: vote_record_pda(proposal_id, voter),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn clear_ix(cranker: &Pubkey, region_id: u16) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::ClearRegionState { region_id }.data(),
        education_regions::accounts::ClearRegionState {
            cranker: *cranker,
            region_state: region_state(region_id),
        }
        .to_account_metas(None),
    )
}

/// Push the clock past the proposal voting window so finalize can run.
pub fn warp_past_voting(svm: &mut LiteSVM) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp += 2_000;
    svm.set_sysvar(&clock);
}

// Loads both programs, seeds the XCAV mint, wires up a compliant
// RegionalOperator (with XCAV), and initializes the regions config. Returns
// (svm, operator, authority); the authority's XCAV account is the treasury.
pub fn setup() -> (LiteSVM, Keypair, Keypair) {
    let mut svm = LiteSVM::new();
    svm.add_program(
        roles_id(),
        include_bytes!("../../../../target/deploy/xcavate_roles.so"),
    )
    .unwrap();
    svm.add_program(
        rid(),
        include_bytes!("../../../../target/deploy/education_regions.so"),
    )
    .unwrap();
    set_mint(&mut svm);

    let authority = funded(&mut svm);
    give_xcav(&mut svm, &authority.pubkey(), 0); // treasury account
    ok(&mut svm, roles_init_ix(&authority.pubkey()), &authority, &[&authority]);

    let admin = funded(&mut svm);
    ok(&mut svm, roles_add_admin_ix(&authority.pubkey(), &admin.pubkey()), &authority, &[&authority]);

    let operator = actor(&mut svm);
    ok(
        &mut svm,
        roles_assign_ix(&admin.pubkey(), &operator.pubkey(), Role::RegionalOperator),
        &admin,
        &[&admin],
    );

    ok(&mut svm, regions_init_ix(&authority.pubkey()), &authority, &[&authority]);
    (svm, operator, authority)
}

pub fn next_proposal_id(svm: &LiteSVM) -> u64 {
    let acc = svm.get_account(&regions_config()).unwrap();
    RegionsConfig::try_deserialize(&mut &acc.data[..]).unwrap().proposal_counter
}

// Adds a second compliant RegionalOperator (with XCAV).
pub fn new_operator(svm: &mut LiteSVM, authority: &Keypair) -> Keypair {
    let admin = funded(svm);
    ok(svm, roles_add_admin_ix(&authority.pubkey(), &admin.pubkey()), authority, &[authority]);
    let op = actor(svm);
    ok(
        svm,
        roles_assign_ix(&admin.pubkey(), &op.pubkey(), Role::RegionalOperator),
        &admin,
        &[&admin],
    );
    op
}

// Drives region 1 to the Auctioning state and returns the proposal id.
pub fn reach_auctioning(svm: &mut LiteSVM, operator: &Keypair, authority: &Keypair) -> u64 {
    let id = next_proposal_id(svm);
    ok(svm, propose_ix(&operator.pubkey(), 1, id), operator, &[operator]);
    let voter = actor(svm);
    ok(svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
    warp_past_voting(svm);
    let cranker = funded(svm);
    ok(
        svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey(), &authority.pubkey()),
        &cranker,
        &[&cranker],
    );
    id
}

pub fn region_state_of(svm: &LiteSVM, region_id: u16) -> RegionState {
    RegionState::try_deserialize(&mut &svm.get_account(&region_state(region_id)).unwrap().data[..])
        .unwrap()
}

// --- owner removal / replacement ---

pub fn removal_proposal_pda(region_id: u16) -> Pubkey {
    Pubkey::find_program_address(
        &[education_regions::REMOVAL_PROPOSAL_SEED, &region_id.to_le_bytes()],
        &rid(),
    )
    .0
}
pub fn removal_vote_pda(proposal_id: u64, voter: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[education_regions::REMOVAL_VOTE_SEED, &proposal_id.to_le_bytes(), voter.as_ref()],
        &rid(),
    )
    .0
}
pub fn replacement_auction_pda(region_id: u16) -> Pubkey {
    Pubkey::find_program_address(
        &[education_regions::REPLACEMENT_AUCTION_SEED, &region_id.to_le_bytes()],
        &rid(),
    )
    .0
}

pub fn region_of(svm: &LiteSVM, region_id: u16) -> education_regions::state::Region {
    education_regions::state::Region::try_deserialize(
        &mut &svm.get_account(&region_pda(region_id)).unwrap().data[..],
    )
    .unwrap()
}

/// Advance the clock by `secs` seconds.
pub fn warp(svm: &mut LiteSVM, secs: i64) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp += secs;
    svm.set_sysvar(&clock);
}

/// Drives region 1 all the way to a created region owned by `operator` with a
/// 600M collateral.
pub fn reach_created(svm: &mut LiteSVM, operator: &Keypair, authority: &Keypair) {
    reach_auctioning(svm, operator, authority);
    ok(svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), operator, &[operator]);
    warp_past_voting(svm);
    ok(svm, create_region_ix(&operator.pubkey(), 1), operator, &[operator]);
}

/// Drives region 1 to a created region, then opens the seat via the operator's
/// resignation and warping past the notice period (5_000).
pub fn reach_seat_open(svm: &mut LiteSVM, operator: &Keypair, authority: &Keypair) {
    reach_created(svm, operator, authority);
    ok(svm, resign_ix(&operator.pubkey(), 1), operator, &[operator]);
    warp(svm, 6_000);
}

pub fn propose_remove_ix(proposer: &Pubkey, region_id: u16) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::ProposeRemoveOperator { region_id }.data(),
        education_regions::accounts::ProposeRemoveOperator {
            proposer: *proposer,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            proposer_token: token_acc(proposer),
            vault: vault(),
            region: region_pda(region_id),
            removal_proposal: removal_proposal_pda(region_id),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn vote_removal_ix(voter: &Pubkey, region_id: u16, proposal_id: u64, vote: Vote, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::VoteOnRemoval { region_id, vote, amount }.data(),
        education_regions::accounts::VoteOnRemoval {
            voter: *voter,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            voter_token: token_acc(voter),
            vault: vault(),
            removal_proposal: removal_proposal_pda(region_id),
            vote_record: removal_vote_pda(proposal_id, voter),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn finalize_removal_ix(cranker: &Pubkey, region_id: u16, proposer: &Pubkey, treasury_owner: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::FinalizeRemoval { region_id }.data(),
        education_regions::accounts::FinalizeRemoval {
            cranker: *cranker,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            region: region_pda(region_id),
            removal_proposal: removal_proposal_pda(region_id),
            proposer: *proposer,
            proposer_token: token_acc(proposer),
            treasury: token_acc(treasury_owner),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn unlock_removal_ix(voter: &Pubkey, proposal_id: u64) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::UnlockRemovalVote { proposal_id }.data(),
        education_regions::accounts::UnlockRemovalVote {
            voter: *voter,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            voter_token: token_acc(voter),
            vault: vault(),
            vote_record: removal_vote_pda(proposal_id, voter),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn bid_replacement_ix(bidder: &Pubkey, region_id: u16, amount: u64, previous: Option<Pubkey>) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::BidOnReplacement { region_id, amount }.data(),
        education_regions::accounts::BidOnReplacement {
            bidder: *bidder,
            config: regions_config(),
            operator_role: role_pda(bidder, Role::RegionalOperator),
            xcav_mint: xcav_mint(),
            bidder_token: token_acc(bidder),
            vault: vault(),
            region: region_pda(region_id),
            auction: replacement_auction_pda(region_id),
            previous_bidder_token: previous.as_ref().map(token_acc),
            token_program: TOKEN_PROGRAM_ID,
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

pub fn finalize_replacement_ix(cranker: &Pubkey, region_id: u16, old_owner: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
        &education_regions::instruction::FinalizeReplacement { region_id }.data(),
        education_regions::accounts::FinalizeReplacement {
            cranker: *cranker,
            config: regions_config(),
            xcav_mint: xcav_mint(),
            vault: vault(),
            region: region_pda(region_id),
            auction: replacement_auction_pda(region_id),
            old_owner_token: token_acc(old_owner),
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn resign_ix(operator: &Pubkey, region_id: u16) -> Instruction {
    Instruction::new_with_bytes(
        rid(),
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
