//! Authorization negatives and claim/score correctness: every ownership gate
//! (creator, school, lecturer, sponsor) rejects the wrong caller, the role gate
//! rejects a signer with no role, the concurrent-claim deposit ceiling holds,
//! and scoring guards (already-scored, no-lecturer, self-claim) fire.

mod common;
use common::*;

/// Assign an additional role to an existing keypair (a user can hold several).
fn grant(w: &mut World, kp: &Keypair, role: Role) {
    let admin = w.admin.insecure_clone();
    ok(
        &mut w.svm,
        roles_assign_ix(&admin.pubkey(), &kp.pubkey(), role),
        &admin,
        &[&admin],
    );
}

// ============================ authorization negatives ============================

#[test]
fn no_role_caller_rejected() {
    let mut w = setup();
    // A funded signer with no ModuleCreator role: the role PDA does not exist.
    let stranger = actor(&mut w.svm);
    err(
        &mut w.svm,
        create_module_ix(&stranger.pubkey(), 1, 0, 10),
        &stranger,
        &[&stranger],
        "AccountNotInitialized",
    );
}

#[test]
fn burn_non_creator_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let other = with_role(&mut w, Role::ModuleCreator);
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );

    // A different ModuleCreator cannot burn a module they do not own.
    err(
        &mut w.svm,
        burn_ix(&other.pubkey(), 0, 1),
        &other,
        &[&other],
        "NoPermission",
    );
}

#[test]
fn remove_module_non_creator_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let other = with_role(&mut w, Role::ModuleCreator);
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );

    err(
        &mut w.svm,
        remove_module_ix(&other.pubkey(), 0),
        &other,
        &[&other],
        "NoPermission",
    );
}

#[test]
fn reclaim_non_sponsor_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let other = with_role(&mut w, Role::ModuleSponsor);
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&sponsor.pubkey(), 0, 0, 5),
        &sponsor,
        &[&sponsor],
    );

    // A different ModuleSponsor cannot reclaim someone else's sponsorship.
    err(
        &mut w.svm,
        reclaim_sponsorship_ix(&other.pubkey(), 0, 0, 1),
        &other,
        &[&other],
        "NoPermission",
    );
}

#[test]
fn cancel_booking_wrong_school_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let other = with_role(&mut w, Role::ModuleBooker);
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&sponsor.pubkey(), 0, 0, 5),
        &sponsor,
        &[&sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 0, d),
            &school,
            &[&school],
        );
    }

    // Another ModuleBooker cannot cancel a booking they did not create.
    err(
        &mut w.svm,
        cancel_booking_ix(&other.pubkey(), 0, 0, 0, None),
        &other,
        &[&other],
        "NoPermission",
    );
}

#[test]
fn cancel_claim_wrong_lecturer_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let other = with_role(&mut w, Role::ModuleDeliverer);
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&sponsor.pubkey(), 0, 0, 5),
        &sponsor,
        &[&sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 0, d),
            &school,
            &[&school],
        );
    }
    ok(
        &mut w.svm,
        register_deliverer_ix(&lecturer.pubkey()),
        &lecturer,
        &[&lecturer],
    );
    ok(
        &mut w.svm,
        register_deliverer_ix(&other.pubkey()),
        &other,
        &[&other],
    );
    ok(
        &mut w.svm,
        claim_ix(&lecturer.pubkey(), 0, 0),
        &lecturer,
        &[&lecturer],
    );

    // A different registered lecturer cannot cancel someone else's claim.
    err(
        &mut w.svm,
        cancel_claim_ix(&other.pubkey(), 0, 0),
        &other,
        &[&other],
        "NoPermission",
    );
}

#[test]
fn finish_booking_wrong_school_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let other = with_role(&mut w, Role::ModuleBooker);
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&sponsor.pubkey(), 0, 0, 5),
        &sponsor,
        &[&sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 0, d),
            &school,
            &[&school],
        );
    }

    err(
        &mut w.svm,
        finish_ix(&other.pubkey(), 0, 0),
        &other,
        &[&other],
        "NoPermission",
    );
}

