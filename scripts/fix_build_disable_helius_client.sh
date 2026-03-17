#!/usr/bin/env bash
set -euo pipefail

echo "🔧 Disabling src/helius/client.rs from compilation (we'll re-enable later)..."

# Typical module file:
MOD="src/helius/mod.rs"
if [[ -f "$MOD" ]]; then
  # Comment out `pub mod client;` or `mod client;`
  if rg -n '^\s*(pub\s+)?mod\s+client\s*;' "$MOD" >/dev/null; then
    perl -0777 -i -pe 's/^(\s*)(pub\s+)?mod\s+client\s*;\s*$/\1\/\/ (disabled) mod client; \/\/ was: $2mod client;/mg' "$MOD"
    echo "✅ Patched $MOD (client module disabled)"
  else
    echo "ℹ️ No 'mod client;' line found in $MOD (maybe already disabled)"
  fi
else
  echo "⚠️ $MOD not found. If your helius modules are declared elsewhere, we'll patch that file instead."
fi

echo "🔧 Fixing Config initializer missing helius_rpc_url..."

CFG="src/config.rs"
if [[ -f "$CFG" ]]; then
  # If struct has field helius_rpc_url but initializer doesn't, insert a sane default in initializer.
  # We’ll add: helius_rpc_url: env_str("HELIUS_RPC_URL", ""),
  python3 - <<'PY'
from pathlib import Path
p = Path("src/config.rs")
s = p.read_text()

if "helius_rpc_url" not in s:
    print("ℹ️ No helius_rpc_url anywhere in src/config.rs; nothing to patch.")
    raise SystemExit(0)

# Find initializer "Config { ... }" in default() function-ish area.
# We'll inject after a nearby helius_api_key or sqlite_path entry if present.
if "Config {" in s and "helius_rpc_url:" not in s:
    insert_after_keys = ["helius_api_key:", "sqlite_path:"]
    lines = s.splitlines(True)
    out = []
    injected = False
    for line in lines:
        out.append(line)
        if (not injected) and any(k in line for k in insert_after_keys):
            indent = line.split("h")[0]  # cheap indent guess
            out.append(f'{indent}helius_rpc_url: env_str("HELIUS_RPC_URL", ""),\n')
            injected = True
    if injected:
        p.write_text("".join(out))
        print("✅ Injected helius_rpc_url into Config initializer")
    else:
        print("⚠️ Couldn't find a good insert point. Add manually inside Config { ... } initializer.")
else:
    print("ℹ️ Config initializer already has helius_rpc_url or not found; skipping.")
PY
else
  echo "⚠️ src/config.rs not found."
fi

echo
echo "✅ Done. Try: cargo build"
