#!/usr/bin/env bash
# Builds one scratch hippo tree from a read-only copy of the log.
# run_tree.sh NAME BACKEND MODEL EFFORT DAY...   (BACKEND claude|codex|omp)
# The store lives in /tmp/memtest/trees/NAME; hippo serve runs in a clean
# environment with its own HIPPO_DIR, never the live one.
set -euo pipefail
name=$1 backend=$2 model=$3 effort=$4; shift 4
root=/tmp/memtest; dir=$root/trees/$name
HIPPO=${HIPPO:?path to the branch hippo binary}
mkdir -p "$dir/main" "$root/empty"
for d in "$@"; do [ -e "$dir/main/$d.jsonl" ] || cp "${SRC:-$root/renum}/$d.jsonl" "$dir/main/"; done
# HIPPO_JOBS: the tree's compactor calls at once (6 for a tree alone;
# build_trees.sh gives each of its six 1).
envs=(HOME=$HOME PATH=$PATH HIPPO_JOBS=${HIPPO_JOBS:-6} HIPPO_DIR=$dir HIPPO_RULES_FILE=$root/rules.md HIPPO_MODEL=$model HIPPO_EFFORT=$effort)
case $backend in
  omp) port=${OMP_PORT:?a free port for the omp adapter}
       python3 -I "$(dirname "$0")/omp_shim.py" "$port" "$dir/omp-ledger.jsonl" "$root/empty" &
       echo $! > "$dir/shim.pid"
       envs+=(HIPPO_BACKEND=http HIPPO_URL=http://127.0.0.1:$port) ;;
  *) envs+=(HIPPO_BACKEND=$backend) ;;
esac
cd "$root/empty"
exec env -i "${envs[@]}" "$HIPPO" serve chat >> "$dir/serve.log" 2>&1
