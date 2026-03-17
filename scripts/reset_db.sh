#!/usr/bin/env bash
set -euo pipefail

CANON="./data/solana_meme.sqlite"

mkdir -p backups data

# backup any sqlite files we find (maxdepth 3)
ts="$(date +%Y%m%d_%H%M%S)"
find . -maxdepth 3 -name "*.sqlite" -print0 | while IFS= read -r -d '' f; do
  bn="$(basename "$f")"
  cp -v "$f" "./backups/${bn}.${ts}" || true
done

# reset canonical db
rm -f "$CANON"
sqlite3 "$CANON" < src/db/schema.sql

# write .env (only DB stuff, safe)
cat > .env <<ENV
SQLITE_PATH=$CANON
DB_PATH=$CANON
ENV

echo "✅ reset canonical DB -> $CANON"
sqlite3 "$CANON" ".tables"
