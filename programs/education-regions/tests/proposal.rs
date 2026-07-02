//! Region proposal lifecycle: proposing, voting, finalizing, and reclaiming or
//! clearing state afterwards.

mod common;
use common::*;

// ============================ propose ============================

#[test]
fn propose_new_region_works() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    let op_before = xcav_balance(&svm, &operator.pubkey());

    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let acc = svm.get_account(&proposal_pda(id)).unwrap();
    let proposal = RegionProposal::try_deserialize(&mut &acc.data[..]).unwrap();
    assert_eq!(proposal.proposer, operator.pubkey());
    assert_eq!(proposal.region_id, 1);
    assert_eq!(proposal.deposit, DEPOSIT);
    assert_eq!(proposal.yes_power, 0);
    // Deposit moved from proposer into the vault.
    assert_eq!(op_before - xcav_balance(&svm, &operator.pubkey()), DEPOSIT);
    assert_eq!(vault_balance(&svm), DEPOSIT);
    // Counter advanced.
    assert_eq!(next_proposal_id(&svm), id + 1);
}

#[test]
fn propose_fails_for_non_operator() {
    let (mut svm, _operator, _authority) = setup();
    let stranger = actor(&mut svm);
    let id = next_proposal_id(&svm);
    // No RegionalOperator role -> the role PDA doesn't exist.
    fails_with(
        &mut svm,
        propose_ix(&stranger.pubkey(), 1, id),
        &stranger,
        &[&stranger],
        "AccountNotInitialized",
    );
}

#[test]
fn propose_fails_for_unknown_region() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    fails_with(
        &mut svm,
        propose_ix(&operator.pubkey(), 99, id),
        &operator,
        &[&operator],
        "InvalidRegion",
    );
}

#[test]
fn propose_fails_when_region_already_has_proposal() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    // Second proposal for region 1 -> the region pointer already exists.
    let id2 = next_proposal_id(&svm);
    fails_with(
        &mut svm,
        propose_ix(&operator.pubkey(), 1, id2),
        &operator,
        &[&operator],
        "already in use",
    );
}

// ============================ vote ============================

#[test]
fn vote_records_power() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let voter = actor(&mut svm);
    let voter_before = xcav_balance(&svm, &voter.pubkey());
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);

    let proposal = RegionProposal::try_deserialize(
        &mut &svm.get_account(&proposal_pda(id)).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(proposal.yes_power, 200_000_000);

    let vr_acc = svm.get_account(&vote_record_pda(id, &voter.pubkey())).unwrap();
    let vr = VoteRecord::try_deserialize(&mut &vr_acc.data[..]).unwrap();
    assert_eq!(vr.power, 200_000_000);
    assert_eq!(vr.vote, Vote::Yes);
    // Power moved from voter into the vault (on top of the proposal deposit).
    assert_eq!(voter_before - xcav_balance(&svm, &voter.pubkey()), 200_000_000);
    assert_eq!(vault_balance(&svm), DEPOSIT + 200_000_000);
}

#[test]
fn vote_below_minimum_fails() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let voter = actor(&mut svm);
    fails_with(
        &mut svm,
        vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 1),
        &voter,
        &[&voter],
        "BelowMinimumVotingAmount",
    );
}

#[test]
fn revote_replaces_previous_vote() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let voter = actor(&mut svm);
    let voter_before = xcav_balance(&svm, &voter.pubkey());
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
    // Change vote to No with a different amount.
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::No, 300_000_000), &voter, &[&voter]);

    let proposal = RegionProposal::try_deserialize(
        &mut &svm.get_account(&proposal_pda(id)).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(proposal.yes_power, 0);
    assert_eq!(proposal.no_power, 300_000_000);

    let vr = VoteRecord::try_deserialize(
        &mut &svm.get_account(&vote_record_pda(id, &voter.pubkey())).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(vr.vote, Vote::No);
    assert_eq!(vr.power, 300_000_000);
    // Net XCAV locked equals only the new vote; the old lock was refunded.
    assert_eq!(voter_before - xcav_balance(&svm, &voter.pubkey()), 300_000_000);
    assert_eq!(vault_balance(&svm), DEPOSIT + 300_000_000);
}

#[test]
fn vote_after_expiry_fails() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    warp_past_voting(&mut svm);
    let voter = actor(&mut svm);
    fails_with(
        &mut svm,
        vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000),
        &voter,
        &[&voter],
        "ProposalExpired",
    );
}

#[test]
fn vote_with_insufficient_balance_fails() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    // A voter who holds more than the minimum but less than they try to lock.
    let poor = funded(&mut svm);
    give_xcav(&mut svm, &poor.pubkey(), 150_000_000);
    fails_with(
        &mut svm,
        vote_ix(&poor.pubkey(), 1, id, Vote::Yes, 200_000_000),
        &poor,
        &[&poor],
        "insufficient funds",
    );
}

