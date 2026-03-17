#!/usr/bin/env bash
set -e

echo "📂 Project tree (SQLite files highlighted):"
echo

find . \
  -type f \
  \( -name "*.sqlite" -o -name "*.db" \) \
  -print | sed 's/^/🟣 /'

echo
echo "📁 Full directory tree (depth 4):"
echo

if command -v tree >/dev/null 2>&1; then
  tree -L 4
else
  find . -maxdepth 4 -print | sed 's|[^/]*/|  |g'
fi
