//! Tests for region governance.
//!
//! Governance stakes are XCAV (a classic SPL token) held in the program vault.
//! LiteSVM loads the SPL Token program by default, and the XCAV mint and each
//! participant's token account are seeded directly with `set_account`. The roles
//! program is loaded alongside regions so the cross-program RegionalOperator gate
//! is exercised for real.

use anchor_lang::solana_program::clock::Clock;
use anchor_lang::solana_program::program_option::COption;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_lang::{
    prelude::Pubkey, solana_program::instruction::Instruction, AccountDeserialize,
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

use education_regions::instructions::ConfigParams;
use education_regions::state::{
    Config as RegionsConfig, RegionProposal, RegionState, RegionStatus, Vote, VoteRecord,
};
use education_regions::{CONFIG_SEED, PROPOSAL_SEED, REGION_SEED, REGION_STATE_SEED, VAULT_SEED, VOTE_SEED};
use xcavate_roles::state::Role;

const SYS: Pubkey = anchor_lang::system_program::ID;
const DEPOSIT: u64 = 1_000_000_000;
const DECIMALS: u8 = 6;
const FUND_XCAV: u64 = 100_000_000_000;

// --- ids / PDAs ---

fn rid() -> Pubkey {
    education_regions::id()
}
fn roles_id() -> Pubkey {
    xcavate_roles::id()
}

fn regions_config() -> Pubkey {
    Pubkey::find_program_address(&[CONFIG_SEED], &rid()).0
}
fn vault() -> Pubkey {
    Pubkey::find_program_address(&[VAULT_SEED], &rid()).0
}
fn region_state(region_id: u16) -> Pubkey {
    Pubkey::find_program_address(&[REGION_STATE_SEED, &region_id.to_le_bytes()], &rid()).0
}
fn proposal_pda(proposal_id: u64) -> Pubkey {
    Pubkey::find_program_address(&[PROPOSAL_SEED, &proposal_id.to_le_bytes()], &rid()).0
}
fn vote_record_pda(proposal_id: u64, voter: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[VOTE_SEED, &proposal_id.to_le_bytes(), voter.as_ref()],
        &rid(),
    )
    .0
}
fn region_pda(region_id: u16) -> Pubkey {
    Pubkey::find_program_address(&[REGION_SEED, &region_id.to_le_bytes()], &rid()).0
}

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

// --- XCAV mint / token accounts (seeded directly) ---

/// Fixed address for the test XCAV mint.
fn xcav_mint() -> Pubkey {
    Pubkey::new_from_array([7u8; 32])
}

/// Deterministic XCAV token account for an owner. Not a real ATA — the program
/// only checks the mint and authority, so any token account works.
fn token_acc(owner: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"xcav_token", owner.as_ref()], &rid()).0
}

