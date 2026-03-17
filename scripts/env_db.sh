#!/usr/bin/env bash
# Usage:
#   source scripts/env_db.sh
#   echo $SQLITE_PATH
#
# Canonical DB location:
export SQLITE_PATH="${SQLITE_PATH:-./data/solana_meme.sqlite}"
export DB_PATH="${DB_PATH:-$SQLITE_PATH}"

# Ensure folders + file exist
mkdir -p "$(dirname "$SQLITE_PATH")"
touch "$SQLITE_PATH"

echo "✅ SQLITE_PATH=$SQLITE_PATH"
