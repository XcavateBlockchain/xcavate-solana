//! Money-math reconciliation for the score settlement: every payout plus the
//! sponsor refund must equal the escrowed price exactly, with nothing stranded
//! or over-paid. Covers partial scores (where `bps_floor` truncates), the
//! threshold boundary, a second payment asset with different decimals (GBP, 2
//! decimals), and fee round-up at creation.
//!
//! Config fees (from `default_params`): content creator 8.3%, regional operator
//! 8.3%, protocol 5%, DBS 3.4%; min impact score 50%. Module price 100 units.

mod common;
use common::*;

struct Cast {
    creator: Keypair,
    sponsor: Keypair,
    school: Keypair,
    lecturer: Keypair,
    agent: Keypair,
}

/// Grant one keypair for each role in the delivery flow.
fn cast(w: &mut World) -> Cast {
    Cast {
        creator: with_role(w, Role::ModuleCreator),
        sponsor: with_role(w, Role::ModuleSponsor),
        school: with_role(w, Role::ModuleBooker),
        lecturer: with_role(w, Role::ModuleDeliverer),
        agent: with_role(w, Role::ModuleAIAgent),
    }
}

#[test]
fn usdc_partial_score_split_exact() {
    let mut w = setup();
    let c = cast(&mut w);
    let operator = w.operator.pubkey();
    let protocol = w.protocol.pubkey();

    ok(
        &mut w.svm,
        create_module_ix(&c.creator.pubkey(), 1, 0, 10),
        &c.creator,
        &[&c.creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&c.sponsor.pubkey(), 0, 0, 5),
        &c.sponsor,
        &[&c.sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&c.school.pubkey(), 0, 0, 0, d),
            &c.school,
            &[&c.school],
        );
    }
    ok(
        &mut w.svm,
        register_deliverer_ix(&c.lecturer.pubkey()),
        &c.lecturer,
        &[&c.lecturer],
    );
    ok(
        &mut w.svm,
        claim_ix(&c.lecturer.pubkey(), 0, 0),
        &c.lecturer,
        &[&c.lecturer],
    );

    let cre_before = balance(&w.svm, &usdc_mint(), &c.creator.pubkey());
    let op_before = balance(&w.svm, &usdc_mint(), &operator);
    let proto_before = balance(&w.svm, &usdc_mint(), &protocol);
    let lec_before = balance(&w.svm, &usdc_mint(), &c.lecturer.pubkey());
    let spon_before = balance(&w.svm, &usdc_mint(), &c.sponsor.pubkey());

    // 75% score, above the 50% threshold. base 100e6, fees 8.3/8.3/5/3.4e6.
    settle_score(
        &mut w.svm,
        &c.agent,
        0,
        0,
        7_500,
        1,
        &c.creator.pubkey(),
        &operator,
        &protocol,
        &c.lecturer.pubkey(),
        &c.sponsor.pubkey(),
    );

    let cc = balance(&w.svm, &usdc_mint(), &c.creator.pubkey()) - cre_before;
    let ro = balance(&w.svm, &usdc_mint(), &operator) - op_before;
    let proto = balance(&w.svm, &usdc_mint(), &protocol) - proto_before;
    let lec = balance(&w.svm, &usdc_mint(), &c.lecturer.pubkey()) - lec_before;
    let refund = balance(&w.svm, &usdc_mint(), &c.sponsor.pubkey()) - spon_before;

    // Each part is its fee floored by the score; lecturer is base*0.75 + dbs*0.75.
    assert_eq!(cc, 6_225_000); // 8.3e6 * 0.75
    assert_eq!(ro, 6_225_000); // 8.3e6 * 0.75
    assert_eq!(proto, 3_750_000); // 5e6 * 0.75
    assert_eq!(lec, 77_550_000); // 75e6 + 2.55e6
    assert_eq!(refund, 31_250_000); // remainder
                                    // Conservation: the whole escrow is accounted for, nothing left behind.
    assert_eq!(cc + ro + proto + lec + refund, PER_TOKEN);
    assert_eq!(spl_amount(&w.svm, &book_escrow_pda(0, 0)), 0);
}

