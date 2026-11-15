# Libri

- Libri holds the product only, a standalone project with no knowledge of any
  one install. Hosts, accounts, people and paths come from Libri's own
  documented settings, with neutral defaults; tests use placeholder names
  (Ada, Bob, Kit, box, libri.example.com). Deploying Libri to a particular
  set of machines, and proving it there, lives with that deployment, which
  configures Libri from outside as it would any package. `check_changes.sh`
  fails on names and paths of a particular install.
- Check before committing: `./check_changes.sh` builds the checks covering
  what changed since origin/main (a crate's tests and clippy with warnings
  denied, rustfmt, the page's typecheck, tests and build).
- `nix flake check` runs every check, including end-to-end runs of the
  release build; run it before a merge to main. Builds run side by side;
  what times or watches the page must not run beside them. Run builds
  through `nix/with_check_lock.sh --build` (`nix flake check -L`, cargo,
  clippy, gradle) and timing work through `--timing` (the browser checks,
  the 400 ms gate): one timing run at a time, with no build beside it.
  Heavy jobs run side by side but start only when there is room:
  `nix/with_memory.sh GIB` waits until the host and the cgroup have the
  job's usual peak free (Gradle 6, an emulator 5 with `--emulator`, at
  most three at once). `check_changes.sh`, the `browser-checks` and
  `ui-latency` apps, the Android drives and the Android release build take
  their lock and wait for room themselves. Every emulator or browser
  drive, including one run by hand, goes through
  `nix/with_check_lock.sh --timing`. Each browser check fails once it runs
  past `LIBRI_CHECK_LIMIT` seconds (600 by default) instead of hanging.
- A drive sets a short `TMPDIR` (`/tmp/<name>`): Chromium, the emulator and
  the throwaway daemons put sockets there, and a socket path longer than
  107 bytes fails to bind.
- Tests never touch a real hippo store or a real harness login; the
  end-to-end checks run throwaway daemons with stand-in harnesses.
- `nix/check_fresh_home.py` drives real harnesses and spends real model
  calls, so it stays out of `nix flake check`; run it by hand. So does
  `nix/prove_outside_clients.py`, which connects stock Claude Code and Codex
  to a throwaway Libri.
- The UI driver measures the page for speed and stability work: `nix run
  .#ui-driver` (from the checkout) drives a throwaway Libri with a seeded
  long chat in Chromium at desktop and phone sizes, prints a scorecard,
  writes `ui-driver-run/run.json`, and compares pictures with
  `nix/ui_goldens/` (`--update-goldens` after a change meant to alter them).
  `nix run .#ui-driver -- --compare before.json after.json` shows the change.
  `nix/smoke_android.py` is the app's smoke run, inside
  `nix develop .#android-emulator`. Both stay out of `nix flake check`.
- `nix run .#ui-latency` times every interaction the page offers, found by
  clicking through it, against a stand-in host, and fails on any over 400 ms
  unthrottled, a miss timed again alone first; `-- --slow --table
  nix/ui_latency.md` adds a 4x slowed processor and writes the table. Out of
  `nix flake check`, as a timing check beside its builds would time the
  machine; the release runs it after the checks and stops on a miss.
- A stock app run by a test keeps no session on disk (Claude Code
  `--no-session-persistence`, Codex `--ephemeral`): the person's own hippo
  reads those sessions, and a test's words would land in their memory. Build
  rigs on `nix/throwaway_libri.py`, whose `strays()` fails a run that leaves
  its mark outside the scratch root.
- Versions: stay on 0.9.x and bump the last digit for each release
  (`./set_version.sh 0.9.N`) until 0.10.0, the alpha milestone: the iOS app,
  portable cross-platform installs and the full Library. Tag each release
  `vX.Y.Z` on its release commit and add its entry at the top of
  `CHANGELOG.md`. Never bump the minor or major version without the
  maintainer's word.
