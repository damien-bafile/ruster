//! Embed the Windows application icon into the executable.
//!
//! Windows reads a program's icon from its resource table, not from a file
//! beside it, so it has to be linked in at build time — unlike macOS (the
//! `.icns` inside the `.app` bundle) and Linux (a `.desktop` entry pointing at
//! hicolor PNGs). The runtime `set_window_icon` call covers the *window* on
//! every platform; this is what gives Explorer, the taskbar and the Start menu
//! something to show.
//!
//! Gated on `cfg(windows)` rather than the target OS because build scripts are
//! compiled for the **host**, and `embed-resource` is only a dependency there.
//! Cross-compiling to Windows from another host therefore produces a binary
//! with no embedded icon — acceptable, and better than failing the build.

#[cfg(windows)]
fn main() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let ico = manifest.join("../../assets/icon.ico");
    // A missing icon is not worth failing a build over: the editor runs fine
    // without one, and a hard error would block anyone building from a source
    // tree that trimmed the assets.
    if !ico.exists() {
        println!("cargo:warning=assets/icon.ico missing; building without an embedded icon");
        return;
    }
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    println!("cargo:rerun-if-changed=ruster.rc");
    // The result is deliberately ignored. An icon is cosmetic, and a resource
    // compiler that is missing or unhappy must not be the reason a build fails
    // — the same call is what CI on windows-latest exercises.
    let _ = embed_resource::compile(manifest.join("ruster.rc"), embed_resource::NONE);
}

#[cfg(not(windows))]
fn main() {}
