#!/bin/sh
# Put DolphinCraft behind the desktop portal's file chooser, for this user only.
#
# A program asks xdg-desktop-portal for a file; the portal asks whichever
# backend `portals.conf` names for `org.freedesktop.impl.portal.FileChooser`.
# This script installs DolphinCraft's backend, points that one interface at it,
# and leaves every other interface (settings, secrets, notifications) on the
# desktop's own backend.
#
#   sh install-portal.sh                # install, using the dolphincraft on PATH
#   sh install-portal.sh /path/to/bin   # install, using that binary
#   sh install-portal.sh --uninstall    # put the portal back as it was
#
# Nothing here needs root: every file goes under $XDG_DATA_HOME, $XDG_CONFIG_HOME
# or ~/.local.
set -eu

DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CONF="${XDG_CONFIG_HOME:-$HOME/.config}"
HERE=$(cd "$(dirname "$0")" && pwd)
SERVICE_NAME="org.freedesktop.impl.portal.desktop.dolphincraft"
DEST_PORTAL="$DATA/xdg-desktop-portal/portals/dolphincraft.portal"
DEST_BACKEND="$DATA/dolphincraft/dolphincraft-portal"
DEST_SERVICE="$DATA/dbus-1/services/$SERVICE_NAME.service"
DEST_CONF="$CONF/xdg-desktop-portal/portals.conf"
BACKUP="$DEST_CONF.dolphincraft-backup"

say() { printf '%s\n' "$*"; }

restart_portal() {
    if [ -n "${DOLPHINCRAFT_PORTAL_NO_RESTART:-}" ]; then
        say "not restarting xdg-desktop-portal (DOLPHINCRAFT_PORTAL_NO_RESTART is set)"
        return 0
    fi
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user restart xdg-desktop-portal 2>/dev/null &&
            say "xdg-desktop-portal restarted (it re-read portals.conf)" ||
            say "could not restart xdg-desktop-portal; log out and back in"
    else
        say "no systemd here: log out and back in for the portal to re-read its config"
    fi
}

uninstall() {
    rm -f "$DEST_PORTAL" "$DEST_SERVICE" "$DEST_BACKEND" "$(dirname "$DEST_BACKEND")/picker-path"
    if [ -f "$DEST_CONF.dolphincraft-backup" ]; then
        mv "$DEST_CONF.dolphincraft-backup" "$DEST_CONF"
        say "restored the portals.conf that was there before"
    else
        rm -f "$DEST_CONF"
        say "removed portals.conf (the desktop's own one applies again)"
    fi
    restart_portal
    say "DolphinCraft is no longer the portal's file dialog"
    exit 0
}

[ "${1:-}" = "--uninstall" ] && uninstall

# ---- the picker binary ---------------------------------------------------
BIN="${1:-${DOLPHINCRAFT_BIN:-}}"
if [ -z "$BIN" ]; then
    BIN=$(command -v dolphincraft || true)
fi
if [ -z "$BIN" ] && [ -x "$HERE/../../target/release/dolphincraft" ]; then
    BIN="$HERE/../../target/release/dolphincraft"
fi
if [ -z "$BIN" ] || [ ! -x "$BIN" ]; then
    say "no dolphincraft binary found; pass one: sh install-portal.sh /path/to/dolphincraft"
    exit 1
fi
BIN=$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
say "picker: $BIN"

# ---- the backend ---------------------------------------------------------
mkdir -p "$(dirname "$DEST_BACKEND")" "$(dirname "$DEST_PORTAL")" \
         "$(dirname "$DEST_SERVICE")" "$(dirname "$DEST_CONF")"
install -m 0755 "$HERE/dolphincraft-portal" "$DEST_BACKEND"
install -m 0644 "$HERE/dolphincraft.portal" "$DEST_PORTAL"
say "backend: $DEST_BACKEND"
say "portal file: $DEST_PORTAL"

