//! Compositor render loop: builds the render elements for the focused
//! toplevel and composites them onto the output with the GLES renderer.
//!
//! Phase 0 draws the focused xdg toplevel fullscreen over a plain clear
//! color, then ruster's chrome (statusline, editor frame, which-key) on top of
//! it. [`Chrome`] produces a [`ChromeBatch`] of panel quads and glyph quads;
//! panels become solid-color elements and glyphs become textured elements
//! sampling the glyph atlas, which is uploaded once per change.

use std::collections::HashMap;
use std::time::Duration;

use ruster_shell::WindowId;
use smithay::backend::renderer::{
    damage::{Error as OutputDamageTrackerError, OutputDamageTracker},
    element::{
        solid::SolidColorRenderElement, surface::WaylandSurfaceRenderElement,
        texture::TextureRenderElement, AsRenderElements, Kind,
    },
    gles::GlesRenderer,
    Color32F, ImportAll, ImportMem, Renderer, RendererSuper,
};
use smithay::desktop::space::SurfaceTree;
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::desktop::PopupManager;
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::output::Output;
use smithay::utils::{
    Clock, Logical, Monotonic, Physical, Point, Rectangle, Scale, Size, Transform,
};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::ToplevelSurface;

use crate::chrome::{solid_elements_from_verts, Chrome, ChromeBatch, TreeStatus};
use ruster_render_gles::geometry::GlyphQuad;

/// Background the compositor clears the output to each frame.
pub const CLEAR_COLOR: Color32F = Color32F::BLACK;

/// Height of the chrome (statusline) bar, derived from the output height:
/// ~2.5% (`height / 40`), clamped to a 24-64px band.
pub fn chrome_height(output_height: i32) -> i32 {
    (output_height / 40).clamp(24, 64)
}

/// Error raised while rendering an output with the GLES renderer.
pub type RenderError = OutputDamageTrackerError<<GlesRenderer as RendererSuper>::Error>;

smithay::backend::renderer::element::render_elements! {
    #[doc = "The render elements for one output frame. Chrome is composited above the client surface, so it is listed first (elements are in front-to-back order)."]
    pub ChromeRenderElements<R> where R: ImportAll;
    Solid=SolidColorRenderElement,
    Texture=TextureRenderElement<<R as RendererSuper>::TextureId>,
    Surface=WaylandSurfaceRenderElement<R>,
}

