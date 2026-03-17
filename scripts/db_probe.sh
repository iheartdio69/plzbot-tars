#!/bin/zsh
set -euo pipefail
DB="${DB:-./data/solana_meme.sqlite}"

echo "DB=$DB"
ls -la "$DB" || true

echo
echo "== pragma database_list =="
sqlite3 "$DB" "PRAGMA database_list;"

echo
echo "== create test table =="
sqlite3 "$DB" "CREATE TABLE IF NOT EXISTS __schema_test(x INTEGER);"

echo
echo "== sqlite_master rows =="
sqlite3 "$DB" "SELECT COUNT(*) FROM sqlite_master;"

echo
echo "== sqlite_master listing =="
sqlite3 "$DB" "SELECT name, type FROM sqlite_master ORDER BY type, name;"

echo
echo "== select from test table (should work even if empty) =="
sqlite3 "$DB" "SELECT COUNT(*) FROM __schema_test;"
