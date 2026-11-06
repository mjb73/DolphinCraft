#!/bin/sh
# Build the Linux archives that go on a DolphinCraft release.
#
#     packaging/linux-tarball.sh <version> <arch> [binary]
#
# Writes dist/dolphincraft-<version>-<arch>-linux.tar.gz and the identical .zip, each with a
# .sha256 beside it. The tarball is what install.sh downloads; the zip is there for people who
# would rather unzip than untar.
#
# The version is the release without its leading "v" (0.1.2 for tag v0.1.2), the arch is
# x86_64 or aarch64. The workflow runs this inside a Debian bookworm container on purpose:
# the glibc a binary is linked against becomes the glibc every user needs, and an archive built
# on a rolling-release host links a glibc that nothing else can start.
set -eu

VERSION=${1:?usage: linux-tarball.sh <version> <arch> [binary]}
ARCH=${2:?usage: linux-tarball.sh <version> <arch> [binary]}

here=$(cd "$(dirname "$0")/.." && pwd)
BIN=${3:-$here/target/release/dolphincraft}
NAME="dolphincraft-$VERSION-$ARCH-linux"
OUT=$here/dist

[ -x "$BIN" ] || {
    echo "no executable at $BIN — run 'cargo build --release' first, or pass the binary path" >&2
    exit 1
}

rm -rf "$OUT/$NAME" "$OUT/$NAME.tar.gz" "$OUT/$NAME.zip" \
    "$OUT/$NAME.tar.gz.sha256" "$OUT/$NAME.zip.sha256"
mkdir -p "$OUT/$NAME"
install -m 755 "$BIN" "$OUT/$NAME/dolphincraft"
install -m 644 "$here/packaging/dolphincraft.desktop" "$OUT/$NAME/dolphincraft.desktop"
install -m 644 "$here/README.md" "$OUT/$NAME/README.md"
if [ -f "$here/LICENSE" ]; then
    install -m 644 "$here/LICENSE" "$OUT/$NAME/LICENSE"
fi

tar -C "$OUT" -czf "$OUT/$NAME.tar.gz" "$NAME"
if command -v zip >/dev/null 2>&1; then
    ( cd "$OUT" && zip -qr "$NAME.zip" "$NAME" )
else
    echo "no zip on PATH — the .zip is skipped, only the tarball is built" >&2
fi
rm -rf "$OUT/$NAME"

(
    cd "$OUT"
    sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256"
    if [ -f "$NAME.zip" ]; then
        sha256sum "$NAME.zip" > "$NAME.zip.sha256"
    fi
)

ls -l "$OUT/$NAME"*
