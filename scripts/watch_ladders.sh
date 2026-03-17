#!/usr/bin/env bash
set -euo pipefail

DB="${DB_PATH:-./data/solana_meme.sqlite}"

if [[ ! -f "$DB" ]]; then
  echo "DB not found: $DB" >&2
  exit 1
fi

sqlite3 "$DB" <<'SQL'
.headers on
.mode column

-- quick sanity
SELECT COUNT(*) AS snapshots FROM mint_snapshots;

WITH ladder AS (
  SELECT
    mint,
    MIN(fdv_usd) AS start_fdv,
    MAX(fdv_usd) AS peak_fdv,
    COUNT(*)     AS snaps,
    MIN(ts)      AS first_ts,
    MAX(ts)      AS last_ts
  FROM mint_snapshots
  WHERE fdv_usd IS NOT NULL
  GROUP BY mint
)
SELECT
  mint,
  ROUND(start_fdv, 0) AS start_fdv,
  ROUND(peak_fdv, 0)  AS peak_fdv,
  snaps,
  datetime(first_ts, 'unixepoch') AS first_seen,
  datetime(last_ts,  'unixepoch') AS last_seen
FROM ladder
WHERE peak_fdv >= 20000
ORDER BY peak_fdv DESC
LIMIT 25;
SQL
