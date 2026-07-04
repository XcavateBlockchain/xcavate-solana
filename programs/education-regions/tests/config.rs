//! Config lifecycle: initialization validation, parameter and treasury updates,
//! and authority rotation.

mod common;
use common::*;

use anchor_lang::solana_program::program_pack::Pack;
use anchor_spl::token::spl_token::state::Account as SplAccount;

#[test]
fn init_rejects_bad_threshold() {
    let mut svm = LiteSVM::new();
    svm.add_program(
        rid(),
        include_bytes!("../../../target/deploy/education_regions.so"),
    )
    .unwrap();
    set_mint(&mut svm);
    let authority = funded(&mut svm);
    bind_upgrade_authority(&mut svm, &rid(), &authority.pubkey());
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
fn init_creates_program_treasury() {
    let (svm, _operator, _authority) = setup();

    // The treasury is a program-owned XCAV account at a fixed PDA, so it can
    // never point at a foreign account or the wrong mint.
    let acc = svm.get_account(&treasury_pda()).unwrap();
    let token = SplAccount::unpack(&acc.data).unwrap();
    assert_eq!(token.mint, xcav_mint());
    assert_eq!(token.owner, regions_config());
    assert_eq!(token.amount, 0);

    let cfg =
        RegionsConfig::try_deserialize(&mut &svm.get_account(&regions_config()).unwrap().data[..])
            .unwrap();
    assert_eq!(cfg.treasury, treasury_pda());
}

#[test]
fn update_config_by_authority_works() {
    let (mut svm, _operator, authority) = setup();

    let mut params = default_params();
    params.minimum_voting_amount = 250_000_000;
    ok(
        &mut svm,
        update_config_ix(&authority.pubkey(), params),
        &authority,
        &[&authority],
    );

    let cfg =
        RegionsConfig::try_deserialize(&mut &svm.get_account(&regions_config()).unwrap().data[..])
            .unwrap();
    assert_eq!(cfg.minimum_voting_amount, 250_000_000);
}

#[test]
fn update_config_by_non_authority_fails() {
    let (mut svm, _operator, _authority) = setup();
    let stranger = funded(&mut svm);
    fails_with(
        &mut svm,
        update_config_ix(&stranger.pubkey(), default_params()),
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
        update_config_ix(&authority.pubkey(), params),
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
        update_config_ix(&authority.pubkey(), default_params()),
        &authority,
        &[&authority],
        "NotAuthority",
    );
    // The new authority can (treasury stays the original XCAV account).
    ok(
        &mut svm,
        update_config_ix(&new_auth.pubkey(), default_params()),
        &new_auth,
        &[&new_auth],
    );
}

#[test]
fn init_requires_upgrade_authority() {
    let mut svm = LiteSVM::new();
    svm.add_program(
        rid(),
        include_bytes!("../../../target/deploy/education_regions.so"),
    )
    .unwrap();
    set_mint(&mut svm);
    let deployer = funded(&mut svm);
    bind_upgrade_authority(&mut svm, &rid(), &deployer.pubkey());

    // Someone other than the deployer cannot claim the config.
    let imposter = funded(&mut svm);
    give_xcav(&mut svm, &imposter.pubkey(), 0);
    fails_with(
        &mut svm,
        regions_init_ix(&imposter.pubkey()),
        &imposter,
        &[&imposter],
        "NotUpgradeAuthority",
    );
}

#[test]
fn withdraw_treasury_works() {
    let (mut svm, _operator, authority) = setup();
    fund_treasury(&mut svm, 500_000_000);
    let receiver = actor(&mut svm);
    let before = xcav_balance(&svm, &receiver.pubkey());

    ok(
        &mut svm,
        withdraw_treasury_ix(&authority.pubkey(), &receiver.pubkey(), 200_000_000),
        &authority,
        &[&authority],
    );
    assert_eq!(treasury_balance(&svm), 300_000_000);
    assert_eq!(xcav_balance(&svm, &receiver.pubkey()) - before, 200_000_000);
}

#[test]
fn withdraw_treasury_fails_for_non_authority() {
    let (mut svm, _operator, _authority) = setup();
    fund_treasury(&mut svm, 500_000_000);
    let stranger = actor(&mut svm);
    fails_with(
        &mut svm,
        withdraw_treasury_ix(&stranger.pubkey(), &stranger.pubkey(), 1),
        &stranger,
        &[&stranger],
        "NotAuthority",
    );
}
