#!/usr/bin/env bash
# Manage a local validator for development.
#
# Usage: scripts/localnet.sh {up|down|status}
#   up      start a fresh validator (foreground; Ctrl-C or `down` to stop)
#   down    stop the validator and wipe its ledger
#   status  report whether a validator is answering on localhost
#
# The ledger path is kept short on purpose: a long path trips the Unix-socket
# SUN_LEN limit and the validator's admin service fails to start. Override with
# SOLANA_LEDGER if needed.
set -euo pipefail

LEDGER="${SOLANA_LEDGER:-/tmp/xtl}"
URL="http://127.0.0.1:8899"

case "${1:-}" in
  up)
    rm -rf "$LEDGER"
    exec solana-test-validator --reset --ledger "$LEDGER"
    ;;
  down)
    if pkill -f solana-test-validator 2>/dev/null; then
      echo "validator stopped"
    else
      echo "no validator running"
    fi
    rm -rf "$LEDGER"
    echo "ledger wiped ($LEDGER)"
    ;;
  status)
    if solana cluster-version --url "$URL" >/dev/null 2>&1; then
      echo "up: $(solana cluster-version --url "$URL") (slot $(solana slot --url "$URL"))"
    else
      echo "down"
    fi
    ;;
  *)
    echo "usage: scripts/localnet.sh {up|down|status}" >&2
    exit 1
    ;;
esac
