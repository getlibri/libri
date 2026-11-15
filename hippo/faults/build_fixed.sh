#!/bin/sh
# A build: never beside a timing run (nix/with_check_lock.sh).
cd "$(dirname "$0")/../.." || exit 1
exec nix/with_check_lock.sh --build nice -n 10 sh -c '
nix build .#checks.x86_64-linux.daemon -o /tmp/libri-night/hippo/check-daemon -L 2>&1 | tail -40
echo CHECK_DONE
nix build .#libri -o /tmp/libri-night/hippo/pkg-fixed 2>&1 | tail -10
echo PKG_DONE'
