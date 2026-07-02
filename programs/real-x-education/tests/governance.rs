mod common;
use common::*;

#[test]
fn proposal_to_module_flow_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);

    // A creator opens a proposal, staking the proposal deposit.
    let staked_before = balance(&w.svm, &xcav_mint(), &creator.pubkey());
    ok(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 10), &creator, &[&creator]);
    assert_eq!(staked_before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Voting);
    assert_eq!(config(&w.svm).next_proposal_id, 1);

    // A single voter clears the quorum and threshold with a yes vote.
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    assert_eq!(proposal_of(&w.svm, 0).yes_power, 10_000);

    // After voting closes, anyone can finalize; the stake comes back.
    warp_past_voting(&mut w.svm);
    let refund_before = balance(&w.svm, &xcav_mint(), &creator.pubkey());
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &creator.pubkey()), &cranker, &[&cranker]);
    assert_eq!(balance(&w.svm, &xcav_mint(), &creator.pubkey()) - refund_before, MODULE_DEPOSIT);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimable);

    // The proposing creator reserves the build (locking the deposit), uploads
    // the content, the agent passes review, and the module mints. The deposit
    // stays locked the whole way and becomes the module's deposit.
    let deposit_before = balance(&w.svm, &xcav_mint(), &creator.pubkey());
    ok(&mut w.svm, claim_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimed);
    assert_eq!(deposit_before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    ok(&mut w.svm, upload_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::UnderReview);
    assert_eq!(deposit_before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    ok(&mut w.svm, review_proposal_ix(&agent.pubkey(), 0, true), &agent, &[&agent]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Approved);

    ok(&mut w.svm, mint_proposed_ix(&creator.pubkey(), 0, 0), &creator, &[&creator]);
    let m = module_of(&w.svm, 0);
    assert_eq!(m.creator, creator.pubkey());
    assert_eq!(m.deposit, MODULE_DEPOSIT);
    assert_eq!(m.total_token_amount, 10);
    assert_eq!(m.sponsor_allocation, 10);
    assert_eq!(spl_amount(&w.svm, &module_vault_pda(0)), 10);
    // The deposit is still locked, now as the module's deposit.
    assert_eq!(deposit_before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    assert_eq!(config(&w.svm).next_module_id, 1);
    // The proposal record is closed once the module is created.
    assert!(w.svm.get_account(&proposal_pda(0)).map(|a| a.data.is_empty()).unwrap_or(true));

    // The voter unlocks the XCAV they locked.
    let unlock_before = balance(&w.svm, &xcav_mint(), &voter.pubkey());
    ok(&mut w.svm, unlock_vote_ix(&voter.pubkey(), 0), &voter, &[&voter]);
    assert_eq!(balance(&w.svm, &xcav_mint(), &voter.pubkey()) - unlock_before, 10_000);
}

#[test]
fn sponsor_proposal_pre_sponsors_on_mint() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);

    // A sponsor opens a proposal, locking the stake plus the pre-sponsorship
    // payment for two tokens (config.pre_sponsor_amount).
    let usdc_before = balance(&w.svm, &usdc_mint(), &sponsor.pubkey());
    ok(&mut w.svm, create_sponsor_proposal_ix(&sponsor.pubkey(), 1, 0, 10), &sponsor, &[&sponsor]);
    assert_eq!(usdc_before - balance(&w.svm, &usdc_mint(), &sponsor.pubkey()), 2 * PER_TOKEN);
    assert_eq!(spl_amount(&w.svm, &proposal_escrow_pda(0)), 2 * PER_TOKEN);
    assert_eq!(proposal_of(&w.svm, 0).pre_sponsor_amount, 2);

    // Pass the vote and finalize.
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &sponsor.pubkey()), &cranker, &[&cranker]);

    // A creator reserves, uploads, and builds it; on mint the pre-sponsorship
    // converts into a real sponsorship in the sponsor's name.
    ok(&mut w.svm, claim_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    ok(&mut w.svm, upload_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    ok(&mut w.svm, review_proposal_ix(&agent.pubkey(), 0, true), &agent, &[&agent]);
    ok(&mut w.svm, mint_sponsored_ix(&creator.pubkey(), &sponsor.pubkey(), 0, 0, 0), &creator, &[&creator]);

    let m = module_of(&w.svm, 0);
    assert_eq!(m.creator, creator.pubkey());
    assert_eq!(m.total_token_amount, 10);
    assert_eq!(m.sponsor_allocation, 8);
    assert_eq!(m.school_allocation, 2);

    let sp = Sponsorship::try_deserialize(
        &mut &w.svm.get_account(&sponsorship_pda(0, 0)).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(sp.sponsor, sponsor.pubkey());
    assert_eq!(sp.amount, 2);
    assert_eq!(sp.price_per_token, PER_TOKEN);
    // Funds moved from the pre-sponsor escrow into the sponsorship escrow.
    assert_eq!(spl_amount(&w.svm, &sponsor_escrow_pda(0, 0)), 2 * PER_TOKEN);
    assert!(w.svm.get_account(&proposal_escrow_pda(0)).map(|a| a.data.is_empty()).unwrap_or(true));
    assert!(w.svm.get_account(&proposal_pda(0)).map(|a| a.data.is_empty()).unwrap_or(true));
    assert_eq!(config(&w.svm).next_sponsor_id, 1);
}

#[test]
fn unbuilt_sponsor_proposal_expires_and_refunds() {
    let mut w = setup();
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);

    // A sponsor proposal passes the vote but nobody ever builds the module.
    let usdc_before = balance(&w.svm, &usdc_mint(), &sponsor.pubkey());
    ok(&mut w.svm, create_sponsor_proposal_ix(&sponsor.pubkey(), 1, 0, 10), &sponsor, &[&sponsor]);
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &sponsor.pubkey()), &cranker, &[&cranker]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimable);

    // Before the build deadline passes the proposal can't be expired.
    err(&mut w.svm, expire_proposal_ix(&cranker.pubkey(), 0), &cranker, &[&cranker], "BuildDeadlineNotReached");

    // Once it does, anyone can expire it.
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, expire_proposal_ix(&cranker.pubkey(), 0), &cranker, &[&cranker]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Rejected);

    // The sponsor reclaims the pre-sponsorship payment in full and the records
    // are closed.
    ok(&mut w.svm, reclaim_pre_sponsor_ix(&sponsor.pubkey(), 0), &sponsor, &[&sponsor]);
    assert_eq!(balance(&w.svm, &usdc_mint(), &sponsor.pubkey()), usdc_before);
    assert!(w.svm.get_account(&proposal_escrow_pda(0)).map(|a| a.data.is_empty()).unwrap_or(true));
    assert!(w.svm.get_account(&proposal_pda(0)).map(|a| a.data.is_empty()).unwrap_or(true));
}

