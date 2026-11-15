#!/bin/sh
# Libri is a standalone product: no names, hosts or paths of the household
# that runs it, and nothing of the system that deploys it there. That
# deployment configures Libri from outside, as it would any package. NOTICE
# carries the copyright holder's name; "bridge-session" is a Claude Code
# transcript entry type, "app-bridge" a module of the MCP Apps SDK.
#
#   nix/check_names.sh [DIR]
#
# Scans DIR (default: the current directory): in a git checkout its tracked
# and untracked files, so an uncommitted file counts too; elsewhere (the
# flake's source in the store) every file. The flake runs it as checks.names
# and check_changes.sh runs it on the working tree.
set -eu
cd "${1:-.}"
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  files() { git ls-files -z --cached --others --exclude-standard; }
else
  files() { find . -type f -print0; }
fi
leaks=$(files | grep -zvE '(^|/)(NOTICE|check_names\.sh)$' |
  xargs -0 grep -nIiE '\b(water|suki|gabmc|gab|dylan|kestrel|switcharoo|monix|agenix|endgrain|katara|cockpit)\b|\bsu\.is\b|/var/lib/(libri-deploy|libri-build|bridge)|/home/bridge|\bremote_run\b|\bbridge\b' -- |
  grep -v -e 'bridge-session' -e 'ext-apps/app-bridge' || true)
if [ -n "$leaks" ]; then
  printf '%s\n' "$leaks"
  echo "check_names: names or paths of a particular install above; Libri holds the product only"
  exit 1
fi
