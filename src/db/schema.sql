PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;

-- =========================
-- Mint snapshots + calls
-- =========================
CREATE TABLE IF NOT EXISTS mint_snapshots (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ts            INTEGER NOT NULL,
  mint          TEXT NOT NULL,
  fdv_usd       REAL,
  tx_5m         INTEGER,
  score         INTEGER,
  signers_5m    INTEGER,
  events        INTEGER,
  first_seen    INTEGER,
  is_active     INTEGER DEFAULT 0,
  is_call       INTEGER DEFAULT 0
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_mint_snapshots_mint_ts ON mint_snapshots(mint, ts);
CREATE INDEX IF NOT EXISTS idx_mint_snapshots_ts ON mint_snapshots(ts);

CREATE TABLE IF NOT EXISTS calls (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  ts         INTEGER NOT NULL,
  mint       TEXT NOT NULL,
  fdv_usd    REAL,
  score      INTEGER,
  tx_5m      INTEGER,
  signers_5m INTEGER,
  events     INTEGER
);
CREATE INDEX IF NOT EXISTS idx_calls_mint_ts ON calls(mint, ts);
CREATE INDEX IF NOT EXISTS idx_calls_ts ON calls(ts);

-- =========================
-- Wallet learning tables
-- =========================

-- Wallet reputation store
CREATE TABLE IF NOT EXISTS wallets (
  wallet        TEXT PRIMARY KEY,
  score         INTEGER NOT NULL DEFAULT 0,
  last_seen_ts  INTEGER,
  notes         TEXT
);
CREATE INDEX IF NOT EXISTS idx_wallets_last_seen_ts ON wallets(last_seen_ts);

-- Edge list of on-chain interactions
-- src_wallet -> dst_wallet, tagged by action/kind, and optionally mint
CREATE TABLE IF NOT EXISTS wallet_edges (
  ts         INTEGER NOT NULL,
  src_wallet TEXT NOT NULL,
  dst_wallet TEXT,
  mint       TEXT,
  action     TEXT,     -- "fee_payer" / "buy" / "sell" / "transfer" / "create" etc
  sol        REAL,
  sig        TEXT
);

-- Fast lookups
CREATE INDEX IF NOT EXISTS idx_wallet_edges_src_ts ON wallet_edges(src_wallet, ts);
CREATE INDEX IF NOT EXISTS idx_wallet_edges_dst_ts ON wallet_edges(dst_wallet, ts);
CREATE INDEX IF NOT EXISTS idx_wallet_edges_ts ON wallet_edges(ts);
CREATE INDEX IF NOT EXISTS idx_wallet_edges_sig ON wallet_edges(sig);
CREATE INDEX IF NOT EXISTS idx_wallet_edges_mint_ts ON wallet_edges(mint, ts);

-- =========================
-- Dedupe: prevents re-processing the same tx forever
-- =========================
CREATE TABLE IF NOT EXISTS seen_sigs (
  sig TEXT PRIMARY KEY,
  ts  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_seen_sigs_ts ON seen_sigs(ts);

-- =========================
-- Wallet scoring state (incremental)
-- =========================
CREATE TABLE IF NOT EXISTS wallet_scoring_state (
  id              INTEGER PRIMARY KEY CHECK (id = 1),
  last_scored_ts  INTEGER NOT NULL DEFAULT 0
);

INSERT OR IGNORE INTO wallet_scoring_state (id, last_scored_ts) VALUES (1, 0);

-- =========================
-- outcomes table
-- =========================
CREATE TABLE IF NOT EXISTS call_outcomes (
  mint TEXT NOT NULL,
  call_ts INTEGER NOT NULL,
  outcome_ts INTEGER NOT NULL,
  fdv_usd REAL,
  result TEXT NOT NULL, -- 'win' | 'loss'
  PRIMARY KEY (mint, call_ts)
);

CREATE TABLE IF NOT EXISTS call_top_wallets (
  mint TEXT NOT NULL,
  call_ts INTEGER NOT NULL,
  wallet TEXT NOT NULL,
  edges INTEGER NOT NULL,
  PRIMARY KEY (mint, call_ts, wallet)
);

-- Wallet fingerprint per CALL (mint, call_ts)
CREATE TABLE IF NOT EXISTS call_wallet_fingerprints (
  mint     TEXT NOT NULL,
  call_ts  INTEGER NOT NULL,
  wallet   TEXT NOT NULL,
  reason   TEXT NOT NULL, -- 'top_edges' | 'early' | 'winner'
  metric   REAL,          -- edges count, minutes_from_first_seen, win_rate, etc
  PRIMARY KEY (mint, call_ts, wallet, reason)
);
CREATE INDEX IF NOT EXISTS idx_cwf_wallet ON call_wallet_fingerprints(wallet);

-- Wallet watchlist we can export to Axiom later
CREATE TABLE IF NOT EXISTS watchlist_wallets (
  wallet       TEXT PRIMARY KEY,
  first_seen_ts INTEGER,
  last_seen_ts  INTEGER,
  tags          TEXT,     -- csv: "winner,early,top_edges"
  wins          INTEGER NOT NULL DEFAULT 0,
  losses        INTEGER NOT NULL DEFAULT 0
);