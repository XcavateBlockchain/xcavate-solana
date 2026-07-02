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

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 1), &sponsor, &[&sponsor]);
    ok(&mut w.svm, book_ix(&school.pubkey(), 0, 0, 0), &school, &[&school]);
    ok(&mut w.svm, register_deliverer_ix(&lecturer.pubkey()), &lecturer, &[&lecturer]);

    // Three claim/cancel rounds; the third strike slashes 10% of the deposit.
    for _ in 0..3 {
        ok(&mut w.svm, claim_ix(&lecturer.pubkey(), 0, 0), &lecturer, &[&lecturer]);
        ok(&mut w.svm, cancel_claim_ix(&lecturer.pubkey(), 0, 0), &lecturer, &[&lecturer]);
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
        unregister_deliverer_ix(&stranger.pubkey(), &w.authority.pubkey()),
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
fn unregister_strips_role_and_blocks_reregister() {
    let mut w = setup();
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    ok(&mut w.svm, register_deliverer_ix(&lecturer.pubkey()), &lecturer, &[&lecturer]);
    ok(&mut w.svm, unregister_deliverer_ix(&lecturer.pubkey(), &w.authority.pubkey()), &lecturer, &[&lecturer]);

    // Unregistering renounces the role, so strikes cannot be shed by cycling
    // out and straight back in; a new grant is needed first.
    assert!(closed(&w.svm, &role_pda(&lecturer.pubkey(), Role::ModuleDeliverer)));
    err(
        &mut w.svm,
        register_deliverer_ix(&lecturer.pubkey()),
        &lecturer,
        &[&lecturer],
        "AccountNotInitialized",
    );

    let admin = w.admin.insecure_clone();
    ok(&mut w.svm, roles_assign_ix(&admin.pubkey(), &lecturer.pubkey(), Role::ModuleDeliverer), &admin, &[&admin]);
    ok(&mut w.svm, register_deliverer_ix(&lecturer.pubkey()), &lecturer, &[&lecturer]);
    assert_eq!(deliverer_of(&w.svm, &lecturer.pubkey()).deposit, DELIVERER_DEPOSIT);
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
    ok(&mut w.svm, cancel_claim_ix(&lecturer.pubkey(), 0, 0), &lecturer, &[&lecturer]);
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

#[test]
fn unregister_recovers_deposit_after_role_gone() {
    let mut w = setup();
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    ok(&mut w.svm, register_deliverer_ix(&lecturer.pubkey()), &lecturer, &[&lecturer]);

    // An admin removes the role while the deposit is still locked. Unregister
    // passes the now-closed role PDA, reads it as gone, and returns the deposit.
    let admin = w.admin.insecure_clone();
    ok(&mut w.svm, roles_remove_ix(&admin.pubkey(), &lecturer.pubkey(), Role::ModuleDeliverer), &admin, &[&admin]);
    assert!(closed(&w.svm, &role_pda(&lecturer.pubkey(), Role::ModuleDeliverer)));

    let before = balance(&w.svm, &xcav_mint(), &lecturer.pubkey());
    ok(
        &mut w.svm,
        unregister_deliverer_ix(&lecturer.pubkey(), &w.authority.pubkey()),
        &lecturer,
        &[&lecturer],
    );
    assert_eq!(balance(&w.svm, &xcav_mint(), &lecturer.pubkey()) - before, DELIVERER_DEPOSIT);
    assert!(closed(&w.svm, &deliverer_pda(&lecturer.pubkey())));
}

// A deliverer carrying a strike still gives up the role on unregister, so
// re-registering (which would reset the strike) needs a fresh admin grant.
#[test]
fn unregister_with_strike_renounces_role() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 1), &sponsor, &[&sponsor]);
    ok(&mut w.svm, book_ix(&school.pubkey(), 0, 0, 0), &school, &[&school]);
    ok(&mut w.svm, register_deliverer_ix(&lecturer.pubkey()), &lecturer, &[&lecturer]);

    // Earn a strike, then drop back to zero active claims so unregister opens.
    ok(&mut w.svm, claim_ix(&lecturer.pubkey(), 0, 0), &lecturer, &[&lecturer]);
    ok(&mut w.svm, cancel_claim_ix(&lecturer.pubkey(), 0, 0), &lecturer, &[&lecturer]);
    assert_eq!(deliverer_of(&w.svm, &lecturer.pubkey()).active_strikes, 1);

    ok(&mut w.svm, unregister_deliverer_ix(&lecturer.pubkey(), &w.authority.pubkey()), &lecturer, &[&lecturer]);
    assert!(closed(&w.svm, &role_pda(&lecturer.pubkey(), Role::ModuleDeliverer)));

    // With the role gone, re-registering is blocked until an admin grants it again.
    err(
        &mut w.svm,
        register_deliverer_ix(&lecturer.pubkey()),
        &lecturer,
        &[&lecturer],
        "AccountNotInitialized",
    );
}
