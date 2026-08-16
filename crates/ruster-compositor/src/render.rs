//! Compositor render loop: builds the render elements for the focused
//! toplevel and composites them onto the output with the GLES renderer.
//!
//! Phase 0 draws the focused xdg toplevel fullscreen over a plain clear
//! color, then ruster's chrome (statusline, editor frame, which-key) on top of
//! it. The chrome is built as a declarative scene in `scene.rs`, laid out and
//! tessellated into a [`ChromeBatch`] of panel quads and glyph quads; panels
//! become solid-color elements and glyphs become textured elements sampling
//! the glyph atlas, which is uploaded once per change.

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
use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;

use crate::chrome::{solid_elements_from_verts, Chrome, TreeStatus};
use crate::scene;
use ruster_render_elements::{layout, ElementKey, PxRect};
use ruster_render_gles::geometry::GlyphQuad;
use ruster_render_gles::tessellate::{scene_to_chrome_batch, GlesTextMeasurer};

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
    pub clients: &'a HashMap<WindowId, crate::client::Client>,
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
    /// X11 override-redirect windows, which are drawn but never tiled.
    pub x11_unmanaged: &'a [smithay::xwayland::X11Surface],
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
    send_frame_callbacks(scene.geometry, scene.clients, scene.output);
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

    for stage in FRONT_TO_BACK {
        match stage {
            Stage::Cursor => {
                if let Some(chrome) = chrome.as_mut() {
                    elements.extend(cursor_elements(
                        chrome,
                        renderer,
                        scene.cursor_status,
                        scene.cursor_location,
                        scene.output.current_scale().fractional_scale(),
                    ));
                }
            }
            Stage::Layer(which) => elements.extend(layer_elements(scene, renderer, which)),
            Stage::Chrome => elements.extend(chrome_elements(scene, chrome, renderer)),
            Stage::X11Unmanaged => elements.extend(x11_unmanaged_elements(scene, renderer)),
            Stage::Windows => elements.extend(window_elements(scene, renderer)),
        }
    }

    elements
}

/// One band of the frame, in the order the bands are emitted.
///
/// The order used to live in the statement order of `collect_render_elements`,
/// where nothing could assert it and getting it wrong was silent: a bar emitted
/// after the chrome still maps, still configures, still renders one element per
/// frame, and is simply never seen, because ruster draws its own statusline
/// along the same edge. That cost an afternoon. As data it is one array a test
/// can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The pointer, in front of everything including chrome.
    Cursor,
    /// One `wlr-layer-shell` layer.
    Layer(WlrLayer),
    /// Everything the compositor draws itself: statusline, borders, panes,
    /// which-key, the launcher.
    Chrome,
    /// X11 menus, tooltips and drag icons — override-redirect windows, which
    /// the window manager was told to keep its hands off.
    X11Unmanaged,
    /// Tiled toplevels and their popups.
    Windows,
}

/// Every band of a frame, front to back — which is the order a smithay element
/// list wants, not painter's order.
///
/// `Overlay` and `Top` sit in front of [`Stage::Chrome`] deliberately: ruster's
/// statusline is the fallback for when nothing better is there, and a client
/// that went to the trouble of asking for the space wins it. `Bottom` and
/// `Background` — where a wallpaper setter lives — go behind the windows.
pub const FRONT_TO_BACK: [Stage; 8] = [
    Stage::Cursor,
    Stage::Layer(WlrLayer::Overlay),
    Stage::Layer(WlrLayer::Top),
    Stage::Chrome,
    // In front of the windows, because a menu that renders behind the window it
    // was opened from is indistinguishable from one that never opened — the same
    // failure `xdg_popup` had, one protocol over.
    Stage::X11Unmanaged,
    Stage::Windows,
    Stage::Layer(WlrLayer::Bottom),
    Stage::Layer(WlrLayer::Background),
];

