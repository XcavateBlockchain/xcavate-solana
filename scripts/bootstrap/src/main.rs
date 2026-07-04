//! Bring the deployed protocol to life on a cluster: create the mints,
//! initialize the three programs in order (roles, regions, real-x), add the
//! admin, and grant roles to funded demo wallets. With `--with-region` it also
//! drives the governance flow to create a demo region (adds ~50s of waits).
//! Localnet by default, `--cluster devnet` for devnet.
//!
//! The signing wallet must be the programs' upgrade authority, since each
//! `initialize_config` is bound to it. Programs must already be deployed
//! (`scripts/deploy.sh`). Run:
//! `cargo run --manifest-path scripts/bootstrap/Cargo.toml -- --cluster localnet`.

use std::{thread, time::Duration};

use anchor_lang::solana_program::{
    bpf_loader_upgradeable, instruction::Instruction, program_pack::Pack, pubkey::Pubkey,
    system_instruction, system_program,
};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::associated_token::spl_associated_token_account::{
    address::get_associated_token_address, instruction::create_associated_token_account_idempotent,
};
use anchor_spl::token::spl_token;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_keypair::{read_keypair_file, Keypair};
use solana_signer::Signer;
use solana_transaction::Transaction;

use xcavate_roles::state::Role;

const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

// Mirror the production mints' decimals (GBP = tGBP's Solana decimals).
const XCAV_DECIMALS: u8 = 9;
const USDC_DECIMALS: u8 = 6;
const GBP_DECIMALS: u8 = 9;

/// Governance windows are kept short (seconds) so the demo region can be driven
/// end to end while the script waits; production values would be far larger.
const GOV_WINDOW_SECS: i64 = 120;

/// Smallest-unit multipliers, so the amounts below read as whole tokens.
const XCAV: u64 = 1_000_000_000; // 1 XCAV (9 decimals)
const USDC: u64 = 1_000_000; // 1 USDC (6 decimals)

/// Per demo wallet: a little SOL for fees, and enough XCAV / USDC to stake in
/// governance and sponsor modules.
const SOL_PER_WALLET: u64 = LAMPORTS_PER_SOL / 5;
const XCAV_GRANT: u64 = 1_000_000 * XCAV; // 1,000,000 XCAV
const USDC_GRANT: u64 = 1_000_000 * USDC; // 1,000,000 USDC

struct Cx {
    rpc: RpcClient,
    payer: Keypair,
}

fn pda(seeds: &[&[u8]], program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(seeds, program).0
}

fn program_data(program_id: &Pubkey) -> Pubkey {
    pda(&[program_id.as_ref()], &bpf_loader_upgradeable::ID)
}

fn role_pda(user: &Pubkey, role: Role) -> Pubkey {
    pda(
        &[xcavate_roles::ROLE_SEED, user.as_ref(), &[role.seed_byte()]],
        &xcavate_roles::ID,
    )
}

fn ata(mint: &Pubkey, owner: &Pubkey) -> Pubkey {
    get_associated_token_address(owner, mint)
}

/// Keypair from `keys/<name>.json` next to this crate, generated and saved on
/// first use so demo identities and mints keep the same address across runs
/// (import them into a wallet once). The directory is gitignored: dev-only
/// secrets, never for real funds.
fn stable_keypair(name: &str) -> Keypair {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/keys");
    let path = format!("{dir}/{name}.json");
    if std::path::Path::new(&path).exists() {
        // Never regenerate over an existing file: a parse error should be
        // fixed (or the file deleted to rotate), not silently replaced.
        return read_keypair_file(&path)
            .unwrap_or_else(|e| panic!("unreadable keypair {path}: {e}"));
    }
    let kp = Keypair::new();
    std::fs::create_dir_all(dir).expect("create keys dir");
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .expect("create keypair file");
    file.write_all(
        serde_json::to_string(&kp.to_bytes().to_vec())
            .unwrap()
            .as_bytes(),
    )
    .expect("write keypair");
    kp
}

