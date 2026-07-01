//! Operator removal: proposing removal, voting, slashing on the outcome, strike
//! accrual, and reclaiming removal-vote locks.

mod common;
use common::*;

#[test]
fn removal_upheld_slashes_collateral_and_adds_strike() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let challenger = new_operator(&mut svm, &authority);
    let chal_before = xcav_balance(&svm, &challenger.pubkey());
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_remove_ix(&challenger.pubkey(), 1), &challenger, &[&challenger]);

    let voter = actor(&mut svm);
    ok(&mut svm, vote_removal_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);

    warp(&mut svm, 2_000);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_removal_ix(&cranker.pubkey(), 1, &challenger.pubkey(), &authority.pubkey()),
        &cranker,
        &[&cranker],
    );

    let region = region_of(&svm, 1);
    // One strike, collateral down by slash_amount (100M), seat not yet open (1 < 3).
    assert_eq!(region.active_strikes, 1);
    assert_eq!(region.collateral, 600_000_000 - 100_000_000);
    // Treasury received the slash; challenger's deposit was returned.
    assert_eq!(xcav_balance(&svm, &authority.pubkey()), 100_000_000);
    assert_eq!(xcav_balance(&svm, &challenger.pubkey()), chal_before);
    // Removal proposal closed.
    assert!(svm.get_account(&removal_proposal_pda(1)).map_or(true, |a| a.data.is_empty()));
}

#[test]
fn removal_rejected_slashes_proposer() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let challenger = new_operator(&mut svm, &authority);
    let chal_before = xcav_balance(&svm, &challenger.pubkey());
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_remove_ix(&challenger.pubkey(), 1), &challenger, &[&challenger]);

    let voter = actor(&mut svm);
    ok(&mut svm, vote_removal_ix(&voter.pubkey(), 1, id, Vote::No, 200_000_000), &voter, &[&voter]);

    warp(&mut svm, 2_000);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_removal_ix(&cranker.pubkey(), 1, &challenger.pubkey(), &authority.pubkey()),
        &cranker,
        &[&cranker],
    );

    let region = region_of(&svm, 1);
    // Operator untouched; challenger's deposit slashed to the treasury.
    assert_eq!(region.active_strikes, 0);
    assert_eq!(region.collateral, 600_000_000);
    assert_eq!(xcav_balance(&svm, &challenger.pubkey()), chal_before - DEPOSIT);
    assert_eq!(xcav_balance(&svm, &authority.pubkey()), DEPOSIT);
}

#[test]
fn removal_reaching_ceiling_opens_seat() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    // Three upheld removals (allowed_strikes = 3) open the seat.
    for _ in 0..3 {
        let challenger = new_operator(&mut svm, &authority);
        let id = next_proposal_id(&svm);
        ok(&mut svm, propose_remove_ix(&challenger.pubkey(), 1), &challenger, &[&challenger]);
        let voter = actor(&mut svm);
        ok(&mut svm, vote_removal_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
        warp(&mut svm, 2_000);
        let cranker = funded(&mut svm);
        ok(
            &mut svm,
            finalize_removal_ix(&cranker.pubkey(), 1, &challenger.pubkey(), &authority.pubkey()),
            &cranker,
            &[&cranker],
        );
    }

    let now = svm.get_sysvar::<Clock>().unix_timestamp;
    let region = region_of(&svm, 1);
    assert_eq!(region.active_strikes, 3);
    // Seat is open: the owner can be changed as of now.
    assert!(region.next_owner_change <= now);
}

#[test]
fn unlock_removal_vote_works() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let challenger = new_operator(&mut svm, &authority);
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_remove_ix(&challenger.pubkey(), 1), &challenger, &[&challenger]);

    let voter = actor(&mut svm);
    let before = xcav_balance(&svm, &voter.pubkey());
    ok(&mut svm, vote_removal_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
    assert_eq!(xcav_balance(&svm, &voter.pubkey()), before - 200_000_000);

    warp(&mut svm, 2_000);
    ok(&mut svm, unlock_removal_ix(&voter.pubkey(), id), &voter, &[&voter]);
    assert_eq!(xcav_balance(&svm, &voter.pubkey()), before);
}

#[test]
fn vote_removal_below_minimum_fails() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let challenger = new_operator(&mut svm, &authority);
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_remove_ix(&challenger.pubkey(), 1), &challenger, &[&challenger]);

    let voter = actor(&mut svm);
    fails_with(
        &mut svm,
        vote_removal_ix(&voter.pubkey(), 1, id, Vote::Yes, 50_000_000), // below 100M minimum
        &voter,
        &[&voter],
        "BelowMinimumVotingAmount",
    );
}

#[test]
fn vote_removal_after_expiry_fails() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let challenger = new_operator(&mut svm, &authority);
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_remove_ix(&challenger.pubkey(), 1), &challenger, &[&challenger]);

    warp(&mut svm, 2_000); // past removal_voting_period (1_000)
    let voter = actor(&mut svm);
    fails_with(
        &mut svm,
        vote_removal_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000),
        &voter,
        &[&voter],
        "ProposalExpired",
    );
}

#[test]
fn finalize_removal_fails_while_voting_ongoing() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let challenger = new_operator(&mut svm, &authority);
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_remove_ix(&challenger.pubkey(), 1), &challenger, &[&challenger]);
    let voter = actor(&mut svm);
    ok(&mut svm, vote_removal_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);

    let cranker = funded(&mut svm);
    fails_with(
        &mut svm,
        finalize_removal_ix(&cranker.pubkey(), 1, &challenger.pubkey(), &authority.pubkey()),
        &cranker,
        &[&cranker],
        "VotingStillOngoing",
    );
}

#[test]
fn unlock_removal_fails_while_voting_ongoing() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let challenger = new_operator(&mut svm, &authority);
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_remove_ix(&challenger.pubkey(), 1), &challenger, &[&challenger]);
    let voter = actor(&mut svm);
    ok(&mut svm, vote_removal_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);

    fails_with(
        &mut svm,
        unlock_removal_ix(&voter.pubkey(), id),
        &voter,
        &[&voter],
        "VotingStillOngoing",
    );
}
