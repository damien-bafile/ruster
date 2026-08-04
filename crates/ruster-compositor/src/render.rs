//! Compositor render loop: builds the render elements for the focused
//! toplevel and composites them onto the output with the GLES renderer.
//!
//! Phase 0 draws the focused xdg toplevel fullscreen over a plain clear
//! color; chrome/statusline drawing lands in Task 8.

use std::collections::HashMap;
use std::time::Duration;

use ruster_shell::WindowId;
use smithay::backend::renderer::{
    damage::{Error as OutputDamageTrackerError, OutputDamageTracker},
    element::{surface::WaylandSurfaceRenderElement, AsRenderElements},
    gles::GlesRenderer,
    Color32F, RendererSuper,
};
use smithay::desktop::space::SurfaceTree;
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::output::Output;
use smithay::utils::{Clock, Monotonic, Physical, Rectangle, Scale};
use smithay::wayland::shell::xdg::ToplevelSurface;

/// Background the compositor clears the output to each frame.
const CLEAR_COLOR: Color32F = Color32F::BLACK;

/// Height of the chrome (statusline) bar that Task 8 will draw, derived from
/// the output height: ~2.5% (`height / 40`), clamped to a 24-64px band.
pub fn chrome_height(output_height: i32) -> i32 {
    (output_height / 40).clamp(24, 64)
}

/// Error raised while rendering an output with the GLES renderer.
pub type RenderError = OutputDamageTrackerError<<GlesRenderer as RendererSuper>::Error>;

/// Composite the focused toplevel fullscreen onto the output and render it.
///
/// Returns the damage produced by the render (in physical coordinates) so the
/// caller can submit it to the backend; `None` means nothing changed and no
/// buffer swap is needed. Frame callbacks are sent to the focused surface so
/// its client schedules the next redraw.
pub fn render_frame(
    focus: Option<WindowId>,
    toplevels: &HashMap<WindowId, ToplevelSurface>,
    damage_tracker: &mut OutputDamageTracker,
    output: &Output,
    renderer: &mut GlesRenderer,
    framebuffer: &mut <GlesRenderer as RendererSuper>::Framebuffer<'_>,
    age: usize,
) -> Result<Option<Vec<Rectangle<i32, Physical>>>, RenderError> {
    let mut elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
    let mut frame_surface = None;
    if let Some(surface) = focus.and_then(|id| toplevels.get(&id)) {
        let wl_surface = surface.wl_surface().clone();
        let tree = SurfaceTree::from_surface(&wl_surface);
        let scale = Scale::from(output.current_scale().fractional_scale());
        elements.extend(AsRenderElements::<GlesRenderer>::render_elements(
            &tree,
            renderer,
            (0, 0).into(),
            scale,
            1.0,
        ));
        frame_surface = Some(wl_surface);
    }

    let result =
        damage_tracker.render_output(renderer, framebuffer, age, &elements, CLEAR_COLOR)?;
    let damage = result.damage.cloned();

    if let Some(surface) = frame_surface {
        // Frame callbacks are delivered against the time of the next frame
        // (one refresh interval in the future).
        let frame_time = Clock::<Monotonic>::new().now() + Duration::from_millis(16);
        send_frames_surface_tree(
            &surface,
            output,
            frame_time,
            Some(Duration::from_secs(1)),
            |_, _| None,
        );
    }

    Ok(damage)
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
