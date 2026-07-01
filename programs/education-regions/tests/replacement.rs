//! Operator replacement: resignation opening the seat, the replacement auction
//! (bidding, refunds, previous-bidder authorization), and finalizing the change.

mod common;
use common::*;

#[test]
fn resignation_then_replacement_changes_owner() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);
    let op_after_create = xcav_balance(&svm, &operator.pubkey());

    // Operator gives notice; the seat opens after notice_period (5_000).
    ok(&mut svm, resign_ix(&operator.pubkey(), 1), &operator, &[&operator]);
    warp(&mut svm, 6_000);

    // A new operator bids on the open seat.
    let op2 = new_operator(&mut svm, &authority);
    ok(&mut svm, bid_replacement_ix(&op2.pubkey(), 1, 600_000_000, None), &op2, &[&op2]);

    warp(&mut svm, 2_000); // past the replacement auction
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_replacement_ix(&cranker.pubkey(), 1, &operator.pubkey()),
        &cranker,
        &[&cranker],
    );

    let region = region_of(&svm, 1);
    assert_eq!(region.owner, op2.pubkey());
    assert_eq!(region.collateral, 600_000_000);
    assert_eq!(region.active_strikes, 0);
    // Outgoing operator's collateral was returned.
    assert_eq!(xcav_balance(&svm, &operator.pubkey()), op_after_create + 600_000_000);
    // Replacement auction closed.
    assert!(svm.get_account(&replacement_auction_pda(1)).map_or(true, |a| a.data.is_empty()));
}

#[test]
fn replacement_bid_fails_before_seat_open() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let op2 = new_operator(&mut svm, &authority);
    // No resignation / removal yet, so the owner-change lock is still in the future.
    fails_with(
        &mut svm,
        bid_replacement_ix(&op2.pubkey(), 1, 600_000_000, None),
        &op2,
        &[&op2],
        "RegionOwnerCantBeChanged",
    );
}

#[test]
fn resignation_requires_owner() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let op2 = new_operator(&mut svm, &authority);
    fails_with(
        &mut svm,
        resign_ix(&op2.pubkey(), 1),
        &op2,
        &[&op2],
        "NotRegionOwner",
    );
}

#[test]
fn resign_fails_when_change_already_scheduled() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    ok(&mut svm, resign_ix(&operator.pubkey(), 1), &operator, &[&operator]);
    // A second resignation without time passing cannot bring the change forward.
    fails_with(
        &mut svm,
        resign_ix(&operator.pubkey(), 1),
        &operator,
        &[&operator],
        "OwnerChangeAlreadyScheduled",
    );
}

#[test]
fn replacement_outbid_refunds_previous() {
    let (mut svm, operator, authority) = setup();
    reach_seat_open(&mut svm, &operator, &authority);

    let op2 = new_operator(&mut svm, &authority);
    let op3 = new_operator(&mut svm, &authority);
    ok(&mut svm, bid_replacement_ix(&op2.pubkey(), 1, 600_000_000, None), &op2, &[&op2]);
    let op2_after_bid = xcav_balance(&svm, &op2.pubkey());
    let op3_before = xcav_balance(&svm, &op3.pubkey());

    // op3 outbids, refunding op2 exactly.
    ok(
        &mut svm,
        bid_replacement_ix(&op3.pubkey(), 1, 700_000_000, Some(op2.pubkey())),
        &op3,
        &[&op3],
    );
    assert_eq!(xcav_balance(&svm, &op2.pubkey()), op2_after_bid + 600_000_000);
    assert_eq!(xcav_balance(&svm, &op3.pubkey()), op3_before - 700_000_000);
}

#[test]
fn replacement_same_bidder_can_raise() {
    let (mut svm, operator, authority) = setup();
    reach_seat_open(&mut svm, &operator, &authority);

    let op2 = new_operator(&mut svm, &authority);
    let before = xcav_balance(&svm, &op2.pubkey());
    ok(&mut svm, bid_replacement_ix(&op2.pubkey(), 1, 600_000_000, None), &op2, &[&op2]);
    // Raising own bid locks only the delta, not the full amount again.
    ok(&mut svm, bid_replacement_ix(&op2.pubkey(), 1, 650_000_000, None), &op2, &[&op2]);
    assert_eq!(xcav_balance(&svm, &op2.pubkey()), before - 650_000_000);
}