impl Cx {
    fn send(&self, ixs: &[Instruction], signers: &[&Keypair]) {
        let blockhash = self.rpc.get_latest_blockhash().expect("blockhash");
        let mut all = vec![&self.payer];
        all.extend_from_slice(signers);
        let tx =
            Transaction::new_signed_with_payer(ixs, Some(&self.payer.pubkey()), &all, blockhash);
        self.rpc
            .send_and_confirm_transaction(&tx)
            .expect("transaction failed");
    }

    fn airdrop(&self, target: &Pubkey, lamports: u64) {
        let sig = self.rpc.request_airdrop(target, lamports).expect("airdrop");
        for _ in 0..40 {
            if self.rpc.confirm_transaction(&sig).unwrap_or(false) {
                return;
            }
            thread::sleep(Duration::from_millis(500));
        }
        panic!("airdrop not confirmed");
    }

    fn create_mint(&self, name: &str, decimals: u8) -> Pubkey {
        let mint = stable_keypair(name);
        let rent = self
            .rpc
            .get_minimum_balance_for_rent_exemption(spl_token::state::Mint::LEN)
            .expect("rent");
        let ixs = [
            system_instruction::create_account(
                &self.payer.pubkey(),
                &mint.pubkey(),
                rent,
                spl_token::state::Mint::LEN as u64,
                &spl_token::id(),
            ),
            spl_token::instruction::initialize_mint2(
                &spl_token::id(),
                &mint.pubkey(),
                &self.payer.pubkey(),
                None,
                decimals,
            )
            .unwrap(),
        ];
        self.send(&ixs, &[&mint]);
        mint.pubkey()
    }

    /// Instructions to create `owner`'s ATA (idempotent) and mint `amount` to it.
    fn mint_ixs(&self, mint: &Pubkey, owner: &Pubkey, amount: u64) -> Vec<Instruction> {
        vec![
            create_associated_token_account_idempotent(
                &self.payer.pubkey(),
                owner,
                mint,
                &spl_token::id(),
            ),
            spl_token::instruction::mint_to(
                &spl_token::id(),
                mint,
                &ata(mint, owner),
                &self.payer.pubkey(),
                &[],
                amount,
            )
            .unwrap(),
        ]
    }
}

fn regions_params() -> education_regions::instructions::ConfigParams {
    education_regions::instructions::ConfigParams {
        minimum_voting_amount: 100 * XCAV,
        voting_period: GOV_WINDOW_SECS,
        owner_change_period: 60,
        threshold_bps: 5_000,
        quorum: 50_000 * XCAV,
        removal_deposit: 1_000 * XCAV,
        removal_voting_period: GOV_WINDOW_SECS,
        slash_amount: 5_000 * XCAV,
        notice_period: 30,
        allowed_strikes: 3,
    }
}

fn realx_params(usdc: Pubkey, gbp: Pubkey) -> real_x_education::instructions::ConfigParams {
    real_x_education::instructions::ConfigParams {
        module_deposit: 1_000 * XCAV,
        booking_deposit: 100 * XCAV,
        deliverer_deposit: 500 * XCAV,
        module_price: 100, // whole currency units per module token
        max_module_tokens: 1_000,
        content_creator_bps: 830,
        regional_operator_bps: 830,
        protocol_bps: 500,
        dbs_bps: 340,
        min_impact_score_bps: 5_000,
        sponsorship_window: 3_600,
        cancellation_window: 3_600,
        no_show_grace: 3_600,
        max_cancellations: 3,
        max_strikes: 3,
        strike_slash_bps: 1_000,
        deliveries_per_strike_reduction: 5,
        proposal_deposit: 1_000 * XCAV,
        minimum_voting_amount: 100 * XCAV,
        voting_period: GOV_WINDOW_SECS,
        threshold_bps: 5_000,
        quorum: 50_000 * XCAV,
        pre_sponsor_amount: 2,
        claim_period: 3_600,
        upload_period: 3_600,
        accepted_assets: [usdc, gbp, Pubkey::default()],
    }
}

