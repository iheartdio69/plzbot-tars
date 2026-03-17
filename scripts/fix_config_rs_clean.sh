#!/usr/bin/env bash
set -euo pipefail

FILE="src/config.rs"

python3 - <<'PY'
from pathlib import Path
import re

p = Path("src/config.rs")
s = p.read_text()

# 1) Remove the illegal initializer line inside the struct (the env_str one)
s = re.sub(r'^\s*helius_rpc_url\s*:\s*env_str\(\s*"HELIUS_RPC_URL"\s*,\s*""\s*\)\s*,\s*\n', '', s, flags=re.M)

# 2) Remove any illegal "pub helius_rpc_url: String," that appears inside load_config() body
# (keep only struct fields, remove the one that got inserted after `pub fn load_config() -> Config {`)
s = re.sub(r'(\bpub\s+fn\s+load_config\(\)\s*->\s*Config\s*\{\s*\n)\s*pub\s+helius_rpc_url\s*:\s*String\s*,\s*\n', r'\1', s)

# 3) Ensure struct has exactly ONE helius_rpc_url field.
# First, delete ALL occurrences of that field in struct, then re-insert once in a sane spot.
# We'll rebuild the struct field list minimally by:
# - removing duplicate field lines anywhere
s = re.sub(r'^\s*pub\s+helius_rpc_url\s*:\s*String\s*,\s*\n', '', s, flags=re.M)

# Now insert ONE struct field right after helius_api_key OR near APIs block.
# Find "pub helius_api_key: String," inside the struct and insert helius_rpc_url after it.
def insert_after_api_key(match):
    return match.group(0) + "    pub helius_rpc_url: String,\n"

if re.search(r'^\s*pub\s+helius_api_key\s*:\s*String\s*,\s*$', s, flags=re.M):
    s = re.sub(r'^\s*pub\s+helius_api_key\s*:\s*String\s*,\s*\n', insert_after_api_key, s, flags=re.M, count=1)
else:
    # fallback: insert near top of struct
    s = re.sub(r'(pub\s+struct\s+Config\s*\{\s*\n)', r'\1    pub helius_rpc_url: String,\n', s, count=1)

# 4) Ensure Config { ... } initializer includes helius_rpc_url
# Add it right after helius_api_key: ... if missing.
if not re.search(r'\bhelius_rpc_url\s*:\s*env_str\(\s*"HELIUS_RPC_URL"', s):
    s = re.sub(
        r'(\bhelius_api_key\s*:\s*env_str\(\s*"HELIUS_API_KEY"\s*,\s*""\s*\)\s*,\s*\n)',
        r'\1        helius_rpc_url: env_str("HELIUS_RPC_URL", ""),\n',
        s,
        count=1
    )

p.write_text(s)
print("✅ src/config.rs cleaned (struct fields + initializer fixed)")
PY

echo "---- quick sanity ----"
rg -n "pub struct Config|helius_api_key|helius_rpc_url|pub fn load_config\\(\\)" "$FILE" || true
echo "----------------------"
echo "✅ now run: cargo build"
