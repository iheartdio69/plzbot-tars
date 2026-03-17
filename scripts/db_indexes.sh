#!/usr/bin/env bash
set -e

DB_PATH="${DB_PATH:-./data/solana_meme.sqlite}"

if [ ! -f "$DB_PATH" ]; then
  echo "❌ DB not found: $DB_PATH"
  exit 1
fi

echo "✅ Using DB: $DB_PATH"
sqlite3 "$DB_PATH" <<'SQL'
-- wallets
CREATE INDEX IF NOT EXISTS idx_wallets_last_seen
  ON wallets(last_seen_ts);

-- wallet_edges
CREATE INDEX IF NOT EXISTS idx_edges_ts
  ON wallet_edges(ts);

CREATE INDEX IF NOT EXISTS idx_edges_src
  ON wallet_edges(src_wallet);

CREATE INDEX IF NOT EXISTS idx_edges_dst
  ON wallet_edges(dst_wallet);

CREATE INDEX IF NOT EXISTS idx_edges_mint
  ON wallet_edges(mint);

-- calls
CREATE INDEX IF NOT EXISTS idx_calls_mint_ts
  ON calls(mint, ts);

-- mint_snapshots
CREATE INDEX IF NOT EXISTS idx_snapshots_mint_ts
  ON mint_snapshots(mint, ts);
SQL

echo "✅ Indexes applied"#!/usr/bin/env bash
set -euo pipefail

DB_PATH="${DB_PATH:-./data/solana_meme.sqlite}"

if [[ ! -f "$DB_PATH" ]]; then
  echo "DB not found at: $DB_PATH"
  echo "Set DB_PATH or create DB first."
  exit 1
fi

echo "Using DB: $DB_PATH"
sqlite3 "$DB_PATH" ".tables"

sqlite3 "$DB_PATH" <<'SQL'
PRAGMA foreign_keys=ON;

-- calls
CREATE INDEX IF NOT EXISTS idx_calls_ts      ON calls(ts);
CREATE INDEX IF NOT EXISTS idx_calls_mint_ts ON calls(mint, ts);

-- mint_snapshots
CREATE INDEX IF NOT EXISTS idx_snapshots_ts       ON mint_snapshots(ts);
CREATE INDEX IF NOT EXISTS idx_snapshots_mint_ts  ON mint_snapshots(mint, ts);

-- wallets
CREATE INDEX IF NOT EXISTS idx_wallets_last_seen_ts ON wallets(last_seen_ts);
CREATE INDEX IF NOT EXISTS idx_wallets_score        ON wallets(score);

-- wallet_edges
CREATE INDEX IF NOT EXISTS idx_wallet_edges_ts      ON wallet_edges(ts);
CREATE INDEX IF NOT EXISTS idx_wallet_edges_src     ON wallet_edges(src_wallet);
CREATE INDEX IF NOT EXISTS idx_wallet_edges_dst     ON wallet_edges(dst_wallet);
CREATE INDEX IF NOT EXISTS idx_wallet_edges_mint    ON wallet_edges(mint);
CREATE INDEX IF NOT EXISTS idx_wallet_edges_sig     ON wallet_edges(sig);
SQL

echo "✅ indexes ok"
