//! Compositor render loop: builds the render elements for the focused
//! toplevel and composites them onto the output with the GLES renderer.
//!
//! Phase 0 draws the focused xdg toplevel fullscreen over a plain clear
//! color, then ruster's chrome (statusline, editor frame, which-key) on top of
//! it (Task 8). Chrome geometry is produced by [`Chrome`] as a vertex batch and
//! converted into smithay solid-color render elements — the chrome text glyphs
//! are solid blocks sized by the atlas until Task 13 rasterizes them.

use std::collections::HashMap;
use std::time::Duration;

use ruster_shell::WindowId;
use smithay::backend::renderer::{
    damage::{Error as OutputDamageTrackerError, OutputDamageTracker},
    element::{
        solid::SolidColorRenderElement, surface::WaylandSurfaceRenderElement, AsRenderElements,
    },
    gles::GlesRenderer,
    Color32F, ImportAll, Renderer, RendererSuper,
};
use smithay::desktop::space::SurfaceTree;
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::output::Output;
use smithay::utils::{Clock, Monotonic, Physical, Rectangle, Scale};
use smithay::wayland::shell::xdg::ToplevelSurface;

use crate::chrome::{solid_elements_from_verts, translate_verts, Chrome};

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
) -> Result<Option<Vec<Rectangle<i32, Physical>>>, RenderError> {
    let elements = collect_render_elements(
        focus,
        toplevels,
        output,
        chrome,
        workspace,
        focused_title,
        renderer,
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
pub fn collect_render_elements<R: Renderer + ImportAll>(
    focus: Option<WindowId>,
    toplevels: &HashMap<WindowId, ToplevelSurface>,
    output: &Output,
    chrome: &mut Option<Chrome>,
    workspace: u32,
    focused_title: &str,
    renderer: &mut R,
) -> Vec<ChromeRenderElements<R>>
where
    <R as RendererSuper>::TextureId: Clone + 'static,
{
    let mut elements: Vec<ChromeRenderElements<R>> = Vec::new();

    // Chrome is drawn unconditionally and sits above the client surface; the
    // statusline bar spans the bottom of the output, the which-key overlay
    // floats top-left, and the welcome editor frame is centred.
    if let Some(chrome) = chrome {
        let size = output
            .current_mode()
            .map(|mode| mode.size)
            .unwrap_or_default();
        let mut verts = Vec::new();
        chrome.draw_statusline(size.w, size.h, workspace, focused_title, &mut verts);

        let editor_start = verts.len();
        let frame_w = (size.w / 2).clamp(120, 360);
        let frame_h = (size.h / 2).clamp(80, 240);
        let welcome: Vec<String> = WELCOME_BUFFER.iter().map(|line| line.to_string()).collect();
        chrome.draw_editor_frame(frame_w, frame_h, &welcome, "welcome", &mut verts);
        let editor_end = verts.len();
        translate_verts(
            &mut verts[editor_start..editor_end],
            ((size.w - frame_w) / 2) as f32,
            ((size.h - frame_h) / 2) as f32,
        );

        chrome.draw_whichkey(
            &WHICHKEY_BINDS
                .iter()
                .map(|(k, d)| (k.to_string(), d.to_string()))
                .collect::<Vec<_>>(),
            &mut verts,
        );

        elements.extend(
            solid_elements_from_verts(&verts)
                .into_iter()
                .map(ChromeRenderElements::Solid),
        );
    }

    if let Some(surface) = focus.and_then(|id| toplevels.get(&id)) {
        let wl_surface = surface.wl_surface().clone();
        let tree = SurfaceTree::from_surface(&wl_surface);
        let scale = Scale::from(output.current_scale().fractional_scale());
        elements.extend(
            AsRenderElements::<R>::render_elements(&tree, renderer, (0, 0).into(), scale, 1.0)
                .into_iter()
                .map(ChromeRenderElements::Surface),
        );
    }

    elements
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
