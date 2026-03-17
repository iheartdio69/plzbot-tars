#!/usr/bin/env bash
set -euo pipefail

echo "== tree (trimmed) =="
if command -v tree >/dev/null 2>&1; then
  tree -a -I "target|.git|node_modules|backups" || true
else
  find . \
    -path "./target" -prune -o \
    -path "./.git" -prune -o \
    -path "./node_modules" -prune -o \
    -path "./backups" -prune -o \
    -print
fi
echo

echo "== sqlite files =="
find . -maxdepth 3 -name "*.sqlite" -print | sed "s|^./||" || true
echo

scan_db () {
  local f="$1"
  echo "---- DB: $f ----"
  ls -lh "$f" || true
  echo "tables: $(sqlite3 "$f" "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;" 2>/dev/null | tr "\n" " ")"
  echo "counts:"
  sqlite3 "$f" "SELECT COUNT(*) AS mint_snapshots FROM mint_snapshots;" 2>/dev/null || echo "  mint_snapshots: (missing)"
  sqlite3 "$f" "SELECT COUNT(*) AS calls FROM calls;" 2>/dev/null || echo "  calls: (missing)"
  sqlite3 "$f" "SELECT COUNT(*) AS wallet_edges FROM wallet_edges;" 2>/dev/null || echo "  wallet_edges: (missing)"
  sqlite3 "$f" "SELECT COUNT(*) AS wallets FROM wallets;" 2>/dev/null || echo "  wallets: (missing)"
  echo "recent wallet_edges:"
  sqlite3 "$f" -cmd ".headers on" -cmd ".mode column" \
    "SELECT ts, substr(src_wallet,1,6) AS src, substr(mint,1,6) AS mint, action, sol, substr(sig,1,8) AS sig
     FROM wallet_edges
     ORDER BY ts DESC
     LIMIT 5;" 2>/dev/null || echo "  (wallet_edges missing)"
  echo
}

while IFS= read -r f; do
  scan_db "$f"
done < <(find . -maxdepth 3 -name "*.sqlite" -print)
