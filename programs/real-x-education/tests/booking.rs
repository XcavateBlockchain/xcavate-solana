mod common;
use common::*;

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

    // Booking escrow funded; school deposit locked.
    assert_eq!(spl_amount(&w.svm, &book_escrow_pda(0, 0)), PER_TOKEN);
    let m = module_of(&w.svm, 0);
    assert_eq!(m.school_allocation, 4);
    assert_eq!(m.student_allocation, 1);

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

    let cre_before = balance(&w.svm, &usdc_mint(), &creator.pubkey());
    let op_before = balance(&w.svm, &usdc_mint(), &operator);
    let proto_before = balance(&w.svm, &usdc_mint(), &protocol);
    let lec_before = balance(&w.svm, &usdc_mint(), &lecturer.pubkey());

    ok(
        &mut w.svm,
        submit_score_ix(
            &agent.pubkey(),
            0,
            0,
            10_000,
            1,
            &creator.pubkey(),
            &operator,
            &protocol,
            &lecturer.pubkey(),
            &sponsor.pubkey(),
        ),
        &agent,
        &[&agent],
    );

    // Full score: creator 8.3%, operator 8.3%, protocol 5%, lecturer base+dbs.
    assert_eq!(
        balance(&w.svm, &usdc_mint(), &creator.pubkey()) - cre_before,
        8_300_000
    );
    assert_eq!(
        balance(&w.svm, &usdc_mint(), &operator) - op_before,
        8_300_000
    );
    assert_eq!(
        balance(&w.svm, &usdc_mint(), &protocol) - proto_before,
        5_000_000
    );
    assert_eq!(
        balance(&w.svm, &usdc_mint(), &lecturer.pubkey()) - lec_before,
        103_400_000
    );
    // Escrow drained, delivered token burned.
    assert_eq!(spl_amount(&w.svm, &book_escrow_pda(0, 0)), 0);
    assert_eq!(spl_amount(&w.svm, &module_vault_pda(0)), 9);

    let b = Booking::try_deserialize(&mut &w.svm.get_account(&booking_pda(0, 0)).unwrap().data[..])
        .unwrap();
    assert_eq!(b.score, Some(10_000));

    // School reclaims its deposit and the booking closes.
    let school_before = balance(&w.svm, &xcav_mint(), &school.pubkey());
    ok(
        &mut w.svm,
        finish_ix(&school.pubkey(), 0, 0),
        &school,
        &[&school],
    );
    assert_eq!(
        balance(&w.svm, &xcav_mint(), &school.pubkey()) - school_before,
        BOOKING_DEPOSIT
    );
    assert!(w
        .svm
        .get_account(&booking_pda(0, 0))
        .map(|a| a.data.is_empty())
        .unwrap_or(true));
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
    ok(
        &mut w.svm,
        submit_score_ix(
            &agent.pubkey(),
            0,
            0,
            10_000,
            1,
            &creator.pubkey(),
            &operator,
            &protocol,
            &lecturer.pubkey(),
            &sponsor.pubkey(),
        ),
        &agent,
        &[&agent],
    );

    // The booking is settled: cancelling the (already scored) claim must be
    // rejected so the released claim can't be double-counted.
    let claims_before = deliverer_of(&w.svm, &lecturer.pubkey()).active_claims;
    err(
        &mut w.svm,
        cancel_claim_ix(&lecturer.pubkey(), 0, 0),
        &lecturer,
        &[&lecturer],
        "ScoreAlreadySet",
    );
    assert_eq!(
        deliverer_of(&w.svm, &lecturer.pubkey()).active_claims,
        claims_before
    );
}

#[test]
fn book_module_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
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

    // Book the one sponsored token; the module then has nothing left to book.
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
        err(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 1, d),
            &school,
            &[&school],
            "NotEnoughTokenAvailable",
        );
    }
}

