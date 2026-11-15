#!/bin/sh
# Installs Libri from a release tarball into your home, needing no root and
# no Nix: `sh install.sh ARCHIVE`, or with no argument the tarball's own files
# when this script runs from an unpacked release. ARCHIVE is a path or a URL.
set -eu

prefix=${LIBRI_INSTALL_DIR:-"$HOME/.local/share/libri"}
bin="$HOME/.local/bin"
here=$(cd "$(dirname "$0")" && pwd)

source=${1:-}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
case "$source" in
    "")
        [ -x "$here/bin/libri" ] || {
            echo "install.sh: give a release tarball's path or URL" >&2
            exit 1
        }
        unpacked=$here
        ;;
    http://* | https://*)
        curl -fsSL "$source" -o "$work/libri.tar.gz"
        tar -xzf "$work/libri.tar.gz" -C "$work"
        unpacked=$(dirname "$(dirname "$(find "$work" -path '*/bin/libri')")")
        ;;
    *)
        tar -xzf "$source" -C "$work"
        unpacked=$(dirname "$(dirname "$(find "$work" -path '*/bin/libri')")")
        ;;
esac

machine=$(uname -sm | tr ' ' '-')
built=$(cat "$unpacked/machine")
[ "$machine" = "$built" ] || {
    echo "install.sh: this release is for $built, not $machine" >&2
    exit 1
}

mkdir -p "$prefix" "$bin"
rm -rf "$prefix/bin" "$prefix/share"
cp -R "$unpacked/bin" "$unpacked/share" "$prefix/"
ln -sf "$prefix/bin/libri" "$prefix/bin/hippo" "$prefix/bin/wiki" "$bin/"

echo "Libri is in $prefix."
case ":$PATH:" in
    *":$bin:"*) ;;
    *) echo "$bin is not on your PATH: add it to your shell's profile." ;;
esac
echo "Run it now with \`$bin/libri\`, or keep it running with \`$bin/libri service install\`."
"$prefix/bin/libri" host >/dev/null
