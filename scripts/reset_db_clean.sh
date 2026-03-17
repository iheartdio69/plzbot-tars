#!/usr/bin/env bash
set -euo pipefail

mkdir -p data
TS="$(date +%Y%m%d_%H%M%S)"
BK="data/_backup_${TS}"
mkdir -p "$BK"

echo "🧹 Moving existing sqlite/db files into: $BK"
shopt -s nullglob
for f in data/*.sqlite data/*.db; do
  mv "$f" "$BK/"
done
shopt -u nullglob

DB="data/solana_meme.sqlite"
echo "✅ Creating fresh DB at: $DB"
rm -f "$DB"
sqlite3 "$DB" "PRAGMA journal_mode=WAL;"

export SQLITE_PATH="./$DB"
export DB_PATH="./$DB"

echo "✅ SQLITE_PATH=$SQLITE_PATH"
echo "✅ DB_PATH=$DB_PATH"
echo "ℹ️ Now run: cargo run"