fn set_mint(svm: &mut LiteSVM) {
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

fn give_xcav(svm: &mut LiteSVM, owner: &Pubkey, amount: u64) {
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

fn xcav_balance(svm: &LiteSVM, owner: &Pubkey) -> u64 {
    let acc = svm.get_account(&token_acc(owner)).unwrap();
    SplAccount::unpack(&acc.data).unwrap().amount
}

fn vault_balance(svm: &LiteSVM) -> u64 {
    let acc = svm.get_account(&vault()).unwrap();
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
        panic!("expected success, failed with: {:?}", failed.err);
    }
}

fn fails_with(svm: &mut LiteSVM, ix: Instruction, payer: &Keypair, signers: &[&Keypair], expected: &str) {
    match process(svm, ix, payer, signers) {
        Ok(_) => panic!("expected failure `{expected}`, but it succeeded"),
        Err(failed) => {
            let detail = format!("{:?}\n{}", failed.err, failed.meta.logs.join("\n"));
            assert!(detail.contains(expected), "expected `{expected}`, got:\n{detail}");
        }
    }
}

/// A SOL-funded keypair (for fees + account rent).
fn funded(svm: &mut LiteSVM) -> Keypair {
    let kp = Keypair::new();
    svm.airdrop(&kp.pubkey(), 100_000_000_000).unwrap();
    kp
}

/// A SOL-funded keypair that also holds XCAV.
fn actor(svm: &mut LiteSVM) -> Keypair {
    let kp = funded(svm);
    give_xcav(svm, &kp.pubkey(), FUND_XCAV);
    kp
}

// --- roles instruction builders ---

fn roles_init_ix(authority: &Pubkey) -> Instruction {
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

// --- regions instruction builders ---

fn default_params() -> ConfigParams {
    ConfigParams {
        proposal_deposit: DEPOSIT,
        minimum_voting_amount: 100_000_000,
        minimum_region_deposit: 500_000_000,
        voting_period: 1_000,
        auction_period: 1_000,
        owner_change_period: 10_000,
        threshold_bps: 5_000,
        quorum: 100_000_000,
    }
}

fn regions_init_ix(authority: &Pubkey) -> Instruction {
    regions_init_ix_with(authority, default_params())
}

// The authority's own XCAV account doubles as the treasury in tests.
fn regions_init_ix_with(authority: &Pubkey, params: ConfigParams) -> Instruction {
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

fn update_config_ix(authority: &Pubkey, treasury_owner: &Pubkey, params: ConfigParams) -> Instruction {
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

fn update_authority_ix(authority: &Pubkey, new_authority: &Pubkey) -> Instruction {
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

fn propose_ix(proposer: &Pubkey, region_id: u16, proposal_id: u64) -> Instruction {
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

fn vote_ix(voter: &Pubkey, region_id: u16, proposal_id: u64, vote: Vote, amount: u64) -> Instruction {
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

fn finalize_ix(cranker: &Pubkey, region_id: u16, proposal_id: u64, proposer: &Pubkey, treasury_owner: &Pubkey) -> Instruction {
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

fn bid_ix(bidder: &Pubkey, region_id: u16, amount: u64, previous: Option<Pubkey>) -> Instruction {
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

fn create_region_ix(creator: &Pubkey, region_id: u16) -> Instruction {
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

fn unlock_ix(voter: &Pubkey, proposal_id: u64) -> Instruction {
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

fn clear_ix(cranker: &Pubkey, region_id: u16) -> Instruction {
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
fn warp_past_voting(svm: &mut LiteSVM) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp += 2_000;
    svm.set_sysvar(&clock);
}

// Loads both programs, seeds the XCAV mint, wires up a compliant
// RegionalOperator (with XCAV), and initializes the regions config. Returns
// (svm, operator, authority); the authority's XCAV account is the treasury.
fn setup() -> (LiteSVM, Keypair, Keypair) {
    let mut svm = LiteSVM::new();
    svm.add_program(
        roles_id(),
        include_bytes!("../../../target/deploy/xcavate_roles.so"),
    )
    .unwrap();
    svm.add_program(
        rid(),
        include_bytes!("../../../target/deploy/education_regions.so"),
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

fn next_proposal_id(svm: &LiteSVM) -> u64 {
    let acc = svm.get_account(&regions_config()).unwrap();
    RegionsConfig::try_deserialize(&mut &acc.data[..]).unwrap().proposal_counter
}

// Adds a second compliant RegionalOperator (with XCAV).
fn new_operator(svm: &mut LiteSVM, authority: &Keypair) -> Keypair {
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
fn reach_auctioning(svm: &mut LiteSVM, operator: &Keypair, authority: &Keypair) -> u64 {
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

fn region_state_of(svm: &LiteSVM, region_id: u16) -> RegionState {
    RegionState::try_deserialize(&mut &svm.get_account(&region_state(region_id)).unwrap().data[..])
        .unwrap()
}

// ============================ propose ============================

#[test]
fn propose_new_region_works() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    let op_before = xcav_balance(&svm, &operator.pubkey());

    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let acc = svm.get_account(&proposal_pda(id)).unwrap();
    let proposal = RegionProposal::try_deserialize(&mut &acc.data[..]).unwrap();
    assert_eq!(proposal.proposer, operator.pubkey());
    assert_eq!(proposal.region_id, 1);
    assert_eq!(proposal.deposit, DEPOSIT);
    assert_eq!(proposal.yes_power, 0);
    // Deposit moved from proposer into the vault.
    assert_eq!(op_before - xcav_balance(&svm, &operator.pubkey()), DEPOSIT);
    assert_eq!(vault_balance(&svm), DEPOSIT);
    // Counter advanced.
    assert_eq!(next_proposal_id(&svm), id + 1);
}

#[test]
fn propose_fails_for_non_operator() {
    let (mut svm, _operator, _authority) = setup();
    let stranger = actor(&mut svm);
    let id = next_proposal_id(&svm);
    // No RegionalOperator role -> the role PDA doesn't exist.
    fails_with(
        &mut svm,
        propose_ix(&stranger.pubkey(), 1, id),
        &stranger,
        &[&stranger],
        "AccountNotInitialized",
    );
}

#[test]
fn propose_fails_for_unknown_region() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    fails_with(
        &mut svm,
        propose_ix(&operator.pubkey(), 99, id),
        &operator,
        &[&operator],
        "InvalidRegion",
    );
}

#[test]
fn propose_fails_when_region_already_has_proposal() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    // Second proposal for region 1 -> the region pointer already exists.
    let id2 = next_proposal_id(&svm);
    fails_with(
        &mut svm,
        propose_ix(&operator.pubkey(), 1, id2),
        &operator,
        &[&operator],
        "already in use",
    );
}

// ============================ vote ============================

#[test]
fn vote_records_power() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let voter = actor(&mut svm);
    let voter_before = xcav_balance(&svm, &voter.pubkey());
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);

    let proposal = RegionProposal::try_deserialize(
        &mut &svm.get_account(&proposal_pda(id)).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(proposal.yes_power, 200_000_000);

    let vr_acc = svm.get_account(&vote_record_pda(id, &voter.pubkey())).unwrap();
    let vr = VoteRecord::try_deserialize(&mut &vr_acc.data[..]).unwrap();
    assert_eq!(vr.power, 200_000_000);
    assert_eq!(vr.vote, Vote::Yes);
    // Power moved from voter into the vault (on top of the proposal deposit).
    assert_eq!(voter_before - xcav_balance(&svm, &voter.pubkey()), 200_000_000);
    assert_eq!(vault_balance(&svm), DEPOSIT + 200_000_000);
}

#[test]
fn vote_below_minimum_fails() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let voter = actor(&mut svm);
    fails_with(
        &mut svm,
        vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 1),
        &voter,
        &[&voter],
        "BelowMinimumVotingAmount",
    );
}

#[test]
fn revote_replaces_previous_vote() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let voter = actor(&mut svm);
    let voter_before = xcav_balance(&svm, &voter.pubkey());
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
    // Change vote to No with a different amount.
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::No, 300_000_000), &voter, &[&voter]);

    let proposal = RegionProposal::try_deserialize(
        &mut &svm.get_account(&proposal_pda(id)).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(proposal.yes_power, 0);
    assert_eq!(proposal.no_power, 300_000_000);

    let vr = VoteRecord::try_deserialize(
        &mut &svm.get_account(&vote_record_pda(id, &voter.pubkey())).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(vr.vote, Vote::No);
    assert_eq!(vr.power, 300_000_000);
    // Net XCAV locked equals only the new vote; the old lock was refunded.
    assert_eq!(voter_before - xcav_balance(&svm, &voter.pubkey()), 300_000_000);
    assert_eq!(vault_balance(&svm), DEPOSIT + 300_000_000);
}

// ============================ finalize ============================

#[test]
fn finalize_passes_and_starts_auction() {
    let (mut svm, operator, authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);

    let op_before = xcav_balance(&svm, &operator.pubkey());
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey(), &authority.pubkey()),
        &cranker,
        &[&cranker],
    );

    let rs = region_state_of(&svm, 1);
    assert_eq!(rs.status, RegionStatus::Auctioning);
    assert_eq!(rs.collateral, 500_000_000); // minimum_region_deposit
    // Deposit returned to the proposer; only the voter's lock remains in the vault.
    assert_eq!(xcav_balance(&svm, &operator.pubkey()) - op_before, DEPOSIT);
    assert_eq!(vault_balance(&svm), 200_000_000);
    // Proposal account was closed.
    assert!(svm.get_account(&proposal_pda(id)).map_or(true, |a| a.data.is_empty()));
}

#[test]
fn finalize_rejects_and_slashes_deposit() {
    let (mut svm, operator, authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    // No votes -> quorum not met -> rejected.

    let treasury_before = xcav_balance(&svm, &authority.pubkey());
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey(), &authority.pubkey()),
        &cranker,
        &[&cranker],
    );

    let rs = region_state_of(&svm, 1);
    assert_eq!(rs.status, RegionStatus::Rejected);
    // Deposit slashed from the vault to the treasury.
    assert_eq!(xcav_balance(&svm, &authority.pubkey()) - treasury_before, DEPOSIT);
    assert_eq!(vault_balance(&svm), 0);
}

#[test]
fn finalize_fails_while_voting_ongoing() {
    let (mut svm, operator, authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let cranker = funded(&mut svm);
    fails_with(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey(), &authority.pubkey()),
        &cranker,
        &[&cranker],
        "VotingStillOngoing",
    );
}

// ============================ bid ============================

#[test]
fn bid_places_and_outbid_refunds() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    let vault_before = vault_balance(&svm); // the winning voter's lock still sits here

    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);
    let rs = region_state_of(&svm, 1);
    assert_eq!(rs.highest_bidder, Some(operator.pubkey()));
    assert_eq!(rs.collateral, 600_000_000);
    assert_eq!(vault_balance(&svm), vault_before + 600_000_000);

    let op2 = new_operator(&mut svm, &authority);
    let op1_before = xcav_balance(&svm, &operator.pubkey());
    ok(
        &mut svm,
        bid_ix(&op2.pubkey(), 1, 700_000_000, Some(operator.pubkey())),
        &op2,
        &[&op2],
    );

    let rs = region_state_of(&svm, 1);
    assert_eq!(rs.highest_bidder, Some(op2.pubkey()));
    assert_eq!(rs.collateral, 700_000_000);
    // Operator was refunded their exact outbid collateral.
    assert_eq!(xcav_balance(&svm, &operator.pubkey()) - op1_before, 600_000_000);
    // Vault now holds the new top bid (plus the earlier voter lock).
    assert_eq!(vault_balance(&svm), vault_before + 700_000_000);
}

#[test]
fn bid_fails_before_auction() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    // Still in Proposing, not Auctioning.
    fails_with(
        &mut svm,
        bid_ix(&operator.pubkey(), 1, 600_000_000, None),
        &operator,
        &[&operator],
        "NotAuctioning",
    );
}

// ============================ create_new_region ============================

#[test]
fn create_region_works() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);
    let vault_before = vault_balance(&svm);

    warp_past_voting(&mut svm); // push past the auction expiry too
    ok(&mut svm, create_region_ix(&operator.pubkey(), 1), &operator, &[&operator]);

    let region_acc = svm.get_account(&region_pda(1)).unwrap();
    let region = education_regions::state::Region::try_deserialize(&mut &region_acc.data[..]).unwrap();
    assert_eq!(region.region_id, 1);
    assert_eq!(region.owner, operator.pubkey());
    assert_eq!(region.collateral, 600_000_000);
    // Collateral stays in the vault; the region just records it.
    assert_eq!(vault_balance(&svm), vault_before);
    // Region state was closed.
    assert!(svm.get_account(&region_state(1)).map_or(true, |a| a.data.is_empty()));
}