// ============================ finalize ============================

#[test]
fn finalize_passes_and_starts_auction() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);

    let op_before = xcav_balance(&svm, &operator.pubkey());
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey()),
        &cranker,
        &[&cranker],
    );

    let rs = region_state_of(&svm, 1);
    assert_eq!(rs.status, RegionStatus::Auctioning);
    assert_eq!(rs.collateral, 500_000_000); // minimum_region_deposit
    // Deposit returned to the proposer; only the voter's lock remains in the vault.
    assert_eq!(xcav_balance(&svm, &operator.pubkey()) - op_before, DEPOSIT);
    assert_eq!(vault_balance(&svm), 200_000_000);
    // Proposal account was closed.
    assert!(svm.get_account(&proposal_pda(id)).map_or(true, |a| a.data.is_empty()));
}

#[test]
fn finalize_rejects_and_slashes_deposit() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    // No votes -> quorum not met -> rejected.

    let treasury_before = treasury_balance(&svm);
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey()),
        &cranker,
        &[&cranker],
    );

    let rs = region_state_of(&svm, 1);
    assert_eq!(rs.status, RegionStatus::Rejected);
    // Deposit slashed from the vault to the treasury.
    assert_eq!(treasury_balance(&svm) - treasury_before, DEPOSIT);
    assert_eq!(vault_balance(&svm), 0);
}

#[test]
fn finalize_fails_while_voting_ongoing() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let cranker = funded(&mut svm);
    fails_with(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey()),
        &cranker,
        &[&cranker],
        "VotingStillOngoing",
    );
}

// ============================ cleanup ============================

#[test]
fn unlock_voting_token_works() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
    let voter_before = xcav_balance(&svm, &voter.pubkey());

    warp_past_voting(&mut svm);
    ok(&mut svm, unlock_ix(&voter.pubkey(), id), &voter, &[&voter]);

    // Vote record closed and the locked power returned as XCAV.
    assert!(svm.get_account(&vote_record_pda(id, &voter.pubkey())).map_or(true, |a| a.data.is_empty()));
    assert_eq!(xcav_balance(&svm, &voter.pubkey()) - voter_before, 200_000_000);
    // Only the proposal deposit is left in the vault.
    assert_eq!(vault_balance(&svm), DEPOSIT);
}

#[test]
fn unlock_fails_while_voting_ongoing() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    let voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
    // Window is still open, so the lock can't be reclaimed yet.
    fails_with(
        &mut svm,
        unlock_ix(&voter.pubkey(), id),
        &voter,
        &[&voter],
        "VotingStillOngoing",
    );
}

#[test]
fn unlock_without_vote_fails() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    // Someone who never voted has no vote record to unlock.
    let stranger = actor(&mut svm);
    warp_past_voting(&mut svm);
    fails_with(
        &mut svm,
        unlock_ix(&stranger.pubkey(), id),
        &stranger,
        &[&stranger],
        "AccountNotInitialized",
    );
}

#[test]
fn clear_region_state_after_reject() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    // No votes -> rejected on finalize.
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey()),
        &cranker,
        &[&cranker],
    );
    assert_eq!(region_state_of(&svm, 1).status, RegionStatus::Rejected);

    ok(&mut svm, clear_ix(&cranker.pubkey(), 1), &cranker, &[&cranker]);
    assert!(svm.get_account(&region_state(1)).map_or(true, |a| a.data.is_empty()));
}

// ============================ threshold / quorum ============================

#[test]
fn finalize_all_abstain_passes_if_quorum() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    // Abstain counts toward quorum but not the approval base, so an all-abstain
    // proposal that clears quorum passes (intended behavior).
    let voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Abstain, 200_000_000), &voter, &[&voter]);
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey()),
        &cranker,
        &[&cranker],
    );
    assert_eq!(region_state_of(&svm, 1).status, RegionStatus::Auctioning);
}

#[test]
fn finalize_exact_threshold_passes() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    // yes == no is exactly the 50% threshold (yes*2 >= yes+no holds at equality).
    let yes_voter = actor(&mut svm);
    let no_voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&yes_voter.pubkey(), 1, id, Vote::Yes, 100_000_000), &yes_voter, &[&yes_voter]);
    ok(&mut svm, vote_ix(&no_voter.pubkey(), 1, id, Vote::No, 100_000_000), &no_voter, &[&no_voter]);
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey()),
        &cranker,
        &[&cranker],
    );
    assert_eq!(region_state_of(&svm, 1).status, RegionStatus::Auctioning);
}