#[test]
fn score_at_threshold_boundary_pays() {
    let mut w = setup();
    let c = cast(&mut w);
    let operator = w.operator.pubkey();
    let protocol = w.protocol.pubkey();

    ok(
        &mut w.svm,
        create_module_ix(&c.creator.pubkey(), 1, 0, 10),
        &c.creator,
        &[&c.creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&c.sponsor.pubkey(), 0, 0, 5),
        &c.sponsor,
        &[&c.sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&c.school.pubkey(), 0, 0, 0, d),
            &c.school,
            &[&c.school],
        );
    }
    ok(
        &mut w.svm,
        register_deliverer_ix(&c.lecturer.pubkey()),
        &c.lecturer,
        &[&c.lecturer],
    );
    ok(
        &mut w.svm,
        claim_ix(&c.lecturer.pubkey(), 0, 0),
        &c.lecturer,
        &[&c.lecturer],
    );

    let cre_before = balance(&w.svm, &usdc_mint(), &c.creator.pubkey());
    let lec_before = balance(&w.svm, &usdc_mint(), &c.lecturer.pubkey());
    let spon_before = balance(&w.svm, &usdc_mint(), &c.sponsor.pubkey());

    // Exactly at the 50% threshold: the branch is `>=`, so it pays out.
    settle_score(
        &mut w.svm,
        &c.agent,
        0,
        0,
        5_000,
        1,
        &c.creator.pubkey(),
        &operator,
        &protocol,
        &c.lecturer.pubkey(),
        &c.sponsor.pubkey(),
    );

    let cc = balance(&w.svm, &usdc_mint(), &c.creator.pubkey()) - cre_before;
    let lec = balance(&w.svm, &usdc_mint(), &c.lecturer.pubkey()) - lec_before;
    let refund = balance(&w.svm, &usdc_mint(), &c.sponsor.pubkey()) - spon_before;
    // Non-zero creator pay confirms the boundary pays rather than refunding.
    assert_eq!(cc, 4_150_000); // 8.3e6 * 0.50
    assert_eq!(lec, 51_700_000); // 50e6 + 1.7e6 dbs
    assert_eq!(refund, 62_500_000); // half the escrow refunded
    assert_eq!(booking_of(&w.svm, 0, 0).score, Some(5_000));
    assert_eq!(spl_amount(&w.svm, &book_escrow_pda(0, 0)), 0);
}

#[test]
fn score_zero_refunds_full_no_strike() {
    let mut w = setup();
    let c = cast(&mut w);
    let operator = w.operator.pubkey();
    let protocol = w.protocol.pubkey();

    ok(
        &mut w.svm,
        create_module_ix(&c.creator.pubkey(), 1, 0, 10),
        &c.creator,
        &[&c.creator],
    );
    ok(
        &mut w.svm,
        sponsor_ix(&c.sponsor.pubkey(), 0, 0, 5),
        &c.sponsor,
        &[&c.sponsor],
    );
    {
        let d = now_ts(&w.svm);
        ok(
            &mut w.svm,
            book_ix_at(&c.school.pubkey(), 0, 0, 0, d),
            &c.school,
            &[&c.school],
        );
    }
    ok(
        &mut w.svm,
        register_deliverer_ix(&c.lecturer.pubkey()),
        &c.lecturer,
        &[&c.lecturer],
    );
    ok(
        &mut w.svm,
        claim_ix(&c.lecturer.pubkey(), 0, 0),
        &c.lecturer,
        &[&c.lecturer],
    );

    let cre_before = balance(&w.svm, &usdc_mint(), &c.creator.pubkey());
    let spon_before = balance(&w.svm, &usdc_mint(), &c.sponsor.pubkey());

    // Score 0 is below threshold: nobody is paid, the escrow is refunded in
    // full, the token is burned, and no strike is recorded.
    settle_score(
        &mut w.svm,
        &c.agent,
        0,
        0,
        0,
        1,
        &c.creator.pubkey(),
        &operator,
        &protocol,
        &c.lecturer.pubkey(),
        &c.sponsor.pubkey(),
    );

    assert_eq!(
        balance(&w.svm, &usdc_mint(), &c.creator.pubkey()),
        cre_before
    );
    assert_eq!(
        balance(&w.svm, &usdc_mint(), &c.sponsor.pubkey()) - spon_before,
        PER_TOKEN
    );
    assert_eq!(spl_amount(&w.svm, &book_escrow_pda(0, 0)), 0);
    assert_eq!(spl_amount(&w.svm, &module_vault_pda(0)), 9); // one token burned
    let d = deliverer_of(&w.svm, &c.lecturer.pubkey());
    assert_eq!(d.active_strikes, 0);
    assert_eq!(d.active_claims, 0); // the claim was released
}

