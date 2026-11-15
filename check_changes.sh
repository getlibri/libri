#!/bin/sh
# The everyday check: builds the flake checks that cover what changed since
# the branch left main, and nothing else. A crate's check is its tests and
# clippy; the page's is its typecheck, tests and build; rustfmt runs on any
# Rust change. The full `nix flake check`, which builds the release Libri and
# runs it end to end, gates the merge to main instead, as a project's CI runs
# its quick jobs per push and its slow suite before merge.
#
#   ./check_changes.sh            changes since origin/main, committed or not
#   ./check_changes.sh BASE       changes since BASE
#
# Extra arguments after `--` go to nix build (e.g. -- --max-jobs 2 -L).
set -eu
cd "$(dirname "$0")"
base=origin/main
if [ $# -gt 0 ] && [ "$1" != "--" ]; then
  base=$1
  shift
fi
[ "${1:-}" = "--" ] && shift

changed=$(
  {
    git diff --name-only "$(git merge-base "$base" HEAD)"
    git ls-files --others --exclude-standard
  } | sort -u
)

# The install-name scan (nix/check_names.sh, the flake's checks.names) runs
# on the working tree whatever changed: it is quick, and a name can enter anywhere.
nix/check_names.sh
# And the page's dialogs are Libri's own (nix/check_dialogs.sh, checks.dialogs).
nix/check_dialogs.sh

checks=""
add() {
  case " $checks " in
  *" $1 "*) ;;
  *) checks="$checks $1" ;;
  esac
}
for path in $changed; do
  case $path in
  # What every check is built from: all of them.
  flake.nix | flake.lock)
    for c in daemon hippo wiki client site rustfmt module; do add "$c"; done
    ;;
  nix/rust-tool.nix) for c in daemon hippo wiki client; do add "$c"; done ;;
  libri/site/* | android/*) add site; add tiles ;;
  libri/src/tiles.rs) add daemon; add tiles ;;
  libri/*) add daemon ;;
  hippo/*) add hippo ;;
  wiki/*) add wiki ;;
  client/*) add client ;;
  nix/module.nix) add module ;;
  # The end-to-end checks run the release build: changed, they run here too.
  nix/check_memory.py) add memory ;;
  nix/check_delivery.py) add delivery ;;
  nix/check_memories.py | libri/src/memories.rs) add memories ;;
  esac
  case $path in
  *.rs) add rustfmt ;;
  esac
done

if [ -z "$checks" ]; then
  echo "check_changes: nothing since $base that a flake check covers"
  exit 0
fi
echo "check_changes:$checks"
targets=""
for c in $checks; do targets="$targets .#checks.x86_64-linux.$c"; done
# shellcheck disable=SC2086 # one word per target
# Builds run side by side, never beside a timing run (nix/with_check_lock.sh),
# each once the host has room for it (nix/with_memory.sh).
exec nix/with_check_lock.sh --build nix/with_memory.sh 8 nix build --no-link $targets "$@"
