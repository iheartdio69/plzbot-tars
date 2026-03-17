#!/usr/bin/env bash
set -e

FILE="src/helius/client.rs"

echo "🔧 Patching insert_wallet_event → insert_wallet_edge in $FILE"

if ! grep -q "insert_wallet_event" "$FILE"; then
  echo "ℹ️ No insert_wallet_event found (already patched?)"
  exit 0
fi

sed -i '' 's/insert_wallet_event/insert_wallet_edge/g' "$FILE"
echo "✅ Function name replaced"