#[test]
fn claim_booking_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lec1 = with_role(&mut w, Role::ModuleDeliverer);
    let lec2 = with_role(&mut w, Role::ModuleDeliverer);
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

    // Has the role but never registered a deposit, so the Deliverer account the
    // claim needs doesn't exist.
    err(
        &mut w.svm,
        claim_ix(&lec1.pubkey(), 0, 0),
        &lec1,
        &[&lec1],
        "AccountNotInitialized",
    );

    // Once registered, the first lecturer claims and a second can't take it.
    ok(
        &mut w.svm,
        register_deliverer_ix(&lec1.pubkey()),
        &lec1,
        &[&lec1],
    );
    ok(
        &mut w.svm,
        register_deliverer_ix(&lec2.pubkey()),
        &lec2,
        &[&lec2],
    );
    ok(&mut w.svm, claim_ix(&lec1.pubkey(), 0, 0), &lec1, &[&lec1]);
    err(
        &mut w.svm,
        claim_ix(&lec2.pubkey(), 0, 0),
        &lec2,
        &[&lec2],
        "LecturerAlreadySet",
    );
}

#[test]
fn score_below_threshold_refunds_sponsor() {
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

    let cre_before = balance(&w.svm, &usdc_mint(), &creator.pubkey());
    let sponsor_before = balance(&w.svm, &usdc_mint(), &sponsor.pubkey());
    // A score below the 50% threshold: the token is burned but nobody is paid,
    // and the escrowed payment is refunded to the sponsor.
    ok(
        &mut w.svm,
        submit_score_ix(
            &agent.pubkey(),
            0,
            0,
            4_000,
            1,
            &creator.pubkey(),
            &operator,
            &protocol,
            &lecturer.pubkey(),
            &sponsor.pubkey(),
        ),
        &agent,
        &[&agent],
    );

    assert_eq!(balance(&w.svm, &usdc_mint(), &creator.pubkey()), cre_before);
    assert_eq!(
        balance(&w.svm, &usdc_mint(), &sponsor.pubkey()) - sponsor_before,
        PER_TOKEN
    );
    assert_eq!(spl_amount(&w.svm, &book_escrow_pda(0, 0)), 0);
    assert_eq!(spl_amount(&w.svm, &module_vault_pda(0)), 9);
    assert_eq!(booking_of(&w.svm, 0, 0).score, Some(4_000));
}

#[test]
fn submit_score_out_of_range_fails() {
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

    err(
        &mut w.svm,
        submit_score_ix(
            &agent.pubkey(),
            0,
            0,
            10_001,
            1,
            &creator.pubkey(),
            &operator,
            &protocol,
            &lecturer.pubkey(),
            &sponsor.pubkey(),
        ),
        &agent,
        &[&agent],
        "InvalidScore",
    );
}

#[test]
fn mint_credential_works() {
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
    ok(
        &mut w.svm,
        submit_score_ix(
            &agent.pubkey(),
            0,
            0,
            8_000,
            1,
            &creator.pubkey(),
            &operator,
            &protocol,
            &lecturer.pubkey(),
            &sponsor.pubkey(),
        ),
        &agent,
        &[&agent],
    );

    // A student credential carries the booking's score.
    let student = Pubkey::new_unique();
    ok(
        &mut w.svm,
        mint_credential_ix(&agent.pubkey(), 0, 0, CredentialKind::Student, &student),
        &agent,
        &[&agent],
    );
    let c = credential_of(&w.svm, 0, CredentialKind::Student, &student);
    assert_eq!(c.recipient, student);
    assert_eq!(c.score, Some(8_000));

    // Other credential kinds don't record a score.
    ok(
        &mut w.svm,
        mint_credential_ix(
            &agent.pubkey(),
            0,
            0,
            CredentialKind::School,
            &school.pubkey(),
        ),
        &agent,
        &[&agent],
    );
    assert_eq!(
        credential_of(&w.svm, 0, CredentialKind::School, &school.pubkey()).score,
        None
    );
}

#[test]
fn credential_and_finish_require_score() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
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

    // No score submitted yet.
    let student = Pubkey::new_unique();
    err(
        &mut w.svm,
        mint_credential_ix(&agent.pubkey(), 0, 0, CredentialKind::Student, &student),
        &agent,
        &[&agent],
        "NoTestResultsSubmitted",
    );
    err(
        &mut w.svm,
        finish_ix(&school.pubkey(), 0, 0),
        &school,
        &[&school],
        "NoTestResultsSubmitted",
    );
}

