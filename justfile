# ruster — justfile
#   just         run in GUI mode (default)
#   just run     run in TUI mode
#   just gui     run in GUI mode (raylib)
#   just build   build only
#   just clean   clean build artifacts
#   just test    run all tests
#   just check   cargo check all crates
#   just doc     build docs
#   just release build in release mode

default: gui

build:
    cargo build

run file="main.rs":
    cargo run -- --tui {{file}}

gui file="main.rs":
    cargo run -- {{file}}

clean:
    cargo clean

test:
    cargo test

check:
    cargo check

doc:
    cargo doc --no-deps

release:
    cargo build --release

# Assemble the macOS .app bundle (icon, Dock identity, menu-bar name).
bundle profile="release":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{profile}}" = "release" ]; then cargo build --release; else cargo build; fi
    ./scripts/bundle-macos.sh {{profile}}

# Regenerate every derived icon from the assets/icon.png master.
#
# One master, three platforms: .icns for the macOS bundle, .ico for the Windows
# resource table, hicolor PNGs for a Linux .desktop entry. Replacing the icon is
# this one file plus this recipe.
#
# The .icns step needs macOS (`iconutil`); the rest run anywhere with
# ImageMagick.
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
