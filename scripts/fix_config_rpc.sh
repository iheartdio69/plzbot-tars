#!/usr/bin/env bash
set -e

FILE="src/config.rs"

echo "🔧 Ensuring helius_rpc_url exists in Config initializer in $FILE"

# If the struct has the field but initializer missing, this may not help—so we check both.
if ! rg -n "pub helius_rpc_url:" "$FILE" >/dev/null; then
  echo "❌ Config struct does not contain helius_rpc_url field. Add it to struct first."
  exit 1
fi

# Add initializer line if missing
if rg -n "helius_rpc_url:" "$FILE" >/dev/null; then
  echo "ℹ️ helius_rpc_url already present in initializer"
  exit 0
fi

# Insert after sqlite_path: line in default Config initializer block
sed -i '' '/sqlite_path:/a\
        helius_rpc_url: env_str("HELIUS_RPC_URL", ""),\
' "$FILE"

echo "✅ helius_rpc_url added to initializer"
