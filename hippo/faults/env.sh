# Source me. Every test instance lives under $NIGHT/run/<name> with its own
# HOME, XDG dirs and HIPPO_DIR; nothing here may name the live store.
NIGHT=/tmp/libri-night/hippo
HIPPO=${HIPPO:-$NIGHT/build/debug/hippo}

# fresh NAME [SEED]: a new instance from a reflink copy of the seed.
fresh() {
  local n=$1 seed=${2:-${HIPPO_FAULT_SEED:-none}}
  R=$NIGHT/run/$n
  [ -d "$R" ] && stop "$n"
  rm -rf "$R"
  mkdir -p "$R"/{home,cache,config,data,runtime}
  if [ "$seed" = none ]; then mkdir -p "$R/store"
  else cp -a --reflink=auto "$seed" "$R/store"; fi
  echo ok > "$R/mode"
  S=$R/store
}

# run NAME CMD...: runs a command in the instance's whole environment.
inst_env() {
  local R=$NIGHT/run/$1
  shift
  env -i PATH="$PATH" HOME=$R/home XDG_CACHE_HOME=$R/cache XDG_CONFIG_HOME=$R/config \
    XDG_DATA_HOME=$R/data XDG_RUNTIME_DIR=$R/runtime HIPPO_DIR=$R/store \
    HIPPO_AGENT=Iris HIPPO_BACKEND=${BACKEND-libri} "$@"
}

# start NAME [serve-mode]: starts hippo after confirming it cannot touch live.
start() {
  local n=$1 mode=${2:-code} R=$NIGHT/run/$1
  case "$R" in /tmp/libri-night/hippo/run/*) ;; *) echo refuse; return 1 ;; esac
  inst_env "$n" setsid "$HIPPO" serve $mode >>"$R/hippo.log" 2>&1 &
  echo $! >"$R/hippo.pid"
  for _ in $(seq 1 ${WAIT_TICKS:-3000}); do
    [ -S "$R/store/hippo.sock" ] && hc "$n" health >/dev/null 2>&1 && break
    kill -0 "$(cat "$R/hippo.pid")" 2>/dev/null || { echo "died: $(tail -1 "$R/hippo.log")"; return 1; }
    sleep 0.05
  done
}

stub() {
  local R=$NIGHT/run/$1
  python3 "$NIGHT/repro/compactor_stub.py" "$R/store" "$R/mode" >>"$R/stub.log" 2>&1 &
  echo $! >"$R/stub.pid"
  for _ in $(seq 1 100); do [ -S "$R/store/compactor.sock" ] && break; sleep 0.05; done
}

# mine PID: true only when the pid's HIPPO_DIR or argv is under the night dir.
mine() {
  [ -r /proc/$1/environ ] && tr '\0' '\n' </proc/$1/environ | grep -q "^HIPPO_DIR=$NIGHT/run/" && return 0
  tr '\0' ' ' </proc/$1/cmdline 2>/dev/null | grep -q "$NIGHT/"
}

sig() { # sig NAME SIGNAL
  local p; p=$(cat "$NIGHT/run/$1/hippo.pid")
  mine "$p" && kill -"$2" "$p" || echo "not mine: $p"
}

stop() {
  local R=$NIGHT/run/$1 p
  for f in hippo.pid stub.pid; do
    p=$(cat "$R/$f" 2>/dev/null) || continue
    if mine "$p"; then kill -CONT "$p" 2>/dev/null; kill -9 "$p" 2>/dev/null; fi
    rm -f "$R/$f"
  done
  sleep 0.1
}

# hc NAME ARGS...: the hippo CLI aimed at the instance, as a client.
hc() {
  local n=$1; shift
  inst_env "$n" timeout ${HC_WAIT:-10} "$HIPPO" "$@"
}

# raw NAME JSON: sends one request line and prints the reply, with a timeout.
raw() {
  timeout ${HC_WAIT:-10} python3 - "$NIGHT/run/$1/store/hippo.sock" "$2" <<'E'
import socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
s.sendall(sys.argv[2].encode() + b"\n")
b = b""
while not b.endswith(b"\n"):
    g = s.recv(1 << 20)
    if not g: break
    b += g
print(b.decode(errors="replace"))
E
}