#[test]
fn cancel_booking_refunds_and_returns_token() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);

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

    let before = balance(&w.svm, &xcav_mint(), &school.pubkey());
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&school.pubkey(), 0, 0, 0, d),
            &school,
            &[&school],
        );
    }
    assert_eq!(sponsorship_of(&w.svm, 0, 0).amount, 1);

    ok(
        &mut w.svm,
        cancel_booking_ix(&school.pubkey(), 0, 0, 0, None),
        &school,
        &[&school],
    );
    // Deposit refunded, token returned to the sponsor, booking and escrow closed.
    assert_eq!(balance(&w.svm, &xcav_mint(), &school.pubkey()), before);
    assert_eq!(sponsorship_of(&w.svm, 0, 0).amount, 2);
    assert_eq!(sponsorship_of(&w.svm, 0, 0).active_bookings, 0);
    assert!(closed(&w.svm, &booking_pda(0, 0)));
    assert!(closed(&w.svm, &book_escrow_pda(0, 0)));
}

#[test]
fn cancel_booking_third_time_slashes() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);

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

    let treasury_before = treasury_balance(&w.svm);
    // Book and cancel the same token three times; the third cancellation hits the
    // ceiling and the deposit is slashed instead of refunded.
    for booking_id in 0..3 {
        {
            let d = now_ts(&w.svm);
            ok(
                &mut w.svm,
                book_ix_at(&school.pubkey(), 0, 0, booking_id, d),
                &school,
                &[&school],
            );
        }
        ok(
            &mut w.svm,
            cancel_booking_ix(&school.pubkey(), 0, 0, booking_id, None),
            &school,
            &[&school],
        );
    }
    assert_eq!(treasury_balance(&w.svm) - treasury_before, BOOKING_DEPOSIT);
}

#[test]
fn clear_old_cancellation_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);

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
        cancel_booking_ix(&school.pubkey(), 0, 0, 0, None),
        &school,
        &[&school],
    );

    // Not old enough yet.
    err(
        &mut w.svm,
        clear_old_cancellation_ix(&school.pubkey(), 0),
        &school,
        &[&school],
        "CancellationNotClearable",
    );
    // Past the window it clears.
    warp(&mut w.svm, 2_000);
    ok(
        &mut w.svm,
        clear_old_cancellation_ix(&school.pubkey(), 0),
        &school,
        &[&school],
    );
    assert!(closed(&w.svm, &cancellation_pda(&school.pubkey(), 0)));
}

#[test]
fn cancel_claim_third_strike_slashes() {
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

    let treasury_before = treasury_balance(&w.svm);
    // Claim then cancel three times; the third strike slashes the deposit.
    for _ in 0..3 {
        ok(
            &mut w.svm,
            claim_ix(&lecturer.pubkey(), 0, 0),
            &lecturer,
            &[&lecturer],
        );
        ok(
            &mut w.svm,
            cancel_claim_ix(&lecturer.pubkey(), 0, 0),
            &lecturer,
            &[&lecturer],
        );
    }
    let slash = DELIVERER_DEPOSIT / 10; // 1000 bps
    assert_eq!(deliverer_of(&w.svm, &lecturer.pubkey()).active_strikes, 3);
    assert_eq!(
        deliverer_of(&w.svm, &lecturer.pubkey()).deposit,
        DELIVERER_DEPOSIT - slash
    );
    assert_eq!(treasury_balance(&w.svm) - treasury_before, slash);
}

#[test]
fn register_deliverer_works() {
    let mut w = setup();
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);

    let before = balance(&w.svm, &xcav_mint(), &lecturer.pubkey());
    ok(
        &mut w.svm,
        register_deliverer_ix(&lecturer.pubkey()),
        &lecturer,
        &[&lecturer],
    );
    let d = deliverer_of(&w.svm, &lecturer.pubkey());
    assert_eq!(d.deposit, DELIVERER_DEPOSIT);
    assert_eq!(d.active_claims, 0);
    assert_eq!(
        before - balance(&w.svm, &xcav_mint(), &lecturer.pubkey()),
        DELIVERER_DEPOSIT
    );
}

#[test]
fn unregister_deliverer_works() {
    let mut w = setup();
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);

    let before = balance(&w.svm, &xcav_mint(), &lecturer.pubkey());
    ok(
        &mut w.svm,
        register_deliverer_ix(&lecturer.pubkey()),
        &lecturer,
        &[&lecturer],
    );
    ok(
        &mut w.svm,
        unregister_deliverer_ix(&lecturer.pubkey(), &w.authority.pubkey()),
        &lecturer,
        &[&lecturer],
    );
    assert_eq!(balance(&w.svm, &xcav_mint(), &lecturer.pubkey()), before);
    assert!(closed(&w.svm, &deliverer_pda(&lecturer.pubkey())));
}