fn main() {
    // --- args ---
    let mut cluster = "localnet".to_string();
    let mut keypair_path = shellexpand("~/.config/solana/id.json");
    let mut with_region = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cluster" => cluster = args.next().expect("--cluster needs a value"),
            "--keypair" => keypair_path = args.next().expect("--keypair needs a path"),
            "--with-region" => with_region = true,
            other => panic!("unknown argument: {other}"),
        }
    }
    let (url, localnet) = match cluster.as_str() {
        "localnet" => ("http://127.0.0.1:8899".to_string(), true),
        "devnet" => ("https://api.devnet.solana.com".to_string(), false),
        other => panic!("unknown cluster: {other} (use localnet or devnet)"),
    };

    let payer = read_keypair_file(&keypair_path).expect("could not read keypair");
    // Confirmed (not the default finalized) so each send returns in ~a second.
    let rpc = RpcClient::new_with_commitment(url.clone(), CommitmentConfig::confirmed());
    let cx = Cx { rpc, payer };

    println!("cluster:   {cluster} ({url})");
    println!("authority: {}", cx.payer.pubkey());

    // On localnet the authority likely starts empty; top it up.
    if localnet {
        cx.airdrop(&cx.payer.pubkey(), 500 * LAMPORTS_PER_SOL);
    }

    // --- mints ---
    let xcav = cx.create_mint("mint-xcav", XCAV_DECIMALS);
    let usdc = cx.create_mint("mint-usdc", USDC_DECIMALS);
    let gbp = cx.create_mint("mint-gbp", GBP_DECIMALS);
    println!("xcav mint: {xcav}");
    println!("usdc mint: {usdc}");
    println!("gbp mint:  {gbp}");

    // --- fixed PDAs ---
    let roles_config = pda(&[xcavate_roles::CONFIG_SEED], &xcavate_roles::ID);
    let regions_config = pda(&[education_regions::CONFIG_SEED], &education_regions::ID);
    let treasury = pda(&[education_regions::TREASURY_SEED], &education_regions::ID);
    let regions_vault = pda(&[education_regions::VAULT_SEED], &education_regions::ID);
    let realx_config = pda(&[real_x_education::CONFIG_SEED], &real_x_education::ID);
    let realx_vault = pda(&[real_x_education::VAULT_SEED], &real_x_education::ID);

    // --- 1) roles ---
    cx.send(
        &[Instruction::new_with_bytes(
            xcavate_roles::ID,
            &xcavate_roles::instruction::InitializeConfig {}.data(),
            xcavate_roles::accounts::InitializeConfig {
                authority: cx.payer.pubkey(),
                program: xcavate_roles::ID,
                program_data: program_data(&xcavate_roles::ID),
                config: roles_config,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        )],
        &[],
    );
    println!("roles initialized");

    // Make the authority an admin so it can grant roles.
    cx.send(
        &[Instruction::new_with_bytes(
            xcavate_roles::ID,
            &xcavate_roles::instruction::AddAdmin {}.data(),
            xcavate_roles::accounts::AddAdmin {
                authority: cx.payer.pubkey(),
                config: roles_config,
                new_admin: cx.payer.pubkey(),
                admin: pda(
                    &[xcavate_roles::ADMIN_SEED, cx.payer.pubkey().as_ref()],
                    &xcavate_roles::ID,
                ),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        )],
        &[],
    );

    // --- 2) regions (creates the shared treasury + vault) ---
    cx.send(
        &[Instruction::new_with_bytes(
            education_regions::ID,
            &education_regions::instruction::InitializeConfig {
                params: regions_params(),
            }
            .data(),
            education_regions::accounts::InitializeConfig {
                authority: cx.payer.pubkey(),
                program: education_regions::ID,
                program_data: program_data(&education_regions::ID),
                config: regions_config,
                xcav_mint: xcav,
                treasury,
                vault: regions_vault,
                token_program: spl_token::id(),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        )],
        &[],
    );
    println!("regions initialized (treasury {treasury})");

    // --- 3) real-x (pins the regions treasury) ---
    cx.send(
        &[Instruction::new_with_bytes(
            real_x_education::ID,
            &real_x_education::instruction::InitializeConfig {
                params: realx_params(usdc, gbp),
            }
            .data(),
            real_x_education::accounts::InitializeConfig {
                authority: cx.payer.pubkey(),
                program: real_x_education::ID,
                program_data: program_data(&real_x_education::ID),
                config: realx_config,
                xcav_mint: xcav,
                treasury,
                protocol_authority: cx.payer.pubkey(),
                vault: realx_vault,
                token_program: spl_token::id(),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        )],
        &[],
    );
    println!("real-x initialized");

    // --- demo wallets + roles ---
    let roles = [
        ("operator", Role::RegionalOperator),
        ("creator", Role::ModuleCreator),
        ("sponsor", Role::ModuleSponsor),
        ("school", Role::ModuleBooker),
        ("lecturer", Role::ModuleDeliverer),
        ("agent", Role::ModuleAIAgent),
    ];
    let mut wallets: Vec<(&str, Keypair, Role)> = Vec::new();
    for (name, role) in roles {
        let kp = stable_keypair(name);
        // Fund, grant the role, and stock XCAV (for staking) + USDC (for
        // payments) in a single transaction per wallet.
        let mut ixs = vec![
            system_instruction::transfer(&cx.payer.pubkey(), &kp.pubkey(), SOL_PER_WALLET),
            Instruction::new_with_bytes(
                xcavate_roles::ID,
                &xcavate_roles::instruction::AssignRole { role }.data(),
                xcavate_roles::accounts::AssignRole {
                    admin_signer: cx.payer.pubkey(),
                    admin: pda(
                        &[xcavate_roles::ADMIN_SEED, cx.payer.pubkey().as_ref()],
                        &xcavate_roles::ID,
                    ),
                    user: kp.pubkey(),
                    role_account: role_pda(&kp.pubkey(), role),
                    system_program: system_program::ID,
                }
                .to_account_metas(None),
            ),
        ];
        ixs.extend(cx.mint_ixs(&xcav, &kp.pubkey(), XCAV_GRANT));
        ixs.extend(cx.mint_ixs(&usdc, &kp.pubkey(), USDC_GRANT));
        cx.send(&ixs, &[]);
        println!("{name}: {} ({role:?})", kp.pubkey());
        wallets.push((name, kp, role));
    }
    let operator = &wallets[0].1;

    // --- governed region 1 (optional) ---
    if with_region {
        seed_region(
            &cx,
            operator,
            xcav,
            treasury,
            regions_config,
            regions_vault,
            1,
        );
    } else {
        println!("skipped region seed (pass --with-region to create one)");
    }

    // --- addresses.json ---
    let out = serde_json::json!({
        "cluster": cluster,
        "authority": cx.payer.pubkey().to_string(),
        "region_seeded": with_region,
        "programs": {
            "xcavate_roles": xcavate_roles::ID.to_string(),
            "education_regions": education_regions::ID.to_string(),
            "real_x_education": real_x_education::ID.to_string(),
        },
        "pdas": {
            "roles_config": roles_config.to_string(),
            "regions_config": regions_config.to_string(),
            "treasury": treasury.to_string(),
            "regions_vault": regions_vault.to_string(),
            "realx_config": realx_config.to_string(),
            "realx_vault": realx_vault.to_string(),
            "region_1": pda(&[education_regions::REGION_SEED, &1u16.to_le_bytes()], &education_regions::ID).to_string(),
        },
        "mints": { "xcav": xcav.to_string(), "usdc": usdc.to_string(), "gbp": gbp.to_string() },
        "wallets": wallets.iter().map(|(name, kp, role)| {
            serde_json::json!({
                "name": name,
                "role": format!("{role:?}"),
                "pubkey": kp.pubkey().to_string(),
                "secret": kp.to_bytes().to_vec(),
            })
        }).collect::<Vec<_>>(),
    });
    // Written next to the crate (regardless of the caller's cwd) so the
    // secret-key-bearing file always lands in the gitignored location.
    let out_path = concat!(env!("CARGO_MANIFEST_DIR"), "/addresses.json");
    std::fs::write(out_path, serde_json::to_string_pretty(&out).unwrap()).expect("write");
    println!("\nwrote {out_path} (demo wallet secret keys included for frontend import)");
}

/// Drive the regions governance flow so region `region_id` exists and is owned
/// by `operator`: propose, self-vote, wait the voting window, finalize into an
/// auction, bid, wait the auction window, and create it.
fn seed_region(
    cx: &Cx,
    operator: &Keypair,
    xcav: Pubkey,
    treasury: Pubkey,
    regions_config: Pubkey,
    regions_vault: Pubkey,
    region_id: u16,
) {
    let rid = education_regions::ID;
    let proposal_id: u64 = 0;
    let region = pda(
        &[education_regions::REGION_SEED, &region_id.to_le_bytes()],
        &rid,
    );
    let region_state = pda(
        &[
            education_regions::REGION_STATE_SEED,
            &region_id.to_le_bytes(),
        ],
        &rid,
    );
    let proposal = pda(
        &[education_regions::PROPOSAL_SEED, &proposal_id.to_le_bytes()],
        &rid,
    );
    let vote_record = pda(
        &[
            education_regions::VOTE_SEED,
            &proposal_id.to_le_bytes(),
            operator.pubkey().as_ref(),
        ],
        &rid,
    );
    let op_xcav = ata(&xcav, &operator.pubkey());
    let op_role = role_pda(&operator.pubkey(), Role::RegionalOperator);

    // Propose.
    cx.send(
        &[Instruction::new_with_bytes(
            rid,
            &education_regions::instruction::ProposeNewRegion { region_id }.data(),
            education_regions::accounts::ProposeNewRegion {
                proposer: operator.pubkey(),
                config: regions_config,
                xcav_mint: xcav,
                proposer_token: op_xcav,
                vault: regions_vault,
                operator_role: op_role,
                region,
                region_state,
                proposal,
                token_program: spl_token::id(),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        )],
        &[operator],
    );

    // Self-vote past quorum.
    cx.send(
        &[Instruction::new_with_bytes(
            rid,
            &education_regions::instruction::VoteOnRegionProposal {
                region_id,
                vote: education_regions::state::Vote::Yes,
                amount: 60_000 * XCAV, // above the 50,000 quorum
            }
            .data(),
            education_regions::accounts::VoteOnRegionProposal {
                voter: operator.pubkey(),
                config: regions_config,
                xcav_mint: xcav,
                voter_token: op_xcav,
                vault: regions_vault,
                region_state,
                proposal,
                vote_record,
                token_program: spl_token::id(),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        )],
        &[operator],
    );

    println!("region {region_id}: proposed and voted, waiting out the voting window...");
    thread::sleep(Duration::from_secs(GOV_WINDOW_SECS as u64 + 5));

    // Finalize into an auction (permissionless; the payer cranks).
    cx.send(
        &[Instruction::new_with_bytes(
            rid,
            &education_regions::instruction::FinalizeRegionProposal { region_id }.data(),
            education_regions::accounts::FinalizeRegionProposal {
                cranker: cx.payer.pubkey(),
                config: regions_config,
                xcav_mint: xcav,
                vault: regions_vault,
                region_state,
                proposal,
                proposer: operator.pubkey(),
                proposer_token: Some(op_xcav),
                treasury,
                token_program: spl_token::id(),
            }
            .to_account_metas(None),
        )],
        &[],
    );

    // Bid.
    cx.send(
        &[Instruction::new_with_bytes(
            rid,
            &education_regions::instruction::BidOnRegion {
                region_id,
                amount: 12_000 * XCAV,
            }
            .data(),
            education_regions::accounts::BidOnRegion {
                bidder: operator.pubkey(),
                config: regions_config,
                operator_role: op_role,
                xcav_mint: xcav,
                bidder_token: op_xcav,
                vault: regions_vault,
                region_state,
                previous_bidder_token: None,
                token_program: spl_token::id(),
            }
            .to_account_metas(None),
        )],
        &[operator],
    );

    println!("region {region_id}: bid placed, waiting out the auction window...");
    thread::sleep(Duration::from_secs(GOV_WINDOW_SECS as u64 + 5));

    // Create the region.
    cx.send(
        &[Instruction::new_with_bytes(
            rid,
            &education_regions::instruction::CreateNewRegion { region_id }.data(),
            education_regions::accounts::CreateNewRegion {
                creator: operator.pubkey(),
                creator_role: op_role,
                config: regions_config,
                region_state,
                region,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        )],
        &[operator],
    );
    println!(
        "region {region_id}: created, owned by {}",
        operator.pubkey()
    );
}

/// Minimal `~` expansion for the default keypair path.
fn shellexpand(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}
