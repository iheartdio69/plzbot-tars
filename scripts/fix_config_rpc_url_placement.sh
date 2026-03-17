#!/usr/bin/env bash
set -euo pipefail

FILE="src/config.rs"

python3 - <<'PY'
from pathlib import Path
import re

p = Path("src/config.rs")
lines = p.read_text().splitlines(True)

# Track which line indexes are inside a `Config { ... }` struct literal
inside = [False]*len(lines)
i = 0
while i < len(lines):
    if re.search(r'\bConfig\s*\{\s*$', lines[i]) or re.search(r'\bConfig\s*\{\s*\n$', lines[i]):
        depth = 0
        j = i
        # from i onward, mark until we close the first { ... } at same nesting
        # count braces across lines
        started = False
        while j < len(lines):
            for ch in lines[j]:
                if ch == '{':
                    depth += 1
                    started = True
                elif ch == '}':
                    depth -= 1
            inside[j] = True
            j += 1
            if started and depth == 0:
                break
        i = j
    else:
        i += 1

out = []
for idx, line in enumerate(lines):
    # Fix struct field if someone put env_str in the struct definition
    # (this exact mistake has shown up in your file)
    if re.match(r'^\s*(pub\s+)?helius_rpc_url\s*:\s*env_str\(\s*"HELIUS_RPC_URL"\s*,\s*""\s*\)\s*,\s*$', line):
        out.append("    pub helius_rpc_url: String,\n")
        continue

    # Remove stray initializer line if it's NOT inside a Config { ... } block
    if (not inside[idx]) and re.match(r'^\s*helius_rpc_url\s*:\s*env_str\(\s*"HELIUS_RPC_URL"\s*,\s*""\s*\)\s*,\s*$', line):
        continue

    out.append(line)

s = "".join(out)

# Ensure struct has the type field
if not re.search(r'^\s*pub\s+helius_rpc_url\s*:\s*String\s*,\s*$', s, flags=re.M):
    s = re.sub(r'(pub\s+struct\s+Config\s*\{\s*\n)', r'\1    pub helius_rpc_url: String,\n', s, count=1)

# Ensure at least one Config initializer has the env_str line
if not re.search(r'\bhelius_rpc_url\s*:\s*env_str\(\s*"HELIUS_RPC_URL"\s*,\s*""\s*\)\s*,', s):
    s = re.sub(r'(\bConfig\s*\{\s*\n)', r'\1        helius_rpc_url: env_str("HELIUS_RPC_URL", ""),\n', s, count=1)

p.write_text(s)
print("✅ fixed placement of helius_rpc_url in src/config.rs")
PY

echo "---- where helius_rpc_url appears now ----"
rg -n "helius_rpc_url" "$FILE" || true
echo "-----------------------------------------"
echo "✅ now run: cargo build"
