# scripts

Dev tooling to deploy the three programs and bring up on-chain state so the
protocol can be driven from a frontend.

| script | what it does |
| --- | --- |
| `localnet.sh {up\|down\|status}` | start / stop / check a local validator |
| `deploy.sh` | build and deploy the three programs to a cluster |
| `bootstrap/` | init the configs + treasury, add the admin, grant roles to funded demo wallets; `--with-region` also seeds a governed region |

## Local loop

```bash
scripts/localnet.sh up                                   # terminal 1: fresh validator (stays running)

scripts/deploy.sh --cluster localnet                     # terminal 2: build + deploy
cargo run --manifest-path scripts/bootstrap/Cargo.toml -- --cluster localnet [--with-region]

# ... test from a frontend using scripts/bootstrap/addresses.json + target/idl/*.json ...

scripts/localnet.sh down                                 # stop + wipe everything
```

The deploy wallet (`~/.config/solana/id.json`) becomes each program's upgrade
authority, and `bootstrap` must run as that same wallet, since every
`initialize_config` is bound to the upgrade authority.

`bootstrap` writes `scripts/bootstrap/addresses.json` with every program ID,
PDA, mint, and demo wallet — **including the demo wallets' secret keys** so they
can be imported into a frontend. That file is gitignored; do not commit it.

Demo wallet and mint keypairs are persisted in `scripts/bootstrap/keys/`
(one JSON file per identity, generated on first run), so the same addresses
come back on every bootstrap — import them into a wallet once and they survive
validator resets. The directory is gitignored like `addresses.json`: localnet
throwaways only, never for real funds. Delete a file (or the directory) to
rotate identities.

## Devnet

```bash
solana config set --url devnet && solana airdrop 2       # fund the deploy wallet
scripts/deploy.sh --cluster devnet
cargo run --manifest-path scripts/bootstrap/Cargo.toml -- --cluster devnet [--with-region]
```

There is no teardown on devnet: deployed programs persist. `solana program
close <program-id>` reclaims rent if you want to remove one.

## Notes

- **One-shot per fresh cluster.** `bootstrap` initializes singleton configs, so
  re-running against an already-initialized cluster fails with "account already
  in use". Reset the validator (`localnet.sh down` then `up`) between runs.
- **`--with-region` takes ~1 min**: it drives the real governance flow (propose,
  vote, finalize, bid, create) and waits out the voting/auction windows. Without
  the flag it finishes in a few seconds but leaves no region, so module creation
  can't be exercised until one exists.
- **Short ledger path.** `localnet.sh` uses `/tmp/xtl`; a long ledger path trips
  the validator's Unix-socket limit. Override with `SOLANA_LEDGER`.
