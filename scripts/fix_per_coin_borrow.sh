#!/usr/bin/env bash
set -euo pipefail

cp -v src/helius/per_coin.rs "src/helius/per_coin.rs.bak.$(date +%s)"

python3 - <<'PY'
from pathlib import Path
import re

p = Path("src/helius/per_coin.rs")
s = p.read_text()

needle = "let Some(pair) = coins.get(mint) else { continue; };"
if needle not in s:
    raise SystemExit("Couldn't find the expected line in per_coin.rs. Run: nl -ba src/helius/per_coin.rs | sed -n '120,220p'")

replacement = """let Some(pair) = coins.get(mint) else { continue; };

// clone what we need from the immutable borrow
let pair_address = pair.pair_address.clone();
let first_seen = pair.first_seen;
"""

s2 = s.replace(needle, replacement, 1)

# Replace closure captures like unwrap_or_else(|| pair....) so it doesn't keep `pair` borrowed
s2 = re.sub(
    r'unwrap_or_else\(\|\|\s*pair\.[^)]+\)',
    'unwrap_or_else(|| pair_address.clone().unwrap_or_default())',
    s2
)

p.write_text(s2)
print("✅ patched src/helius/per_coin.rs")
PY

echo "Now run: cargo check"
