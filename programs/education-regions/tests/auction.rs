//! Region auction: bidding to operate a passed region, previous-bidder refunds
//! and authorization, and creating the region once the auction ends.

mod common;
use common::*;

// ============================ bid ============================

#[test]
fn bid_places_and_outbid_refunds() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    let vault_before = vault_balance(&svm); // the winning voter's lock still sits here

    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);
    let rs = region_state_of(&svm, 1);
    assert_eq!(rs.highest_bidder, Some(operator.pubkey()));
    assert_eq!(rs.collateral, 600_000_000);
    assert_eq!(vault_balance(&svm), vault_before + 600_000_000);

    let op2 = new_operator(&mut svm, &authority);
    let op1_before = xcav_balance(&svm, &operator.pubkey());
    ok(
        &mut svm,
        bid_ix(&op2.pubkey(), 1, 700_000_000, Some(operator.pubkey())),
        &op2,
        &[&op2],
    );

    let rs = region_state_of(&svm, 1);
    assert_eq!(rs.highest_bidder, Some(op2.pubkey()));
    assert_eq!(rs.collateral, 700_000_000);
    // Operator was refunded their exact outbid collateral.
    assert_eq!(xcav_balance(&svm, &operator.pubkey()) - op1_before, 600_000_000);
    // Vault now holds the new top bid (plus the earlier voter lock).
    assert_eq!(vault_balance(&svm), vault_before + 700_000_000);
}

#[test]
fn bid_fails_before_auction() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    // Still in Proposing, not Auctioning.
    fails_with(
        &mut svm,
        bid_ix(&operator.pubkey(), 1, 600_000_000, None),
        &operator,
        &[&operator],
        "NotAuctioning",
    );
}

#[test]
fn bid_same_bidder_can_raise() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);

    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);
    let vault_after_first = vault_balance(&svm);
    let op_after_first = xcav_balance(&svm, &operator.pubkey());

    // The current leader raises their own bid; only the difference is locked.
    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 700_000_000, None), &operator, &[&operator]);

    let rs = region_state_of(&svm, 1);
    assert_eq!(rs.highest_bidder, Some(operator.pubkey()));
    assert_eq!(rs.collateral, 700_000_000);
    // Vault and bidder moved by exactly the top-up, not the whole new bid.
    assert_eq!(vault_balance(&svm) - vault_after_first, 100_000_000);
    assert_eq!(op_after_first - xcav_balance(&svm, &operator.pubkey()), 100_000_000);
}

#[test]
fn bid_same_bidder_cannot_lower() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);
    // Re-bidding at the same amount is not a raise.
    fails_with(
        &mut svm,
        bid_ix(&operator.pubkey(), 1, 600_000_000, None),
        &operator,
        &[&operator],
        "BidTooLow",
    );
}

#[test]
fn bid_below_minimum_fails() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    // The opening bid must clear the minimum region deposit (500_000_000).
    fails_with(
        &mut svm,
        bid_ix(&operator.pubkey(), 1, 100_000_000, None),
        &operator,
        &[&operator],
        "BidBelowMinimum",
    );
}

#[test]
fn bid_too_low_fails() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);

    // A second bidder must strictly beat the standing bid.
    let op2 = new_operator(&mut svm, &authority);
    fails_with(
        &mut svm,
        bid_ix(&op2.pubkey(), 1, 600_000_000, Some(operator.pubkey())),
        &op2,
        &[&op2],
        "BidTooLow",
    );
}

#[test]
fn bid_after_auction_ends_fails() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    warp_past_voting(&mut svm); // push past the auction expiry
    fails_with(
        &mut svm,
        bid_ix(&operator.pubkey(), 1, 600_000_000, None),
        &operator,
        &[&operator],
        "AuctionEnded",
    );
}

#[test]
fn bid_rejects_wrong_previous_bidder() {
    let (mut svm, operator, authority) = setup();
    let _ = reach_auctioning(&mut svm, &operator, &authority);

    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);
    let op2 = new_operator(&mut svm, &authority);
    let op3 = new_operator(&mut svm, &authority);
    // op2 outbids but points previous_bidder_token at a third party, not the leader.
    fails_with(
        &mut svm,
        bid_ix(&op2.pubkey(), 1, 700_000_000, Some(op3.pubkey())),
        &op2,
        &[&op2],
        "WrongPreviousBidder",
    );
}

#[test]
fn bid_rejects_missing_previous_bidder() {
    let (mut svm, operator, authority) = setup();
    let _ = reach_auctioning(&mut svm, &operator, &authority);

    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);
    let op2 = new_operator(&mut svm, &authority);
    fails_with(
        &mut svm,
        bid_ix(&op2.pubkey(), 1, 700_000_000, None),
        &op2,
        &[&op2],
        "MissingPreviousBidder",
    );
}

#[test]
fn bid_fails_for_non_operator() {
    let (mut svm, operator, authority) = setup();
    let _ = reach_auctioning(&mut svm, &operator, &authority);

    let stranger = actor(&mut svm);
    fails_with(
        &mut svm,
        bid_ix(&stranger.pubkey(), 1, 600_000_000, None),
        &stranger,
        &[&stranger],
        "AccountNotInitialized",
    );
}

// ============================ create_new_region ============================

#[test]
fn create_region_works() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);
    let vault_before = vault_balance(&svm);

    warp_past_voting(&mut svm); // push past the auction expiry too
    ok(&mut svm, create_region_ix(&operator.pubkey(), 1), &operator, &[&operator]);

    let region_acc = svm.get_account(&region_pda(1)).unwrap();
    let region = education_regions::state::Region::try_deserialize(&mut &region_acc.data[..]).unwrap();
    assert_eq!(region.region_id, 1);
    assert_eq!(region.owner, operator.pubkey());
    assert_eq!(region.collateral, 600_000_000);
    // Collateral stays in the vault; the region just records it.
    assert_eq!(vault_balance(&svm), vault_before);
    // Region state was closed.
    assert!(svm.get_account(&region_state(1)).map_or(true, |a| a.data.is_empty()));
}

#[test]
fn create_region_fails_for_non_winner() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);

    let op2 = new_operator(&mut svm, &authority);
    warp_past_voting(&mut svm);
    fails_with(
        &mut svm,
        create_region_ix(&op2.pubkey(), 1),
        &op2,
        &[&op2],
        "NotWinner",
    );
}

#[test]
fn create_region_fails_after_role_revoked() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);
    warp_past_voting(&mut svm);

    // The winner loses the RegionalOperator role between the auction and the
    // creation call; the seat cannot be taken without it.
    let admin = funded(&mut svm);
    ok(&mut svm, roles_add_admin_ix(&authority.pubkey(), &admin.pubkey()), &authority, &[&authority]);
    ok(&mut svm, roles_remove_ix(&admin.pubkey(), &operator.pubkey(), Role::RegionalOperator), &admin, &[&admin]);
    fails_with(
        &mut svm,
        create_region_ix(&operator.pubkey(), 1),
        &operator,
        &[&operator],
        "AccountNotInitialized",
    );
}

#[test]
fn create_region_fails_before_auction_ends() {
    let (mut svm, operator, authority) = setup();
    reach_auctioning(&mut svm, &operator, &authority);
    ok(&mut svm, bid_ix(&operator.pubkey(), 1, 600_000_000, None), &operator, &[&operator]);
    // Winner is set, but the auction window is still open.
    fails_with(
        &mut svm,
        create_region_ix(&operator.pubkey(), 1),
        &operator,
        &[&operator],
        "AuctionNotFinished",
    );
}
