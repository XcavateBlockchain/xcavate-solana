mod common;
use common::*;

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
fn update_config_works() {
    let mut w = setup();
    let auth = w.authority.insecure_clone();
    let protocol = w.protocol.pubkey();
    let mut p = default_params();
    p.module_price = 250;
    ok(&mut w.svm, update_config_ix(&auth.pubkey(), &protocol, p), &auth, &[&auth]);
    assert_eq!(config(&w.svm).module_price, 250);
}

#[test]
fn update_config_requires_authority() {
    let mut w = setup();
    let protocol = w.protocol.pubkey();
    let mallory = actor(&mut w.svm);
    err(
        &mut w.svm,
        update_config_ix(&mallory.pubkey(), &protocol, default_params()),
        &mallory,
        &[&mallory],
        "NotAuthority",
    );
}

#[test]
fn update_config_rejects_bad_params() {
    let mut w = setup();
    let auth = w.authority.insecure_clone();
    let protocol = w.protocol.pubkey();

    // Fee splits over 100%.
    let mut p = default_params();
    p.content_creator_bps = 9_000;
    p.regional_operator_bps = 2_000;
    err(&mut w.svm, update_config_ix(&auth.pubkey(), &protocol, p), &auth, &[&auth], "InvalidConfig");

    // Approval threshold over 100%.
    let mut p = default_params();
    p.threshold_bps = 10_001;
    err(&mut w.svm, update_config_ix(&auth.pubkey(), &protocol, p), &auth, &[&auth], "InvalidConfig");

    // A non-positive window.
    let mut p = default_params();
    p.voting_period = 0;
    err(&mut w.svm, update_config_ix(&auth.pubkey(), &protocol, p), &auth, &[&auth], "InvalidConfig");

    // Zero base price.
    let mut p = default_params();
    p.module_price = 0;
    err(&mut w.svm, update_config_ix(&auth.pubkey(), &protocol, p), &auth, &[&auth], "InvalidConfig");
}

#[test]
fn update_authority_rotates() {
    let mut w = setup();
    let auth = w.authority.insecure_clone();
    let protocol = w.protocol.pubkey();
    let new_auth = actor(&mut w.svm);

    ok(&mut w.svm, update_authority_ix(&auth.pubkey(), &new_auth.pubkey()), &auth, &[&auth]);
    assert_eq!(config(&w.svm).authority, new_auth.pubkey());

    // The old authority can no longer update config.
    err(
        &mut w.svm,
        update_config_ix(&auth.pubkey(), &protocol, default_params()),
        &auth,
        &[&auth],
        "NotAuthority",
    );

    // The new authority can.
    let mut p = default_params();
    p.module_price = 300;
    ok(&mut w.svm, update_config_ix(&new_auth.pubkey(), &protocol, p), &new_auth, &[&new_auth]);
    assert_eq!(config(&w.svm).module_price, 300);
}