#[test]
fn unregister_deliverer_with_active_claim_fails() {
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

    err(
        &mut w.svm,
        unregister_deliverer_ix(&lecturer.pubkey(), &w.authority.pubkey()),
        &lecturer,
        &[&lecturer],
        "ModuleDelivererStillActive",
    );
}

// A score cannot be settled before the booking's scheduled delivery
// time. The agent must wait until `delivery_at`.
#[test]
fn score_before_delivery_time_fails() {
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
    let future = now_ts(&w.svm) + 10_000;
    ok(
        &mut w.svm,
        book_ix_at(&school.pubkey(), 0, 0, 0, future),
        &school,
        &[&school],
    );
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

    // Too early: the session hasn't happened yet.
    err(
        &mut w.svm,
        submit_score_ix(
            &agent.pubkey(),
            0,
            0,
            10_000,
            1,
            &creator.pubkey(),
            &operator,
            &protocol,
            &lecturer.pubkey(),
            &sponsor.pubkey(),
        ),
        &agent,
        &[&agent],
        "DeliveryNotReached",
    );

    // Once the delivery time passes, scoring settles.
    warp(&mut w.svm, 10_001);
    ok(
        &mut w.svm,
        submit_score_ix(
            &agent.pubkey(),
            0,
            0,
            10_000,
            1,
            &creator.pubkey(),
            &operator,
            &protocol,
            &lecturer.pubkey(),
            &sponsor.pubkey(),
        ),
        &agent,
        &[&agent],
    );
    assert_eq!(booking_of(&w.svm, 0, 0).score, Some(10_000));
}

// A lecturer who claims and never delivers is struck permissionlessly
// once the no-show grace window past the delivery time elapses, without charging
// the school a cancellation.
#[test]
fn expire_claim_strikes_noshow_lecturer() {
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
        sponsor_ix(&sponsor.pubkey(), 0, 0, 1),
        &sponsor,
        &[&sponsor],
    );
    let d = now_ts(&w.svm);
    ok(
        &mut w.svm,
        book_ix_at(&school.pubkey(), 0, 0, 0, d),
        &school,
        &[&school],
    );
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

    // A third party cranks the expiry; before the grace window it's rejected.
    let cranker = funded(&mut w.svm);
    err(
        &mut w.svm,
        expire_claim_ix(&cranker.pubkey(), &lecturer.pubkey(), 0, 0),
        &cranker,
        &[&cranker],
        "NoShowWindowNotExpired",
    );

    // Past delivery + no_show_grace, anyone can strike the no-show lecturer.
    warp(&mut w.svm, 2_000);
    ok(
        &mut w.svm,
        expire_claim_ix(&cranker.pubkey(), &lecturer.pubkey(), 0, 0),
        &cranker,
        &[&cranker],
    );

    let del = deliverer_of(&w.svm, &lecturer.pubkey());
    assert_eq!(del.active_strikes, 1);
    assert_eq!(del.active_claims, 0);
    assert!(booking_of(&w.svm, 0, 0).lecturer.is_none());
    // The token is free to claim again.
    assert_eq!(module_of(&w.svm, 0).student_allocation, 1);
    // The school was never charged a cancellation.
    assert!(w.svm.get_account(&counter_pda(&school.pubkey())).is_none());
}

// A sponsor can reclaim unbooked escrow even after losing the
// ModuleSponsor role; recovering your own funds must not depend on the role.
#[test]
fn reclaim_sponsorship_works_after_role_revoked() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);

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

    // Revoke the role, then reclaim once the sponsorship window has passed.
    let admin = w.admin.insecure_clone();
    ok(
        &mut w.svm,
        roles_remove_ix(&admin.pubkey(), &sponsor.pubkey(), Role::ModuleSponsor),
        &admin,
        &[&admin],
    );
    warp(&mut w.svm, 2_000);
    ok(
        &mut w.svm,
        reclaim_sponsorship_ix(&sponsor.pubkey(), 0, 0, 2),
        &sponsor,
        &[&sponsor],
    );

    assert_eq!(sponsorship_of(&w.svm, 0, 0).amount, 0);
}