#[test]
fn submit_score_wrong_lecturer_payout_acct() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let other = with_role(&mut w, Role::ModuleDeliverer);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let operator = w.operator.pubkey();
    let protocol = w.protocol.pubkey();
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&sponsor.pubkey(), 0, 0, 5),
        &sponsor,
        &[&sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 0, d),
            &school,
            &[&school],
        );
    }
    ok(
        &mut w.svm,
        register_deliverer_ix(&lecturer.pubkey()),
        &lecturer,
        &[&lecturer],
    );
    ok(
        &mut w.svm,
        register_deliverer_ix(&other.pubkey()),
        &other,
        &[&other],
    );
    ok(
        &mut w.svm,
        claim_ix(&lecturer.pubkey(), 0, 0),
        &lecturer,
        &[&lecturer],
    );

    ok(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 10_000),
        &agent,
        &[&agent],
    );

    // `other` is a registered deliverer (so its accounts resolve) but is not the
    // booking's lecturer, so paying its account out must revert.
    err(
        &mut w.svm,
        finalize_score_ix(
            &agent.pubkey(),
            0,
            0,
            1,
            &creator.pubkey(),
            &operator,
            &protocol,
            &other.pubkey(),
            &sponsor.pubkey(),
        ),
        &agent,
        &[&agent],
        "WrongPayoutRecipient",
    );
}

// ============================ claim / score correctness ============================

#[test]
fn claim_exhausts_deposit_fails() {
    let mut w = setup();
    // Raise the per-strike slash so the deposit only backs a single claim: at
    // 60% of a 2e9 deposit, one claim needs 1.2e9 and two would need 2.4e9.
    let mut params = default_params();
    params.strike_slash_bps = 6_000;
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
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&sponsor.pubkey(), 0, 0, 2),
        &sponsor,
        &[&sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 0, d),
            &school,
            &[&school],
        );
    }
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 1, d),
            &school,
            &[&school],
        );
    }
    ok(
        &mut w.svm,
        register_deliverer_ix(&lecturer.pubkey()),
        &lecturer,
        &[&lecturer],
    );

    // First claim fits the deposit; the second concurrent claim exceeds it.
    ok(
        &mut w.svm,
        claim_ix(&lecturer.pubkey(), 0, 0),
        &lecturer,
        &[&lecturer],
    );
    err(
        &mut w.svm,
        claim_ix(&lecturer.pubkey(), 0, 1),
        &lecturer,
        &[&lecturer],
        "InsufficientDepositToClaim",
    );
    assert_eq!(deliverer_of(&w.svm, &lecturer.pubkey()).active_claims, 1);
}

#[test]
fn submit_score_double_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let operator = w.operator.pubkey();
    let protocol = w.protocol.pubkey();
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&sponsor.pubkey(), 0, 0, 5),
        &sponsor,
        &[&sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 0, d),
            &school,
            &[&school],
        );
    }
    ok(
        &mut w.svm,
        register_deliverer_ix(&lecturer.pubkey()),
        &lecturer,
        &[&lecturer],
    );
    ok(
        &mut w.svm,
        claim_ix(&lecturer.pubkey(), 0, 0),
        &lecturer,
        &[&lecturer],
    );
    settle_score(
        &mut w.svm,
        &agent,
        0,
        0,
        10_000,
        1,
        &creator.pubkey(),
        &operator,
        &protocol,
        &lecturer.pubkey(),
        &sponsor.pubkey(),
    );

    // A second score on the same booking must be rejected.
    err(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 10_000),
        &agent,
        &[&agent],
        "ScoreAlreadySet",
    );
}