#[test]
fn gbp_full_score_reconciles_escrow() {
    let mut w = setup();
    let c = cast(&mut w);
    let operator = w.operator.pubkey();
    let protocol = w.protocol.pubkey();

    // GBP has 2 decimals: base 10_000, fees 830/830/500/340, escrow 12_500.
    ok(
        &mut w.svm,
        create_module_ix(&c.creator.pubkey(), 1, 0, 10),
        &c.creator,
        &[&c.creator],
    );
    ok(
        &mut w.svm,
        sponsor_asset_ix(&c.sponsor.pubkey(), 0, 0, 5, &gbp_mint()),
        &c.sponsor,
        &[&c.sponsor],
    );
    let d = now_ts(&w.svm);
    ok(
        &mut w.svm,
        book_asset_ix(&c.school.pubkey(), 0, 0, 0, d, &gbp_mint()),
        &c.school,
        &[&c.school],
    );
    assert_eq!(spl_amount(&w.svm, &book_escrow_pda(0, 0)), PER_TOKEN_GBP);
    ok(
        &mut w.svm,
        register_deliverer_ix(&c.lecturer.pubkey()),
        &c.lecturer,
        &[&c.lecturer],
    );
    ok(
        &mut w.svm,
        claim_ix(&c.lecturer.pubkey(), 0, 0),
        &c.lecturer,
        &[&c.lecturer],
    );

    let cre_before = balance(&w.svm, &gbp_mint(), &c.creator.pubkey());
    let op_before = balance(&w.svm, &gbp_mint(), &operator);
    let proto_before = balance(&w.svm, &gbp_mint(), &protocol);
    let lec_before = balance(&w.svm, &gbp_mint(), &c.lecturer.pubkey());
    let spon_before = balance(&w.svm, &gbp_mint(), &c.sponsor.pubkey());

    settle_score_asset(
        &mut w.svm,
        &c.agent,
        0,
        0,
        10_000,
        1,
        &c.creator.pubkey(),
        &operator,
        &protocol,
        &c.lecturer.pubkey(),
        &c.sponsor.pubkey(),
        &gbp_mint(),
    );

    let cc = balance(&w.svm, &gbp_mint(), &c.creator.pubkey()) - cre_before;
    let ro = balance(&w.svm, &gbp_mint(), &operator) - op_before;
    let proto = balance(&w.svm, &gbp_mint(), &protocol) - proto_before;
    let lec = balance(&w.svm, &gbp_mint(), &c.lecturer.pubkey()) - lec_before;
    let refund = balance(&w.svm, &gbp_mint(), &c.sponsor.pubkey()) - spon_before;

    assert_eq!(cc, 830);
    assert_eq!(ro, 830);
    assert_eq!(proto, 500);
    assert_eq!(lec, 10_340); // base 10_000 + dbs 340
    assert_eq!(refund, 0); // full score leaves nothing
    assert_eq!(cc + ro + proto + lec + refund, PER_TOKEN_GBP);
    assert_eq!(spl_amount(&w.svm, &book_escrow_pda(0, 0)), 0);
}