#[test]
fn create_region_fails_for_non_winner() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);

    let op2 = new_operator(&mut svm, &authority);
    warp_past_voting(&mut svm);
    fails_with(
        &mut svm,
        create_region_ix(&op2.pubkey(), 1),
        &op2,
        &[&op2],
        "NotWinner",
    );
}

// ============================ cleanup ============================

#[test]
fn unlock_voting_token_works() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
    let voter_before = xcav_balance(&svm, &voter.pubkey());

    warp_past_voting(&mut svm);
    ok(&mut svm, unlock_ix(&voter.pubkey(), id), &voter, &[&voter]);

    // Vote record closed and the locked power returned as XCAV.
    assert!(svm.get_account(&vote_record_pda(id, &voter.pubkey())).map_or(true, |a| a.data.is_empty()));
    assert_eq!(xcav_balance(&svm, &voter.pubkey()) - voter_before, 200_000_000);
    // Only the proposal deposit is left in the vault.
    assert_eq!(vault_balance(&svm), DEPOSIT);
}

#[test]
fn clear_region_state_after_reject() {
    let (mut svm, operator, authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    // No votes -> rejected on finalize.
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey(), &authority.pubkey()),
        &cranker,
        &[&cranker],
    );
    assert_eq!(region_state_of(&svm, 1).status, RegionStatus::Rejected);

    ok(&mut svm, clear_ix(&cranker.pubkey(), 1), &cranker, &[&cranker]);
    assert!(svm.get_account(&region_state(1)).map_or(true, |a| a.data.is_empty()));
}