#[test]
fn abandoned_reservation_slashes_bond_and_reopens() {
    let mut w = setup();
    let school = with_role(&mut w, Role::ModuleBooker);
    let creator1 = with_role(&mut w, Role::ModuleCreator);
    let creator2 = with_role(&mut w, Role::ModuleCreator);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);

    // A school opens a proposal that passes, so any creator may build it.
    ok(&mut w.svm, create_proposal_ix(&school.pubkey(), Role::ModuleBooker, 1, 0, 10), &school, &[&school]);
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &school.pubkey()), &cranker, &[&cranker]);

    // The first creator reserves the build, locking the bond.
    let before = balance(&w.svm, &xcav_mint(), &creator1.pubkey());
    ok(&mut w.svm, claim_proposal_ix(&creator1.pubkey(), 0), &creator1, &[&creator1]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimed);
    assert_eq!(before - balance(&w.svm, &xcav_mint(), &creator1.pubkey()), MODULE_DEPOSIT);

    // The reservation can't be released until the upload deadline passes.
    err(&mut w.svm, release_claim_ix(&cranker.pubkey(), 0), &cranker, &[&cranker], "UploadDeadlineNotReached");

    // After it lapses, anyone can release it: the bond is slashed to the
    // treasury and the proposal reopens.
    let treasury_before = treasury_balance(&w.svm);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, release_claim_ix(&cranker.pubkey(), 0), &cranker, &[&cranker]);
    assert_eq!(treasury_balance(&w.svm) - treasury_before, MODULE_DEPOSIT);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimable);
    assert_eq!(proposal_of(&w.svm, 0).claimant, None);

    // A different creator can now reserve it.
    ok(&mut w.svm, claim_proposal_ix(&creator2.pubkey(), 0), &creator2, &[&creator2]);
    assert_eq!(proposal_of(&w.svm, 0).claimant, Some(creator2.pubkey()));
}

#[test]
fn second_review_fail_slashes_deposit_and_bans() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let agent = with_role(&mut w, Role::ModuleAIAgent);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);

    ok(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &creator.pubkey()), &cranker, &[&cranker]);

    // Claim locks the deposit; it rides through the first failed review.
    let before = balance(&w.svm, &xcav_mint(), &creator.pubkey());
    ok(&mut w.svm, claim_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    assert_eq!(before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    ok(&mut w.svm, upload_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    ok(&mut w.svm, review_proposal_ix(&agent.pubkey(), 0, false), &agent, &[&agent]);
    // First fail: back to reserved with the deposit still locked, no slash.
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimed);
    assert_eq!(before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);

    // Re-upload and fail again: the deposit is slashed and the creator banned.
    let treasury_before = treasury_balance(&w.svm);
    ok(&mut w.svm, upload_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    ok(&mut w.svm, review_proposal_ix(&agent.pubkey(), 0, false), &agent, &[&agent]);
    assert_eq!(treasury_balance(&w.svm) - treasury_before, MODULE_DEPOSIT);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Claimable);
    assert_eq!(proposal_of(&w.svm, 0).claimant, None);
    assert!(proposal_of(&w.svm, 0).banned.contains(&creator.pubkey()));

    // The banned creator can't re-claim.
    err(&mut w.svm, claim_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator], "CreatorBanned");
}

