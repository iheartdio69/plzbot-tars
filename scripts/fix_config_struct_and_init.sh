#!/usr/bin/env bash
set -euo pipefail

FILE="src/config.rs"

python3 - <<'PY'
from pathlib import Path
import re

p = Path("src/config.rs")
s = p.read_text()

# --- 1) Fix struct field if it's accidentally an expression ---
# Replace any struct-line like:
#   helius_rpc_url: env_str("HELIUS_RPC_URL", ""),
# with:
#   pub helius_rpc_url: String,
#
# Do it generally (this expression should not exist anywhere except an initializer).
s = re.sub(
    r'^\s*(pub\s+)?helius_rpc_url\s*:\s*env_str\(\s*"HELIUS_RPC_URL"\s*,\s*""\s*\)\s*,\s*$',
    '    pub helius_rpc_url: String,',
    s,
    flags=re.M
)

# --- 2) Ensure the struct contains the helius_rpc_url field (type) at all ---
# If it's missing entirely, insert it near other helius fields.
if not re.search(r'^\s*pub\s+helius_rpc_url\s*:\s*String\s*,\s*$', s, flags=re.M):
    # Insert after a helius_api_key field if present, else after the opening brace.
    if re.search(r'^\s*pub\s+helius_api_key\s*:\s*String\s*,\s*$', s, flags=re.M):
        s = re.sub(
            r'^(\s*pub\s+helius_api_key\s*:\s*String\s*,\s*)$',
            r'\1\n    pub helius_rpc_url: String,',
            s,
            flags=re.M
        )
    else:
        s = re.sub(r'(pub\s+struct\s+Config\s*\{\s*\n)', r'\1    pub helius_rpc_url: String,\n', s, count=1)

# --- 3) Ensure initializer has helius_rpc_url: env_str("HELIUS_RPC_URL", ""), ---
# Find all "Config { ... }" initializer blocks and inject if missing.
def inject_all_initializers(text: str) -> str:
    out = []
    i = 0
    while True:
        m = re.search(r'\bConfig\s*\{\s*\n', text[i:])
        if not m:
            out.append(text[i:])
            break
        start = i + m.start()
        out.append(text[i:start])
        j = i + m.end()

        # scan to matching }
        depth = 1
        k = j
        while k < len(text) and depth > 0:
            if text[k] == '{': depth += 1
            elif text[k] == '}': depth -= 1
            k += 1
        block = text[start:k]

        if re.search(r'\bhelius_rpc_url\s*:', block):
            out.append(block)
        else:
            block = re.sub(
                r'(Config\s*\{\s*\n)',
                r'\1        helius_rpc_url: env_str("HELIUS_RPC_URL", ""),\n',
                block,
                count=1
            )
            out.append(block)

        i = k
    return ''.join(out)

s = inject_all_initializers(s)

p.write_text(s)
print("✅ Fixed src/config.rs (struct field + initializer(s))")
PY

echo "---- sanity peek (top of file) ----"
nl -ba "$FILE" | sed -n '1,120p'
echo "-----------------------------------"
echo "✅ Now run: cargo build"
