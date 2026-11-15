#!/bin/sh
# Runs a command under the host's check locks, in one of two modes.
#
#   nix/with_check_lock.sh --build COMMAND [ARG...]
#   nix/with_check_lock.sh --timing COMMAND [ARG...]
#   nix/with_check_lock.sh --timing-only COMMAND [ARG...]
#
# --build: nix builds, `nix flake check`, cargo and clippy, gradle. These run
# side by side; each takes the builds lock shared, only so a timing run can
# wait for them to finish.
# --timing: work that times or watches the page (the browser checks, the 400
# ms gate, emulator drives). One runs at a time, under the timing lock, and
# while it runs it holds the builds lock exclusively: it waits for running
# builds to finish and builds started meanwhile wait for it. Loaded by
# parallel builds, the browser checks time out on page loads that take
# seconds alone (0.9.78's second cut: check_call_chat's first load passed 30 s
# with builds free to run beside it, and passed 3 of 3 alone). Each browser
# check stops at LIBRI_CHECK_LIMIT, so the browser checks hold the builds
# lock for minutes, not hours.
# --timing-only: the timing lock alone, for a timing run that builds nothing
# once started and runs for long (the 400 ms gate, half an hour): holding the
# builds lock that long stops every build on the host. Builds may run beside
# it; the gate times a miss again alone before it fails.
#
# LIBRI_TIMING_LOCK and LIBRI_BUILD_LOCK name the lock files (default
# /tmp/libri-check.lock and /tmp/libri-build.lock). A command run under a lock
# gets LIBRI_TIMING_HELD=1 or LIBRI_BUILD_HELD=1, and what it starts runs
# without taking the lock again: a timing run may build, a build may build.
set -eu
mode=${1:-}
case $mode in
--build | --timing | --timing-only) shift ;;
*)
  echo "usage: nix/with_check_lock.sh --build|--timing|--timing-only COMMAND [ARG...]" >&2
  exit 2
  ;;
esac
timing=${LIBRI_TIMING_LOCK:-/tmp/libri-check.lock}
builds=${LIBRI_BUILD_LOCK:-/tmp/libri-build.lock}
if [ -n "${LIBRI_TIMING_HELD:-}" ]; then
  exec "$@"
fi
if [ "$mode" = --build ]; then
  if [ -n "${LIBRI_BUILD_HELD:-}" ]; then
    exec "$@"
  fi
  # Past the timing lock first: a timing run holds it while it waits for the
  # running builds, so a build started then waits too instead of keeping the
  # builds lock shared, and the timing run's turn, from ever coming.
  flock -x "$timing" true
  LIBRI_BUILD_HELD=1
  export LIBRI_BUILD_HELD
  exec flock -s "$builds" "$@"
fi
if [ -n "${LIBRI_BUILD_HELD:-}" ]; then
  # Holding the builds lock shared, waiting for it exclusively never ends.
  echo "with_check_lock: a timing run can't start inside a build" >&2
  exit 2
fi
LIBRI_TIMING_HELD=1
export LIBRI_TIMING_HELD
if [ "$mode" = --timing-only ]; then
  exec flock -x "$timing" "$@"
fi
exec flock -x "$timing" flock -x "$builds" "$@"
