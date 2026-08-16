#!/usr/bin/env bash
# Build ruster and publish it into a local pacman repository.
#
# The point of a repository rather than a bare `makepkg -si` is that pacman then
# treats ruster like any other package: `pacman -Syu` notices a new version,
# upgrades it alongside everything else, and `pacman -Qo` can say which package
# owns a file. A hand-installed package is invisible to all of that.
#
# Run this after pushing; it builds whatever is on the remote's default branch,
# because that is what the PKGBUILD tracks.
set -euo pipefail

REPO_DIR="${RUSTER_REPO_DIR:-$HOME/.local/share/ruster-repo}"
REPO_NAME=ruster
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

mkdir -p "$REPO_DIR"

# Built in a scratch directory so the source tree never accumulates makepkg's
# `src/` and `pkg/` output, which would otherwise land in `git status` and in
# every subsequent `cargo` invocation's search path.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cp "$HERE/PKGBUILD" "$WORK/"

echo "==> building in $WORK"
(
  cd "$WORK"
  # -f to overwrite an existing package of the same version. Not -s: pulling
  # dependencies needs root, and the one build dependency that matters (cargo)
  # comes from rustup rather than from pacman. Not -i either: installing is
  # pacman's job below, through the repository.
  makepkg -f --noconfirm
)

PKG="$(find "$WORK" -maxdepth 1 -name '*.pkg.tar.zst' | head -1)"
if [ -z "$PKG" ]; then
  echo "no package was produced" >&2
  exit 1
fi
echo "==> built $(basename "$PKG")"

cp "$PKG" "$REPO_DIR/"
# --new so an older package of the same name is replaced rather than duplicated;
# --remove so the superseded file is deleted instead of accumulating a copy of
# every build ever made, which for a Rust workspace is tens of megabytes each.
repo-add --new --remove "$REPO_DIR/$REPO_NAME.db.tar.zst" "$REPO_DIR/$(basename "$PKG")"

# Readable by pacman, which runs as root but whose sandboxing (DownloadUser)
# can drop privileges for file:// transfers.
chmod -R a+rX "$REPO_DIR"

cat <<EOF

==> done. $REPO_DIR now holds:
$(ls -1 "$REPO_DIR" | sed 's/^/      /')

If this is the first run, add the repository to /etc/pacman.conf, above
[core] so it takes precedence, then upgrade:

    [$REPO_NAME]
    SigLevel = Optional TrustAll
    Server = file://$REPO_DIR

    sudo pacman -Syu

Thereafter: push, re-run this script, and 'pacman -Syu' picks up the new commit.
EOF
