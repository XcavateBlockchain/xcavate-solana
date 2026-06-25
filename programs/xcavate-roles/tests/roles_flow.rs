//! Behavioural tests for the roles & compliance registry.
//!
//! The program does no token transfers, so LiteSVM covers every path end to
//! end — there's nothing here that needs a Surfpool integration run.

use anchor_lang::{
    prelude::Pubkey, solana_program::instruction::Instruction, AccountDeserialize,
    InstructionData, ToAccountMetas,
};
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use litesvm::LiteSVM;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;
use xcavate_roles::state::{AccessPermission, Admin, Config, Role, RoleAccount};
use xcavate_roles::{ADMIN_SEED, CONFIG_SEED, ROLE_SEED};

const SYS: Pubkey = anchor_lang::system_program::ID;

// --- PDA helpers ---

fn pid() -> Pubkey {
    xcavate_roles::id()
}

fn config_pda() -> Pubkey {
    Pubkey::find_program_address(&[CONFIG_SEED], &pid()).0
}

fn admin_pda(who: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[ADMIN_SEED, who.as_ref()], &pid()).0
}

fn role_pda(user: &Pubkey, role: Role) -> Pubkey {
    Pubkey::find_program_address(&[ROLE_SEED, user.as_ref(), &[role.seed_byte()]], &pid()).0
}

// --- instruction builders ---

fn init_ix(authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        pid(),
        &xcavate_roles::instruction::InitializeConfig {}.data(),
        xcavate_roles::accounts::InitializeConfig {
            authority: *authority,
            config: config_pda(),
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

fn add_admin_ix(authority: &Pubkey, new_admin: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        pid(),
        &xcavate_roles::instruction::AddAdmin {}.data(),
        xcavate_roles::accounts::AddAdmin {
            authority: *authority,
            config: config_pda(),
            new_admin: *new_admin,
            admin: admin_pda(new_admin),
            system_program: SYS,
        }
        .to_account_metas(None),
    )
}

fn remove_admin_ix(authority: &Pubkey, target: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        pid(),
        &xcavate_roles::instruction::RemoveAdmin {}.data(),
        xcavate_roles::accounts::RemoveAdmin {
            authority: *authority,
            config: config_pda(),
            admin: admin_pda(target),
        }
        .to_account_metas(None),
    )
}

