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
