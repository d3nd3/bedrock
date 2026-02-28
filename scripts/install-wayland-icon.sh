#!/bin/sh
# Install desktop file and icon so Wayland taskbar shows the Bedrock diamond icon.
# Run from repo root: ./scripts/install-wayland-icon.sh
set -e
REPO="$(cd "$(dirname "$0")/.." && pwd)"
ICON_SRC="$REPO/src-tauri/icons/128x128.png"
DESKTOP_SRC="$REPO/src-tauri/com.bedrock.notes.desktop"
ICON_DIR="$HOME/.local/share/icons/hicolor/128x128/apps"
APPS_DIR="$HOME/.local/share/applications"
mkdir -p "$ICON_DIR" "$APPS_DIR"
cp "$ICON_SRC" "$ICON_DIR/com.bedrock.notes.png"
cp "$DESKTOP_SRC" "$APPS_DIR/com.bedrock.notes.desktop"
echo "Installed. Restart cargo tauri dev for the taskbar icon to update."
