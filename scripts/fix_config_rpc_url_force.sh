#!/usr/bin/env bash
set -euo pipefail

FILE="src/config.rs"

python3 - <<'PY'
from pathlib import Path
p = Path("src/config.rs")
s = p.read_text().splitlines(True)

# If already present anywhere, still might be missing in *initializer*,
# so we specifically patch the first "Config {" block.
out=[]
in_config=False
inserted=False
seen_field_anywhere = any("helius_rpc_url" in line for line in s)

for i,line in enumerate(s):
    out.append(line)
    if (not inserted) and ("Config {" in line):
        in_config=True
        # inject right after Config { line
        indent = line[:len(line)-len(line.lstrip())] + "    "
        out.append(f'{indent}helius_rpc_url: env_str("HELIUS_RPC_URL", ""),\n')
        inserted=True

p.write_text("".join(out))
print("✅ Injected helius_rpc_url into the first Config { ... } initializer")
PY

echo "✅ Patched $FILE"
echo "Now run: cargo build"
