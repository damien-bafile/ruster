//! scratch measurement
//!
//! `ruster_render_gles` is gated to Linux at the crate level (`#![cfg(target_os
//! = "linux")]`), so this integration test cannot compile off Linux — and
//! `cargo test --workspace --all-targets` runs on every CI matrix runner.
//! Mirror the gate here.
#![cfg(target_os = "linux")]

use ruster_render_gles::atlas::{Atlas, FontFamily};
use std::collections::HashSet;

fn colors_of(line: &ruster_render::StyledLine, chars: &[char]) -> Vec<[u8; 3]> {
    const FG: [u8; 3] = [205, 214, 244];
    let mut style_at: Vec<[u8; 3]> = vec![FG; chars.len()];
    for &(off, len, style) in &line.highlights {
        let rgb = match style.fg {
            ruster_render::Color::Rgb(r, g, b) => [r, g, b],
            ruster_render::Color::Default => FG,
        };
        for s in style_at
            .iter_mut()
            .take((off + len).min(chars.len()))
            .skip(off)
        {
            *s = rgb;
        }
    }
    style_at
}

fn cells_of(path: &str, ext: &str, into: &mut HashSet<(u32, [u8; 3], char)>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(engine) = ruster_syntax::SyntaxEngine::new(&text, ext) else {
        return;
    };
    for line in engine.styled_lines() {
        let chars: Vec<char> = line.text.chars().collect();
        let style_at = colors_of(line, &chars);
        for (c, rgb) in chars.iter().zip(&style_at) {
            if c.is_control() || *c == ' ' {
                continue;
            }
            into.insert((14, *rgb, *c));
        }
    }
}

fn pack(cells: &HashSet<(u32, [u8; 3], char)>, family: FontFamily, size: u32) -> (f32, u64) {
    let mut atlas = Atlas::with_texture_size(size);
    for (fs, rgb, c) in cells {
        atlas.glyph_in(*fs, *rgb, *c, family);
    }
    (atlas.fill_fraction() * 100.0, atlas.dropped_glyphs())
}

#[test]
fn measure() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    // One screenful and one whole file, in the compositor's own source.
    let mut one_file = HashSet::new();
    cells_of(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/compositor.rs"),
        "rs",
        &mut one_file,
    );
    println!("one whole .rs file: {} cells", one_file.len());

    // Three panes, three languages — a plausible session.
    let mut session = HashSet::new();
    for (p, e) in [
        ("crates/ruster-compositor/src/compositor.rs", "rs"),
        ("Cargo.toml", "toml"),
        ("AGENTS.md", "md"),
    ] {
        cells_of(&format!("{root}/{p}"), e, &mut session);
    }
    println!("three panes, three languages: {} cells", session.len());

    // Every source file in the workspace: the ceiling a session could reach,
    // since the atlas never evicts.
    let mut all: HashSet<(u32, [u8; 3], char)> = HashSet::new();
    for entry in walk(root) {
        let Some(ext) = entry
            .rsplit('.')
            .next()
            .filter(|e| ["rs", "toml", "lua", "md", "json", "py"].contains(e))
            .map(str::to_string)
        else {
            continue;
        };
        cells_of(&entry, &ext, &mut all);
    }
    let colors: HashSet<[u8; 3]> = all.iter().map(|c| c.1).collect();
    let chars: HashSet<char> = all.iter().map(|c| c.2).collect();
    println!(
        "whole workspace: {} cells, {} colours, {} chars",
        all.len(),
        colors.len(),
        chars.len()
    );

    for size in [1024u32, 2048] {
        println!("-- {size}^2 --");
        for (name, set) in [
            ("one file", &one_file),
            ("three panes", &session),
            ("whole workspace", &all),
        ] {
            let (fill, dropped) = pack(set, FontFamily::Mono, size);
            println!("  pane text, {name}: fill {fill:.1}% dropped {dropped}");
        }
        // Chrome on top: it draws at 16 (statusline, frame titles), 14
        // (which-key) and a minibuffer size derived from the output height.
        let mut atlas = Atlas::with_texture_size(size);
        for (fs, rgb, c) in &all {
            atlas.glyph_in(*fs, *rgb, *c, FontFamily::Mono);
        }
        let mono_only = atlas.fill_fraction() * 100.0;
        for fs in [14u32, 16, 22] {
            for c in &chars {
                for rgb in [[205, 214, 244], [30, 30, 46], [137, 180, 250]] {
                    atlas.glyph_in(fs, rgb, *c, FontFamily::Ui);
                }
            }
        }
        println!(
            "  workspace mono {mono_only:.1}% + chrome (3 sizes x 3 colours x {} chars) = {:.1}%, dropped {}",
            chars.len(),
            atlas.fill_fraction() * 100.0,
            atlas.dropped_glyphs()
        );
    }
}

fn walk(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name == "target" || name == ".git" {
            continue;
        }
        if p.is_dir() {
            out.extend(walk(&p.to_string_lossy()));
        } else {
            out.push(p.to_string_lossy().to_string());
        }
    }
    out
}
