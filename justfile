# ruster — justfile
#
# `just` on its own lists what is here. The list is generated from the doc
# comment above each recipe, so it cannot fall out of step with the recipes the
# way a hand-written index does — this header used to name nine commands when
# the file had thirteen.

# List the available commands.
default:
    @just --list --unsorted

# Build the workspace.
build:
    cargo build

# Run the editor in TUI mode.
run file="main.rs":
    cargo run --bin ruster -- --tui {{file}}

# Run the editor in GUI mode (raylib).
gui file="main.rs":
    cargo run --bin ruster -- {{file}}

# Remove build artifacts.
clean:
    cargo clean

# Run the whole test suite.
test:
    cargo test

# Type-check every crate.
check:
    cargo check

# Lint everything as CI does, including the compositor's udev backend.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy --workspace --all-targets --features ruster-compositor/udev -- -D warnings

# Build the API docs.
doc:
    cargo doc --no-deps

#   just verify                 every surface, both backends
#   just verify sidebar         one surface, both backends
#   just verify "--gui hover"   one backend
#   just verify --list          the surface names
#
# The GUI half needs an unlocked screen (macOS will not create a window for a
# locked session); the script says so rather than letting raylib panic.
#
# `just --list` shows the *last* comment line above a recipe, and a blank line
# ends the block — so the detail sits above and the summary directly below it.

# Capture a user-visible surface in both backends into docs/verification/.
verify surface="all":
    ./scripts/verify-capture.sh {{surface}}

# Build in release mode.
release:
    cargo build --release

# Assemble the macOS .app bundle (icon, Dock identity, menu-bar name).
bundle profile="release":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{profile}}" = "release" ]; then cargo build --release; else cargo build; fi
    ./scripts/bundle-macos.sh {{profile}}

# One master, three platforms: .icns for the macOS bundle, .ico for the Windows
# resource table, hicolor PNGs for a Linux .desktop entry. Replacing the icon is
# this one file plus this recipe.
#
# The .icns step needs macOS (`iconutil`); the rest run anywhere with
# ImageMagick.

# Regenerate every derived icon from the assets/icon.png master.
icon:
    #!/usr/bin/env bash
    set -euo pipefail
    # macOS: .icns
    if command -v iconutil >/dev/null; then
        rm -rf /tmp/ruster.iconset && mkdir -p /tmp/ruster.iconset
        for s in 16 32 128 256 512; do
            magick assets/icon.png -resize ${s}x${s} /tmp/ruster.iconset/icon_${s}x${s}.png
            magick assets/icon.png -resize $((s*2))x$((s*2)) /tmp/ruster.iconset/icon_${s}x${s}@2x.png
        done
        iconutil -c icns /tmp/ruster.iconset -o assets/ruster.icns
        echo "wrote assets/ruster.icns"
    else
        echo "skipping .icns — iconutil is macOS-only"
    fi
    # Windows: a multi-resolution .ico, which is what the resource table wants.
    magick assets/icon.png -define icon:auto-resize=256,128,64,48,32,16 assets/icon.ico
    echo "wrote assets/icon.ico"
    # Linux: hicolor sizes for the .desktop entry.
    mkdir -p assets/hicolor
    for s in 16 32 48 64 128 256 512; do
        magick assets/icon.png -resize ${s}x${s} assets/hicolor/ruster-${s}.png
    done
    echo "wrote assets/hicolor/ruster-{16,32,48,64,128,256,512}.png"

# Run the compositor nested in a winit window (dev).
compositor:
    cargo run -p ruster-compositor

# Must be launched from a shell on the VT you want it on. logind only grants DRM
# master to the session that owns the seat, so running this from a terminal
# inside your graphical session fails with "failed to initialize libseat
# session" — correctly, and without touching your display.
#
#   Ctrl+Alt+F3, log in, then: just compositor-drm
#
# Escape hatches, best first: Ctrl+Alt+F2 (switches VT, leaves it running),
# Super+Shift+q (quits whatever the config says).
#
# This runs scripts/drm-test.sh rather than a bare `cargo run` — it builds
# first, tees the log to /tmp/ruster-drm.log, and afterwards reports the exit
# status, any screenshots taken and whether a VT switch was seen. On a VT that
# summary is the whole diagnosis, because the screen is gone by the time you can
# read anything.

# Run the compositor on DRM (needs a free VT + seatd/logind access).
compositor-drm:
    ./scripts/drm-test.sh

# Not a plain `makepkg -si`: a repository is what makes pacman treat ruster like
# any other package, so `pacman -Syu` upgrades it alongside everything else and
# `pacman -Qo` can say which package owns a file. A hand-installed package is
# invisible to both.
#
# It packages what is on the *remote*, not the working tree — push first. The
# version is the commit count, so every pushed commit is an available upgrade.

# Build ruster and publish it into the local pacman repository.
package:
    ./packaging/update-repo.sh
