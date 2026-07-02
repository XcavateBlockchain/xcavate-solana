//! Deliverer deposit lifecycle and sponsorship reclaim boundaries: re-registering
//! tops a slashed deposit back up, unregistering an unknown deliverer fails,
//! reclaim restores the module allocation and rejects bad amounts, booking an
//! absent sponsorship fails, and a successful delivery reduces a strike.

mod common;
use common::*;

#[test]
fn register_tops_up_slashed_deposit() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let authority = w.authority.pubkey();

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 1), &sponsor, &[&sponsor]);
    ok(&mut w.svm, book_ix(&school.pubkey(), 0, 0, 0), &school, &[&school]);
    ok(&mut w.svm, register_deliverer_ix(&lecturer.pubkey()), &lecturer, &[&lecturer]);

    // Three claim/cancel rounds; the third strike slashes 10% of the deposit.
    for _ in 0..3 {
        ok(&mut w.svm, claim_ix(&lecturer.pubkey(), 0, 0), &lecturer, &[&lecturer]);
        ok(&mut w.svm, cancel_claim_ix(&lecturer.pubkey(), &authority, 0, 0), &lecturer, &[&lecturer]);
    }
    let slash = DELIVERER_DEPOSIT / 10; // strike_slash_bps = 1000
    assert_eq!(deliverer_of(&w.svm, &lecturer.pubkey()).deposit, DELIVERER_DEPOSIT - slash);

    // Re-registering tops the deposit back up to the requirement.
    let before = balance(&w.svm, &xcav_mint(), &lecturer.pubkey());
    ok(&mut w.svm, register_deliverer_ix(&lecturer.pubkey()), &lecturer, &[&lecturer]);
    assert_eq!(deliverer_of(&w.svm, &lecturer.pubkey()).deposit, DELIVERER_DEPOSIT);
    assert_eq!(before - balance(&w.svm, &xcav_mint(), &lecturer.pubkey()), slash);
}

#[test]
fn unregister_unregistered_fails() {
    let mut w = setup();
    // Never registered, so there is no Deliverer account to close.
    let stranger = actor(&mut w.svm);
    err(
        &mut w.svm,
        unregister_deliverer_ix(&stranger.pubkey()),
        &stranger,
        &[&stranger],
        "AccountNotInitialized",
    );
}

#[test]
fn reclaim_full_restores_allocation() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 5), &sponsor, &[&sponsor]);
    // After sponsoring, 5 tokens moved from the sponsor pool into the school pool.
    assert_eq!(module_of(&w.svm, 0).sponsor_allocation, 5);
    assert_eq!(module_of(&w.svm, 0).school_allocation, 5);

    let spon_before = balance(&w.svm, &usdc_mint(), &sponsor.pubkey());
    warp(&mut w.svm, 2_000); // past the sponsorship window
    ok(&mut w.svm, reclaim_sponsorship_ix(&sponsor.pubkey(), 0, 0, 5), &sponsor, &[&sponsor]);

    // Full reclaim: the escrowed payment is refunded and the tokens return to
    // the unsponsored pool.
    assert_eq!(balance(&w.svm, &usdc_mint(), &sponsor.pubkey()) - spon_before, 5 * PER_TOKEN);
    assert_eq!(sponsorship_of(&w.svm, 0, 0).amount, 0);
    assert_eq!(module_of(&w.svm, 0).sponsor_allocation, 10);
    assert_eq!(module_of(&w.svm, 0).school_allocation, 0);
}

#[test]
fn reclaim_amount_boundaries_fail() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 5), &sponsor, &[&sponsor]);

    // Zero is rejected before anything else.
    err(
        &mut w.svm,
        reclaim_sponsorship_ix(&sponsor.pubkey(), 0, 0, 0),
        &sponsor,
        &[&sponsor],
        "AmountCannotBeZero",
    );
    // Reclaiming more than is sponsored is rejected (before the window check).
    err(
        &mut w.svm,
        reclaim_sponsorship_ix(&sponsor.pubkey(), 0, 0, 6),
        &sponsor,
        &[&sponsor],
        "NotEnoughTokenAvailable",
    );
}

#[test]
fn book_nonexistent_sponsorship_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let school = with_role(&mut w, Role::ModuleBooker);

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    // No sponsorship 0 exists, so its PDA does not resolve.
    err(
        &mut w.svm,
        book_ix(&school.pubkey(), 0, 0, 0),
        &school,
        &[&school],
        "AccountNotInitialized",
    );
}

#[test]
fn strike_reduced_after_n_deliveries() {
    let mut w = setup();
    // One successful delivery per strike reduction, to keep the flow short.
    let mut params = default_params();
    params.deliveries_per_strike_reduction = 1;
    ok(
        &mut w.svm,
        update_config_ix(&w.authority.pubkey(), &w.protocol.pubkey(), params),
        &w.authority,
        &[&w.authority],
    );

    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let operator = w.operator.pubkey();
    let protocol = w.protocol.pubkey();

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 1), &sponsor, &[&sponsor]);
    ok(&mut w.svm, book_ix(&school.pubkey(), 0, 0, 0), &school, &[&school]);
    ok(&mut w.svm, register_deliverer_ix(&lecturer.pubkey()), &lecturer, &[&lecturer]);

    // Earn a strike by claiming then cancelling.
    ok(&mut w.svm, claim_ix(&lecturer.pubkey(), 0, 0), &lecturer, &[&lecturer]);
    ok(&mut w.svm, cancel_claim_ix(&lecturer.pubkey(), &w.authority.pubkey(), 0, 0), &lecturer, &[&lecturer]);
    assert_eq!(deliverer_of(&w.svm, &lecturer.pubkey()).active_strikes, 1);

    // A clean re-delivery scored above threshold reduces the strike.
    ok(&mut w.svm, claim_ix(&lecturer.pubkey(), 0, 0), &lecturer, &[&lecturer]);
    ok(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 10_000, 1, &creator.pubkey(), &operator, &protocol, &lecturer.pubkey(), &sponsor.pubkey()),
        &agent,
        &[&agent],
    );

    let d = deliverer_of(&w.svm, &lecturer.pubkey());
    assert_eq!(d.successful_deliveries, 1);
    assert_eq!(d.active_strikes, 0);
}