#[test]
fn finalize_below_threshold_rejects() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);

    // yes < no fails the threshold even though quorum is met.
    let yes_voter = actor(&mut svm);
    let no_voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&yes_voter.pubkey(), 1, id, Vote::Yes, 100_000_000), &yes_voter, &[&yes_voter]);
    ok(&mut svm, vote_ix(&no_voter.pubkey(), 1, id, Vote::No, 200_000_000), &no_voter, &[&no_voter]);
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey()),
        &cranker,
        &[&cranker],
    );
    assert_eq!(region_state_of(&svm, 1).status, RegionStatus::Rejected);
}

#[test]
fn finalize_below_quorum_rejects() {
    let (mut svm, _operator, authority) = setup();
    // Raise the quorum above a single vote so a lone yes cannot reach it.
    let mut params = default_params();
    params.quorum = 300_000_000;
    ok(&mut svm, update_config_ix(&authority.pubkey(), params), &authority, &[&authority]);

    let operator = new_operator(&mut svm, &authority);
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    // Unanimous yes, but total 200M is below the 300M quorum.
    let voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey()),
        &cranker,
        &[&cranker],
    );
    assert_eq!(region_state_of(&svm, 1).status, RegionStatus::Rejected);
}

#[test]
fn finalize_exact_quorum_rejects() {
    let (mut svm, _operator, authority) = setup();
    // The quorum is strict: a turnout exactly at the quorum is not enough.
    let mut params = default_params();
    params.quorum = 200_000_000;
    ok(&mut svm, update_config_ix(&authority.pubkey(), params), &authority, &[&authority]);

    let operator = new_operator(&mut svm, &authority);
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    let voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey()),
        &cranker,
        &[&cranker],
    );
    assert_eq!(region_state_of(&svm, 1).status, RegionStatus::Rejected);
}

#[test]
fn finalize_reject_without_proposer_token_works() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    warp_past_voting(&mut svm);

    // Rejection slashes to the treasury, so the crank settles even with no
    // proposer token account at all.
    let cranker = funded(&mut svm);
    ok(
        &mut svm,
        finalize_no_token_ix(&cranker.pubkey(), 1, id, &operator.pubkey()),
        &cranker,
        &[&cranker],
    );
    assert_eq!(region_state_of(&svm, 1).status, RegionStatus::Rejected);
    assert_eq!(treasury_balance(&svm), DEPOSIT);
}

#[test]
fn finalize_pass_requires_proposer_token() {
    let (mut svm, operator, _authority) = setup();
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    let voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);
    warp_past_voting(&mut svm);

    // A passing proposal pays the deposit back, so the account is mandatory.
    let cranker = funded(&mut svm);
    fails_with(
        &mut svm,
        finalize_no_token_ix(&cranker.pubkey(), 1, id, &operator.pubkey()),
        &cranker,
        &[&cranker],
        "MissingRecipientToken",
    );
}

#[test]
fn finalize_pass_pays_treasury_reward() {
    let (mut svm, operator, _authority) = setup();
    // A funded treasury matches the returned deposit with a reward.
    fund_treasury(&mut svm, 2 * DEPOSIT);
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    let voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);

    let op_before = xcav_balance(&svm, &operator.pubkey());
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(&mut svm, finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey()), &cranker, &[&cranker]);

    // Deposit back plus the deposit-matching reward.
    assert_eq!(xcav_balance(&svm, &operator.pubkey()) - op_before, 2 * DEPOSIT);
    assert_eq!(treasury_balance(&svm), DEPOSIT);
}

#[test]
fn finalize_pass_skips_reward_when_treasury_poor() {
    let (mut svm, operator, _authority) = setup();
    // Below the deposit, so no reward is paid and nothing is taken.
    fund_treasury(&mut svm, DEPOSIT - 1);
    let id = next_proposal_id(&svm);
    ok(&mut svm, propose_ix(&operator.pubkey(), 1, id), &operator, &[&operator]);
    let voter = actor(&mut svm);
    ok(&mut svm, vote_ix(&voter.pubkey(), 1, id, Vote::Yes, 200_000_000), &voter, &[&voter]);

    let op_before = xcav_balance(&svm, &operator.pubkey());
    warp_past_voting(&mut svm);
    let cranker = funded(&mut svm);
    ok(&mut svm, finalize_ix(&cranker.pubkey(), 1, id, &operator.pubkey()), &cranker, &[&cranker]);

    assert_eq!(region_state_of(&svm, 1).status, RegionStatus::Auctioning);
    assert_eq!(xcav_balance(&svm, &operator.pubkey()) - op_before, DEPOSIT);
    assert_eq!(treasury_balance(&svm), DEPOSIT - 1);
}
