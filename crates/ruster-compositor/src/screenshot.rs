//! Capture the composited output to a PNG.
//!
//! This exists to make a DRM session able to describe itself. Nested, the
//! screen can be captured from outside with `grim`; on a real boot the
//! compositor *is* the display server, and it implements no screencopy
//! protocol, so nothing outside it can see the screen. Without this, every
//! claim about a hardware boot rests on someone's description of what they saw —
//! which is the same footing that let three defects hide behind a verification
//! matrix of "not run" rows.
//!
//! The readback goes through smithay's `ExportMem`, so it works on whichever
//! renderer the backend supplies: `GlesRenderer` nested, `MultiRenderer` on DRM.

use std::path::{Path, PathBuf};

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{ExportMem, Renderer, RendererSuper};
use smithay::utils::{Buffer as BufferCoord, Rectangle, Size};

// The DRM capture reaches for smithay's drm module, which only exists when the
// udev backend is compiled in — a nested build has no swapchain to blit out of.
#[cfg(feature = "udev")]
use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
#[cfg(feature = "udev")]
use smithay::backend::allocator::Buffer as AllocBuffer;
#[cfg(feature = "udev")]
use smithay::backend::drm::compositor::RenderFrameResult;
#[cfg(feature = "udev")]
use smithay::backend::drm::Framebuffer as DrmFramebuffer;
#[cfg(feature = "udev")]
use smithay::backend::renderer::element::{Element, RenderElement};
#[cfg(feature = "udev")]
use smithay::backend::renderer::gles::GlesTexture;
#[cfg(feature = "udev")]
use smithay::backend::renderer::{Bind, Blit, Offscreen};
#[cfg(feature = "udev")]
use smithay::utils::{Physical, Scale, Transform};

/// Where captures go, and under what name.
///
/// `$XDG_RUNTIME_DIR` because it is guaranteed writable and per-session, and
/// needs no display server to resolve — `~` would work too, but a capture is a
/// transient artifact and this keeps it out of the way. Falls back to `/tmp`
/// when the variable is unset, as it is on a bare VT login.
pub fn capture_path(index: u32) -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    Path::new(&dir).join(format!("ruster-shot-{index}.png"))
}

/// Read the framebuffer back and write it out as a PNG.
///
/// Called after the frame is rendered and before it is submitted: the contents
/// are complete by then, and `copy_framebuffer` is documented as
/// non-destructive, so the frame the user sees is unaffected.
pub fn capture<R>(
    renderer: &mut R,
    framebuffer: &<R as RendererSuper>::Framebuffer<'_>,
    size: Size<i32, BufferCoord>,
    path: &Path,
) -> Result<PathBuf, String>
where
    R: Renderer + ExportMem,
{
    let region = Rectangle::from_size(size);
    // Abgr8888 is bytes R,G,B,A in memory on a little-endian machine, which is
    // exactly what the PNG encoder wants — asking for the other order would mean
    // swapping every pixel by hand.
    let mapping = renderer
        .copy_framebuffer(framebuffer, region, Fourcc::Abgr8888)
        .map_err(|err| format!("failed to copy the framebuffer: {err}"))?;
    let pixels = renderer
        .map_texture(&mapping)
        .map_err(|err| format!("failed to map the captured framebuffer: {err}"))?;

    let upright = flip_rows(pixels, size.w as usize, size.h as usize);
    write_png(path, &upright, size.w as u32, size.h as u32)?;

    // Confirm the file arrived rather than reporting a save that did not
    // happen — the raylib backend's capture learned the same lesson.
    if path.is_file() {
        Ok(path.to_path_buf())
    } else {
        Err(format!("{} was not written", path.display()))
    }
}

/// Capture a DRM frame that has been composited but not yet scanned out.
///
/// The winit path can hand [`capture`] the framebuffer it just drew into. On
/// DRM there is no such framebuffer to borrow: the frame lives in a swapchain
/// buffer owned by the `DrmOutput`, destined for the display and not for us. So
/// the composited result is blitted into an offscreen texture we do own, and
/// read back from there.
///
/// This is the whole reason the screenshot exists — a DRM session implements no
/// screencopy protocol, so nothing outside the compositor can see its screen —
/// and for a while it was implemented only on the backend that did not need it.
#[cfg(feature = "udev")]
pub fn capture_drm_frame<R, B, F, E>(
    result: &RenderFrameResult<'_, B, F, E>,
    renderer: &mut R,
    size: Size<i32, Physical>,
    transform: Transform,
    path: &Path,
) -> Result<PathBuf, String>
where
    R: Renderer + Bind<Dmabuf> + Bind<GlesTexture> + Offscreen<GlesTexture> + Blit + ExportMem,
    <R as RendererSuper>::TextureId: 'static,
    B: AllocBuffer + AsDmabuf,
    <B as AsDmabuf>::Error: std::fmt::Debug + Send + Sync + 'static,
    F: DrmFramebuffer,
    E: Element + RenderElement<R>,
{
    // `Size<_, Buffer>` is what both the allocator and the readback speak; the
    // output's size is in physical pixels and the two are numerically the same
    // here because the capture buffer has no scale of its own.
    let buffer_size = Size::<i32, BufferCoord>::from((size.w, size.h));
    let mut target = renderer
        .create_buffer(Fourcc::Abgr8888, buffer_size)
        .map_err(|err| format!("failed to allocate the capture buffer: {err}"))?;

    // The framebuffer borrows the *target*, not the renderer, so the renderer
    // stays usable for the blit and the readback that follow.
    let mut framebuffer = renderer
        .bind(&mut target)
        .map_err(|err| format!("failed to bind the capture buffer: {err}"))?;

    // Blit the whole output rather than only the frame's damage: a freshly
    // allocated buffer is blank, so a damage-only copy of a mostly-static
    // screen would come back mostly black.
    let sync = result
        .blit_frame_result(
            size,
            transform,
            Scale::from(1.0),
            renderer,
            &mut framebuffer,
            [Rectangle::from_size(size)],
            [],
        )
        .map_err(|err| format!("failed to blit the frame: {err:?}"))?;

    // The blit is asynchronous. Reading the texture back before the GPU has
    // finished writing it captures whatever was there first — which for a
    // freshly allocated buffer is a plausible-looking black frame, the most
    // misleading result this could produce.
    sync.wait()
        .map_err(|_| "interrupted waiting for the frame blit".to_string())?;

    capture(renderer, &framebuffer, buffer_size, path)
}

