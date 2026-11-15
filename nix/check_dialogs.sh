#!/bin/sh
# The page asks and tells in Libri's own dialogs (site/src/dialog.tsx), never
# the browser's confirm(), alert() or prompt(), which look like the system's
# and not the app.
#
#   nix/check_dialogs.sh [DIR]
#
# Scans DIR's libri/site/src (default: the current directory). The flake
# runs it as checks.dialogs and check_changes.sh on the working tree.
set -eu
cd "${1:-.}/libri/site/src"
found=$(grep -rnE '(^|[^.A-Za-z_$])(window\.)?(confirm|alert|prompt)\(' --include='*.ts' --include='*.tsx' . |
  grep -v '^\./dialog\.tsx:' || true)
if [ -n "$found" ]; then
  printf '%s\n' "$found"
  echo "check_dialogs: the browser's own dialogs above; use confirmAction or tell (dialog.tsx)"
  exit 1
fi