/// Everything the compositor draws itself, as render elements.
fn chrome_elements<R: Renderer + ImportAll + ImportMem>(
    scene: &FrameInput<'_>,
    chrome: &mut Option<Chrome>,
    renderer: &mut R,
) -> Vec<ChromeRenderElements<R>>
where
    <R as RendererSuper>::TextureId: Clone + 'static,
{
    let mut elements: Vec<ChromeRenderElements<R>> = Vec::new();
    // Chrome is drawn unconditionally and sits above the client surface; the
    // statusline bar spans the bottom of the output, the which-key overlay
    // floats top-left, and the editor panes sit where the layout put them.
    if let Some(chrome) = chrome.as_mut() {
        let size = scene
            .output
            .current_mode()
            .map(|mode| mode.size)
            .unwrap_or_default();
        let render_scale = scene.output.current_scale().fractional_scale();

        // The whole frame is one declarative scene: `chrome_scene` assembles
        // the widgets, `layout` turns the tree into pure geometry, and
        // `scene_to_chrome_batch` tessellates it. Anything that covers text —
        // the launcher and the hover panel — comes back as its own overlay
        // element, so it is emitted in front of the base batch's glyphs as well
        // as its panels.
        let mut measurer = GlesTextMeasurer;
        let (base, overlay) = scene::chrome_scene(scene, chrome.theme(), &mut measurer);
        let area = PxRect {
            x: 0.0,
            y: 0.0,
            w: size.w as f32,
            h: size.h as f32,
        };
        let base_scene = layout(area, &base, &mut measurer);
        let mut base_batch = scene_to_chrome_batch(&base_scene, &mut chrome.atlas);
        let overlay_batch = overlay.map(|o| {
            let scene = layout(area, &o, &mut measurer);
            scene_to_chrome_batch(&scene, &mut chrome.atlas)
        });

        // Synthetic load, when asked for. Real glyphs from the atlas rather than
        // empty quads, so the measurement includes the texture the renderer
        // actually samples.
        let bench = bench_glyph_count();
        if bench > 0 {
            chrome.bench_glyphs(bench, &mut base_batch);
        }

        // Glyphs first, then panels. Within a panel the glyphs are drawn on top
        // of its background, and chrome panels never overlap each other, so
        // hoisting every glyph in front of every panel is equivalent to a strict
        // reverse of painter's order and saves interleaving the two lists.
        //
        // The overlay layer first. A smithay element list is front-to-back, so
        // emitting it ahead of the base layer puts it in front of the base
        // layer's *glyphs* too — which is the whole reason it exists, since a
        // panel that covers text loses to that text under the hoist below.
        if let Some(overlay_batch) = &overlay_batch {
            elements.extend(
                glyph_elements(
                    chrome,
                    renderer,
                    &overlay_batch.glyphs,
                    &overlay_batch.glyph_keys,
                    render_scale,
                )
                .into_iter()
                .map(ChromeRenderElements::Texture),
            );
            elements.extend(
                solid_elements_from_verts(&overlay_batch.verts)
                    .into_iter()
                    .rev()
                    .map(ChromeRenderElements::Solid),
            );
        }
        elements.extend(
            glyph_elements(
                chrome,
                renderer,
                &base_batch.glyphs,
                &base_batch.glyph_keys,
                render_scale,
            )
            .into_iter()
            .map(ChromeRenderElements::Texture),
        );
        // The panel batch is in painter's order but a smithay element list is
        // front-to-back. Reverse it, or every background occludes the accent
        // segments drawn on top of it.
        elements.extend(
            solid_elements_from_verts(&base_batch.verts)
                .into_iter()
                .rev()
                .map(ChromeRenderElements::Solid),
        );
    }

    elements
}