/// Reverse the row order of an RGBA image.
///
/// GL's framebuffer origin is bottom-left and a PNG's is top-left, so a
/// straight readback is upside down. That is a property of reading a GL
/// framebuffer and holds on both backends, independently of any output
/// transform: nested, the screen is upright because the output carries
/// `Transform::Flipped180`, but that is applied on the way *out* to the display
/// and says nothing about what a read-back sees — which is how a capture can be
/// inverted while the screen it captured is not.
fn flip_rows(pixels: &[u8], width: usize, height: usize) -> Vec<u8> {
    let stride = width * 4;
    let mut out = Vec::with_capacity(stride * height);
    for row in (0..height).rev() {
        let start = row * stride;
        match pixels.get(start..start + stride) {
            Some(slice) => out.extend_from_slice(slice),
            None => break,
        }
    }
    out
}

fn write_png(path: &Path, pixels: &[u8], width: u32, height: u32) -> Result<(), String> {
    let expected = width as usize * height as usize * 4;
    if pixels.len() < expected {
        return Err(format!(
            "framebuffer is {} bytes, expected {expected} for {width}x{height}",
            pixels.len()
        ));
    }
    let file = std::fs::File::create(path)
        .map_err(|err| format!("cannot create {}: {err}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|err| format!("cannot write the png header: {err}"))?
        .write_image_data(&pixels[..expected])
        .map_err(|err| format!("cannot write the png data: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_land_in_the_runtime_dir_when_there_is_one() {
        // Safety: this test sets a process-wide variable, but it only reads it
        // back immediately and no other test in this module touches it.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", "/run/user/testing") };
        assert_eq!(
            capture_path(3),
            PathBuf::from("/run/user/testing/ruster-shot-3.png")
        );
        unsafe { std::env::remove_var("XDG_RUNTIME_DIR") };
        // A bare VT login often has no XDG_RUNTIME_DIR, and a capture that
        // cannot find a home is a capture that does not happen.
        assert_eq!(capture_path(0), PathBuf::from("/tmp/ruster-shot-0.png"));
    }

    #[test]
    fn a_png_round_trips_its_pixels() {
        let dir = std::env::temp_dir().join("ruster-shot-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("one.png");
        // 2x1: one opaque red pixel, one opaque blue.
        let pixels = [255u8, 0, 0, 255, 0, 0, 255, 255];
        write_png(&path, &pixels, 2, 1).unwrap();

        let decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(&buf[..8], &pixels, "the channels survive the round trip");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_capture_comes_out_the_right_way_up() {
        // Two rows, one red then one blue as GL hands them over; the image must
        // open with blue on top. Caught by looking at the first real capture,
        // which was a perfectly good picture of the screen upside down.
        let red = [255u8, 0, 0, 255];
        let blue = [0u8, 0, 255, 255];
        let gl_order: Vec<u8> = red.iter().chain(blue.iter()).copied().collect();
        let flipped = flip_rows(&gl_order, 1, 2);
        assert_eq!(&flipped[..4], &blue, "the last GL row is the top PNG row");
        assert_eq!(&flipped[4..], &red);
    }

    #[test]
    fn flipping_an_odd_sized_image_keeps_every_row() {
        let pixels: Vec<u8> = (0..3u8 * 2 * 4).collect();
        let flipped = flip_rows(&pixels, 2, 3);
        assert_eq!(flipped.len(), pixels.len());
        // Row 2 of the source becomes row 0 of the output.
        assert_eq!(&flipped[..8], &pixels[16..24]);
    }

    #[test]
    fn a_short_framebuffer_is_refused_rather_than_written() {
        // Encoding past the end of the buffer would panic inside the encoder;
        // a partial capture reported as a success would be worse still.
        let path = std::env::temp_dir().join("ruster-shot-short.png");
        let err = write_png(&path, &[0; 4], 2, 1).unwrap_err();
        assert!(err.contains("expected 8"), "got: {err}");
        assert!(!path.is_file(), "nothing should be left behind");
    }
}
