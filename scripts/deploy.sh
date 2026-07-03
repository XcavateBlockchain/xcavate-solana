#!/usr/bin/env bash
# Build and deploy the three programs to a cluster. The Solana CLI wallet that
# signs these deploys becomes each program's upgrade authority, which is the
# same wallet `scripts/bootstrap` must run as.
#
# Usage: scripts/deploy.sh [--cluster localnet|devnet] [--skip-build]
set -euo pipefail

CLUSTER="localnet"
SKIP_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --cluster) CLUSTER="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 1 ;;
  esac
done

case "$CLUSTER" in
  localnet) URL="http://127.0.0.1:8899" ;;
  devnet)   URL="https://api.devnet.solana.com" ;;
  *) echo "unknown cluster: $CLUSTER (use localnet or devnet)" >&2; exit 1 ;;
esac

cd "$(dirname "$0")/.."

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  anchor build
fi

for p in xcavate_roles education_regions real_x_education; do
  echo "deploying $p to $CLUSTER..."
  solana program deploy --url "$URL" \
    --program-id "target/deploy/${p}-keypair.json" \
    "target/deploy/${p}.so"
done

echo "deployed to $CLUSTER; bring the state up with:"
echo "  cargo run --manifest-path scripts/bootstrap/Cargo.toml -- --cluster $CLUSTER"
