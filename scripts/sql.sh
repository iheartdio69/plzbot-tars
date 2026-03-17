#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/env_db.sh" >/dev/null
exec sqlite3 "$SQLITE_PATH" "$@"