PYTHON=$(command -v /usr/bin/python3 || command -v python3)
cat > "$DEST_SERVICE" <<EOF
[D-BUS Service]
Name=$SERVICE_NAME
Exec=$PYTHON $DEST_BACKEND
EOF
chmod 0644 "$DEST_SERVICE"
# The picker's own path goes beside the backend rather than into the service's
# Exec line: D-Bus splits that line on spaces, and this person's binary lives
# under a folder with one in its name.
printf '%s\n' "$BIN" > "$(dirname "$DEST_BACKEND")/picker-path"
say "service: $DEST_SERVICE"

# A newly written `.service` file is not noticed by a bus that is already
# running: this session's bus is dbus-broker, which reads the service directory
# once when it starts. Until the bus knows the name, the portal's call fails
# with "The name is not activatable" — which reads like a broken backend. One
# ReloadConfig is the difference between working now and working after a login.
reload_bus() {
    if command -v busctl >/dev/null 2>&1; then
        busctl --user call org.freedesktop.DBus /org/freedesktop/DBus \
            org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 &&
            say "session bus reloaded: $SERVICE_NAME is activatable now" ||
            say "could not reload the session bus; log out and back in"
    fi
}
reload_bus

# ---- point the FileChooser interface at it -------------------------------
# Every other interface keeps the backend it had, so the desktop's own
# portals.conf is the base and only the one line this feature is about is
# written. The base is, in order: the copy taken before the first install (a
# second run must not build on its own edit and drop the other lines), whatever
# portals.conf is there now, or the file the desktop ships for itself.
DESKTOP=$(printf '%s' "${XDG_CURRENT_DESKTOP:-}" | cut -d: -f1 | tr '[:upper:]' '[:lower:]')
if [ -z "$DESKTOP" ]; then
    # A shell started by a service or a scheduler carries no desktop variable;
    # the backend that is actually running does say which desktop this is.
    for impl in kde gnome gtk wlr; do
        # `pgrep -x` cannot see these: a process name is cut to fifteen
        # characters, so `xdg-desktop-portal-kde` reads as `xdg-desktop-port`.
        # The full command line is the binary's own path, with nothing after it.
        if pgrep -f "/xdg-desktop-portal-$impl$" >/dev/null 2>&1; then
            DESKTOP="$impl"
            break
        fi
    done
fi

if [ -f "$BACKUP" ]; then
    SOURCE="$BACKUP"
    say "base: the portals.conf kept from before the first install"
elif [ -f "$DEST_CONF" ]; then
    cp "$DEST_CONF" "$BACKUP"
    SOURCE="$DEST_CONF"
    say "base: the portals.conf that was already there (kept as $BACKUP)"
else
    SOURCE=""
    for candidate in \
        "/usr/share/xdg-desktop-portal/${DESKTOP}-portals.conf" \
        "/etc/xdg-desktop-portal/${DESKTOP}-portals.conf" \
        /usr/share/xdg-desktop-portal/portals.conf \
        /etc/xdg-desktop-portal/portals.conf; do
        if [ -n "$DESKTOP" ] && [ -f "$candidate" ]; then
            SOURCE="$candidate"
            break
        fi
    done
    if [ -n "$SOURCE" ]; then
        say "base: $SOURCE"
    else
        say "no portals.conf found for '${DESKTOP:-unknown desktop}'; writing a minimal one"
        printf '[preferred]\ndefault=kde\n' > "$DEST_CONF.tmp"
    fi
fi

if [ -n "$SOURCE" ]; then
    grep -v '^[[:space:]]*org\.freedesktop\.impl\.portal\.FileChooser[[:space:]]*=' "$SOURCE" \
        > "$DEST_CONF.tmp" || true
fi
if ! grep -q '^\[preferred\]' "$DEST_CONF.tmp" 2>/dev/null; then
    printf '[preferred]\ndefault=kde\n' >> "$DEST_CONF.tmp"
fi
printf 'org.freedesktop.impl.portal.FileChooser=dolphincraft\n' >> "$DEST_CONF.tmp"
mv "$DEST_CONF.tmp" "$DEST_CONF"
say "portals.conf: $DEST_CONF"
sed 's/^/    /' "$DEST_CONF"

restart_portal
say ""
say "Done. Programs that use the desktop portal will now get DolphinCraft's dialog."
say "Undo with: sh $0 --uninstall"
