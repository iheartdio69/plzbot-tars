#!/usr/bin/env bash
set -euo pipefail

DB="${SQLITE_PATH:-./data/solana_meme.sqlite}"
MINT="${1:-}"

if [[ -z "$MINT" ]]; then
  echo "usage: inspect_runner.sh <mint>"
  exit 1
fi

sqlite3 "$DB" -cmd ".headers on" -cmd ".mode column" "
SELECT ts, fdv_usd, tx_5m, signers_5m, score, is_active, is_call
FROM mint_snapshots
WHERE mint='$MINT'
ORDER BY ts ASC;
"
