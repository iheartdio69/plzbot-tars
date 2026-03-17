#!/usr/bin/env bash
set -euo pipefail

DB="${SQLITE_PATH:-./data/solana_meme.sqlite}"

echo "Using DB: $DB"
sqlite3 "$DB" "SELECT COUNT(*) AS mint_snapshots FROM mint_snapshots;"
sqlite3 "$DB" "SELECT COUNT(*) AS wallet_edges FROM wallet_edges;"
sqlite3 "$DB" "SELECT COUNT(*) AS wallets FROM wallets;"
echo "Recent wallet_edges:"
sqlite3 "$DB" -cmd ".mode column" -cmd ".headers on" \
  "SELECT ts, substr(src_wallet,1,6) AS src, substr(dst_wallet,1,6) AS dst, action, sol, substr(sig,1,8) AS sig
   FROM wallet_edges
   ORDER BY ts DESC
   LIMIT 10;"
