# Fault drivers

The drivers behind the 10-31 hippo fault review. Each runs throwaway
instances under `/tmp/libri-night/hippo` with their own HOME and HIPPO_DIR, on a
copy of the store named by `HIPPO_FAULT_SEED`, and checks each pid's own
environment before signalling it. The seed must be a synthetic store, never a
copy of anyone's memory; with none set, the drivers start from an empty one. Ports 9180-9199.

    a2_corrupt.py   damaged store files
    a3_kill.py      kill -9 at exact system calls, by strace injection
    a4_compactor.py the compactor slow, hung, refusing, lying
    a6_disk.py      disk and inodes full (run under `unshare -Urm`)
    a7_input.py     hostile input on the socket, bursts, huge messages
    a8_misc.py      two daemons, clock, permissions, id reuse
    a9_long.py      long outages; x10_run.py, build_x10.py for a store 10x the size
    a10_stall.py    slow posts on a loaded host
    a11_states.py   forget stopped between renames, power-cut state, EIO on load
    c1-c4_*.py      a real Libri and hippo with a stand-in model; c4 is the ride-through

`HIPPO=` chooses the hippo binary, `PKG=` the built package for the chat rigs.
