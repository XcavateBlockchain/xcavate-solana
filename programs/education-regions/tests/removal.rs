//! Operator removal: proposing removal, voting, slashing on the outcome, strike
//! accrual, and reclaiming removal-vote locks.

mod common;
use common::*;

/// Read a region's open removal proposal.
fn removal_of(svm: &LiteSVM, region_id: u16) -> education_regions::state::RemovalProposal {
    education_regions::state::RemovalProposal::try_deserialize(
        &mut &svm.get_account(&removal_proposal_pda(region_id)).unwrap().data[..],
    )
    .unwrap()
}

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

#[test]
fn removal_slash_capped_at_collateral() {
    let (mut svm, operator, authority) = setup();
    // A strike slash larger than the whole collateral must cap at the collateral.
    let mut params = default_params();
    params.slash_amount = 700_000_000; // > the 600M collateral below
    ok(&mut svm, update_config_ix(&authority.pubkey(), &authority.pubkey(), params), &authority, &[&authority]);

    reach_created(&mut svm, &operator, &authority); // collateral 600M

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

    // Collateral floors at zero; only the 600M that existed is slashed, not 700M.
    assert_eq!(region_of(&svm, 1).collateral, 0);
    assert_eq!(xcav_balance(&svm, &authority.pubkey()), 600_000_000);
}

#[test]
fn revote_removal_replaces_previous() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let challenger = new_operator(&mut svm, &authority);
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_remove_ix(&challenger.pubkey(), 1), &challenger, &[&challenger]);

    let voter = actor(&mut svm);
    let voter_before = xcav_balance(&svm, &voter.pubkey());
    ok(&mut svm, vote_removal_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
    // Switch to No with a larger amount; the prior yes lock is refunded.
    ok(&mut svm, vote_removal_ix(&voter.pubkey(), 1, id, Vote::No, 300_000_000), &voter, &[&voter]);

    let rp = removal_of(&svm, 1);
    assert_eq!(rp.yes_power, 0);
    assert_eq!(rp.no_power, 300_000_000);
    // Only the latest vote stays locked.
    assert_eq!(voter_before - xcav_balance(&svm, &voter.pubkey()), 300_000_000);
}

#[test]
fn removal_rejected_on_quorum_not_met() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let challenger = new_operator(&mut svm, &authority);
    let chal_before = xcav_balance(&svm, &challenger.pubkey());
    ok(&mut svm, propose_remove_ix(&challenger.pubkey(), 1), &challenger, &[&challenger]);

    // No votes at all: quorum is not met, so the removal is rejected and the
    // proposer's deposit is slashed.
    warp(&mut svm, 2_000);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_removal_ix(&cranker.pubkey(), 1, &challenger.pubkey(), &authority.pubkey()),
        &cranker,
        &[&cranker],
    );

    let region = region_of(&svm, 1);
    assert_eq!(region.active_strikes, 0);
    assert_eq!(region.collateral, 600_000_000);
    assert_eq!(xcav_balance(&svm, &challenger.pubkey()), chal_before - DEPOSIT);
    assert_eq!(xcav_balance(&svm, &authority.pubkey()), DEPOSIT);
}

#[test]
fn propose_remove_fails_when_already_ongoing() {
    let (mut svm, operator, authority) = setup();
    reach_created(&mut svm, &operator, &authority);

    let challenger = new_operator(&mut svm, &authority);
    ok(&mut svm, propose_remove_ix(&challenger.pubkey(), 1), &challenger, &[&challenger]);
    // A second removal proposal for the same region hits the existing PDA.
    let other = new_operator(&mut svm, &authority);
    fails_with(
        &mut svm,
        propose_remove_ix(&other.pubkey(), 1),
        &other,
        &[&other],
        "already in use",
    );
}

#[test]
fn propose_remove_fails_for_unknown_region() {
    let (mut svm, _operator, authority) = setup();
    // Region 99 was never created, so its account does not resolve.
    let challenger = new_operator(&mut svm, &authority);
    fails_with(
        &mut svm,
        propose_remove_ix(&challenger.pubkey(), 99),
        &challenger,
        &[&challenger],
        "AccountNotInitialized",
    );
}
