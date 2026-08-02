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

# Regenerate assets/ruster.icns from assets/icon.png.
icon:
    #!/usr/bin/env bash
    set -euo pipefail
    rm -rf /tmp/ruster.iconset && mkdir -p /tmp/ruster.iconset
    for s in 16 32 128 256 512; do
        magick assets/icon.png -resize ${s}x${s} /tmp/ruster.iconset/icon_${s}x${s}.png
        magick assets/icon.png -resize $((s*2))x$((s*2)) /tmp/ruster.iconset/icon_${s}x${s}@2x.png
    done
    iconutil -c icns /tmp/ruster.iconset -o assets/ruster.icns
    echo "wrote assets/ruster.icns"
