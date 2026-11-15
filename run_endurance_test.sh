#!/bin/sh
# Builds a commit of dev, boots a throwaway VM on the previous build, upgrades
# it to dev and rolls it back, runs the soak and the faults on the stand-in
# harness, writes report.md, and wipes the VM.
# Usage: ./run_endurance_test.sh [minutes] [ref] [previous ref]
set -eu

# All in one function: sh reads a script as it runs, so an edit during a long
# run would otherwise land in the middle of it.
main() {

minutes=${1:-60}
ref=${2:-dev}
previous=${3:-origin/main}
base=/var/tmp/libri-endurance
# Stops the VM below this much available memory, in GiB. 8 protects a shared
# workstation; a dedicated 16GB build machine can set 3.
floor=${ENDURANCE_FLOOR:-8}
# Stops the VM when the group it runs in holds this much unreclaimable memory,
# in GiB, for a run inside a service whose MemoryHigh would otherwise throttle
# the chat itself; a slice of its own removes this. The guest's memory is
# shared with virtiofsd, so it counts as shmem, not anon.
group_limit=${ENDURANCE_GROUP_LIMIT:-10}
# Seconds between faults.
fault_every=${ENDURANCE_FAULT_EVERY:-180}

available() { awk '/^MemAvailable/ { print int($2 / 1048576) }' /proc/meminfo; }
group_held() {
  awk '/^(anon|shmem) / { n += $2 } END { print int(n / 1073741824) }' \
    "/sys/fs/cgroup$(cut -d: -f3 /proc/self/cgroup)/memory.stat" 2>/dev/null || echo 0
}

repo=$(cd "$(git rev-parse --git-common-dir)/.." && pwd)
rev=$(git rev-parse "$ref")
previous_rev=$(git rev-parse "$previous^{commit}")
mkdir -p "$base"
exec 9>"$base/lock"
flock -n 9 || {
  echo "another endurance run holds $base/lock" >&2
  exit 1
}
[ "$(available)" -ge $((floor + 6)) ] || {
  echo "only $(available) GiB available; a run needs $((floor + 6))" >&2
  exit 1
}

run=$base/$(date +%Y%m%d-%H%M%S)-$(git rev-parse --short "$rev")
mkdir -p "$run/shared" "$run/vm"
nice -n 19 nix build --impure -o "$run/build" --expr "
  let
    flake = rev: builtins.getFlake \"git+file://$repo?rev=\${rev}\";
    system = \"x86_64-linux\";
  in
  (flake \"$rev\").packages.\${system}.endurance.override {
    previous = (flake \"$previous_rev\").packages.\${system}.libri;
  }"

# A listener on this host's loopback that the VM must never reach.
canary=$run/canary
python3 -c '
import socket, sys
s = socket.socket()
s.bind(("127.0.0.1", 0))
s.listen()
open(sys.argv[1] + ".port", "w").write(str(s.getsockname()[1]))
while True:
    s.accept()[0].close()
    open(sys.argv[1] + ".hits", "a").write("hit\n")
' "$canary" &
canary_pid=$!
until [ -s "$canary.port" ]; do sleep 0.1; done

cat >"$run/shared/plan.json" <<PLAN
{"minutes": $minutes, "fault_every_seconds": $fault_every, "rev": "$rev", "previous": "$previous_rev", "seed_turns": ${ENDURANCE_SEED_TURNS:-120}, "canary_port": $(cat "$canary.port")}
PLAN

SHARED_DIR=$run/shared USE_TMPDIR=1 TMPDIR=$run/vm NIX_DISK_IMAGE=$run/vm/disk.qcow2 \
  nice -n 19 "$run/build/bin/run_endurance_vm" >"$run/console.log" 2>&1 </dev/null &
vm=$!
trap 'kill "$vm" "$canary_pid" 2>/dev/null || true' EXIT

# The soak, a slow fault at its end, and the load scenarios.
deadline=$(($(date +%s) + minutes * 60 + 3600))
stopped=
while kill -0 "$vm" 2>/dev/null; do
  now=$(date +%s)
  if [ "$(available)" -lt "$floor" ]; then
    stopped="available memory fell under $floor GiB"
  elif [ "$(group_held)" -ge "$group_limit" ]; then
    stopped="its memory group held $group_limit GiB it could not reclaim"
  elif [ "$now" -gt "$deadline" ]; then
    stopped="the suite overran its deadline"
  fi
  if [ -n "$stopped" ]; then
    echo "$stopped" >"$run/stopped"
    kill "$vm"
    break
  fi
  sleep 10
done
wait "$vm" || true

"$run/build/bin/write_endurance_report" "$run"
rm -rf "$run/vm"
echo "$run/report.md"
}

main "$@"
