#!/usr/bin/env bash
set -euo pipefail

echo "🔧 Fixing src/helius/client.rs + src/config.rs"

# -----------------------------
# 1) Fix src/helius/client.rs
# -----------------------------
python3 - <<'PY'
from pathlib import Path
import re

p = Path("src/helius/client.rs")
if not p.exists():
    raise SystemExit("❌ src/helius/client.rs not found")

s = p.read_text()
orig = s

# A) Fix the URL format! second arg: it must be the address string, NOT Option<&str>
# Specifically replace base, Some(mint.as_str()), -> base, mint,
s = re.sub(
    r'(format!\(\s*"\{}/v0/addresses/\{\}/transactions\?api-[^"]*"\s*,\s*base\s*,\s*)Some\(mint\.as_str\(\)\)(\s*,)',
    r'\1mint\2',
    s,
    flags=re.MULTILINE,
)

# Also handle variant where it’s not on same line
s = re.sub(
    r'(\{\}/v0/addresses/\{\}/transactions[^\n]*\n\s*base,\n\s*)Some\(mint\.as_str\(\)\)(\s*,)',
    r'\1mint\2',
    s,
    flags=re.MULTILINE,
)

# B) Ensure insert_wallet_edge mint arg is Option<&str>
# Replace a bare "&mint," or "mint," inside insert_wallet_edge(...) with Some(mint.as_str()),
def fix_insert_wallet_edge(block: str) -> str:
    lines = block.splitlines(True)
    out = []
    replaced = False
    for line in lines:
        if not replaced and re.match(r'^\s*&?mint\s*,\s*$', line):
            out.append(re.sub(r'^\s*&?mint\s*,\s*$', '        Some(mint.as_str()),\n', line))
            replaced = True
        else:
            out.append(line)
    return "".join(out)

# Find insert_wallet_edge call blocks and patch them
pattern = re.compile(r'insert_wallet_edge\s*\(\s*(?:.|\n)*?\);\s*', re.MULTILINE)
def repl(m):
    return fix_insert_wallet_edge(m.group(0))
s2 = pattern.sub(repl, s)
s = s2

# C) Remove any "?;" that snuck into this file (this function returns Vec<String>, not Result)
s = s.replace(")?;", ");")

# D) Fix the "collect into ()" problem:
# If there's a line "discovered.into_iter().collect()" as a statement, make it a real return.
# Turn:
#   discovered.into_iter().collect()
# into:
#   return discovered.into_iter().collect::<Vec<String>>();
s = re.sub(
    r'^\s*discovered\.into_iter\(\)\.collect\(\)\s*;?\s*$',
    r'    return discovered.into_iter().collect::<Vec<String>>();',
    s,
    flags=re.MULTILINE
)

# E) Ensure the function returns Vec<String> even if it currently ends with a for-loop.
# If there's no explicit return anywhere, append a final return.
if "-> Vec<String>" in s and "return discovered.into_iter().collect::<Vec<String>>();" not in s:
    # append just before the last "}" in file
    idx = s.rfind("}")
    if idx != -1:
        s = s[:idx].rstrip() + "\n\n    return discovered.into_iter().collect::<Vec<String>>();\n}\n"

if s != orig:
    p.write_text(s)
    print("✅ Patched src/helius/client.rs")
else:
    print("ℹ️ No changes made to src/helius/client.rs (maybe already fixed)")
PY

# -----------------------------
# 2) Fix missing helius_rpc_url in src/config.rs
# -----------------------------
python3 - <<'PY'
from pathlib import Path
import re

p = Path("src/config.rs")
if not p.exists():
    raise SystemExit("❌ src/config.rs not found")

s = p.read_text()
orig = s

# If field exists in struct, fine — your error is about initializer missing it.
# We will patch the first Config { ... } initializer that does NOT contain helius_rpc_url:
def patch_first_initializer(text: str) -> str:
    # naive but effective: locate "Config {" then find matching closing "}" at same nesting
    start = text.find("Config {")
    if start == -1:
        return text

    # walk braces from start
    i = start
    depth = 0
    end = None
    while i < len(text):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                end = i
                break
        i += 1

    if end is None:
        return text

    block = text[start:end+1]
    if "helius_rpc_url" in block:
        return text

    # Insert after helius_api_key if present, else near top of block
    ins_line = '        helius_rpc_url: env_str("HELIUS_RPC_URL", ""),\n'
    m = re.search(r'(helius_api_key\s*:\s*env_str\([^\)]*\)\s*,\s*\n)', block)
    if m:
        block2 = block[:m.end(1)] + ins_line + block[m.end(1):]
    else:
        # insert right after "Config {\n"
        block2 = block.replace("Config {\n", "Config {\n" + ins_line, 1)

    return text[:start] + block2 + text[end+1:]

s = patch_first_initializer(s)

if s != orig:
    p.write_text(s)
    print("✅ Patched src/config.rs (added helius_rpc_url to initializer)")
else:
    print("ℹ️ No changes made to src/config.rs (maybe already fixed)")
PY

echo "✅ Done. Now run: cargo build"
