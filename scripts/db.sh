#!/usr/bin/env bash
set -euo pipefail

DB="${DB:-./data/solana_meme.sqlite}"
SQLITE_BIN="${SQLITE_BIN:-sqlite3}"

die() { echo "ERR: $*" >&2; exit 1; }

need_db() {
  [[ -f "$DB" ]] || die "DB not found: $DB (set DB=/path/to.sqlite)"
}

q() {
  need_db
  "$SQLITE_BIN" "$DB" -cmd ".mode column" -cmd ".headers on" "$1"
}

usage() {
  cat <<'USAGE'
Usage:
  DB=./data/solana_meme.sqlite scripts/db.sh <cmd> [args]

Commands:
  tables                     Show tables
  schema [table]             Show schema (optionally for a table)
  calls [N]                  Latest calls (default N=25)
  calls_mint <MINT> [N]      Calls for a mint (default N=20)
  conc [N]                   Latest concentration summaries from fingerprints (default N=30)
  conc_mint <MINT> [N]       Concentration summaries for a mint (default N=30)
  conc_marks [N]             Raw conc_risk fingerprint rows (default N=50)
  snapshots [N]              Latest mint_snapshots (default N=30)
  mint <MINT> [N]            Latest snapshots for a mint (default N=30)
  wallets [N]                Latest wallets by last_seen_ts (default N=25)
  wallet <WALLET> [N]        Show wallet row + outcomes summary (default N=50)
  outcomes [N]               Latest call outcomes (default N=50)
  last_scored                Wallet scoring state last_scored_ts
USAGE
}

cmd="${1:-}"; shift || true

case "$cmd" in
  ""|-h|--help) usage; exit 0 ;;

  tables)
    need_db
    "$SQLITE_BIN" "$DB" ".tables"
    ;;

  schema)
    need_db
    if [[ "${1:-}" == "" ]]; then
      "$SQLITE_BIN" "$DB" ".schema"
    else
      "$SQLITE_BIN" "$DB" ".schema $1"
    fi
    ;;

  calls)
    N="${1:-25}"
    q "
SELECT ts, mint, fdv_usd, score, tx_5m, signers_5m, events, tag
FROM calls
ORDER BY ts DESC
LIMIT $N;"
    ;;

  calls_mint)
    MINT="${1:-}"; [[ -n "$MINT" ]] || die "calls_mint requires <MINT>"
    N="${2:-20}"
    q "
SELECT ts, mint, fdv_usd, score, tx_5m, signers_5m, events, tag
FROM calls
WHERE mint = '$MINT'
ORDER BY ts DESC
LIMIT $N;"
    ;;

  conc)
    N="${1:-30}"
    q "
SELECT call_ts, mint,
       MAX(CASE WHEN reason='top1_share' THEN metric END) AS top1,
       MAX(CASE WHEN reason='top5_share' THEN metric END) AS top5,
       MAX(CASE WHEN reason='total_edges' THEN metric END) AS total_edges,
       MAX(CASE WHEN reason='conc_risk' THEN metric END) AS conc_risk
FROM call_wallet_fingerprints
WHERE wallet='__agg__'
GROUP BY mint, call_ts
ORDER BY call_ts DESC
LIMIT $N;"
    ;;

  conc_mint)
    MINT="${1:-}"; [[ -n "$MINT" ]] || die "conc_mint requires <MINT>"
    N="${2:-30}"
    q "
SELECT call_ts, mint,
       MAX(CASE WHEN reason='top1_share' THEN metric END) AS top1,
       MAX(CASE WHEN reason='top5_share' THEN metric END) AS top5,
       MAX(CASE WHEN reason='total_edges' THEN metric END) AS total_edges,
       MAX(CASE WHEN reason='conc_risk' THEN metric END) AS conc_risk
FROM call_wallet_fingerprints
WHERE wallet='__agg__' AND mint='$MINT'
GROUP BY mint, call_ts
ORDER BY call_ts DESC
LIMIT $N;"
    ;;

  conc_marks)
    N="${1:-50}"
    q "
SELECT call_ts, mint, wallet, reason, metric
FROM call_wallet_fingerprints
WHERE reason='conc_risk'
ORDER BY call_ts DESC
LIMIT $N;"
    ;;

  snapshots)
    N="${1:-30}"
    q "
SELECT ts, mint, fdv_usd, score, tx_5m, signers_5m, events, first_seen, is_active, is_call, is_mayhem
FROM mint_snapshots
ORDER BY ts DESC
LIMIT $N;"
    ;;

  mint)
    MINT="${1:-}"; [[ -n "$MINT" ]] || die "mint requires <MINT>"
    N="${2:-30}"
    q "
SELECT ts, mint, fdv_usd, score, tx_5m, signers_5m, events, first_seen, is_active, is_call, is_mayhem
FROM mint_snapshots
WHERE mint='$MINT'
ORDER BY ts DESC
LIMIT $N;"
    ;;

  wallets)
    N="${1:-25}"
    q "
SELECT wallet, score, last_seen_ts, COALESCE(notes,'') AS notes
FROM wallets
ORDER BY last_seen_ts DESC
LIMIT $N;"
    ;;

  wallet)
    WALLET="${1:-}"; [[ -n "$WALLET" ]] || die "wallet requires <WALLET>"
    N="${2:-50}"
    echo "== wallets table =="; echo
    q "
SELECT wallet, score, last_seen_ts, COALESCE(notes,'') AS notes
FROM wallets
WHERE wallet='$WALLET'
LIMIT 1;"
    echo
    echo "== wallet_outcomes (latest) =="; echo
    q "
SELECT call_ts, mint, result, call_fdv, peak_fdv, edges
FROM wallet_outcomes
WHERE wallet='$WALLET'
ORDER BY call_ts DESC
LIMIT $N;"
    echo
    echo "== outcomes summary =="; echo
    q "
SELECT
  SUM(CASE WHEN result='win'  THEN 1 ELSE 0 END) AS wins,
  SUM(CASE WHEN result='loss' THEN 1 ELSE 0 END) AS losses,
  COUNT(*) AS total
FROM wallet_outcomes
WHERE wallet='$WALLET';"
    ;;

  outcomes)
    N="${1:-50}"
    q "
SELECT outcome_ts, mint, call_ts, fdv_usd, result
FROM call_outcomes
ORDER BY outcome_ts DESC
LIMIT $N;"
    ;;

  last_scored)
    q "SELECT id, last_scored_ts FROM wallet_scoring_state;"
    ;;

  *)
    die "Unknown command: $cmd (run with --help)"
    ;;
esac