#[test]
fn create_proposal_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let agent = with_role(&mut w, Role::ModuleAIAgent);

    // The AI agent role isn't allowed to open proposals.
    err(
        &mut w.svm,
        create_proposal_ix(&agent.pubkey(), Role::ModuleAIAgent, 1, 0, 10),
        &agent,
        &[&agent],
        "InvalidProposalRole",
    );
    // Amount boundaries.
    err(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 0), &creator, &[&creator], "AmountCannotBeZero");
    err(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 1_001), &creator, &[&creator], "TooManyTokens");
}

#[test]
fn vote_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let voter = actor(&mut w.svm);
    ok(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 10), &creator, &[&creator]);

    // Below the configured minimum.
    err(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 500), &voter, &[&voter], "BelowMinimumVotingAmount");
    // After the voting window closes.
    warp_past_voting(&mut w.svm);
    err(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter], "ProposalExpired");
}

#[test]
fn vote_replace_refunds() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let voter = actor(&mut w.svm);
    ok(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 10), &creator, &[&creator]);

    let before = balance(&w.svm, &xcav_mint(), &voter.pubkey());
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    assert_eq!(before - balance(&w.svm, &xcav_mint(), &voter.pubkey()), 10_000);

    // Re-vote No with a smaller stake: the old lock is refunded, the new one taken.
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::No, 5_000), &voter, &[&voter]);
    assert_eq!(before - balance(&w.svm, &xcav_mint(), &voter.pubkey()), 5_000);
    let p = proposal_of(&w.svm, 0);
    assert_eq!(p.yes_power, 0);
    assert_eq!(p.no_power, 5_000);
}

#[test]
fn finalize_reject_slashes_stake() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);
    ok(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 10), &creator, &[&creator]);
    // Meets quorum but is voted down, so it fails the threshold.
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::No, 10_000), &voter, &[&voter]);
    warp_past_voting(&mut w.svm);

    let treasury_before = treasury_balance(&w.svm);
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &creator.pubkey()), &cranker, &[&cranker]);
    assert_eq!(treasury_balance(&w.svm) - treasury_before, MODULE_DEPOSIT);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Rejected);
}

#[test]
fn finalize_while_voting_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);
    ok(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);

    err(
        &mut w.svm,
        finalize_proposal_ix(&cranker.pubkey(), 0, &creator.pubkey()),
        &cranker,
        &[&cranker],
        "VotingStillOngoing",
    );
}

#[test]
fn unlock_vote_while_voting_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let voter = actor(&mut w.svm);
    ok(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);

    err(&mut w.svm, unlock_vote_ix(&voter.pubkey(), 0), &voter, &[&voter], "VotingStillOngoing");
}

#[test]
fn clear_proposal_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);
    ok(&mut w.svm, create_proposal_ix(&creator.pubkey(), Role::ModuleCreator, 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::No, 10_000), &voter, &[&voter]);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &creator.pubkey()), &cranker, &[&cranker]);

    // A rejected non-sponsor proposal can be cleared, returning its rent.
    ok(&mut w.svm, clear_proposal_ix(&cranker.pubkey(), 0, &creator.pubkey()), &cranker, &[&cranker]);
    assert!(closed(&w.svm, &proposal_pda(0)));
}

#[test]
fn expire_slashes_riding_deposit() {
    let mut w = setup();
    let school = with_role(&mut w, Role::ModuleBooker);
    let creator = with_role(&mut w, Role::ModuleCreator);
    let voter = actor(&mut w.svm);
    let cranker = funded(&mut w.svm);

    // A school proposal passes, so any creator may build it.
    ok(&mut w.svm, create_proposal_ix(&school.pubkey(), Role::ModuleBooker, 1, 0, 10), &school, &[&school]);
    ok(&mut w.svm, vote_ix(&voter.pubkey(), 0, ModuleVote::Yes, 10_000), &voter, &[&voter]);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, finalize_proposal_ix(&cranker.pubkey(), 0, &school.pubkey()), &cranker, &[&cranker]);

    // A creator reserves and uploads; the deposit is now riding on the proposal.
    ok(&mut w.svm, claim_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    ok(&mut w.svm, upload_proposal_ix(&creator.pubkey(), 0), &creator, &[&creator]);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::UnderReview);

    // The build window lapses with no mint: expiring it slashes the riding
    // deposit to the treasury.
    let treasury_before = treasury_balance(&w.svm);
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, expire_proposal_ix(&cranker.pubkey(), 0), &cranker, &[&cranker]);
    assert_eq!(treasury_balance(&w.svm) - treasury_before, MODULE_DEPOSIT);
    assert_eq!(proposal_of(&w.svm, 0).status, ProposalStatus::Rejected);
}
