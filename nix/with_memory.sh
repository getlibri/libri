#!/bin/sh
# Starts a heavy job once there is room for it, so jobs run side by side
# without pushing the host into swap.
#
#   nix/with_memory.sh GIB [--emulator] COMMAND [ARG...]
#
# Waits until both the host's MemAvailable and the headroom left in this
# process's own cgroup (memory.max less memory.current, plus the inactive
# page cache the kernel would drop first, where a cap is set, as on Libri's
# harness cgroup) exceed GIB, the job's usual peak, plus a margin
# (LIBRI_MEMORY_MARGIN_GIB, default 4), then runs COMMAND. Usual
# peaks: an Android emulator 5 (a 3 GB guest and QEMU), Gradle 6 (a 4 GB
# heap and the Kotlin daemon), a browser-check run 3 (Chromium under its
# 4 GB cap and a throwaway Libri), Libri's nix build 8 (a release rustc
# peaks at 4.4 GB beside the rest).
#
# --emulator also takes one of LIBRI_EMULATORS_MOST (default 3) emulator
# slots, held while COMMAND runs, waiting for one to free up.
#
# It gives up after LIBRI_MEMORY_WAIT seconds (default 1800) with exit 75.
set -eu
need=${1:?usage: nix/with_memory.sh GIB [--emulator] COMMAND [ARG...]}
shift
if [ "${1:-}" = --emulator ]; then
  shift
  most=${LIBRI_EMULATORS_MOST:-3}
  while :; do
    i=1
    while [ "$i" -le "$most" ]; do
      # A slot is a lock file held for the run; -E 75 tells "taken" from
      # the job's own failure.
      status=0
      flock -n -E 75 "/tmp/libri-emulator-$i.lock" "$0" "$need" "$@" || status=$?
      [ "$status" -eq 75 ] || exit "$status"
      i=$((i + 1))
    done
    echo "with_memory: all $most emulator slots are taken; waiting" >&2
    sleep 15
  done
fi
margin=${LIBRI_MEMORY_MARGIN_GIB:-4}
limit=${LIBRI_MEMORY_WAIT:-1800}
want=$(((need + margin) * 1024 * 1024))
room() {
  host=$(awk '/^MemAvailable:/ { print $2 }' /proc/meminfo)
  group=$(sed -n 's/^0:://p' /proc/self/cgroup)
  max=$(cat "/sys/fs/cgroup$group/memory.max" 2>/dev/null || echo max)
  if [ "$max" != max ]; then
    used=$(cat "/sys/fs/cgroup$group/memory.current")
    # memory.current counts page cache, which the kernel drops before it
    # holds the cgroup at its cap; count the inactive part as room, as
    # MemAvailable does for the host.
    spare=$(awk '/^inactive_file / { print $2 }' "/sys/fs/cgroup$group/memory.stat")
    left=$(((max - used + spare) / 1024))
    [ "$left" -lt "$host" ] && host=$left
  fi
  echo "$host"
}
waited=0
while [ "$(room)" -lt "$want" ]; do
  if [ "$waited" -ge "$limit" ]; then
    echo "with_memory: no room for ${need}G (+${margin}G) after ${limit}s; not started" >&2
    exit 75
  fi
  [ $((waited % 60)) -eq 0 ] && echo "with_memory: waiting for ${need}G (+${margin}G) free; $(($(room) / 1048576))G now" >&2
  sleep 5
  waited=$((waited + 5))
done
exec "$@"
