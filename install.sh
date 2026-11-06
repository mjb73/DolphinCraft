#!/bin/sh
# Install DolphinCraft on Linux from the latest GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/mjb73/DolphinCraft/main/install.sh | sh
#
# Installs into ~/.local (no root, no package manager) and puts a DolphinCraft entry on the
# application menu with an absolute path, so the desktop's own Exec does not depend on PATH.
# Set PREFIX to install somewhere else, or DOLPHINCRAFT_VERSION to pin a release.
set -eu

REPO=${DOLPHINCRAFT_REPO:-mjb73/DolphinCraft}
PREFIX=${PREFIX:-$HOME/.local}
VERSION=${DOLPHINCRAFT_VERSION:-}

WORK=$(mktemp -d "${TMPDIR:-/tmp}/dolphincraft-install.XXXXXX")
trap 'rm -rf "$WORK"' EXIT INT TERM

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        echo "note: need curl or wget to download DolphinCraft" >&2
        exit 1
    fi
}

case "$(uname -s)" in
    Linux) ;;
    *) echo "note: this script is for Linux — DolphinCraft has no other build yet" >&2; exit 1 ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) ARCH=x86_64 ;;
    aarch64 | arm64) ARCH=aarch64 ;;
    *) echo "note: no prebuilt DolphinCraft for $(uname -m) — build it with cargo" >&2; exit 1 ;;
esac

if [ -z "$VERSION" ]; then
    # /releases/latest redirects to /releases/tag/vX.Y.Z; read the tag off the final URL.
    if command -v curl >/dev/null 2>&1; then
        FINAL=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")
    else
        FINAL=$(wget -qS --spider "https://github.com/${REPO}/releases/latest" 2>&1 | awk '/Location:/{print $2}' | tail -1)
    fi
    VERSION=$(printf '%s' "$FINAL" | sed -n 's#.*/tag/##p')
    [ -n "$VERSION" ] || { echo "note: could not work out the latest DolphinCraft version" >&2; exit 1; }
fi

NUM=${VERSION#v}
ASSET="dolphincraft-${NUM}-${ARCH}-linux.tar.gz"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"

echo "installing DolphinCraft ${VERSION} (${ARCH}) into ${PREFIX}"
fetch "$URL" "$WORK/$ASSET"
if fetch "$URL.sha256" "$WORK/$ASSET.sha256" 2>/dev/null; then
    ( cd "$WORK" && sha256sum -c "$ASSET.sha256" >/dev/null ) || {
        echo "note: checksum mismatch — the download is corrupt, nothing was installed" >&2
        exit 1
    }
else
    echo "note: no .sha256 published for this release, skipping the checksum check" >&2
fi

tar -C "$WORK" -xzf "$ASSET"

mkdir -p "$PREFIX/bin" "$PREFIX/share/applications"
install -m 755 "$WORK/${ASSET%.tar.gz}/dolphincraft" "$PREFIX/bin/dolphincraft"

BIN="$PREFIX/bin/dolphincraft"
cat > "$PREFIX/share/applications/dolphincraft.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Version=1.0
Name=DolphinCraft
GenericName=File Manager
Comment=A file manager with extra tools
Exec="$BIN" %U
TryExec=$BIN
Icon=system-file-manager
Terminal=false
StartupNotify=true
StartupWMClass=dolphincraft
Categories=System;FileTools;FileManager;
Keywords=files;folder;manager;browse;dolphin;
MimeType=inode/directory;
DESKTOP

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$PREFIX/share/applications" >/dev/null 2>&1 || true
command -v kbuildsycoca6 >/dev/null 2>&1 && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true

echo
echo "DolphinCraft ${VERSION} installed:"
echo "  binary  $BIN"
echo "  menu    $PREFIX/share/applications/dolphincraft.desktop"
case ":$PATH:" in
    *":$PREFIX/bin:"*) echo "  run     dolphincraft" ;;
    *) echo "  run     $BIN    (add $PREFIX/bin to PATH to just type dolphincraft)" ;;
esac
echo
echo "It needs a Wayland or X11 session and the usual OpenGL bits (libglvnd, mesa)."
