PRAGMA journal_mode=WAL;

CREATE TABLE IF NOT EXISTS mints (
  mint TEXT PRIMARY KEY,
  first_seen INTEGER NOT NULL,
  last_seen INTEGER NOT NULL,
  symbol TEXT,
  name TEXT
);

CREATE TABLE IF NOT EXISTS mint_snapshots (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  mint TEXT NOT NULL,
  fdv REAL,
  tx_5m INTEGER,
  buys_5m INTEGER,
  sells_5m INTEGER,
  signers_5m INTEGER,
  events_count INTEGER,
  score INTEGER,
  active INTEGER,
  called INTEGER,
  FOREIGN KEY(mint) REFERENCES mints(mint)
);

CREATE INDEX IF NOT EXISTS idx_snapshots_mint_ts ON mint_snapshots(mint, ts);
CREATE INDEX IF NOT EXISTS idx_snapshots_ts ON mint_snapshots(ts);

CREATE TABLE IF NOT EXISTS calls (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  mint TEXT NOT NULL,
  fdv REAL,
  score INTEGER,
  tx_5m INTEGER,
  signers_5m INTEGER,
  events_count INTEGER,
  reason TEXT,
  FOREIGN KEY(mint) REFERENCES mints(mint)
);

CREATE INDEX IF NOT EXISTS idx_calls_ts ON calls(ts);
CREATE INDEX IF NOT EXISTS idx_calls_mint_ts ON calls(mint, ts);

CREATE TABLE IF NOT EXISTS wallets (
  wallet TEXT PRIMARY KEY,
  first_seen INTEGER NOT NULL,
  last_seen INTEGER NOT NULL,
  green INTEGER DEFAULT 0,
  red INTEGER DEFAULT 0,
  black INTEGER DEFAULT 0,
  notes TEXT
);

CREATE TABLE IF NOT EXISTS wallet_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  mint TEXT NOT NULL,
  wallet TEXT NOT NULL,
  role TEXT,         -- feePayer|buyer|seller|signer
  amount REAL,
  sig TEXT,
  FOREIGN KEY(mint) REFERENCES mints(mint),
  FOREIGN KEY(wallet) REFERENCES wallets(wallet)
);

CREATE INDEX IF NOT EXISTS idx_wallet_events_wallet_ts ON wallet_events(wallet, ts);
CREATE INDEX IF NOT EXISTS idx_wallet_events_mint_ts ON wallet_events(mint, ts);

CREATE TABLE IF NOT EXISTS outcomes (
  mint TEXT PRIMARY KEY,
  t0 INTEGER NOT NULL,
  fdv_t0 REAL,
  fdv_5m REAL,
  fdv_15m REAL,
  fdv_60m REAL,
  fdv_max_60m REAL,
  rug_flag INTEGER DEFAULT 0,
  notes TEXT,
  FOREIGN KEY(mint) REFERENCES mints(mint)
);
