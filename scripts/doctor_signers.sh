#!/usr/bin/env bash
set -euo pipefail

DB="${SQLITE_PATH:-./data/solana_meme.sqlite}"
echo "=== doctor_signers ==="
echo "SQLITE_PATH=$DB"
echo "HELIUS_API_KEY=${HELIUS_API_KEY:+(set)}${HELIUS_API_KEY:- (NOT SET)}"
echo "HELIUS_RPC_URL=${HELIUS_RPC_URL:- (NOT SET)}"
echo "HELIUS_ADDR_URL=${HELIUS_ADDR_URL:- (NOT SET)}"
echo "HELIUS_WALLETS=${HELIUS_WALLETS:- (NOT SET)}"
echo

if [[ ! -f "$DB" ]]; then
  echo "❌ DB file not found: $DB"
  exit 1
fi

echo "DB tables:"
sqlite3 "$DB" ".tables"
echo

echo "Counts:"
sqlite3 "$DB" "SELECT COUNT(*) AS mint_snapshots FROM mint_snapshots;"
sqlite3 "$DB" "SELECT COUNT(*) AS wallet_edges FROM wallet_edges;"
sqlite3 "$DB" "SELECT COUNT(*) AS wallets FROM wallets;"
echo

echo "Recent wallet_edges (if any):"
sqlite3 "$DB" -cmd ".headers on" -cmd ".mode column" \
  "SELECT ts, substr(src_wallet,1,6) AS src, substr(dst_wallet,1,6) AS dst, action, sol, substr(sig,1,8) AS sig, substr(mint,1,8) AS mint
   FROM wallet_edges
   ORDER BY ts DESC
   LIMIT 8;" || true
echo

# Warn about the #1 reason signers stay 0
if [[ -z "${HELIUS_WALLETS:-}" ]]; then
  echo "⚠️ HELIUS_WALLETS is empty -> ingest won't run -> signers_5m stays 0."
fi

if [[ -z "${HELIUS_API_KEY:-}" ]]; then
  echo "⚠️ HELIUS_API_KEY is empty -> ingest can't fetch txs."
fi