#[test]
fn submit_score_no_lecturer_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&sponsor.pubkey(), 0, 0, 5),
        &sponsor,
        &[&sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 0, d),
            &school,
            &[&school],
        );
    }
    // `lecturer` is registered so its accounts resolve, but never claimed, so
    // the booking has no lecturer set.
    ok(
        &mut w.svm,
        register_deliverer_ix(&lecturer.pubkey()),
        &lecturer,
        &[&lecturer],
    );

    err(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 10_000),
        &agent,
        &[&agent],
        "NoLecturerSet",
    );
}

#[test]
fn claim_own_booking_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    // The school also registers as a deliverer so it could try to claim.
    let school = with_role(&mut w, Role::ModuleBooker);
    grant(&mut w, &school, Role::ModuleDeliverer);
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&sponsor.pubkey(), 0, 0, 5),
        &sponsor,
        &[&sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 0, d),
            &school,
            &[&school],
        );
    }
    ok(
        &mut w.svm,
        register_deliverer_ix(&school.pubkey()),
        &school,
        &[&school],
    );

    // The school that booked cannot also deliver its own booking.
    err(
        &mut w.svm,
        claim_ix(&school.pubkey(), 0, 0),
        &school,
        &[&school],
        "SchoolCannotClaimOwnBooking",
    );
}

#[test]
fn cancel_booking_after_claim_decrements_claims() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&sponsor.pubkey(), 0, 0, 5),
        &sponsor,
        &[&sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 0, d),
            &school,
            &[&school],
        );
    }
    ok(
        &mut w.svm,
        register_deliverer_ix(&lecturer.pubkey()),
        &lecturer,
        &[&lecturer],
    );
    ok(
        &mut w.svm,
        claim_ix(&lecturer.pubkey(), 0, 0),
        &lecturer,
        &[&lecturer],
    );

    assert_eq!(deliverer_of(&w.svm, &lecturer.pubkey()).active_claims, 1);
    let student_after_claim = module_of(&w.svm, 0).student_allocation;

    // Cancelling a claimed booking passes the deliverer account; the claim is
    // rolled back and the student allocation stays put (it moved at claim time).
    ok(
        &mut w.svm,
        cancel_booking_ix(&school.pubkey(), 0, 0, 0, Some(&lecturer.pubkey())),
        &school,
        &[&school],
    );

    assert_eq!(deliverer_of(&w.svm, &lecturer.pubkey()).active_claims, 0);
    assert_eq!(module_of(&w.svm, 0).student_allocation, student_after_claim);
    assert!(closed(&w.svm, &booking_pda(0, 0)));
}

// Settling a scored booking recovers the school's own deposit and
// must not depend on still holding the ModuleBooker role. A revoked role used to
// strand the deposit (and the sponsorship, which never settled).
#[test]
fn finish_booking_works_after_booker_role_revoked() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let operator = w.operator.pubkey();
    let protocol = w.protocol.pubkey();

    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&sponsor.pubkey(), 0, 0, 1),
        &sponsor,
        &[&sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 0, d),
            &school,
            &[&school],
        );
    }
    ok(
        &mut w.svm,
        register_deliverer_ix(&lecturer.pubkey()),
        &lecturer,
        &[&lecturer],
    );
    ok(
        &mut w.svm,
        claim_ix(&lecturer.pubkey(), 0, 0),
        &lecturer,
        &[&lecturer],
    );
    settle_score(
        &mut w.svm,
        &agent,
        0,
        0,
        10_000,
        1,
        &creator.pubkey(),
        &operator,
        &protocol,
        &lecturer.pubkey(),
        &sponsor.pubkey(),
    );

    // The school loses its role before settling; finishing must still succeed so
    // the deposit and sponsorship aren't stranded.
    let admin = w.admin.insecure_clone();
    ok(
        &mut w.svm,
        roles_remove_ix(&admin.pubkey(), &school.pubkey(), Role::ModuleBooker),
        &admin,
        &[&admin],
    );
    ok(
        &mut w.svm,
        finish_ix(&school.pubkey(), 0, 0),
        &school,
        &[&school],
    );
}