/// How many extra glyph quads to emit per frame, from `RUSTER_BENCH_GLYPHS`.
///
/// Nobody knows how many render elements a frame can carry. `glyph_elements`
/// emits one `TextureRenderElement` per glyph, and an 80x40 editor pane is about
/// 3,200 of them against roughly a hundred for all of today's chrome. Stage 2 of
/// the Phase 3 plan has to choose between per-glyph quads and one texture per
/// text row, and that choice should follow a number rather than an argument.
///
/// Read once: this is on the render path, and a `var()` per frame would measure
/// the environment lookup as much as the renderer.
fn bench_glyph_count() -> usize {
    static COUNT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *COUNT.get_or_init(|| {
        std::env::var("RUSTER_BENCH_GLYPHS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    })
}

/// Everything one frame needs to know about *what* to draw, as opposed to the
/// backend machinery that draws it.
///
/// This exists because both entry points — winit's [`render_frame`] and the DRM
/// backend, which calls [`collect_render_elements`] directly — need the same ten
/// values, and passing them positionally had already reached fourteen arguments
/// behind an `allow(too_many_arguments)`. At that width the compiler stops
/// helping: two `&str`s or two `u32`s in the wrong order still compile.
pub struct FrameInput<'a> {
    /// The focused window. The surfaces themselves come from `geometry`; this
    /// is what the chrome names.
    pub focus: Option<WindowId>,
    pub toplevels: &'a HashMap<WindowId, ToplevelSurface>,
    pub output: &'a Output,
    pub workspace: u32,
    pub focused_title: &'a str,
    pub cursor_status: &'a CursorImageStatus,
    pub cursor_location: Point<f64, Logical>,
    /// Where each visible window sits, bottom to top.
    pub geometry: &'a [(WindowId, ruster_shell::Rect)],
    pub tree_status: TreeStatus,
    /// Editor panes, drawn where the layout put them.
    pub panes: &'a crate::pane::Panes,
    /// The documents those panes are showing.
    pub buffers: &'a ruster_core::workspace::BufferStore,
    /// The syntax parses, one per document.
    ///
    /// A `RefCell` because highlighting *is* a mutation — the parse is cached
    /// and refreshed when the buffer moves on — while everything else a frame
    /// reads is immutable, and threading `&mut` through the scene for one field
    /// would make every other caller pay for it.
    pub highlights: &'a std::cell::RefCell<crate::highlight::Highlights>,
    /// Diagnostics per document, for the signs a pane draws in its gutter.
    pub lsp: &'a ruster_lsp::state::LspState<crate::compositor::LspPending>,
    /// The bindings in force, so the welcome frame can say how to quit.
    pub keymap: &'a crate::keymap::Keymap,
    /// The which-key overlay, drawn only while a chord is half-typed.
    pub whichkey: Option<ruster_render::WhichKeyView>,
    /// The `:` line, when open or showing a result.
    pub minibuffer: Option<&'a crate::minibuffer::MiniBuffer>,
    /// The hover panel, when a language server has answered and nothing has
    /// dismissed it since.
    pub hover: Option<&'a crate::compositor::HoverPanel>,
    /// The launcher overlay, when open.
    pub launcher: Option<ruster_render::LauncherView>,
}

/// Composite the focused toplevel fullscreen onto the output, draw ruster's
/// chrome on top, and render it.
///
/// Returns the damage produced by the render (in physical coordinates) so the
/// caller can submit it to the backend; `None` means nothing changed and no
/// buffer swap is needed. Frame callbacks are delivered to the focused surface
/// every frame (it is on the primary scan-out output), so its client schedules
/// the next redraw; the 1s throttle only backstops surfaces not on a scan-out
/// output.
///
/// [`FrameInput`] carries the compositor's chrome state and shell focus in so
/// this stays a free, testable function — it does not need the whole
/// [`CompositorState`](crate::compositor::CompositorState).
pub fn render_frame(
    scene: &FrameInput<'_>,
    chrome: &mut Option<Chrome>,
    damage_tracker: &mut OutputDamageTracker,
    renderer: &mut GlesRenderer,
    framebuffer: &mut <GlesRenderer as RendererSuper>::Framebuffer<'_>,
    age: usize,
) -> Result<Option<Vec<Rectangle<i32, Physical>>>, RenderError> {
    let elements = collect_render_elements(scene, chrome, renderer);
    let result =
        damage_tracker.render_output(renderer, framebuffer, age, &elements, CLEAR_COLOR)?;
    let damage = result.damage.cloned();
    send_frame_callbacks(scene.geometry, scene.toplevels, scene.output);
    Ok(damage)
}

/// Build the render elements for one output frame: ruster's chrome (statusline,
/// editor frame, which-key) followed by the focused toplevel, both composited
/// by the renderer in front-to-back order. Generic over the renderer so both
/// the winit (`GlesRenderer`) and DRM (`MultiRenderer`) backends share it.
pub fn collect_render_elements<R: Renderer + ImportAll + ImportMem>(
    scene: &FrameInput<'_>,
    chrome: &mut Option<Chrome>,
    renderer: &mut R,
) -> Vec<ChromeRenderElements<R>>
where
    <R as RendererSuper>::TextureId: Clone + 'static,
{
    let mut elements: Vec<ChromeRenderElements<R>> = Vec::new();

    // The pointer goes in front of everything, chrome included.
    if let Some(chrome) = chrome.as_mut() {
        elements.extend(cursor_elements(
            chrome,
            renderer,
            scene.cursor_status,
            scene.cursor_location,
            scene.output.current_scale().fractional_scale(),
        ));
    }

    // Chrome is drawn unconditionally and sits above the client surface; the
    // statusline bar spans the bottom of the output, the which-key overlay
    // floats top-left, and the welcome editor frame is centred.
    if let Some(chrome) = chrome {
        let size = scene
            .output
            .current_mode()
            .map(|mode| mode.size)
            .unwrap_or_default();
        let mut batch = ChromeBatch::default();
        // Anything that covers text goes here instead, and is emitted in front
        // of this batch's glyphs as well as its panels. See `OverlayBatch`.
        let mut overlay = crate::chrome::OverlayBatch::default();
        // First in the batch, so everything else lands in front of it: the
        // tiling area covers the whole output, statusline included, so a
        // full-height window's border would otherwise sit on top of the bar.
        chrome.draw_window_borders(
            scene.geometry,
            scene.focus,
            scene.output.current_scale().fractional_scale(),
            &mut batch,
        );
        chrome.draw_statusline(
            size.w,
            size.h,
            scene.workspace,
            scene.focused_title,
            scene.tree_status,
            &mut batch,
        );

        if let Some(mb) = scene.minibuffer {
            chrome.draw_minibuffer(size.w, size.h, &mb.display(), mb.sigil_len(), &mut batch);
        }

        // No editor frame. It drew a hardcoded welcome buffer over the middle
        // of the screen — a Phase 3 placeholder standing in for a real
        // `ruster-core` buffer in a tile, and until that exists it is an
        // obstruction with nothing behind it. `Chrome::draw_editor_frame` stays
        // for the tile that will replace it.

        // Only while something is pending. It used to be drawn every frame
        // from a hardcoded pair, so it was permanently on screen and never
        // about anything.
        if let Some(view) = &scene.whichkey {
            chrome.draw_whichkey(size.w, size.h, view, &mut batch);
        }

        // Editor panes, at the rectangles the layout gave them. Drawn inside the
        // chrome batch so they sit above client surfaces and below the
        // statusline, which is where a tile belongs — and so a pane's frame is
        // translated by the same `translate_since` the rest of the chrome uses
        // rather than a second positioning scheme.
        // Chrome is measured in physical pixels and the layout in logical ones,
        // the same conversion `draw_window_borders` does.
        let chrome_scale = scene.output.current_scale().fractional_scale() as f32;
        // Where the hover panel's caret ended up, filled in by the pane that
        // owns it. Resolved inside the loop because the gutter — and therefore
        // the first text column — depends on which lines that pane is showing,
        // and worked out again here it would be a second opinion about the same
        // grid.
        let mut hover_at: Option<(crate::chrome::HoverAnchor, &[String])> = None;
        for (id, rect) in scene.geometry {
            let Some(pane) = scene.panes.get(id) else {
                continue;
            };
            let mark = batch.mark();
            // The text lives in the store; the pane holds a handle to it.
            let Some(doc) = scene.buffers.get(pane.doc) else {
                continue;
            };
            let (first_line, lines) = pane.visible_lines(&doc.buffer);
            // Highlighted here rather than in the pane: the parse belongs to the
            // document, and two panes on one file share it.
            let extension = doc
                .file_path
                .as_ref()
                .and_then(|p| p.extension())
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let lines = scene.highlights.borrow_mut().styled_lines(
                pane.doc,
                extension,
                &doc.buffer,
                first_line,
                &lines,
            );
            let severities =
                pane.line_severities(scene.lsp.diagnostics(pane.doc), first_line, lines.len());
            // Only while the line it describes is actually on screen: a panel
            // anchored to a caret that has been scrolled away would sit at the
            // frame's edge pointing at whatever is there now.
            if let Some(hover) = scene.hover.filter(|h| h.pane == *id) {
                if (first_line..first_line + lines.len()).contains(&hover.row) {
                    let body = crate::chrome::FrameBody::new(first_line, lines.len());
                    let (bx, by) = body.cell_origin(hover.row - first_line, hover.col);
                    hover_at = Some((
                        crate::chrome::HoverAnchor {
                            x: rect.x as f32 * chrome_scale + bx,
                            y: rect.y as f32 * chrome_scale + by,
                            cell_h: body.cell_h,
                        },
                        &hover.lines,
                    ));
                }
            }
            chrome.draw_editor_frame(
                (rect.w as f32 * chrome_scale) as i32,
                (rect.h as f32 * chrome_scale) as i32,
                &lines,
                first_line,
                &severities,
                &doc.name,
                &mut batch,
            );
            batch.translate_since(
                mark,
                rect.x as f32 * chrome_scale,
                rect.y as f32 * chrome_scale,
            );
        }

        // The launcher owns the screen while it is open, so it is drawn last
        // and into the overlay layer — over the panes, and clear of the bars at
        // the bottom by construction. See `launcher_layout`.
        if let Some(view) = &scene.launcher {
            chrome.draw_launcher(size.w, size.h, view, &mut overlay);
        }

        // After every pane, so it sits above the text it explains rather than
        // under the next tile the loop draws.
        if let Some((anchor, lines)) = hover_at {
            chrome.draw_hover(size.w, size.h, anchor, lines, &mut overlay);
        }

        // Synthetic load, when asked for. Real glyphs from the atlas rather than
        // empty quads, so the measurement includes the texture the renderer
        // actually samples.
        let bench = bench_glyph_count();
        if bench > 0 {
            chrome.bench_glyphs(bench, &mut batch);
        }

        // Glyphs first, then panels. Within a panel the glyphs are drawn on top
        // of its background, and chrome panels never overlap each other, so
        // hoisting every glyph in front of every panel is equivalent to a strict
        // reverse of painter's order and saves interleaving the two lists.
        let render_scale = scene.output.current_scale().fractional_scale();
        // The overlay layer first. A smithay element list is front-to-back, so
        // emitting it ahead of the base layer puts it in front of the base
        // layer's *glyphs* too — which is the whole reason it exists, since a
        // panel that covers text loses to that text under the hoist below.
        elements.extend(
            glyph_elements(chrome, renderer, &overlay.0.glyphs, render_scale)
                .into_iter()
                .map(ChromeRenderElements::Texture),
        );
        elements.extend(
            solid_elements_from_verts(&overlay.0.verts)
                .into_iter()
                .rev()
                .map(ChromeRenderElements::Solid),
        );
        elements.extend(
            glyph_elements(chrome, renderer, &batch.glyphs, render_scale)
                .into_iter()
                .map(ChromeRenderElements::Texture),
        );
        // The panel batch is in painter's order but a smithay element list is
        // front-to-back. Reverse it, or every background occludes the accent
        // segments drawn on top of it.
        elements.extend(
            solid_elements_from_verts(&batch.verts)
                .into_iter()
                .rev()
                .map(ChromeRenderElements::Solid),
        );
    }

    // Every tiled window, at the rectangle the container tree gave it. Drawn
    // back to front in reverse layout order so the focused window — which is
    // listed first below — ends up nearest the front; tiled windows do not
    // overlap, so the order only matters for the moment during a resize when
    // two rectangles briefly disagree.
    let scale = Scale::from(scene.output.current_scale().fractional_scale());
    for (id, rect) in scene.geometry.iter().rev() {
        let Some(surface) = scene.toplevels.get(id) else {
            continue;
        };
        let wl_surface = surface.wl_surface().clone();
        let origin_logical = Point::<i32, Logical>::from((rect.x, rect.y));
        let origin = origin_logical.to_physical_precise_round(scale);

        // Popups first, so they land in front of the window that owns them —
        // this list is front-to-back. A menu drawn behind its own toplevel is
        // indistinguishable from one that never opened.
        for (popup, offset) in PopupManager::popups_for_surface(&wl_surface) {
            let popup_origin =
                (origin_logical + offset - popup.geometry().loc).to_physical_precise_round(scale);
            let tree = SurfaceTree::from_surface(popup.wl_surface());
            elements.extend(
                AsRenderElements::<R>::render_elements(&tree, renderer, popup_origin, scale, 1.0)
                    .into_iter()
                    .map(ChromeRenderElements::Surface),
            );
        }

        let surface_tree = SurfaceTree::from_surface(&wl_surface);
        elements.extend(
            AsRenderElements::<R>::render_elements(&surface_tree, renderer, origin, scale, 1.0)
                .into_iter()
                .map(ChromeRenderElements::Surface),
        );
    }

    elements
}

/// Build the render elements for the pointer, which sit in front of everything.
///
/// A client that has taken pointer focus supplies its own cursor surface, and
/// its hotspot is subtracted so the image tracks the point that actually
/// clicks. Otherwise the compositor draws its own arrow — over its chrome, over
/// an empty output, and before any client has had the chance to set one. A
/// hidden cursor draws nothing.
fn cursor_elements<R: Renderer + ImportAll + ImportMem>(
    chrome: &mut Chrome,
    renderer: &mut R,
    status: &CursorImageStatus,
    location: Point<f64, Logical>,
    scale: f64,
) -> Vec<ChromeRenderElements<R>>
where
    <R as RendererSuper>::TextureId: Clone + 'static,
{
    let physical = location.to_physical(scale);
    match status {
        CursorImageStatus::Hidden => Vec::new(),
        CursorImageStatus::Surface(surface) => {
            // The hotspot lives on the surface, put there by `set_cursor`.
            let hotspot = with_states(surface, |states| {
                states
                    .data_map
                    .get::<CursorImageSurfaceData>()
                    .map(|attrs| attrs.lock().unwrap().hotspot)
                    .unwrap_or_default()
            });
            let origin = location - hotspot.to_f64();
            let tree = SurfaceTree::from_surface(surface);
            AsRenderElements::<R>::render_elements(
                &tree,
                renderer,
                origin.to_physical(scale).to_i32_round(),
                Scale::from(scale),
                1.0,
            )
            .into_iter()
            .map(ChromeRenderElements::Surface)
            .collect()
        }
        CursorImageStatus::Named(_) => chrome
            .cursor_element(renderer, physical)
            .into_iter()
            .map(ChromeRenderElements::Texture)
            .collect(),
    }
}

/// Upload the glyph atlas (once per change) and build one textured element per
/// glyph, sampling that glyph's cell out of the shared atlas texture.
///
/// Chrome geometry is in physical pixels, but a `TextureRenderElement` sizes
/// itself in logical pixels and scales by the output scale at render time — so
/// the destination size is divided by that scale here to land back on the exact
/// physical rect the atlas rasterized for.
fn glyph_elements<R: Renderer + ImportMem>(
    chrome: &mut Chrome,
    renderer: &mut R,
    glyphs: &[GlyphQuad],
    render_scale: f64,
) -> Vec<TextureRenderElement<<R as RendererSuper>::TextureId>>
where
    <R as RendererSuper>::TextureId: Clone + 'static,
{
    if glyphs.is_empty() {
        return Vec::new();
    }
    let Some(texture) = chrome.atlas_texture(renderer) else {
        return Vec::new();
    };
    let context_id = renderer.context_id();
    let atlas_size = chrome.atlas.texture_size as f64;
    glyphs
        .iter()
        .enumerate()
        .map(|(index, g)| {
            // `src` is in the texture's own pixels (the buffer has scale 1, so
            // logical and buffer coordinates coincide).
            let src = Rectangle::new(
                Point::from((g.u0 as f64 * atlas_size, g.v0 as f64 * atlas_size)),
                Size::from((
                    (g.u1 - g.u0) as f64 * atlas_size,
                    (g.v1 - g.v0) as f64 * atlas_size,
                )),
            );
            let logical = Size::from((
                (g.w as f64 / render_scale).round() as i32,
                (g.h as f64 / render_scale).round() as i32,
            ));
            TextureRenderElement::from_static_texture(
                chrome.glyph_id(index),
                context_id.clone(),
                Point::<f64, Physical>::from((g.x as f64, g.y as f64)),
                texture.clone(),
                1,
                Transform::Normal,
                None,
                Some(src),
                Some(logical),
                None,
                Kind::Unspecified,
            )
        })
        .collect()
}

/// Deliver frame callbacks to every window on screen, against the time of the
/// next frame, so their clients schedule the next redraw.
///
/// Every *visible* window, not just the focused one. This served `focus` alone
/// until now, which meant an unfocused client — a terminal running `top`, a
/// video, anything that draws by itself — got nothing but the 1s throttle,
/// updating once a second while the compositor was rendering at full rate.
///
/// The version that only served focus also had a much worse failure waiting for
/// it: once focus can be something other than a client, the lookup yields
/// `None` and *no window anywhere* gets a callback. The whole desktop freezes,
/// with no error and nothing in the log. That is the shape of the bug rather
/// than a hypothetical — the editor panes of Phase 3 are exactly such a focus.
///
/// The 1s throttle still backstops surfaces not on a scan-out output.
pub fn send_frame_callbacks(
    geometry: &[(WindowId, ruster_shell::Rect)],
    toplevels: &HashMap<WindowId, ToplevelSurface>,
    output: &Output,
) {
    let frame_time = Clock::<Monotonic>::new().now() + Duration::from_millis(16);
    // `geometry` is the active workspace's layout, so a window on a hidden
    // workspace is not in it and correctly gets nothing: it is not on screen,
    // and telling it to redraw would be asking for work nobody can see.
    for (id, _) in geometry {
        let Some(toplevel) = toplevels.get(id) else {
            continue;
        };
        send_frames_surface_tree(
            toplevel.wl_surface(),
            output,
            frame_time,
            Some(Duration::from_secs(1)),
            |_, _| Some(output.clone()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::chrome::{HoverAnchor, OverlayBatch};
    use crate::compositor::HoverPanel;
    use crate::highlight::Highlights;
    use crate::keymap::Keymap;
    use crate::minibuffer::{MiniBuffer, Prompt};
    use crate::pane::{EditorPane, Panes};
    use crate::scene;
    use ruster_core::buffer::Buffer;
    use ruster_core::workspace::BufferStore;
    use ruster_lsp::state::LspState;
    use ruster_render::{Color, StyledLine, SyntaxStyle, Theme, WhichKeyEntry, WhichKeyView};
    use ruster_render_elements::{div, layout, Elem, PxRect, Styled};
    use ruster_render_gles::atlas::Atlas;
    use ruster_render_gles::geometry::ChromeBatch;
    use ruster_render_gles::tessellate::{scene_to_chrome_batch, GlesTextMeasurer};
    use ruster_shell::{Layout, Rect};
    use smithay::output::{Mode, PhysicalProperties, Scale, Subpixel};

    fn theme() -> Theme {
        Theme::default()
    }

    fn status() -> TreeStatus {
        TreeStatus {
            layout: Some(Layout::Horizontal),
            windows: 2,
            floating: false,
        }
    }

    fn synth(id: &str, x: f32, y: f32) -> Elem {
        let mut e = div();
        e.id(id).absolute().position(x, y).size(40.0, 40.0).bg(Color::Rgb(1, 2, 3));
        e
    }

    /// Lay `elem` over `(w, h)` and tessellate it through `atlas` — the same
    /// atlas a `Chrome` instance draws into, so both paths measure, rasterize
    /// and pick UVs identically. The widget sits inside a viewport root, exactly
    /// as `chrome_scene`'s compose puts it: `layout` treats the root of the tree
    /// as the screen and replaces its geometry, so a widget that carries its own
    /// absolute position must never be the root itself.
    fn new_batch(elem: Elem, w: f32, h: f32, atlas: &mut Atlas) -> ChromeBatch {
        let mut root = div();
        root.size(w, h).children(vec![elem]);
        let mut measurer = GlesTextMeasurer;
        let scene = layout(PxRect { x: 0.0, y: 0.0, w, h }, &root, &mut measurer);
        scene_to_chrome_batch(&scene, atlas)
    }

    fn assert_same(label: &str, old: &ChromeBatch, new: &ChromeBatch) {
        assert_eq!(old.verts, new.verts, "{label}: panel geometry");
        assert_eq!(old.glyphs, new.glyphs, "{label}: glyph geometry");
    }

    /// The migration gate: every widget the scene builds must produce the exact
    /// vertex and glyph batches the old `Chrome::draw_*` path did. Both sides
    /// share one `Chrome` (one atlas), so the only possible difference is
    /// layout — and `f32` equality is deliberate: a scene that renders
    /// differently is a scene that is not ready to replace the draw methods.
    /// This test is deleted at the flip (Task 6).
    #[test]
    fn scene_batches_match_the_old_draw_geometry() {
        let mut chrome = Chrome::new(theme());

        // statusline: workspace 1, a long focused title, a horizontal split.
        {
            let title = "a long focused title that overflows the bar";
            let mut old = ChromeBatch::default();
            chrome.draw_statusline(800, 600, 1, title, status(), &mut old);
            let elem = scene::statusline_elem(800, 600, 1, title, status(), &theme());
            assert_same("statusline", &old, &new_batch(elem, 800.0, 600.0, &mut chrome.atlas));
        }

        // which-key: two columns (41 rows per column at 1080p), a title, and a
        // narrow output so the panel width clamp is exercised too.
        {
            let rows: Vec<WhichKeyEntry> = (0..60)
                .map(|n| WhichKeyEntry {
                    key: format!("M-{n}"),
                    desc: format!("action {n}"),
                })
                .collect();
            let view = WhichKeyView {
                title: "M-w".into(),
                rows,
                anim: 1.0,
            };
            let mut old = ChromeBatch::default();
            chrome.draw_whichkey(200, 1080, &view, &mut old);
            let elem = scene::whichkey_elem(200, 1080, &view, &theme(), &mut GlesTextMeasurer);
            assert_same("which-key", &old, &new_batch(elem, 200.0, 1080.0, &mut chrome.atlas));
        }

        // hover: three lines, below the caret, near the top-left corner.
        {
            let anchor = HoverAnchor {
                x: 100.0,
                y: 100.0,
                cell_h: 16.0,
            };
            let lines = vec![
                "fn main()".to_string(),
                "the entry point".to_string(),
                "third line".to_string(),
            ];
            let mut old = OverlayBatch::default();
            chrome.draw_hover(1920, 1080, anchor, &lines, &mut old);
            let elem = scene::hover_elem(1920, 1080, anchor, &lines, &theme(), &mut GlesTextMeasurer);
            assert_same("hover", &old.0, &new_batch(elem, 1920.0, 1080.0, &mut chrome.atlas));
        }

        // one pane: diagnostics, a multi-span highlighted line, a wide title.
        {
            let hl = |fg: (u8, u8, u8)| SyntaxStyle {
                fg: Color::Rgb(fg.0, fg.1, fg.2),
                ..SyntaxStyle::default()
            };
            let lines = vec![
                StyledLine {
                    text: "fn main() {".to_string(),
                    highlights: vec![(0, 2, hl((1, 0, 0))), (3, 7, hl((0, 1, 0)))],
                },
                StyledLine {
                    text: "    let x = 1;".to_string(),
                    highlights: vec![(4, 3, hl((3, 2, 1)))],
                },
                StyledLine {
                    text: "    println!(\"hi\");".to_string(),
                    highlights: vec![],
                },
                StyledLine {
                    text: "}".to_string(),
                    highlights: vec![],
                },
            ];
            let severities = vec![Some(1), None, Some(3), None];
            let mut old = ChromeBatch::default();
            chrome.draw_editor_frame(600, 400, &lines, 0, &severities, "a wide pane title", &mut old);
            let elem = scene::pane_elem(600, 400, &lines, 0, &severities, "a wide pane title", &theme());
            assert_same("pane", &old, &new_batch(elem, 600.0, 400.0, &mut chrome.atlas));
        }

        // mini-buffer (the sixth surface `chrome_scene` composes).
        {
            let mut old = ChromeBatch::default();
            chrome.draw_minibuffer(800, 600, ":echo hi", 1, &mut old);
            let elem = scene::minibuffer_elem(800, 600, ":echo hi", 1, &theme());
            assert_same("mini-buffer", &old, &new_batch(elem, 800.0, 600.0, &mut chrome.atlas));
        }

        // window borders: focused + unfocused at a fractional scale.
        {
            let windows = vec![
                (WindowId(0), Rect::new(0, 0, 100, 100)),
                (WindowId(1), Rect::new(100, 0, 200, 200)),
            ];
            let mut old = ChromeBatch::default();
            chrome.draw_window_borders(&windows, Some(WindowId(0)), 1.5, &mut old);
            let elem = scene::window_borders_elem(&windows, Some(WindowId(0)), 1.5, &theme());
            assert_same("window borders", &old, &new_batch(elem, 800.0, 600.0, &mut chrome.atlas));
        }
    }

    /// The declarative scene must keep the old painter's order — borders,
    /// statusline, mini-buffer, which-key, then panes — because a smithay
    /// element list is front-to-back and the panel batch is reversed to match.
    #[test]
    fn compose_keeps_the_painter_s_order() {
        let base = scene::compose(
            vec![
                synth("borders", 0.0, 0.0),
                synth("statusline", 0.0, 60.0),
                synth("minibuffer", 0.0, 120.0),
                synth("whichkey", 0.0, 180.0),
                synth("pane", 0.0, 240.0),
            ],
            &theme(),
            800.0,
            600.0,
        );
        let mut measurer = GlesTextMeasurer;
        let laid = layout(PxRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 }, &base, &mut measurer);
        let order: Vec<&str> = laid.boxes.iter().map(|b| b.key.last().unwrap()).collect();
        assert_eq!(order, ["borders", "statusline", "minibuffer", "whichkey", "pane"]);
    }

    /// `chrome_scene` assembles the real widgets from a `FrameInput` and hands
    /// the hover panel back as the overlay element — the only chrome drawn over
    /// the base batch.
    #[test]
    fn chrome_scene_builds_the_base_and_keeps_hover_as_the_overlay() {
        let output = Output::new(
            "test".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "ruster".into(),
                model: "test".into(),
            },
        );
        let mode = Mode {
            size: (800, 600).into(),
            refresh: 60_000,
        };
        output.add_mode(mode);
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Fractional(1.0)),
            Some((0, 0).into()),
        );

        let mut buffers = BufferStore::new();
        let doc = buffers.create_scratch("doc");
        buffers.get_mut(doc).unwrap().buffer =
            Buffer::from_str("fn main() {\n    println!(\"hi\");\n}\n");

        let mut panes = Panes::new();
        let mut pane = EditorPane::new(doc);
        pane.rows = 3;
        panes.insert(WindowId(0), pane);

        let toplevels = HashMap::new();
        let geometry = vec![(WindowId(0), Rect::new(0, 0, 100, 60))];
        let highlights = std::cell::RefCell::new(Highlights::default());
        let lsp = LspState::default();
        let keymap = Keymap::new(&[]);
        let mut mb = MiniBuffer::new(Prompt::Command);
        mb.input = ":echo hi".into();
        let hover = HoverPanel {
            pane: WindowId(0),
            row: 0,
            col: 0,
            lines: vec!["hover line".to_string()],
        };

        let frame = FrameInput {
            focus: Some(WindowId(0)),
            toplevels: &toplevels,
            output: &output,
            workspace: 1,
            focused_title: "doc",
            cursor_status: &CursorImageStatus::Hidden,
            cursor_location: Point::from((0.0, 0.0)),
            geometry: &geometry,
            tree_status: status(),
            panes: &panes,
            buffers: &buffers,
            highlights: &highlights,
            lsp: &lsp,
            keymap: &keymap,
            whichkey: None,
            minibuffer: Some(&mb),
            hover: Some(&hover),
            launcher: None,
        };

        let mut measurer = GlesTextMeasurer;
        let (base, overlay) = scene::chrome_scene(&frame, &theme(), &mut measurer);

        // The overlay is exactly the hover panel: one rounded panel, one line.
        let overlay_laid = layout(
            PxRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 },
            overlay.as_ref().unwrap(),
            &mut measurer,
        );
        assert_eq!(overlay_laid.boxes.len(), 1);
        assert_eq!(overlay_laid.texts.len(), 1);

        // The base is the frame in painter's order: window borders, statusline,
        // mini-buffer, then the pane. (No which-key in this frame.)
        let base_laid = layout(PxRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 }, &base, &mut measurer);
        let mut surfaces: Vec<&str> = Vec::new();
        for b in &base_laid.boxes {
            let top = b.key.0.first().map(String::as_str).unwrap_or("");
            if surfaces.last().copied() != Some(top) {
                surfaces.push(top);
            }
        }
        assert_eq!(surfaces, ["window-borders", "statusline", "minibuffer", "pane"]);
    }

    #[test]
    fn chrome_height_never_exceeds_output() {
        // ~2.5% of the output height, min 24px, max 64px.
        let h = chrome_height(1080);
        assert!(h <= 1080 && h > 0);
        assert_eq!(h, 27);
    }

    #[test]
    fn chrome_height_scales_with_output_height() {
        assert_eq!(chrome_height(2160), 54);
        assert_eq!(chrome_height(2560), 64);
    }

    #[test]
    fn chrome_height_is_bounded() {
        assert_eq!(chrome_height(100), 24);
        assert_eq!(chrome_height(100_000), 64);
    }
}
