#!/usr/bin/env bash
# Sets Libri's one version, kept in libri/Cargo.toml: the server reports it,
# the web shows it, and the Android build takes versionName and versionCode
# from it. Bump it once per release, before build_android_release.sh. It
# also drafts the release's changelog entry.
# Usage: set_version.sh 0.9.18
set -euo pipefail
v=$1
[[ $v =~ ^[0-9]+\.[0-9]{1,3}\.[0-9]{1,3}$ ]] || { echo "want major.minor.patch, each part under 1000"; exit 1; }
cd "$(dirname "$0")/libri"
sed -i "0,/^version = \".*\"/s//version = \"$v\"/" Cargo.toml
# Cargo.lock names the crate's own version too; nothing else in it changes.
sed -i "/^name = \"libri\"$/{n;s/^version = \".*\"/version = \"$v\"/}" Cargo.lock
grep -q "^version = \"$v\"" Cargo.toml && grep -A1 '^name = "libri"$' Cargo.lock | grep -q "\"$v\"" ||
  { echo "version not set"; exit 1; }
# The release's entry in the changelog, which the app shows as What's new:
# drafted from the commits since the last tag when it has none, to be
# rewritten in the person's terms before the release commit.
log=CHANGELOG.md
if ! grep -q "^## $v " "$log"; then
  last=$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)
  draft=$(git log --no-merges --format='- %s' ${last:+"$last"..HEAD})
  [[ -n $draft ]] || draft="- "
  awk -v head="## $v ($(date +%F))" -v draft="$draft" '
    !done && /^## / { print head "\n\n" draft "\n"; done = 1 }
    { print }
    END { if (!done) print "\n" head "\n\n" draft }
  ' "$log" > "$log.new" && mv "$log.new" "$log"
  echo "Drafted the $v entry in libri/$log from the commits${last:+ since $last}: rewrite it before the release commit"
fi
echo "Libri is $v"
