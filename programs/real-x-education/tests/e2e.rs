//! Cross-program end-to-end flow across all three programs in one SVM: roles
//! grants, a region created through the real regions governance flow (propose,
//! vote, finalize, auction, create) rather than seeded, then the full education
//! lifecycle in realXeducation. This is the integration coverage the per-program suites
//! skip: it proves the cross-program reads resolve for real, in particular that
//! the regional-operator payout lands on the owner the regions program recorded.

mod common;
use common::*;

#[test]
fn full_stack_region_to_score_payout() {
    let mut w = setup_governed();
    let operator = w.operator.insecure_clone();

    // Create region 1 through governance: the Region account is written by the
    // regions program and owned by the auction winner, not fabricated.
    govern_region(&mut w, &operator, 1);

    let region_acc = w.svm.get_account(&region_pda(1)).unwrap();
    assert_eq!(region_acc.owner, regions_id());
    let region = Region::try_deserialize(&mut &region_acc.data[..]).unwrap();
    assert_eq!(region.region_id, 1);
    assert_eq!(region.owner, operator.pubkey());

    // Education lifecycle in realXeducation, gated on that region.
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let protocol = w.protocol.pubkey();

    // create_module reads the regions Region PDA, so it only succeeds for a
    // region that actually exists across the program boundary.
    ok(
        &mut w.svm,
        create_module_ix(&creator.pubkey(), 1, 0, 10),
        &creator,
        &[&creator],
    );
    assert_eq!(module_of(&w.svm, 0).region, 1);

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

    // The score pays the regional operator by reading `region.owner` from the
    // regions program; the payout must land in the governance-elected owner's
    // account, at the configured 8.3% of the base for a full score.
    let op_before = balance(&w.svm, &usdc_mint(), &operator.pubkey());
    ok(
        &mut w.svm,
        submit_score_ix(
            &agent.pubkey(),
            0,
            0,
            10_000,
            1,
            &creator.pubkey(),
            &operator.pubkey(),
            &protocol,
            &lecturer.pubkey(),
            &sponsor.pubkey(),
        ),
        &agent,
        &[&agent],
    );
    assert_eq!(
        balance(&w.svm, &usdc_mint(), &operator.pubkey()) - op_before,
        8_300_000
    );
    assert_eq!(booking_of(&w.svm, 0, 0).score, Some(10_000));
}

// A module is created while one operator holds a region, then the seat changes
// hands through the real resignation + replacement auction. Scoring reads the
// live `region.owner`, so the payout must follow the seat: the stale owner
// cannot be paid, the new owner must be.
#[test]
fn region_ownership_change_reroutes_payout() {
    let mut w = setup_governed();
    let old_owner = w.operator.insecure_clone();
    govern_region(&mut w, &old_owner, 1);

    // Full lifecycle up to a claimed booking, all under the original operator.
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
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

    // Hand the region to a new operator: resign, let the notice period elapse,
    // then the new operator claims the open seat by bonding.
    let new_owner = with_role(&mut w, Role::RegionalOperator);
    ok(
        &mut w.svm,
        resign_ix(&old_owner.pubkey(), 1),
        &old_owner,
        &[&old_owner],
    );
    warp(&mut w.svm, 6_000); // past the notice period
    ok(
        &mut w.svm,
        region_claim_open_ix(&new_owner.pubkey(), 1, &old_owner.pubkey()),
        &new_owner,
        &[&new_owner],
    );

    let region_acc = w.svm.get_account(&region_pda(1)).unwrap();
    let region = Region::try_deserialize(&mut &region_acc.data[..]).unwrap();
    assert_eq!(region.owner, new_owner.pubkey());

    // Paying the stale owner is rejected: the payee is pinned to `region.owner`.
    let stale = submit_score_ix(
        &agent.pubkey(),
        0,
        0,
        10_000,
        1,
        &creator.pubkey(),
        &old_owner.pubkey(),
        &protocol,
        &lecturer.pubkey(),
        &sponsor.pubkey(),
    );
    assert!(
        process(&mut w.svm, stale, &agent, &[&agent]).is_err(),
        "stale region owner must not be paid"
    );

    // The new owner receives the regional operator's cut.
    let new_before = balance(&w.svm, &usdc_mint(), &new_owner.pubkey());
    ok(
        &mut w.svm,
        submit_score_ix(
            &agent.pubkey(),
            0,
            0,
            10_000,
            1,
            &creator.pubkey(),
            &new_owner.pubkey(),
            &protocol,
            &lecturer.pubkey(),
            &sponsor.pubkey(),
        ),
        &agent,
        &[&agent],
    );
    assert_eq!(
        balance(&w.svm, &usdc_mint(), &new_owner.pubkey()) - new_before,
        8_300_000
    );
}

// The two heaviest instructions are `create_module` (a mint plus
// fractionalization) and `submit_score` (a five way token split reading a
// cross-program account). Measure their compute so a regression that creeps
// toward the per-instruction budget is caught before it hits the wall.
#[test]
fn hot_instructions_stay_within_compute_budget() {
    let mut w = setup_governed();
    let operator = w.operator.insecure_clone();
    govern_region(&mut w, &operator, 1);

    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);
    let lecturer = with_role(&mut w, Role::ModuleDeliverer);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let protocol = w.protocol.pubkey();

    let create_cu = send_cu(
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
    let score_cu = send_cu(
        &mut w.svm,
        submit_score_ix(
            &agent.pubkey(),
            0,
            0,
            10_000,
            1,
            &creator.pubkey(),
            &operator.pubkey(),
            &protocol,
            &lecturer.pubkey(),
            &sponsor.pubkey(),
        ),
        &agent,
        &[&agent],
    );

    // Observed around 67k and 79k; the ceilings leave headroom for minor churn
    // while still flagging a real blow-up well before the 200k per-ix budget.
    println!("compute units: create_module={create_cu}, submit_score={score_cu}");
    assert!(
        create_cu < 100_000,
        "create_module compute regressed: {create_cu}"
    );
    assert!(
        score_cu < 110_000,
        "submit_score compute regressed: {score_cu}"
    );
}
