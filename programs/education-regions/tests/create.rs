//! v2 region lifecycle: a passed proposal is claimed by its proposer (creating
//! the region), and an open seat is taken over first-come by another operator
//! bonding 0.1% of XCAV supply. No auctions.

mod common;
use common::*;

// ============================ create (claim a passed region) ============================

#[test]
fn create_region_makes_proposer_the_operator() {
    let (mut svm, operator, authority) = setup();
    reach_passed(&mut svm, &operator, &authority);
    assert_eq!(region_state_of(&svm, 1).status, RegionStatus::Passed);

    ok(
        &mut svm,
        create_region_ix(&operator.pubkey(), 1),
        &operator,
        &[&operator],
    );

    let region = region_of(&svm, 1);
    assert_eq!(region.owner, operator.pubkey());
    // The bond locked when proposing (DEPOSIT) is now the region's collateral.
    assert_eq!(region.collateral, DEPOSIT);
    assert_eq!(region.active_strikes, 0);
    // The region state was closed.
    assert!(svm
        .get_account(&region_state(1))
        .map_or(true, |a| a.data.is_empty()));
}

#[test]
fn create_region_only_by_proposer() {
    let (mut svm, operator, authority) = setup();
    reach_passed(&mut svm, &operator, &authority);

    // A different operator can't claim a region they didn't propose.
    let other = new_operator(&mut svm, &authority);
    fails_with(
        &mut svm,
        create_region_ix(&other.pubkey(), 1),
        &other,
        &[&other],
        "NotProposer",
    );
}

#[test]
fn create_region_requires_passed() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(
        &mut svm,
        propose_ix(&operator.pubkey(), 1, id),
        &operator,
        &[&operator],
    );
    // Still Proposing (not finalized), so it can't be created yet.
    fails_with(
        &mut svm,
        create_region_ix(&operator.pubkey(), 1),
        &operator,
        &[&operator],
        "RegionNotPassed",
    );
}

#[test]
fn create_region_rechecks_operator_role() {
    let (mut svm, operator, authority) = setup();
    reach_passed(&mut svm, &operator, &authority);

    // The proposer loses their RegionalOperator role before claiming: the role
    // PDA no longer resolves, so they can't take the seat.
    let admin = funded(&mut svm);
    ok(
        &mut svm,
        roles_add_admin_ix(&authority.pubkey(), &admin.pubkey()),
        &authority,
        &[&authority],
    );
    ok(
        &mut svm,
        roles_remove_ix(&admin.pubkey(), &operator.pubkey(), Role::RegionalOperator),
        &admin,
        &[&admin],
    );
    fails_with(
        &mut svm,
        create_region_ix(&operator.pubkey(), 1),
        &operator,
        &[&operator],
        "AccountNotInitialized",
    );
}

// ============================ claim an open seat ============================

#[test]
fn claim_open_region_changes_operator_and_refunds_old() {
    let (mut svm, operator, authority) = setup();
    // Region created, then the seat opened via resignation + warp past notice.
    reach_seat_open(&mut svm, &operator, &authority);

    let old_before = xcav_balance(&svm, &operator.pubkey());
    let newop = new_operator(&mut svm, &authority);
    let new_before = xcav_balance(&svm, &newop.pubkey());

    ok(
        &mut svm,
        claim_open_region_ix(&newop.pubkey(), 1, &operator.pubkey()),
        &newop,
        &[&newop],
    );

    let region = region_of(&svm, 1);
    assert_eq!(region.owner, newop.pubkey());
    assert_eq!(region.collateral, DEPOSIT); // the new bond
    assert_eq!(region.active_strikes, 0);
    // Outgoing operator got their collateral back; the new one bonded DEPOSIT.
    assert_eq!(xcav_balance(&svm, &operator.pubkey()) - old_before, DEPOSIT);
    assert_eq!(new_before - xcav_balance(&svm, &newop.pubkey()), DEPOSIT);
}

#[test]
fn claim_open_region_rejects_self_claim() {
    let (mut svm, operator, authority) = setup();
    // Seat opened via the operator's own resignation.
    reach_seat_open(&mut svm, &operator, &authority);

    fails_with(
        &mut svm,
        claim_open_region_ix(&operator.pubkey(), 1, &operator.pubkey()),
        &operator,
        &[&operator],
        "DuplicateMutableAccount",
    );
}

#[test]
fn claim_open_region_fails_before_seat_open() {
    let (mut svm, operator, authority) = setup();
    // Region created but the term hasn't elapsed and no resignation.
    reach_created(&mut svm, &operator, &authority);

    let newop = new_operator(&mut svm, &authority);
    fails_with(
        &mut svm,
        claim_open_region_ix(&newop.pubkey(), 1, &operator.pubkey()),
        &newop,
        &[&newop],
        "RegionOwnerCantBeChanged",
    );
}

// ============================ resignation ============================

#[test]
fn resignation_requires_owner() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let other = new_operator(&mut svm, &authority);
    fails_with(
        &mut svm,
        resign_ix(&other.pubkey(), 1),
        &other,
        &[&other],
        "NotRegionOwner",
    );
}

#[test]
fn resign_fails_when_change_already_scheduled() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    ok(
        &mut svm,
        resign_ix(&operator.pubkey(), 1),
        &operator,
        &[&operator],
    );
    // A second resignation can't push the change back out.
    fails_with(
        &mut svm,
        resign_ix(&operator.pubkey(), 1),
        &operator,
        &[&operator],
        "OwnerChangeAlreadyScheduled",
    );
}

// ============================ stale passed cleanup ============================

#[test]
fn stale_passed_region_clears_and_refunds_bond() {
    let (mut svm, operator, authority) = setup();
    reach_passed(&mut svm, &operator, &authority);

    // The proposer never claims; past the claim deadline (owner_change_period,
    // 10_000) anyone can clear the state and the bond is refunded.
    let op_before = xcav_balance(&svm, &operator.pubkey());
    warp(&mut svm, 11_000);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        clear_ix(&cranker.pubkey(), 1, &operator.pubkey()),
        &cranker,
        &[&cranker],
    );

    assert_eq!(xcav_balance(&svm, &operator.pubkey()) - op_before, DEPOSIT);
    assert!(svm
        .get_account(&region_state(1))
        .map_or(true, |a| a.data.is_empty()));
}