fn assign_ix(admin: &Pubkey, user: &Pubkey, role: Role) -> Instruction {
    Instruction::new_with_bytes(
        pid(),
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

fn remove_role_ix(admin: &Pubkey, user: &Pubkey, role: Role) -> Instruction {
    Instruction::new_with_bytes(
        pid(),
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

fn set_perm_ix(admin: &Pubkey, user: &Pubkey, role: Role, permission: AccessPermission) -> Instruction {
    Instruction::new_with_bytes(
        pid(),
        &xcavate_roles::instruction::SetPermission { role, permission }.data(),
        xcavate_roles::accounts::SetPermission {
            admin_signer: *admin,
            admin: admin_pda(admin),
            user: *user,
            role_account: role_pda(user, role),
        }
        .to_account_metas(None),
    )
}

fn update_authority_ix(authority: &Pubkey, new_authority: Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        pid(),
        &xcavate_roles::instruction::UpdateAuthority { new_authority }.data(),
        xcavate_roles::accounts::UpdateAuthority {
            authority: *authority,
            config: config_pda(),
        }
        .to_account_metas(None),
    )
}

// --- send helpers ---

fn process(
    svm: &mut LiteSVM,
    ix: Instruction,
    payer: &Keypair,
    signers: &[&Keypair],
) -> Result<TransactionMetadata, FailedTransactionMetadata> {
    // Fresh blockhash per send: otherwise two identical instructions (e.g. a
    // double-assign) hash to the same signature and the runtime rejects the
    // retry as `AlreadyProcessed` before the program ever runs.
    svm.expire_blockhash();
    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx)
}

fn ok(svm: &mut LiteSVM, ix: Instruction, payer: &Keypair, signers: &[&Keypair]) {
    if let Err(failed) = process(svm, ix, payer, signers) {
        panic!("expected tx to succeed, failed with: {:?}", failed.err);
    }
}

/// Assert the tx fails AND that the Anchor error matches `expected` (matched
/// against the program logs, e.g. "NotAuthority", "AccountNotInitialized").
fn fails_with(
    svm: &mut LiteSVM,
    ix: Instruction,
    payer: &Keypair,
    signers: &[&Keypair],
    expected: &str,
) {
    match process(svm, ix, payer, signers) {
        Ok(_) => panic!("expected tx to fail with `{expected}`, but it succeeded"),
        Err(failed) => {
            let detail = format!("{:?}\n{}", failed.err, failed.meta.logs.join("\n"));
            assert!(
                detail.contains(expected),
                "expected error `{expected}`, got:\n{detail}",
            );
        }
    }
}

fn funded(svm: &mut LiteSVM) -> Keypair {
    let kp = Keypair::new();
    svm.airdrop(&kp.pubkey(), 10_000_000_000).unwrap();
    kp
}

// Fresh SVM with the program loaded and an initialized config whose sudo is
// `authority`.
fn setup() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
    svm.add_program(
        pid(),
        include_bytes!("../../../target/deploy/xcavate_roles.so"),
    )
    .unwrap();
    let authority = funded(&mut svm);
    ok(&mut svm, init_ix(&authority.pubkey()), &authority, &[&authority]);
    (svm, authority)
}

// As above, plus one registered admin.
fn setup_with_admin() -> (LiteSVM, Keypair, Keypair) {
    let (mut svm, authority) = setup();
    let admin = funded(&mut svm);
    ok(&mut svm, add_admin_ix(&authority.pubkey(), &admin.pubkey()), &authority, &[&authority]);
    (svm, authority, admin)
}

fn read_config(svm: &LiteSVM) -> Config {
    let acc = svm.get_account(&config_pda()).unwrap();
    Config::try_deserialize(&mut &acc.data[..]).unwrap()
}

fn read_role(svm: &LiteSVM, user: &Pubkey, role: Role) -> RoleAccount {
    let acc = svm.get_account(&role_pda(user, role)).unwrap();
    RoleAccount::try_deserialize(&mut &acc.data[..]).unwrap()
}

// ============================ add_admin ============================

#[test]
fn add_admin_works() {
    let (mut svm, authority) = setup();
    let admin = Keypair::new().pubkey();
    ok(&mut svm, add_admin_ix(&authority.pubkey(), &admin), &authority, &[&authority]);

    let acc = svm.get_account(&admin_pda(&admin)).unwrap();
    let parsed = Admin::try_deserialize(&mut &acc.data[..]).unwrap();
    assert_eq!(parsed.admin, admin);
}

#[test]
fn add_admin_fails_for_non_authority() {
    let (mut svm, _authority) = setup();
    let imposter = funded(&mut svm);
    let admin = Keypair::new().pubkey();
    fails_with(&mut svm, add_admin_ix(&imposter.pubkey(), &admin), &imposter, &[&imposter], "NotAuthority");
}

#[test]
fn add_admin_fails_when_already_admin() {
    let (mut svm, authority, admin) = setup_with_admin();
    // Re-registering the same admin hits the `init` reinit guard.
    fails_with(&mut svm, add_admin_ix(&authority.pubkey(), &admin.pubkey()), &authority, &[&authority], "already in use");
}

// ============================ remove_admin ============================

#[test]
fn remove_admin_works() {
    let (mut svm, authority, admin) = setup_with_admin();
    ok(&mut svm, remove_admin_ix(&authority.pubkey(), &admin.pubkey()), &authority, &[&authority]);
    assert!(svm.get_account(&admin_pda(&admin.pubkey())).map_or(true, |a| a.data.is_empty()));
}

#[test]
fn remove_admin_fails_for_non_authority() {
    let (mut svm, _authority, admin) = setup_with_admin();
    let imposter = funded(&mut svm);
    fails_with(&mut svm, remove_admin_ix(&imposter.pubkey(), &admin.pubkey()), &imposter, &[&imposter], "NotAuthority");
}

#[test]
fn remove_admin_fails_when_not_admin() {
    let (mut svm, authority) = setup();
    let never_admin = Keypair::new().pubkey();
    fails_with(&mut svm, remove_admin_ix(&authority.pubkey(), &never_admin), &authority, &[&authority], "AccountNotInitialized");
}

// ============================ assign_role ============================

#[test]
fn assign_role_works() {
    let (mut svm, _authority, admin) = setup_with_admin();
    let user = Keypair::new().pubkey();
    ok(&mut svm, assign_ix(&admin.pubkey(), &user, Role::ModuleCreator), &admin, &[&admin]);

    let parsed = read_role(&svm, &user, Role::ModuleCreator);
    assert_eq!(parsed.user, user);
    assert_eq!(parsed.role, Role::ModuleCreator);
    assert!(parsed.is_compliant());
    // A role that was never granted has no account.
    assert!(svm.get_account(&role_pda(&user, Role::ModuleBooker)).map_or(true, |a| a.data.is_empty()));
}

#[test]
fn assign_role_fails_when_already_assigned() {
    let (mut svm, _authority, admin) = setup_with_admin();
    let user = Keypair::new().pubkey();
    ok(&mut svm, assign_ix(&admin.pubkey(), &user, Role::ModuleBooker), &admin, &[&admin]);
    fails_with(&mut svm, assign_ix(&admin.pubkey(), &user, Role::ModuleBooker), &admin, &[&admin], "already in use");
}

#[test]
fn assign_role_fails_for_non_admin() {
    let (mut svm, _authority) = setup();
    let imposter = funded(&mut svm);
    let user = Keypair::new().pubkey();
    fails_with(&mut svm, assign_ix(&imposter.pubkey(), &user, Role::ModuleBooker), &imposter, &[&imposter], "AccountNotInitialized");
}

// ============================ remove_role ============================

#[test]
fn remove_role_works() {
    let (mut svm, _authority, admin) = setup_with_admin();
    let user = Keypair::new().pubkey();
    ok(&mut svm, assign_ix(&admin.pubkey(), &user, Role::ModuleSponsor), &admin, &[&admin]);
    ok(&mut svm, remove_role_ix(&admin.pubkey(), &user, Role::ModuleSponsor), &admin, &[&admin]);
    assert!(svm.get_account(&role_pda(&user, Role::ModuleSponsor)).map_or(true, |a| a.data.is_empty()));
}

#[test]
fn remove_role_fails_for_non_admin() {
    let (mut svm, _authority, admin) = setup_with_admin();
    let user = Keypair::new().pubkey();
    ok(&mut svm, assign_ix(&admin.pubkey(), &user, Role::ModuleSponsor), &admin, &[&admin]);
    let imposter = funded(&mut svm);
    fails_with(&mut svm, remove_role_ix(&imposter.pubkey(), &user, Role::ModuleSponsor), &imposter, &[&imposter], "AccountNotInitialized");
}

#[test]
fn remove_role_fails_when_not_assigned() {
    let (mut svm, _authority, admin) = setup_with_admin();
    let user = Keypair::new().pubkey();
    fails_with(&mut svm, remove_role_ix(&admin.pubkey(), &user, Role::ModuleSponsor), &admin, &[&admin], "AccountNotInitialized");
}

// ============================ set_permission ============================

#[test]
fn set_permission_round_trip() {
    let (mut svm, _authority, admin) = setup_with_admin();
    let user = Keypair::new().pubkey();
    ok(&mut svm, assign_ix(&admin.pubkey(), &user, Role::ModuleCreator), &admin, &[&admin]);
    assert!(read_role(&svm, &user, Role::ModuleCreator).is_compliant());

    ok(&mut svm, set_perm_ix(&admin.pubkey(), &user, Role::ModuleCreator, AccessPermission::Revoked), &admin, &[&admin]);
    assert!(!read_role(&svm, &user, Role::ModuleCreator).is_compliant());

    ok(&mut svm, set_perm_ix(&admin.pubkey(), &user, Role::ModuleCreator, AccessPermission::Compliant), &admin, &[&admin]);
    assert!(read_role(&svm, &user, Role::ModuleCreator).is_compliant());
}

#[test]
fn set_permission_fails_when_role_not_assigned() {
    let (mut svm, _authority, admin) = setup_with_admin();
    let user = Keypair::new().pubkey();
    ok(&mut svm, assign_ix(&admin.pubkey(), &user, Role::ModuleCreator), &admin, &[&admin]);
    // Different, unassigned role -> no account to mutate.
    fails_with(&mut svm, set_perm_ix(&admin.pubkey(), &user, Role::ModuleBooker, AccessPermission::Revoked), &admin, &[&admin], "AccountNotInitialized");
}

#[test]
fn set_permission_fails_for_non_admin() {
    let (mut svm, _authority, admin) = setup_with_admin();
    let user = Keypair::new().pubkey();
    ok(&mut svm, assign_ix(&admin.pubkey(), &user, Role::ModuleCreator), &admin, &[&admin]);
    let imposter = funded(&mut svm);
    fails_with(&mut svm, set_perm_ix(&imposter.pubkey(), &user, Role::ModuleCreator, AccessPermission::Revoked), &imposter, &[&imposter], "AccountNotInitialized");
}

#[test]
fn set_permission_fails_when_already_set() {
    let (mut svm, _authority, admin) = setup_with_admin();
    let user = Keypair::new().pubkey();
    ok(&mut svm, assign_ix(&admin.pubkey(), &user, Role::ModuleCreator), &admin, &[&admin]);
    ok(&mut svm, set_perm_ix(&admin.pubkey(), &user, Role::ModuleCreator, AccessPermission::Revoked), &admin, &[&admin]);
    // Revoking again is a no-op the program rejects.
    fails_with(&mut svm, set_perm_ix(&admin.pubkey(), &user, Role::ModuleCreator, AccessPermission::Revoked), &admin, &[&admin], "PermissionAlreadySet");
}

// ============================ update_authority (Solana-only) ============================

#[test]
fn update_authority_works() {
    let (mut svm, authority) = setup();
    let new_authority = funded(&mut svm);
    ok(&mut svm, update_authority_ix(&authority.pubkey(), new_authority.pubkey()), &authority, &[&authority]);
    assert_eq!(read_config(&svm).authority, new_authority.pubkey());

    // Old authority can no longer manage admins; the new one can.
    let admin = Keypair::new().pubkey();
    fails_with(&mut svm, add_admin_ix(&authority.pubkey(), &admin), &authority, &[&authority], "NotAuthority");
    ok(&mut svm, add_admin_ix(&new_authority.pubkey(), &admin), &new_authority, &[&new_authority]);
}

#[test]
fn update_authority_fails_for_non_authority() {
    let (mut svm, _authority) = setup();
    let imposter = funded(&mut svm);
    fails_with(&mut svm, update_authority_ix(&imposter.pubkey(), imposter.pubkey()), &imposter, &[&imposter], "NotAuthority");
}
