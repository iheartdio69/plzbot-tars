#!/usr/bin/env bash
# Usage: ./scripts/run.sh <any command...>
# Example: ./scripts/run.sh sqlite3 "$DB_PATH" ".tables"
set -euo pipefail

# Load .env if present
if [ -f .env ]; then
  set -a
  source .env
  set +a
fi

exec "$@"
