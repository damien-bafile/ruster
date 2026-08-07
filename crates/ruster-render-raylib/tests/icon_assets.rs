//! The icon assets must exist and be real images.
//!
//! Nobody can test "looks right" — that is a look, and the `gui-check` skill is
//! how to take it. What *can* be tested is the failure that actually happens:
//! an asset renamed, trimmed from a source tree, or regenerated to zero bytes,
//! which turns into a program with no icon and, for the embedded PNG, a build
//! that fails at `include_bytes!` in a way that reads as a compiler problem.
//!
//! Each check looks at the file's magic bytes rather than its extension,
//! because the failure mode of `just icon` on a machine without ImageMagick is
//! an empty or truncated file with the right name.

use std::path::{Path, PathBuf};

fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

fn read(name: &str) -> Vec<u8> {
    let p = assets().join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{} is missing: {e}", p.display()))
}

#[test]
fn the_png_master_is_a_png() {
    let bytes = read("icon.png");
    assert!(bytes.len() > 1024, "icon.png is {} bytes — truncated?", bytes.len());
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "icon.png is not a PNG");
}

#[test]
fn the_embedded_icon_is_the_master() {
    // `RaylibRenderer` embeds this with `include_bytes!`. If the path drifts,
    // the build breaks in a way that looks like a compiler problem rather than
    // a missing asset, so pin the relationship here too.
    const EMBEDDED: &[u8] = include_bytes!("../../../assets/icon.png");
    assert_eq!(EMBEDDED, read("icon.png").as_slice(), "the embedded icon is not the master");
}

#[test]
fn the_windows_icon_is_a_multi_resolution_ico() {
    let bytes = read("icon.ico");
    // ICONDIR: reserved=0, type=1 (icon), then the image count.
    assert_eq!(&bytes[..4], &[0, 0, 1, 0], "icon.ico is not an ICO");
    let count = u16::from_le_bytes([bytes[4], bytes[5]]);
    assert!(
        count >= 4,
        "icon.ico holds {count} image(s); Windows picks per context (16px tray, \
         32px taskbar, 256px Explorer) and a single size gets scaled badly"
    );
}

#[test]
fn the_macos_bundle_icon_is_an_icns() {
    let bytes = read("ruster.icns");
    assert_eq!(&bytes[..4], b"icns", "ruster.icns is not an ICNS");
}

#[test]
fn the_linux_hicolor_sizes_are_all_present() {
    // A `.desktop` entry names the icon without a size; the desktop picks from
    // whatever hicolor holds. A missing size falls back to a scaled one, and
    // 16px scaled from 512 is exactly where this icon stopped being legible.
    for size in [16, 32, 48, 64, 128, 256, 512] {
        let bytes = read(&format!("hicolor/ruster-{size}.png"));
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "hicolor/ruster-{size}.png is not a PNG");
        assert!(bytes.len() > 100, "hicolor/ruster-{size}.png is {} bytes", bytes.len());
    }
}

#[test]
fn the_desktop_entry_declares_what_a_launcher_needs() {
    let text = std::fs::read_to_string(assets().join("ruster.desktop")).expect("ruster.desktop");
    for required in [
        "[Desktop Entry]",
        "Type=Application",
        "Name=ruster",
        "Exec=ruster",
        "Icon=ruster",
        "Terminal=false",
    ] {
        assert!(text.contains(required), "ruster.desktop has no {required:?}");
    }
    // `Icon=` must name the installed icon, not a path into the source tree:
    // the entry is copied to ~/.local/share and the repo may not be there.
    assert!(
        !text.contains("Icon=assets") && !text.contains("Icon=/"),
        "Icon= should be the bare name `ruster`, resolved from hicolor"
    );
    // `%F` not `%f`: ruster takes several paths, and a file manager passing two
    // files should open both rather than launching twice.
    assert!(text.contains("Exec=ruster %F"), "Exec should take multiple files with %F");
}

#[test]
fn the_windows_resource_script_points_at_the_icon() {
    let rc = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../ruster-bin/ruster.rc"),
    )
    .expect("ruster.rc");
    assert!(rc.contains("ICON"), "ruster.rc declares no ICON resource");
    let referenced = rc
        .split('"')
        .nth(1)
        .expect("the .rc names a file in quotes");
    let resolved = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ruster-bin").join(referenced);
    assert!(
        resolved.exists(),
        "ruster.rc points at {referenced}, which does not exist relative to the crate. \
         This only fails the build on Windows, so it would otherwise be found by CI \
         rather than here."
    );
}
