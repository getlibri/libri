#!/usr/bin/env bash
# Builds the six stage-1 hippo trees at once over the Nov 1-5 replay slice.
# Runs in the foreground and waits, so its caller keeps all children alive.
set -u
here=$(dirname "$(readlink -f "$0")")
# One compactor call at a time per tree, six at once in all: the host's load
# passed 300 on 48 threads with 16 a tree (96 at once).
export HIPPO HIPPO_JOBS=${HIPPO_JOBS:-1}
days="2026-11-01 2026-11-02 2026-11-03 2026-11-04 2026-11-05"
run() { setsid "$here/run_tree.sh" "$@" $days & }
run sol-low    codex  gpt-6.1-sol   low
run luna-max   codex  gpt-6-luna    max
run terra-low  codex  gpt-6.1-terra low
run haiku-high claude haiku         high
OMP_PORT=7721 run flash-low  omp opencode-go/deepseek-v4.1-flash low
OMP_PORT=7722 run flash-high omp opencode-go/deepseek-v4.1-flash high
wait
