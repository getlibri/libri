#!/usr/bin/env bash
# Runs a harness command where it sees only what the test gives it: the
# Nix store, its own sign-in, the vault (rw or ro) and, if given, a folder
# of readable files. No live memory, no other transcripts, no later log.
# sandbox.sh claude|codex|omp VAULT_DIR VAULT_MODE(rw|ro) EXTRA_DIR|- -- cmd...
# OPENCODE_KEY_FILE, if set, is the OpenCode Go key file omp reads.
set -euo pipefail
h=$1 vault=$2 mode=$3 extra=$4; shift 5
H=$HOME
a=(--die-with-parent --unshare-pid --unshare-ipc --share-net --proc /proc --dev /dev
   --ro-bind /nix /nix --ro-bind /etc /etc --ro-bind /run/current-system /run/current-system
   --ro-bind /run/systemd/resolve /run/systemd/resolve --ro-bind /bin /bin --ro-bind /usr /usr
   --tmpfs /tmp --tmpfs $H --setenv HOME $H --setenv TMPDIR /tmp)
if [ "$mode" = rw ]; then a+=(--bind "$vault" $H/vault); else a+=(--ro-bind "$vault" $H/vault); fi
[ "$extra" != - ] && a+=(--ro-bind "$extra" $H/log)
case $h in
  claude) mkdir -p /tmp/memtest/home-claude
          a+=(--bind $H/.claude/.credentials.json $H/.claude/.credentials.json
              --bind /tmp/memtest/home-claude $H/.claude-state) ;;
  codex)  mkdir -p /tmp/memtest/codex-home
          printf 'model_reasoning_summary = "none"\n' > /tmp/memtest/codex-home/config.toml
          a+=(--bind /tmp/memtest/codex-home $H/.codex --bind $H/.codex/auth.json $H/.codex/auth.json) ;;
  omp)    a+=(--ro-bind $H/.local/share/opencode/auth.json $H/.local/share/opencode/auth.json
              --bind $H/.omp $H/.omp --tmpfs $H/.omp/agent/sessions --tmpfs $H/.omp/logs
              ${OPENCODE_KEY_FILE:+--ro-bind "$OPENCODE_KEY_FILE" "$OPENCODE_KEY_FILE"})
          [ -d $H/.config/opencode ] && a+=(--ro-bind $H/.config/opencode $H/.config/opencode) ;;
esac
exec bwrap "${a[@]}" --chdir $H/vault "$@"