/// Every tiled window, at the rectangle the container tree gave it, with each
/// window's popups in front of it.
///
/// Drawn back to front in reverse layout order so the focused window — which is
/// listed first — ends up nearest the front; tiled windows do not overlap, so
/// the order only matters for the moment during a resize when two rectangles
/// briefly disagree.
fn window_elements<R: Renderer + ImportAll + ImportMem>(
    scene: &FrameInput<'_>,
    renderer: &mut R,
) -> Vec<ChromeRenderElements<R>>
where
    <R as RendererSuper>::TextureId: Clone + 'static,
{
    let mut elements: Vec<ChromeRenderElements<R>> = Vec::new();
    let scale = Scale::from(scene.output.current_scale().fractional_scale());
    for (id, rect) in scene.geometry.iter().rev() {
        // Skipped rather than drawn empty when an X11 window has no surface
        // yet: there is a real moment between the window existing and its pixels
        // arriving, and it is not an error. See `Client::wl_surface`.
        let Some(wl_surface) = scene.clients.get(id).and_then(|c| c.wl_surface()) else {
            continue;
        };
        // The tile is where the *window* goes. The surface can be bigger — a
        // client-side shadow lives outside the window geometry — so the surface
        // origin is the tile shifted back by that margin. See `surface_origin`.
        let origin_logical = Point::<i32, Logical>::from((rect.x, rect.y));
        let origin = crate::compositor::surface_origin(
            origin_logical,
            crate::compositor::window_geometry(&wl_surface),
        )
        .to_physical_precise_round(scale);

        // Popups first, so they land in front of the window that owns them —
        // this list is front-to-back. A menu drawn behind its own toplevel is
        // indistinguishable from one that never opened.
        //
        // Positioned from `origin_logical`, the *window* origin, not the surface
        // origin above: a popup's offset is measured from its parent's window
        // geometry, so the shadow margin cancels out and must not be subtracted
        // twice.
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

/// X11 override-redirect windows, drawn at the coordinates the client chose.
///
/// These are menus, tooltips and drag icons. They are not in the tree and must
/// not be: an X client positions its own menu relative to the thing that opened
/// it, and a tiling compositor that "helpfully" lays one out moves it somewhere
/// meaningless. So unlike every other window here, the geometry comes from the
/// client rather than from the layout.
fn x11_unmanaged_elements<R: Renderer + ImportAll + ImportMem>(
    scene: &FrameInput<'_>,
    renderer: &mut R,
) -> Vec<ChromeRenderElements<R>>
where
    <R as RendererSuper>::TextureId: Clone + 'static,
{
    let scale = Scale::from(scene.output.current_scale().fractional_scale());
    let mut out = Vec::new();
    for window in scene.x11_unmanaged {
        let Some(surface) = window.wl_surface() else {
            continue;
        };
        let origin = window.geometry().loc.to_physical_precise_round(scale);
        let tree = SurfaceTree::from_surface(&surface);
        out.extend(
            AsRenderElements::<R>::render_elements(&tree, renderer, origin, scale, 1.0)
                .into_iter()
                .map(ChromeRenderElements::Surface),
        );
    }
    out
}

/// Render elements for one layer of the output's `LayerMap`.
///
/// The geometry comes from the map rather than from the surface: `arrange` has
/// already resolved the anchors, margins and exclusive zone the client asked
/// for, and asking the surface where it is would be a second opinion about the
/// same rectangle.
fn layer_elements<R: Renderer + ImportAll + ImportMem>(
    scene: &FrameInput<'_>,
    renderer: &mut R,
    which: WlrLayer,
) -> Vec<ChromeRenderElements<R>>
where
    <R as RendererSuper>::TextureId: Clone + 'static,
{
    let scale = Scale::from(scene.output.current_scale().fractional_scale());
    let map = smithay::desktop::layer_map_for_output(scene.output);
    let mut out = Vec::new();
    for layer in map.layers_on(which) {
        let Some(geometry) = map.layer_geometry(layer) else {
            continue;
        };
        let origin = geometry.loc.to_physical_precise_round(scale);
        let tree = SurfaceTree::from_surface(layer.wl_surface());
        let elements: Vec<_> =
            AsRenderElements::<R>::render_elements(&tree, renderer, origin, scale, 1.0);
        out.extend(elements.into_iter().map(ChromeRenderElements::Surface));
    }
    out
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
///
/// `glyph_keys` pairs each glyph with the element it belongs to. Consecutive
/// glyphs sharing one key draw with one stable run of ids, so the damage
/// tracker keys a whole element (say, one pane's line numbers) to one id run
/// rather than to a position in the batch.
fn glyph_elements<R: Renderer + ImportMem>(
    chrome: &mut Chrome,
    renderer: &mut R,
    glyphs: &[GlyphQuad],
    glyph_keys: &[ElementKey],
    render_scale: f64,
) -> Vec<TextureRenderElement<<R as RendererSuper>::TextureId>>
where
    <R as RendererSuper>::TextureId: Clone + 'static,
{
    if glyphs.is_empty() {
        return Vec::new();
    }
    debug_assert_eq!(
        glyphs.len(),
        glyph_keys.len(),
        "every glyph must carry the key of the element it belongs to"
    );
    let Some(texture) = chrome.atlas_texture(renderer) else {
        return Vec::new();
    };
    let context_id = renderer.context_id();
    let atlas_size = chrome.atlas.texture_size as f64;

    let mut elements = Vec::with_capacity(glyphs.len());
    let mut start = 0;
    while start < glyphs.len() {
        let key = &glyph_keys[start];
        let mut end = start + 1;
        while end < glyphs.len() && &glyph_keys[end] == key {
            end += 1;
        }
        let ids = chrome.element_ids(key, end - start);
        for (g, id) in glyphs[start..end].iter().zip(ids) {
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
            elements.push(TextureRenderElement::from_static_texture(
                id,
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
            ));
        }
        start = end;
    }
    elements
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
    clients: &HashMap<WindowId, crate::client::Client>,
    output: &Output,
) {
    let frame_time = Clock::<Monotonic>::new().now() + Duration::from_millis(16);
    // `geometry` is the active workspace's layout, so a window on a hidden
    // workspace is not in it and correctly gets nothing: it is not on screen,
    // and telling it to redraw would be asking for work nobody can see.
    for (id, _) in geometry {
        // An X11 window that has not yet been paired with its Wayland surface
        // has nothing to send a callback to, and that is a normal moment rather
        // than an error — see `Client::wl_surface`.
        let Some(surface) = clients.get(id).and_then(|c| c.wl_surface()) else {
            continue;
        };
        send_frames_surface_tree(
            &surface,
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

    /// Where a stage sits in the frame, or a panic naming the one that is
    /// missing — an unwrap here would report "called on a None value" about an
    /// array whose contents are the subject of every assertion below.
    fn depth(stage: Stage) -> usize {
        FRONT_TO_BACK
            .iter()
            .position(|s| *s == stage)
            .unwrap_or_else(|| panic!("{stage:?} is not emitted at all"))
    }

    #[test]
    fn a_client_bar_is_drawn_in_front_of_the_compositors_own_statusline() {
        // The bug this exists for: `Top` was emitted after `Chrome`, so a bar
        // anchored to the bottom of the output landed under ruster's statusline
        // and was invisible — while still mapping, configuring and rendering an
        // element every frame, so every log said it was working.
        assert!(
            depth(Stage::Layer(WlrLayer::Top)) < depth(Stage::Chrome),
            "a bar must beat the statusline it overlaps"
        );
        // A lock screen or a notification beats a bar, and both beat chrome.
        assert!(depth(Stage::Layer(WlrLayer::Overlay)) < depth(Stage::Layer(WlrLayer::Top)));
        // The pointer is in front of all of it, including a lock screen.
        assert_eq!(depth(Stage::Cursor), 0);
    }

    #[test]
    fn an_x11_menu_is_drawn_in_front_of_the_window_that_opened_it() {
        // Override-redirect windows are menus, tooltips and drag icons. Behind
        // the windows they are indistinguishable from a menu that never opened —
        // the exact failure `xdg_popup` had before popups were tracked, one
        // protocol over — and in front of the chrome they would cover a
        // statusline they know nothing about.
        assert!(depth(Stage::X11Unmanaged) < depth(Stage::Windows));
        assert!(depth(Stage::X11Unmanaged) > depth(Stage::Chrome));
    }

    #[test]
    fn wallpaper_layers_stay_behind_the_windows_they_are_behind() {
        // The other half: `Bottom` and `Background` are *below* the windows, so
        // a wallpaper setter cannot paint over the desktop it decorates.
        for below in [WlrLayer::Bottom, WlrLayer::Background] {
            assert!(
                depth(Stage::Layer(below)) > depth(Stage::Windows),
                "{below:?} must stay behind the windows"
            );
        }
        // And the wallpaper is behind the layer above it, not level with it.
        assert!(
            depth(Stage::Layer(WlrLayer::Background)) > depth(Stage::Layer(WlrLayer::Bottom)),
            "the wallpaper is the backmost thing there is"
        );
    }

    #[test]
    fn every_layer_is_emitted_exactly_once() {
        // A layer dropped from the array is a protocol the compositor advertises
        // and then silently ignores; a layer listed twice draws it over itself.
        for which in [
            WlrLayer::Background,
            WlrLayer::Bottom,
            WlrLayer::Top,
            WlrLayer::Overlay,
        ] {
            let n = FRONT_TO_BACK
                .iter()
                .filter(|s| **s == Stage::Layer(which))
                .count();
            assert_eq!(n, 1, "{which:?} is emitted {n} times");
        }
        assert_eq!(
            FRONT_TO_BACK.len(),
            8,
            "four layers, plus cursor/chrome/windows/x11-unmanaged"
        );
    }

    use crate::compositor::HoverPanel;
    use crate::highlight::Highlights;
    use crate::keymap::Keymap;
    use crate::minibuffer::{MiniBuffer, Prompt};
    use crate::pane::{EditorPane, Panes};
    use crate::scene;
    use ruster_core::buffer::Buffer;
    use ruster_core::workspace::BufferStore;
    use ruster_lsp::state::LspState;
    use ruster_render::{Color, Theme};
    use ruster_render_elements::{div, layout, Elem, PxRect, Styled};
    use ruster_render_gles::tessellate::GlesTextMeasurer;
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
        e.id(id)
            .absolute()
            .position(x, y)
            .size(40.0, 40.0)
            .bg(Color::Rgb(1, 2, 3));
        e
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
        let laid = layout(
            PxRect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 600.0,
            },
            &base,
            &mut measurer,
        );
        let order: Vec<&str> = laid.boxes.iter().map(|b| b.key.last().unwrap()).collect();
        assert_eq!(
            order,
            ["borders", "statusline", "minibuffer", "whichkey", "pane"]
        );
    }

    /// `chrome_scene` assembles the real widgets from a `FrameInput` and hands
    /// the overlay layer back as its own element — the launcher and the hover
    /// panel, the only chrome drawn over the base batch.
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
            clients: &toplevels,
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
            x11_unmanaged: &[],
            launcher: None,
        };

        let mut measurer = GlesTextMeasurer;
        let (base, overlay) = scene::chrome_scene(&frame, &theme(), &mut measurer);

        // The overlay is exactly the hover panel: one rounded panel, one line.
        let overlay_laid = layout(
            PxRect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 600.0,
            },
            overlay.as_ref().unwrap(),
            &mut measurer,
        );
        assert_eq!(overlay_laid.boxes.len(), 1);
        assert_eq!(overlay_laid.texts.len(), 1);

        // The base is the frame in painter's order: window borders, statusline,
        // mini-buffer, then the pane. (No which-key in this frame.)
        let base_laid = layout(
            PxRect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 600.0,
            },
            &base,
            &mut measurer,
        );
        let mut surfaces: Vec<&str> = Vec::new();
        for b in &base_laid.boxes {
            let top = b.key.0.first().map(String::as_str).unwrap_or("");
            if surfaces.last().copied() != Some(top) {
                surfaces.push(top);
            }
        }
        assert_eq!(
            surfaces,
            ["window-borders", "statusline", "minibuffer", "pane:0"]
        );

        // With a launcher open the overlay carries it too — in front of the
        // hover panel, because the launcher owns the screen while it is open
        // and the hover explains text that may sit under it.
        let launcher_frame = FrameInput {
            x11_unmanaged: &[],
            launcher: Some(ruster_render::LauncherView {
                query: "fire".into(),
                rows: vec![ruster_render::LauncherRow {
                    label: "Firefox".into(),
                    detail: "Web Browser".into(),
                    group: "apps".into(),
                    selected: true,
                }],
                message: String::new(),
                scrolled: 0,
                total: 1,
            }),
            ..frame
        };
        let (_, overlay) = scene::chrome_scene(&launcher_frame, &theme(), &mut measurer);
        let overlay_laid = layout(
            PxRect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 600.0,
            },
            overlay.as_ref().unwrap(),
            &mut measurer,
        );
        let keys: Vec<&str> = overlay_laid
            .boxes
            .iter()
            .map(|b| b.key.last().unwrap())
            .collect();
        let launcher_at = keys
            .iter()
            .position(|k| *k == "launcher")
            .expect("the launcher panel is in the overlay");
        let hover_at = keys
            .iter()
            .position(|k| *k == "hover")
            .expect("the hover panel is still in the overlay");
        assert!(
            launcher_at < hover_at,
            "the launcher is drawn before (in front of) the hover panel"
        );
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