#[test]
fn gbp_partial_score_floor_precision() {
    let mut w = setup();
    let c = cast(&mut w);
    let operator = w.operator.pubkey();
    let protocol = w.protocol.pubkey();

    ok(
        &mut w.svm,
        create_module_ix(&c.creator.pubkey(), 1, 0, 10),
        &c.creator,
        &[&c.creator],
    );
    ok(
        &mut w.svm,
        sponsor_asset_ix(&c.sponsor.pubkey(), 0, 0, 5, &gbp_mint()),
        &c.sponsor,
        &[&c.sponsor],
    );
    let d = now_ts(&w.svm);
    ok(
        &mut w.svm,
        book_asset_ix(&c.school.pubkey(), 0, 0, 0, d, &gbp_mint()),
        &c.school,
        &[&c.school],
    );
    ok(
        &mut w.svm,
        register_deliverer_ix(&c.lecturer.pubkey()),
        &c.lecturer,
        &[&c.lecturer],
    );
    ok(
        &mut w.svm,
        claim_ix(&c.lecturer.pubkey(), 0, 0),
        &c.lecturer,
        &[&c.lecturer],
    );

    let cre_before = balance(&w.svm, &gbp_mint(), &c.creator.pubkey());
    let op_before = balance(&w.svm, &gbp_mint(), &operator);
    let proto_before = balance(&w.svm, &gbp_mint(), &protocol);
    let lec_before = balance(&w.svm, &gbp_mint(), &c.lecturer.pubkey());
    let spon_before = balance(&w.svm, &gbp_mint(), &c.sponsor.pubkey());

    // 77.77% score at only 2 decimals, so each floor truncates a fraction.
    settle_score_asset(
        &mut w.svm,
        &c.agent,
        0,
        0,
        7_777,
        1,
        &c.creator.pubkey(),
        &operator,
        &protocol,
        &c.lecturer.pubkey(),
        &c.sponsor.pubkey(),
        &gbp_mint(),
    );

    let cc = balance(&w.svm, &gbp_mint(), &c.creator.pubkey()) - cre_before;
    let ro = balance(&w.svm, &gbp_mint(), &operator) - op_before;
    let proto = balance(&w.svm, &gbp_mint(), &protocol) - proto_before;
    let lec = balance(&w.svm, &gbp_mint(), &c.lecturer.pubkey()) - lec_before;
    let refund = balance(&w.svm, &gbp_mint(), &c.sponsor.pubkey()) - spon_before;

    // Each part is floor(fee * 0.7777); lecturer is floor(10_000*0.7777)+floor(340*0.7777).
    assert_eq!(cc, 645); // floor(830 * 0.7777)
    assert_eq!(ro, 645); // floor(830 * 0.7777)
    assert_eq!(proto, 388); // floor(500 * 0.7777)
    assert_eq!(lec, 8_041); // 7777 + 264
    assert_eq!(refund, 2_781); // escrow - paid, the truncated remainder
                               // Conservation still holds exactly despite the truncation on every part.
    assert_eq!(cc + ro + proto + lec + refund, PER_TOKEN_GBP);
    assert_eq!(spl_amount(&w.svm, &book_escrow_pda(0, 0)), 0);
}

#[test]
fn fee_ceil_rounds_up_at_creation() {
    let mut w = setup();

    // A price where the fees do not divide evenly: at 2 decimals a price of 7
    // gives base 700, so the 8.3% creator and operator fees are ceil(58.1) = 59
    // and a 3.33% DBS fee is ceil(23.31) = 24, each one unit above its floor.
    let mut params = default_params();
    params.module_price = 7;
    params.dbs_bps = 333;
    ok(
        &mut w.svm,
        update_config_ix(&w.authority.pubkey(), &w.protocol.pubkey(), params),
        &w.authority,
        &[&w.authority],
    );

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
        sponsor_asset_ix(&sponsor.pubkey(), 0, 0, 3, &gbp_mint()),
        &sponsor,
        &[&sponsor],
    );
    let d = now_ts(&w.svm);
    ok(
        &mut w.svm,
        book_asset_ix(&school.pubkey(), 0, 0, 0, d, &gbp_mint()),
        &school,
        &[&school],
    );

    // base 700 + cc 59 + ro 59 + proto 35 + dbs 24 (each rounded up) = 877.
    let escrow = spl_amount(&w.svm, &book_escrow_pda(0, 0));
    assert_eq!(escrow, 877);
    // Had the fractional fees floored (58/58/23) the escrow would be 874.
    assert_eq!(escrow, 700 + 59 + 59 + 35 + 24);
    assert!(escrow > 700 + 58 + 58 + 35 + 23);
}
