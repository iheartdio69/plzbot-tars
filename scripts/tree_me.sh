#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-.}"

echo "📦 Project tree (top-level, plus src/ + scripts/ + data/):"
echo "ROOT=$ROOT"
echo

# Pretty-ish tree without needing `tree`
find "$ROOT" -maxdepth 3 \
  \( -path "$ROOT/.git" -o -path "$ROOT/target" -o -path "$ROOT/node_modules" \) -prune -o \
  -print | sed "s|^$ROOT/||" | awk '
BEGIN{FS="/"}
{
  indent=""
  for (i=1; i<NF; i++) indent=indent"  "
  print indent $NF
}' | head -n 400

echo
echo "��️ SQLite files found:"
find "$ROOT" -type f \( -name "*.sqlite" -o -name "*.db" \) -not -path "*/target/*" -print -exec ls -lh {} \; 2>/dev/null || true
