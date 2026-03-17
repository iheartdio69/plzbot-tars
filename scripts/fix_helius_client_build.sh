#!/usr/bin/env bash
set -euo pipefail

FILE="src/helius/client.rs"
test -f "$FILE" || { echo "❌ Missing $FILE"; exit 1; }

echo "🔧 Patching $FILE for: Option<&str>, no '?', and proper Vec return"

python3 - <<'PY'
from pathlib import Path
import re

p = Path("src/helius/client.rs")
s = p.read_text()

orig = s

# 1) insert_wallet_edge expects mint: Option<&str>
# Replace an argument line that's literally "&mint," (or "mint," in some variants) inside the call.
s = re.sub(r'(\n\s*)(?:&)?mint,\s*\n', r'\1Some(mint.as_str()),\n', s)

# 2) This function returns Vec<String>, so we cannot use '?' here.
# Replace ")?;" (the common pattern after a function call with ?) with ");"
s = s.replace(")?;", ");")

# 3) Ensure we actually return a Vec<String> at end of function.
# If there is no discovered-collect return, append it just before the last closing brace of the outer function.
has_return = ("discovered.into_iter().collect()" in s) or ("discovered.collect()" in s)

if not has_return:
    # Add "discovered.into_iter().collect()" before the final '}' in the file (best-effort)
    # (Assumes this file ends with the function; matches your current layout.)
    idx = s.rfind("}")
    if idx != -1:
        s = s[:idx].rstrip() + "\n\n    discovered.into_iter().collect()\n}\n"
    else:
        raise SystemExit("Could not find closing brace in client.rs")

# 4) If the last statement is a for-loop without semicolon and then nothing,
# this forces a final expression return. (We already added return above if needed.)

if s != orig:
    p.write_text(s)
    print("✅ Patched src/helius/client.rs")
else:
    print("ℹ️ No changes made (may already be patched)")
PY

echo "✅ Done"
