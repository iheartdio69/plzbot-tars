#!/usr/bin/env bash
set -e
FILE="src/helius/client.rs"
echo "=== $FILE (lines 1-140) ==="
nl -ba "$FILE" | sed -n '1,140p'
