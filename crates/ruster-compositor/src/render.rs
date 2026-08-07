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

/// The welcome buffer shown in the editor frame until an embedded editor
/// provides real content (Phase 3).
const WELCOME_BUFFER: &[&str] = &[
    "RUSTER  v0.1.0",
    "────────────",
    "EXWM-style Wayland compositor",
    "M-t  cycle workspace",
    "M-S-q quit",
];

/// The which-key bindings advertised on the overlay (Task 10 makes them real).
const WHICHKEY_BINDS: &[(&str, &str)] = &[("M-t", "cycle workspace"), ("M-S-q", "quit")];

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
/// `chrome`/`workspace`/`focused_title` carry the compositor's chrome state and
/// shell focus in so the function stays a free, testable function (it does not
/// need the whole [`CompositorState`](crate::compositor::CompositorState)).
#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    focus: Option<WindowId>,
    toplevels: &HashMap<WindowId, ToplevelSurface>,
    damage_tracker: &mut OutputDamageTracker,
    output: &Output,
    chrome: &mut Option<Chrome>,
    workspace: u32,
    focused_title: &str,
    renderer: &mut GlesRenderer,
    framebuffer: &mut <GlesRenderer as RendererSuper>::Framebuffer<'_>,
    age: usize,
    cursor_status: &CursorImageStatus,
    cursor_location: Point<f64, Logical>,
    geometry: &[(WindowId, ruster_shell::Rect)],
    tree_status: TreeStatus,
) -> Result<Option<Vec<Rectangle<i32, Physical>>>, RenderError> {
    let elements = collect_render_elements(
        focus,
        toplevels,
        output,
        chrome,
        workspace,
        focused_title,
        renderer,
        cursor_status,
        cursor_location,
        geometry,
        tree_status,
    );
    let result =
        damage_tracker.render_output(renderer, framebuffer, age, &elements, CLEAR_COLOR)?;
    let damage = result.damage.cloned();
    send_frame_callbacks(focus, toplevels, output);
    Ok(damage)
}

/// Build the render elements for one output frame: ruster's chrome (statusline,
/// editor frame, which-key) followed by the focused toplevel, both composited
/// by the renderer in front-to-back order. Generic over the renderer so both
/// the winit (`GlesRenderer`) and DRM (`MultiRenderer`) backends share it.
#[allow(clippy::too_many_arguments)]
pub fn collect_render_elements<R: Renderer + ImportAll + ImportMem>(
    // Kept for the chrome, which still names the focused window; the surfaces
    // themselves now come from the tree rather than from focus.
    _focus: Option<WindowId>,
    toplevels: &HashMap<WindowId, ToplevelSurface>,
    output: &Output,
    chrome: &mut Option<Chrome>,
    workspace: u32,
    focused_title: &str,
    renderer: &mut R,
    cursor_status: &CursorImageStatus,
    cursor_location: Point<f64, Logical>,
    geometry: &[(WindowId, ruster_shell::Rect)],
    tree_status: TreeStatus,
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
            cursor_status,
            cursor_location,
            output.current_scale().fractional_scale(),
        ));
    }

    // Chrome is drawn unconditionally and sits above the client surface; the
    // statusline bar spans the bottom of the output, the which-key overlay
    // floats top-left, and the welcome editor frame is centred.
    if let Some(chrome) = chrome {
        let size = output
            .current_mode()
            .map(|mode| mode.size)
            .unwrap_or_default();
        let mut batch = ChromeBatch::default();
        chrome.draw_statusline(
            size.w,
            size.h,
            workspace,
            focused_title,
            tree_status,
            &mut batch,
        );

        let editor_mark = batch.mark();
        let frame_w = (size.w / 2).clamp(120, 360);
        let frame_h = (size.h / 2).clamp(80, 240);
        let welcome: Vec<String> = WELCOME_BUFFER.iter().map(|line| line.to_string()).collect();
        chrome.draw_editor_frame(frame_w, frame_h, &welcome, "welcome", &mut batch);
        batch.translate_since(
            editor_mark,
            ((size.w - frame_w) / 2) as f32,
            ((size.h - frame_h) / 2) as f32,
        );

        chrome.draw_whichkey(
            &WHICHKEY_BINDS
                .iter()
                .map(|(k, d)| (k.to_string(), d.to_string()))
                .collect::<Vec<_>>(),
            &mut batch,
        );

        // Glyphs first, then panels. Within a panel the glyphs are drawn on top
        // of its background, and chrome panels never overlap each other, so
        // hoisting every glyph in front of every panel is equivalent to a strict
        // reverse of painter's order and saves interleaving the two lists.
        let render_scale = output.current_scale().fractional_scale();
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
    let scale = Scale::from(output.current_scale().fractional_scale());
    for (id, rect) in geometry.iter().rev() {
        let Some(surface) = toplevels.get(id) else {
            continue;
        };
        let wl_surface = surface.wl_surface().clone();
        let surface_tree = SurfaceTree::from_surface(&wl_surface);
        let origin = Point::<i32, Logical>::from((rect.x, rect.y)).to_physical_precise_round(scale);
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

/// Deliver frame callbacks to the focused toplevel against the time of the next
/// frame (one refresh interval in the future), so its client schedules the next
/// redraw. The 1s throttle only backstops surfaces not on a scan-out output.
pub fn send_frame_callbacks(
    focus: Option<WindowId>,
    toplevels: &HashMap<WindowId, ToplevelSurface>,
    output: &Output,
) {
    if let Some(surface) = focus
        .and_then(|id| toplevels.get(&id))
        .map(|toplevel| toplevel.wl_surface().clone())
    {
        let frame_time = Clock::<Monotonic>::new().now() + Duration::from_millis(16);
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
