#!/usr/bin/env bash
set -euo pipefail

FILE="src/config.rs"

python3 - <<'PY'
from pathlib import Path
import re

p = Path("src/config.rs")
s = p.read_text()

# 1) Fix the struct definition if we injected garbage there
# Replace any line like: helius_rpc_url: env_str("HELIUS_RPC_URL", ""),
# inside the struct with a proper type field.
# We'll do a general replace (safe enough because that exact string should never belong in a struct).
s2 = s.replace('helius_rpc_url: env_str("HELIUS_RPC_URL", ""),', 'pub helius_rpc_url: String,')

# Also handle a variant with spaces
s2 = re.sub(r'^\s*helius_rpc_url:\s*env_str\("HELIUS_RPC_URL",\s*""\),\s*$',
            '    pub helius_rpc_url: String,',
            s2,
            flags=re.M)

# 2) Ensure initializer includes helius_rpc_url: env_str("HELIUS_RPC_URL", ""),
# Find a Config { ... } initializer block (first one) and insert field if missing in that block.
# We'll insert right after the line containing "Config {".
def inject_into_first_initializer(text: str) -> str:
    m = re.search(r'Config\s*\{\s*\n', text)
    if not m:
        return text

    start = m.end()
    # Find the matching closing brace for this initializer (best-effort by scanning braces)
    i = start
    depth = 1
    while i < len(text) and depth > 0:
        if text[i] == '{':
            depth += 1
        elif text[i] == '}':
            depth -= 1
        i += 1
    block_end = i  # position after matching '}'
    block = text[m.start():block_end]

    if re.search(r'\bhelius_rpc_url\s*:', block):
        return text  # already present in initializer

    # Insert after "Config {"
    injected = re.sub(
        r'(Config\s*\{\s*\n)',
        r'\1        helius_rpc_url: env_str("HELIUS_RPC_URL", ""),\n',
        block,
        count=1
    )
    return text[:m.start()] + injected + text[block_end:]

s3 = inject_into_first_initializer(s2)

p.write_text(s3)
print("✅ Patched src/config.rs (struct field type + initializer field)")
PY

echo "✅ Done. Now run: cargo build"
