mod common;
use anchor_lang::solana_program::instruction::Instruction;
use common::*;

// A full-score delivery pays the lecturer the base plus the dbs share.
const LECTURER_FULL: u64 = 103_400_000;

/// Book and claim one token, with the dispute window set to `window` seconds.
/// The agent has not proposed a score yet. Returns the world and the actors.
fn claimed_with_window(window: i64) -> (World, Keypair, Keypair, Keypair, Keypair, Keypair) {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let agent = with_role(&mut w, Role::ModuleAIAgent);

    ok(
        &mut w.svm,
        update_config_ix(
            &w.authority.pubkey(),
            &w.protocol.pubkey(),
            ConfigParams {
                dispute_window: window,
                ..default_params()
            },
        ),
        &w.authority,
        &[&w.authority],
    );

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

    (w, creator, sponsor, school, lecturer, agent)
}

/// Finalize signed by an arbitrary cranker (the permissionless path).
fn finalize(
    w: &mut World,
    cranker: &Keypair,
    creator: &Keypair,
    school_operator: &Pubkey,
    lecturer: &Keypair,
    sponsor: &Keypair,
) -> Instruction {
    finalize_score_ix(
        &cranker.pubkey(),
        0,
        0,
        1,
        &creator.pubkey(),
        school_operator,
        &w.protocol.pubkey(),
        &lecturer.pubkey(),
        &sponsor.pubkey(),
    )
}

#[test]
fn cranker_cannot_finalize_before_window() {
    let (mut w, creator, sponsor, _school, lecturer, agent) = claimed_with_window(1_000);
    let operator = w.operator.pubkey();

    ok(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 8_000),
        &agent,
        &[&agent],
    );

    // A stranger can't force settlement while the window is still open.
    let cranker = funded(&mut w.svm);
    let ix = finalize(&mut w, &cranker, &creator, &operator, &lecturer, &sponsor);
    err(&mut w.svm, ix, &cranker, &[&cranker], "DisputeWindowOpen");

    // Once the window lapses anyone can finalize at the agent's score.
    warp(&mut w.svm, 1_001);
    let ix = finalize(&mut w, &cranker, &creator, &operator, &lecturer, &sponsor);
    ok(&mut w.svm, ix, &cranker, &[&cranker]);

    let b = booking_of(&w.svm, 0, 0);
    assert!(b.settled);
    assert_eq!(b.score, Some(8_000));
}

#[test]
fn a_party_can_finalize_before_the_window() {
    let (mut w, creator, sponsor, school, lecturer, agent) = claimed_with_window(100_000);
    let operator = w.operator.pubkey();

    ok(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 8_000),
        &agent,
        &[&agent],
    );

    // The school signing finalize is its consent, so it settles right away even
    // though the window has not elapsed.
    let ix = finalize(&mut w, &school, &creator, &operator, &lecturer, &sponsor);
    ok(&mut w.svm, ix, &school, &[&school]);

    let b = booking_of(&w.svm, 0, 0);
    assert!(b.settled);
    assert_eq!(b.score, Some(8_000));
}

#[test]
fn accepted_dispute_raises_the_score() {
    let (mut w, creator, sponsor, school, lecturer, agent) = claimed_with_window(100_000);
    let operator = w.operator.pubkey();
    let lec_before = balance(&w.svm, &usdc_mint(), &lecturer.pubkey());

    // The agent under-scores below the payout threshold, so the lecturer would
    // get nothing.
    ok(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 4_000),
        &agent,
        &[&agent],
    );

    // The lecturer disputes with a full score and the school accepts it.
    ok(
        &mut w.svm,
        dispute_score_ix(&lecturer.pubkey(), 0, 0, 10_000),
        &lecturer,
        &[&lecturer],
    );
    ok(
        &mut w.svm,
        resolve_dispute_ix(&school.pubkey(), 0, 0, true),
        &school,
        &[&school],
    );

    // A concluded dispute finalizes immediately, without waiting on the window.
    let cranker = funded(&mut w.svm);
    let ix = finalize(&mut w, &cranker, &creator, &operator, &lecturer, &sponsor);
    ok(&mut w.svm, ix, &cranker, &[&cranker]);

    let b = booking_of(&w.svm, 0, 0);
    assert_eq!(b.score, Some(10_000));
    assert_eq!(
        balance(&w.svm, &usdc_mint(), &lecturer.pubkey()) - lec_before,
        LECTURER_FULL
    );
}

