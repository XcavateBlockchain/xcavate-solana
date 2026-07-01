//! Config lifecycle: initialization validation, parameter and treasury updates,
//! and authority rotation.

mod common;
use common::*;

use anchor_lang::solana_program::program_option::COption;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_spl::token::spl_token::state::{Account as SplAccount, AccountState, Mint as SplMint};
use anchor_spl::token::ID as TOKEN_PROGRAM_ID;
use solana_account::Account;

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