#[test]
fn replacement_bid_rejects_wrong_previous_bidder() {
    let (mut svm, operator, authority) = setup();
    reach_seat_open(&mut svm, &operator, &authority);

    let op2 = new_operator(&mut svm, &authority);
    let op3 = new_operator(&mut svm, &authority);
    ok(&mut svm, bid_replacement_ix(&op2.pubkey(), 1, 600_000_000, None), &op2, &[&op2]);
    // op3 outbids but points previous_bidder_token at a third party, not the leader.
    fails_with(
        &mut svm,
        bid_replacement_ix(&op3.pubkey(), 1, 700_000_000, Some(operator.pubkey())),
        &op3,
        &[&op3],
        "WrongPreviousBidder",
    );
}

#[test]
fn replacement_bid_rejects_missing_previous_bidder() {
    let (mut svm, operator, authority) = setup();
    reach_seat_open(&mut svm, &operator, &authority);

    let op2 = new_operator(&mut svm, &authority);
    let op3 = new_operator(&mut svm, &authority);
    ok(&mut svm, bid_replacement_ix(&op2.pubkey(), 1, 600_000_000, None), &op2, &[&op2]);
    fails_with(
        &mut svm,
        bid_replacement_ix(&op3.pubkey(), 1, 700_000_000, None),
        &op3,
        &[&op3],
        "MissingPreviousBidder",
    );
}

#[test]
fn replacement_bid_too_low_fails() {
    let (mut svm, operator, authority) = setup();
    reach_seat_open(&mut svm, &operator, &authority);

    let op2 = new_operator(&mut svm, &authority);
    let op3 = new_operator(&mut svm, &authority);
    ok(&mut svm, bid_replacement_ix(&op2.pubkey(), 1, 600_000_000, None), &op2, &[&op2]);
    fails_with(
        &mut svm,
        bid_replacement_ix(&op3.pubkey(), 1, 600_000_000, Some(op2.pubkey())),
        &op3,
        &[&op3],
        "BidTooLow",
    );
}

#[test]
fn replacement_bid_below_minimum_fails() {
    let (mut svm, operator, authority) = setup();
    reach_seat_open(&mut svm, &operator, &authority);

    let op2 = new_operator(&mut svm, &authority);
    // First bid below minimum_region_deposit (500M).
    fails_with(
        &mut svm,
        bid_replacement_ix(&op2.pubkey(), 1, 400_000_000, None),
        &op2,
        &[&op2],
        "BidBelowMinimum",
    );
}

#[test]
fn replacement_bid_after_auction_ends_fails() {
    let (mut svm, operator, authority) = setup();
    reach_seat_open(&mut svm, &operator, &authority);

    let op2 = new_operator(&mut svm, &authority);
    let op3 = new_operator(&mut svm, &authority);
    ok(&mut svm, bid_replacement_ix(&op2.pubkey(), 1, 600_000_000, None), &op2, &[&op2]);
    warp(&mut svm, 2_000); // past auction_period (1_000)
    fails_with(
        &mut svm,
        bid_replacement_ix(&op3.pubkey(), 1, 700_000_000, Some(op2.pubkey())),
        &op3,
        &[&op3],
        "AuctionEnded",
    );
}

#[test]
fn replacement_bid_fails_for_non_operator() {
    let (mut svm, operator, authority) = setup();
    reach_seat_open(&mut svm, &operator, &authority);

    // A funded XCAV holder without the RegionalOperator role cannot bid.
    let stranger = actor(&mut svm);
    fails_with(
        &mut svm,
        bid_replacement_ix(&stranger.pubkey(), 1, 600_000_000, None),
        &stranger,
        &[&stranger],
        "AccountNotInitialized",
    );
}

#[test]
fn finalize_replacement_fails_before_expiry() {
    let (mut svm, operator, authority) = setup();
    reach_seat_open(&mut svm, &operator, &authority);

    let op2 = new_operator(&mut svm, &authority);
    ok(&mut svm, bid_replacement_ix(&op2.pubkey(), 1, 600_000_000, None), &op2, &[&op2]);
    let cranker = funded(&mut svm);
    fails_with(
        &mut svm,
        finalize_replacement_ix(&cranker.pubkey(), 1, &operator.pubkey()),
        &cranker,
        &[&cranker],
        "AuctionNotFinished",
    );
}
