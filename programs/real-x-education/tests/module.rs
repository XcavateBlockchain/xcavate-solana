mod common;
use common::*;

#[test]
fn create_module_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let before = balance(&w.svm, &xcav_mint(), &creator.pubkey());

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);

    let m = module_of(&w.svm, 0);
    assert_eq!(m.creator, creator.pubkey());
    assert_eq!(m.region, 1);
    assert_eq!(m.total_token_amount, 10);
    assert_eq!(m.sponsor_allocation, 10);
    assert_eq!(m.school_allocation, 0);
    assert_eq!(m.price, MODULE_PRICE);
    // Deposit locked in the vault; full supply minted into the module vault.
    assert_eq!(before - balance(&w.svm, &xcav_mint(), &creator.pubkey()), MODULE_DEPOSIT);
    assert_eq!(spl_amount(&w.svm, &vault()), MODULE_DEPOSIT);
    assert_eq!(spl_amount(&w.svm, &module_vault_pda(0)), 10);
    assert_eq!(config(&w.svm).next_module_id, 1);
}

#[test]
fn sponsor_module_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);

    let before = balance(&w.svm, &usdc_mint(), &sponsor.pubkey());
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 5), &sponsor, &[&sponsor]);

    let sp = Sponsorship::try_deserialize(
        &mut &w.svm.get_account(&sponsorship_pda(0, 0)).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(sp.amount, 5);
    assert_eq!(sp.price_per_token, PER_TOKEN);
    // Payment for 5 tokens escrowed.
    assert_eq!(before - balance(&w.svm, &usdc_mint(), &sponsor.pubkey()), 5 * PER_TOKEN);
    assert_eq!(spl_amount(&w.svm, &sponsor_escrow_pda(0, 0)), 5 * PER_TOKEN);
    let m = module_of(&w.svm, 0);
    assert_eq!(m.sponsor_allocation, 5);
    assert_eq!(m.school_allocation, 5);
}

#[test]
fn close_sponsorship_blocked_while_booking_active() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    let school = with_role(&mut w, Role::ModuleBooker);

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    // Sponsor exactly one token, then book it: amount hits 0 but the booking is
    // still cancellable, so the escrow must not be closeable yet.
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 1), &sponsor, &[&sponsor]);
    ok(&mut w.svm, book_ix(&school.pubkey(), 0, 0, 0), &school, &[&school]);

    let sp = Sponsorship::try_deserialize(
        &mut &w.svm.get_account(&sponsorship_pda(0, 0)).unwrap().data[..],
    )
    .unwrap();
    assert_eq!(sp.amount, 0);
    assert_eq!(sp.active_bookings, 1);

    err(
        &mut w.svm,
        close_sponsorship_ix(&sponsor.pubkey(), 0, 0),
        &sponsor,
        &[&sponsor],
        "SponsorshipNotEmpty",
    );
    // The escrow is still alive to refund a cancellation into.
    assert!(w.svm.get_account(&sponsor_escrow_pda(0, 0)).map(|a| !a.data.is_empty()).unwrap_or(false));
}

#[test]
fn close_sponsorship_after_reclaim_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);

    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 1), &sponsor, &[&sponsor]);

    // Nothing booked: reclaim after the window empties the sponsorship.
    warp_past_voting(&mut w.svm);
    ok(&mut w.svm, reclaim_sponsorship_ix(&sponsor.pubkey(), 0, 0, 1), &sponsor, &[&sponsor]);

    let rent_before = w.svm.get_account(&sponsor.pubkey()).map(|a| a.lamports).unwrap_or(0);
    ok(&mut w.svm, close_sponsorship_ix(&sponsor.pubkey(), 0, 0), &sponsor, &[&sponsor]);

    // Both the record and the escrow are gone, and their rent came back.
    assert!(w.svm.get_account(&sponsorship_pda(0, 0)).map(|a| a.data.is_empty()).unwrap_or(true));
    assert!(w.svm.get_account(&sponsor_escrow_pda(0, 0)).map(|a| a.data.is_empty()).unwrap_or(true));
    assert!(w.svm.get_account(&sponsor.pubkey()).map(|a| a.lamports).unwrap_or(0) > rent_before);
}

#[test]
fn create_module_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);

    // Zero tokens.
    err(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 0), &creator, &[&creator], "AmountCannotBeZero");
    // More than the configured maximum.
    err(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 1_001), &creator, &[&creator], "TooManyTokens");
}

