#!/usr/bin/env bash
# Builds the signed Android release from this checkout and keeps it beside the
# signing key as libri-<versionName>.apk; it stops before publishing. Bump the
# version with set_version.sh first. LIBRI_ANDROID_KEYS names the directory
# holding release.p12 and release.password.
set -euo pipefail
S=${LIBRI_ANDROID_KEYS:-${XDG_DATA_HOME:-$HOME/.local/share}/libri-android}
cd "$(dirname "$0")"
version=$(sed -n '0,/^version = /s/^version = "\(.*\)"/\1/p' libri/Cargo.toml)
[ -n "$version" ] || { echo "no version in libri/Cargo.toml"; exit 1; }
export LIBRI_KEYSTORE=$S/release.p12
LIBRI_KEYSTORE_PASSWORD=$(cat "$S/release.password"); export LIBRI_KEYSTORE_PASSWORD
(cd android && ../nix/with_check_lock.sh --build ../nix/with_memory.sh 6 nix develop ..#android --command gradle --no-daemon assembleRelease)
out=$S/libri-$version.apk
cp android/app/build/outputs/apk/release/app-release.apk "$out"
echo "built $out"
