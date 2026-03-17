#!/usr/bin/env bash
DB="${DB_PATH:-./data/solana_meme.sqlite}"

echo "---- wallets ----"
sqlite3 "$DB" "SELECT COUNT(*) FROM wallets;"

echo "---- wallet_edges ----"
sqlite3 "$DB" "SELECT COUNT(*) FROM wallet_edges;"

echo "---- recent edges ----"
sqlite3 "$DB" "
SELECT ts, src_wallet, mint, action, sol
FROM wallet_edges
ORDER BY ts DESC
LIMIT 10;
"