// ============================ config ============================

#[test]
fn init_rejects_bad_threshold() {
    let mut svm = LiteSVM::new();
    svm.add_program(rid(), include_bytes!("../../../target/deploy/education_regions.so"))
        .unwrap();
    set_mint(&mut svm);
    let authority = funded(&mut svm);
    give_xcav(&mut svm, &authority.pubkey(), 0); // treasury account

    let mut params = default_params();
    params.threshold_bps = 10_001; // above 100%
    fails_with(
        &mut svm,
        regions_init_ix_with(&authority.pubkey(), params),
        &authority,
        &[&authority],
        "InvalidConfig",
    );
}

#[test]
fn init_rejects_non_xcav_treasury() {
    let mut svm = LiteSVM::new();
    svm.add_program(rid(), include_bytes!("../../../target/deploy/education_regions.so"))
        .unwrap();
    set_mint(&mut svm);
    let authority = funded(&mut svm);

    // A token account for a *different* mint, parked where the treasury is read.
    let other_mint = Pubkey::new_from_array([9u8; 32]);
    let m = SplMint {
        mint_authority: COption::None,
        supply: 0,
        decimals: DECIMALS,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    let mut md = vec![0u8; SplMint::LEN];
    m.pack_into_slice(&mut md);
    svm.set_account(
        other_mint,
        Account { lamports: 100_000_000, data: md, owner: TOKEN_PROGRAM_ID, executable: false, rent_epoch: 0 },
    )
    .unwrap();
    let a = SplAccount {
        mint: other_mint,
        owner: authority.pubkey(),
        amount: 0,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut ad = vec![0u8; SplAccount::LEN];
    a.pack_into_slice(&mut ad);
    svm.set_account(
        token_acc(&authority.pubkey()),
        Account { lamports: 100_000_000, data: ad, owner: TOKEN_PROGRAM_ID, executable: false, rent_epoch: 0 },
    )
    .unwrap();

    // The treasury must hold the XCAV mint, so init is rejected.
    fails_with(
        &mut svm,
        regions_init_ix(&authority.pubkey()),
        &authority,
        &[&authority],
        "ConstraintTokenMint",
    );
}

#[test]
fn update_config_by_authority_works() {
    let (mut svm, _operator, authority) = setup();

    let mut params = default_params();
    params.minimum_voting_amount = 250_000_000;
    ok(&mut svm, update_config_ix(&authority.pubkey(), &authority.pubkey(), params), &authority, &[&authority]);

    let cfg = RegionsConfig::try_deserialize(
        &mut &svm.get_account(&regions_config()).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(cfg.minimum_voting_amount, 250_000_000);
}

#[test]
fn update_config_by_non_authority_fails() {
    let (mut svm, _operator, authority) = setup();
    let stranger = funded(&mut svm);
    fails_with(
        &mut svm,
        update_config_ix(&stranger.pubkey(), &authority.pubkey(), default_params()),
        &stranger,
        &[&stranger],
        "NotAuthority",
    );
}

#[test]
fn update_config_rejects_bad_params() {
    let (mut svm, _operator, authority) = setup();
    let mut params = default_params();
    params.quorum = 0;
    fails_with(
        &mut svm,
        update_config_ix(&authority.pubkey(), &authority.pubkey(), params),
        &authority,
        &[&authority],
        "InvalidConfig",
    );
}

#[test]
fn update_authority_rotates() {
    let (mut svm, _operator, authority) = setup();
    let new_auth = funded(&mut svm);
    ok(
        &mut svm,
        update_authority_ix(&authority.pubkey(), &new_auth.pubkey()),
        &authority,
        &[&authority],
    );

    // The old authority can no longer update the config.
    fails_with(
        &mut svm,
        update_config_ix(&authority.pubkey(), &authority.pubkey(), default_params()),
        &authority,
        &[&authority],
        "NotAuthority",
    );
    // The new authority can (treasury stays the original XCAV account).
    ok(
        &mut svm,
        update_config_ix(&new_auth.pubkey(), &authority.pubkey(), default_params()),
        &new_auth,
        &[&new_auth],
    );
}
