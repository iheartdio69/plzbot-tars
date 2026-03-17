#!/usr/bin/env bash
set -euo pipefail

if command -v tree >/dev/null 2>&1; then
  tree -a -I 'target|.git|node_modules|backups'
else
  find . \
    -path './target' -prune -o \
    -path './.git' -prune -o \
    -path './node_modules' -prune -o \
    -path './backups' -prune -o \
    -print
fi
