#!/usr/bin/env bash
set -euo pipefail

echo "🔧 Forcing canonical DB usage via cfg.sqlite_path"

# 1) Ensure .env exists and points at canonical DB
cat > .env <<'ENV'
SQLITE_PATH=./data/solana_meme.sqlite
DB_PATH=./data/solana_meme.sqlite
ENV
echo "✅ wrote .env"

# 2) Patch main.rs: remove hardcoded Db::open("...") and use cfg.sqlite_path
MAIN="src/main.rs"
if [[ -f "$MAIN" ]]; then
  # If it already uses cfg.sqlite_path, fine.
  if ! rg -n "Db::open\\(&cfg\\.sqlite_path\\)" "$MAIN" >/dev/null 2>&1; then
    # Replace any Db::open("...sqlite") with Db::open(&cfg.sqlite_path)
    perl -0777 -i -pe 's/Db::open\("([^"]*\.sqlite)"\)/Db::open(&cfg.sqlite_path)/g' "$MAIN"

    # Make sure cfg exists before opening db
    if ! rg -n "let\\s+cfg\\s*=\\s*crate::config::load_config\\(\\);" "$MAIN" >/dev/null 2>&1; then
      # Insert cfg load near start of main() block
      perl -0777 -i -pe 's/(fn\s+main[^{]*\{\s*)/$1\n    let cfg = crate::config::load_config();\n/g' "$MAIN"
    fi
  fi
  echo "✅ patched $MAIN"
else
  echo "⚠️ missing $MAIN (skipping)"
fi

# 3) Patch app.rs similarly (if it has hardcoded opens)
APP="src/app.rs"
if [[ -f "$APP" ]]; then
  perl -0777 -i -pe 's/Db::open\("([^"]*\.sqlite)"\)/Db::open(&cfg.sqlite_path)/g' "$APP"
  echo "✅ patched $APP"
fi

echo "✅ Done. Build to verify."
