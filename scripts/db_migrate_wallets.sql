PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;

-- 1) WALLETS: new canonical table
CREATE TABLE IF NOT EXISTS wallets_new (
  wallet TEXT PRIMARY KEY,
  score INTEGER NOT NULL DEFAULT 0,
  last_seen_ts INTEGER,
  notes TEXT
);

-- Copy from old wallets (best-effort based on common column names)
-- If a column doesn't exist, insert NULL/defaults.
INSERT OR IGNORE INTO wallets_new(wallet, score, last_seen_ts, notes)
SELECT
  wallet,
  COALESCE(score, 0),
  COALESCE(last_seen_ts, last_seen, seen_ts, ts, NULL),
  COALESCE(notes, NULL)
FROM wallets;

DROP TABLE wallets;
ALTER TABLE wallets_new RENAME TO wallets;

-- 2) WALLET_EDGES: new canonical table
CREATE TABLE IF NOT EXISTS wallet_edges_new (
  ts INTEGER NOT NULL,
  src_wallet TEXT NOT NULL,
  dst_wallet TEXT,
  mint TEXT,
  action TEXT,
  sol REAL,
  sig TEXT
);

-- If your old wallet_edges doesn't match these column names,
-- we will adjust this INSERT after you paste `.schema wallet_edges`.
-- For now, attempt a common mapping:
INSERT INTO wallet_edges_new(ts, src_wallet, dst_wallet, mint, action, sol, sig)
SELECT
  COALESCE(ts, time, timestamp),
  COALESCE(src_wallet, src, from_wallet, from_addr, "from"),
  COALESCE(dst_wallet, dst, to_wallet, to_addr, "to"),
  COALESCE(mint, NULL),
  COALESCE(action, NULL),
  COALESCE(sol, amount_sol, NULL),
  COALESCE(sig, signature, NULL)
FROM wallet_edges;

DROP TABLE wallet_edges;
ALTER TABLE wallet_edges_new RENAME TO wallet_edges;

COMMIT;
PRAGMA foreign_keys=ON;
