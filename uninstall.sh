#!/usr/bin/env bash
# Removes what install.sh installed. Saved mouse settings and the udev rule are kept unless
# you say otherwise.
set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/glorious-linux"
RULE="/etc/udev/rules.d/70-glorious-linux.rules"

ask() {
    local reply
    printf '%s [y/N] ' "$1"
    read -r reply < /dev/tty || reply=""
    [[ "$reply" =~ ^[Yy] ]]
}

rm -f "$PREFIX/bin/glorious" "$DATA/applications/glorious-linux.desktop" "$DATA/icons/hicolor/scalable/apps/glorious-linux.svg"
echo "Removed glorious, its menu entry and icon."

if [ -x "$PREFIX/bin/inglorious-core" ] && ask "Also remove the keyboard configurator (inglorious-core)?"; then
    rm -f "$PREFIX/bin/inglorious-core"
fi
if [ -d "$CONFIG" ] && ask "Delete saved mouse settings in $CONFIG?"; then
    rm -rf "$CONFIG"
fi
if [ -f "$RULE" ] && ask "Remove the udev rule (needs sudo)?"; then
    sudo rm -f "$RULE"
    sudo udevadm control --reload
fi
echo "Done."
