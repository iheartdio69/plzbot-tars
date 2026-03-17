#!/usr/bin/env bash
set -euo pipefail

FILE="src/config.rs"

if [[ ! -f "$FILE" ]]; then
  echo "❌ $FILE not found"
  exit 1
fi

echo "🔧 Patching $FILE to include helius_rpc_url in Config initializer..."

# If already present, do nothing
if rg -n "helius_rpc_url\s*:" "$FILE" >/dev/null; then
  echo "✅ helius_rpc_url already present (nothing to do)"
  exit 0
fi

# Insert right after helius_api_key if that exists in initializer,
# otherwise after sqlite_path, otherwise after the line containing 'Config {'
python3 - <<'PY'
from pathlib import Path
p = Path("src/config.rs")
s = p.read_text().splitlines(True)

def insert_after(match_substr):
    out=[]
    injected=False
    for line in s:
        out.append(line)
        if (not injected) and (match_substr in line):
            indent = line[:len(line)-len(line.lstrip())]
            out.append(f'{indent}helius_rpc_url: env_str("HELIUS_RPC_URL", ""),\n')
            injected=True
    return out, injected

out, injected = insert_after("helius_api_key:")
if not injected:
    out, injected = insert_after("sqlite_path:")
if not injected:
    # fallback: insert after first "Config {" line
    out=[]
    injected=False
    for line in s:
        out.append(line)
        if (not injected) and ("Config {" in line):
            indent = line[:len(line)-len(line.lstrip())] + "    "
            out.append(f'{indent}helius_rpc_url: env_str("HELIUS_RPC_URL", ""),\n')
            injected=True

if not injected:
    raise SystemExit("❌ Could not find a place to inject. Open src/config.rs around Config { ... } and paste it here.")
p.write_text("".join(out))
print("✅ Injected helius_rpc_url: env_str(\"HELIUS_RPC_URL\", \"\") into Config initializer")
PY

echo "✅ Done. Now run: cargo build"
