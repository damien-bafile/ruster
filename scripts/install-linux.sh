#!/usr/bin/env bash
# Install the Linux desktop entry and hicolor icons for the current user.
#
# Deliberately per-user (`$XDG_DATA_HOME`, default `~/.local/share`) rather than
# /usr/share: no sudo, nothing to uninstall with a package manager, and it works
# on a machine where ruster was built rather than packaged.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"

install -Dm644 "$root/assets/ruster.desktop" "$data/applications/ruster.desktop"

for png in "$root"/assets/hicolor/ruster-*.png; do
  size="$(basename "$png" .png)"; size="${size#ruster-}"
  install -Dm644 "$png" "$data/icons/hicolor/${size}x${size}/apps/ruster.png"
done

# Best-effort: the desktop entry works without these, they just make it appear
# without a re-login.
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -qtf "$data/icons/hicolor" || true

echo "Installed ruster.desktop and $(ls "$root"/assets/hicolor/ruster-*.png | wc -l | tr -d ' ') icon sizes to $data"
echo "Note: Exec=ruster requires the binary on PATH."