#[test]
fn rejected_dispute_keeps_the_agent_score() {
    let (mut w, creator, sponsor, school, lecturer, agent) = claimed_with_window(100_000);
    let operator = w.operator.pubkey();
    let lec_before = balance(&w.svm, &usdc_mint(), &lecturer.pubkey());

    ok(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 10_000),
        &agent,
        &[&agent],
    );

    // The school tries to cut the score, but the lecturer rejects it as untrue,
    // so the agent's original score stands.
    ok(
        &mut w.svm,
        dispute_score_ix(&school.pubkey(), 0, 0, 4_000),
        &school,
        &[&school],
    );
    ok(
        &mut w.svm,
        resolve_dispute_ix(&lecturer.pubkey(), 0, 0, false),
        &lecturer,
        &[&lecturer],
    );

    let cranker = funded(&mut w.svm);
    let ix = finalize(&mut w, &cranker, &creator, &operator, &lecturer, &sponsor);
    ok(&mut w.svm, ix, &cranker, &[&cranker]);

    let b = booking_of(&w.svm, 0, 0);
    assert_eq!(b.score, Some(10_000));
    assert_eq!(
        balance(&w.svm, &usdc_mint(), &lecturer.pubkey()) - lec_before,
        LECTURER_FULL
    );
}

#[test]
fn a_pending_dispute_blocks_finalize_until_the_window() {
    let (mut w, creator, sponsor, school, lecturer, agent) = claimed_with_window(1_000);
    let operator = w.operator.pubkey();

    ok(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 8_000),
        &agent,
        &[&agent],
    );
    ok(
        &mut w.svm,
        dispute_score_ix(&school.pubkey(), 0, 0, 10_000),
        &school,
        &[&school],
    );

    // Neither a cranker nor the lecturer can finalize around the open amendment.
    let cranker = funded(&mut w.svm);
    let ix = finalize(&mut w, &cranker, &creator, &operator, &lecturer, &sponsor);
    err(&mut w.svm, ix, &cranker, &[&cranker], "DisputePending");
    let ix = finalize(&mut w, &lecturer, &creator, &operator, &lecturer, &sponsor);
    err(&mut w.svm, ix, &lecturer, &[&lecturer], "DisputePending");

    // Once the window lapses the unresolved amendment falls away and the agent's
    // score is what settles.
    warp(&mut w.svm, 1_001);
    let ix = finalize(&mut w, &cranker, &creator, &operator, &lecturer, &sponsor);
    ok(&mut w.svm, ix, &cranker, &[&cranker]);

    let b = booking_of(&w.svm, 0, 0);
    assert_eq!(b.score, Some(8_000));
    assert!(b.proposed_score.is_none());
}

#[test]
fn dispute_guards() {
    let (mut w, _creator, _sponsor, school, lecturer, agent) = claimed_with_window(1_000);

    ok(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 8_000),
        &agent,
        &[&agent],
    );

    // Someone who is neither the school nor the lecturer can't dispute.
    let outsider = funded(&mut w.svm);
    err(
        &mut w.svm,
        dispute_score_ix(&outsider.pubkey(), 0, 0, 10_000),
        &outsider,
        &[&outsider],
        "NoPermission",
    );

    // The school opens the one allowed dispute.
    ok(
        &mut w.svm,
        dispute_score_ix(&school.pubkey(), 0, 0, 10_000),
        &school,
        &[&school],
    );

    // The disputer can't also resolve it; that's the counterparty's call.
    err(
        &mut w.svm,
        resolve_dispute_ix(&school.pubkey(), 0, 0, true),
        &school,
        &[&school],
        "NoPermission",
    );

    // No second dispute on the same booking.
    err(
        &mut w.svm,
        dispute_score_ix(&lecturer.pubkey(), 0, 0, 9_000),
        &lecturer,
        &[&lecturer],
        "DisputeAlreadyRaised",
    );
}

#[test]
fn cannot_dispute_after_the_window_closes() {
    let (mut w, _creator, _sponsor, school, _lecturer, agent) = claimed_with_window(1_000);

    ok(
        &mut w.svm,
        submit_score_ix(&agent.pubkey(), 0, 0, 8_000),
        &agent,
        &[&agent],
    );

    warp(&mut w.svm, 1_001);
    err(
        &mut w.svm,
        dispute_score_ix(&school.pubkey(), 0, 0, 10_000),
        &school,
        &[&school],
        "DisputeWindowClosed",
    );
}