#[test]
fn sponsor_module_multi_asset_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);

    // First sponsorship in USDC (6 decimals).
    let usdc_before = balance(&w.svm, &usdc_mint(), &sponsor.pubkey());
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 4), &sponsor, &[&sponsor]);

    let sp0 = sponsorship_of(&w.svm, 0, 0);
    assert_eq!(sp0.amount, 4);
    assert_eq!(sp0.payment_asset, usdc_mint());
    assert_eq!(sp0.price_per_token, PER_TOKEN);
    assert_eq!(usdc_before - balance(&w.svm, &usdc_mint(), &sponsor.pubkey()), 4 * PER_TOKEN);
    assert_eq!(spl_amount(&w.svm, &sponsor_escrow_pda(0, 0)), 4 * PER_TOKEN);

    // Second sponsorship on the same module in GBP (2 decimals).
    let gbp_before = balance(&w.svm, &gbp_mint(), &sponsor.pubkey());
    ok(&mut w.svm, sponsor_asset_ix(&sponsor.pubkey(), 0, 1, 3, &gbp_mint()), &sponsor, &[&sponsor]);

    let sp1 = sponsorship_of(&w.svm, 0, 1);
    assert_eq!(sp1.amount, 3);
    assert_eq!(sp1.payment_asset, gbp_mint());
    assert_eq!(sp1.price_per_token, PER_TOKEN_GBP);
    assert_eq!(gbp_before - balance(&w.svm, &gbp_mint(), &sponsor.pubkey()), 3 * PER_TOKEN_GBP);
    assert_eq!(spl_amount(&w.svm, &sponsor_escrow_pda(0, 1)), 3 * PER_TOKEN_GBP);

    // Module allocations reflect both sponsorships (4 + 3 = 7 sponsored).
    let m = module_of(&w.svm, 0);
    assert_eq!(m.sponsor_allocation, 3); // 10 - 4 - 3
    assert_eq!(m.school_allocation, 7);  // 4 + 3
}

#[test]
fn sponsor_module_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);

    // Zero tokens.
    err(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 0), &sponsor, &[&sponsor], "AmountCannotBeZero");
    // More than the module's available allocation.
    err(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 11), &sponsor, &[&sponsor], "NotEnoughTokenAvailable");
    // An asset that isn't accepted (XCAV is the stake mint, not a payment asset).
    err(
        &mut w.svm,
        sponsor_asset_ix(&sponsor.pubkey(), 0, 0, 1, &xcav_mint()),
        &sponsor,
        &[&sponsor],
        "PaymentAssetNotSupported",
    );
}

#[test]
fn reclaim_sponsorship_before_window_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 2), &sponsor, &[&sponsor]);

    // No warp: the sponsorship window has not elapsed yet.
    err(
        &mut w.svm,
        reclaim_sponsorship_ix(&sponsor.pubkey(), 0, 0, 1),
        &sponsor,
        &[&sponsor],
        "SponsorshipWindowNotExpired",
    );
}

#[test]
fn burn_unsponsored_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);

    ok(&mut w.svm, burn_ix(&creator.pubkey(), 0, 4), &creator, &[&creator]);
    let m = module_of(&w.svm, 0);
    assert_eq!(m.sponsor_allocation, 6);
    // total_token_amount records the original supply and is unaffected by burns.
    assert_eq!(m.total_token_amount, 10);
    assert_eq!(spl_amount(&w.svm, &module_vault_pda(0)), 6);
}

#[test]
fn burn_unsponsored_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    let sponsor = with_role(&mut w, Role::ModuleSponsor);
    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);
    // Sponsor 8, leaving only 2 unsponsored.
    ok(&mut w.svm, sponsor_ix(&sponsor.pubkey(), 0, 0, 8), &sponsor, &[&sponsor]);

    err(&mut w.svm, burn_ix(&creator.pubkey(), 0, 0), &creator, &[&creator], "AmountCannotBeZero");
    err(&mut w.svm, burn_ix(&creator.pubkey(), 0, 3), &creator, &[&creator], "CannotBurnMoreThanAvailable");
}

#[test]
fn remove_module_works() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);

    // Burn the whole supply so the module vault is empty, then remove it.
    ok(&mut w.svm, burn_ix(&creator.pubkey(), 0, 10), &creator, &[&creator]);
    let before = balance(&w.svm, &xcav_mint(), &creator.pubkey());
    ok(&mut w.svm, remove_module_ix(&creator.pubkey(), 0), &creator, &[&creator]);

    // Deposit refunded, module and its vault closed.
    assert_eq!(balance(&w.svm, &xcav_mint(), &creator.pubkey()) - before, MODULE_DEPOSIT);
    assert!(closed(&w.svm, &module_pda(0)));
    assert!(closed(&w.svm, &module_vault_pda(0)));
}

#[test]
fn remove_module_with_tokens_fails() {
    let mut w = setup();
    let creator = with_role(&mut w, Role::ModuleCreator);
    ok(&mut w.svm, create_module_ix(&creator.pubkey(), 1, 0, 10), &creator, &[&creator]);

    // The supply is still in the module vault.
    err(
        &mut w.svm,
        remove_module_ix(&creator.pubkey(), 0),
        &creator,
        &[&creator],
        "CannotRemoveModuleWithActiveTokens",
    );
}

