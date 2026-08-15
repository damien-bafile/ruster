//! `wlr-screencopy-v1`: letting a client read the screen.
//!
//! smithay 0.7 does not implement this protocol, so the handlers are ours; only
//! the bindings are generated, out of `wayland-protocols-wlr`.
//!
//! It exists for the same reason the keybind screenshot does — a DRM session is
//! the display server, so nothing outside it can see the screen — but it solves
//! the problem the right way round. The keybind writes a PNG the compositor
//! chose the name of, on the compositor's schedule, and only if a frame happens
//! to be rendered; with this, `grim` works, and so does every screen recorder
//! and portal that speaks it. Verifying anything visual stops being a bespoke
//! harness and becomes a subprocess.
//!
//! The copy is deferred to the render pass rather than served where it is
//! requested, for the same reason the screenshot is: reading the framebuffer
//! needs the renderer and a finished frame, and the request arrives with
//! neither. That does mean a capture waits for a frame, and a compositor whose
//! host is not presenting renders none — [`Pending::asked`] is what lets that be
//! reported rather than hang, which is the defect the keybind screenshot had.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_shm};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Physical, Size};
use wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};

use crate::backend::Backend;
use crate::compositor::CompositorState;

/// The pixel format every capture is served in.
///
/// One format, advertised once: `Xbgr8888` is bytes R,G,B,X in memory on a
/// little-endian machine, which is what `copy_framebuffer` already returns and
/// what `grim` writes into a PNG without swizzling. Offering a menu of formats
/// would mean converting between them here for no reader that wants the others.
const FORMAT: wl_shm::Format = wl_shm::Format::Xbgr8888;

/// How long a capture may wait for a frame before it is failed.
///
/// The same reasoning as the screenshot keybind's timeout, and the same number.
/// A client that asked politely deserves `failed` rather than a frame callback
/// that never comes: `grim` blocks forever on a promise, which is exactly how
/// this session lost seven minutes to a blanked screen.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// A capture that has been asked for and needs a rendered frame to serve.
#[derive(Debug)]
pub struct Pending {
    pub frame: ZwlrScreencopyFrameV1,
    pub buffer: WlBuffer,
    /// When the client asked, so a capture that will never be served can be
    /// failed instead of left waiting.
    pub asked: Instant,
    /// Whether the client wants damage reported (`copy_with_damage`). We always
    /// report the whole output, which is permitted and honest: the damage
    /// tracker's rectangles describe what *ruster* redrew, not what the buffer
    /// differs by.
    pub with_damage: bool,
}

/// Captures waiting on a frame.
#[derive(Debug, Default)]
pub struct ScreencopyState {
    pub pending: Vec<Pending>,
}

/// Whether a capture asked for at `asked` has waited long enough to be failed.
///
/// Split out from [`ScreencopyState::expire`] because everything else there
/// needs a live `wl_resource` and therefore a client: the decision is the part
/// worth a test, and inline it could only have been checked by talking to a real
/// compositor over a socket.
pub fn overdue(asked: Instant, now: Instant) -> bool {
    now.duration_since(asked) >= TIMEOUT
}

impl ScreencopyState {
    /// How long until the oldest waiting capture must be given up on.
    ///
    /// The loop blocks until something happens, so a deadline nothing else
    /// wakes it for is a deadline that never arrives — and a client that asked
    /// politely then waits forever rather than being told `failed`.
    pub fn next_deadline(&self, now: Instant) -> Option<std::time::Duration> {
        let oldest = self.pending.iter().map(|p| p.asked).min()?;
        Some(TIMEOUT.saturating_sub(now.saturating_duration_since(oldest)))
    }

    /// Fail every capture that has waited too long, and say why once.
    ///
    /// Returns how many were failed, which is what the test asserts on — a
    /// `failed` event is not observable from inside this process.
    pub fn expire(&mut self, now: Instant) -> usize {
        let (dead, alive): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| overdue(p.asked, now));
        self.pending = alive;
        if !dead.is_empty() {
            tracing::warn!(
                count = dead.len(),
                "screencopy: no frame was rendered in time, failing the capture. \
                 Rendering waits for the host to invite a frame, and a nested \
                 window that is not being presented is never invited"
            );
        }
        for p in &dead {
            p.frame.failed();
        }
        dead.len()
    }
}

/// The state a frame resource carries: what it was asked to capture.
#[derive(Debug, Clone, Copy)]
pub struct FrameData {
    pub size: Size<i32, Physical>,
}

impl FrameData {
    /// Bytes per row, which the client needs to size its buffer.
    pub fn stride(&self) -> u32 {
        self.size.w as u32 * 4
    }
}

impl<B: Backend + 'static> GlobalDispatch<ZwlrScreencopyManagerV1, ()> for CompositorState<B> {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl<B: Backend + 'static> Dispatch<ZwlrScreencopyManagerV1, ()> for CompositorState<B> {
    fn request(
        state: &mut Self,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        // Region captures are answered with the whole output rather than
        // refused. A client that asked for a rectangle and got the screen can
        // crop; one that got `failed` gets nothing, and `grim -g` is common
        // enough that refusing it would be the difference between working and
        // not for a routine invocation.
        let (frame, overlay_cursor) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                ..
            } => (frame, overlay_cursor),
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                ..
            } => (frame, overlay_cursor),
            zwlr_screencopy_manager_v1::Request::Destroy => return,
            _ => return,
        };
        if overlay_cursor != 0 {
            // The cursor is composited into the frame already, since it is drawn
            // as a render element rather than by a hardware plane. Saying so
            // beats silently ignoring a flag the client set deliberately.
            tracing::debug!("screencopy: cursor is always included in the capture");
        }
        let Some(size) = state.output_size_physical() else {
            let frame = data_init.init(
                frame,
                FrameData {
                    size: (0, 0).into(),
                },
            );
            frame.failed();
            return;
        };
        let data = FrameData { size };
        let frame = data_init.init(frame, data);
        frame.buffer(FORMAT, size.w as u32, size.h as u32, data.stride());
        // `buffer_done` is what tells the client the format list is complete.
        // Without it a v3 client waits forever having been told everything it
        // needs — the protocol's own version of a promise never kept.
        if frame.version() >= 3 {
            frame.buffer_done();
        }
    }
}

impl<B: Backend + 'static> Dispatch<ZwlrScreencopyFrameV1, FrameData> for CompositorState<B> {
    fn request(
        state: &mut Self,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &FrameData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let (buffer, with_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            zwlr_screencopy_frame_v1::Request::Destroy => {
                state
                    .screencopy
                    .pending
                    .retain(|p| p.frame.id() != frame.id());
                return;
            }
            _ => return,
        };
        // The buffer is checked here rather than at copy time so a client that
        // sized it wrong learns immediately, with the frame it asked about,
        // rather than one render pass later when the reason is less obvious.
        let ok = smithay::wayland::shm::with_buffer_contents(&buffer, |_, len, spec| {
            spec.format == FORMAT
                && spec.width == data.size.w
                && spec.height == data.size.h
                && spec.stride == data.stride() as i32
                && len >= (data.stride() as usize * data.size.h.max(0) as usize)
        });
        if !matches!(ok, Ok(true)) {
            tracing::debug!(
                ?ok,
                "screencopy: the client's buffer is not what was offered"
            );
            frame.failed();
            return;
        }
        tracing::debug!(
            w = data.size.w,
            h = data.size.h,
            with_damage,
            "screencopy: capture queued"
        );
        state.screencopy.pending.push(Pending {
            frame: frame.clone(),
            buffer,
            asked: Instant::now(),
            with_damage,
        });
        // And ask for the frame it needs, rather than hoping one arrives.
        state.backend_data.request_redraw();
    }
}

/// Serve every waiting capture out of the frame that has just been rendered.
///
/// Called from the render pass, after the frame is drawn and before it is
/// submitted — the same place and for the same reason as the keybind
/// screenshot: `copy_framebuffer` needs the renderer and finished contents, and
/// the request arrived with neither.
///
/// Takes the pending list by value so a capture is served exactly once. Leaving
/// entries in place and clearing them afterwards would re-serve every one of
/// them on the next frame, which for a screen recorder is every frame it ever
/// asked for, all at once.
pub fn serve<R>(
    pending: Vec<Pending>,
    renderer: &mut R,
    framebuffer: &<R as smithay::backend::renderer::RendererSuper>::Framebuffer<'_>,
    size: Size<i32, Physical>,
) where
    R: smithay::backend::renderer::Renderer + smithay::backend::renderer::ExportMem,
{
    if pending.is_empty() {
        return;
    }
    tracing::debug!(
        count = pending.len(),
        w = size.w,
        h = size.h,
        "screencopy: serving"
    );
    // Read the framebuffer once for all of them: two clients recording at the
    // same time should cost one readback, and the readback is the expensive
    // half.
    let region = smithay::utils::Rectangle::from_size((size.w, size.h).into());
    let mapping = match renderer.copy_framebuffer(
        framebuffer,
        region,
        smithay::backend::allocator::Fourcc::Xbgr8888,
    ) {
        Ok(mapping) => mapping,
        Err(err) => {
            tracing::warn!(%err, "screencopy: could not read the framebuffer");
            for p in &pending {
                p.frame.failed();
            }
            return;
        }
    };
    let pixels = match renderer.map_texture(&mapping) {
        Ok(pixels) => pixels,
        Err(err) => {
            tracing::warn!(%err, "screencopy: could not map the captured framebuffer");
            for p in &pending {
                p.frame.failed();
            }
            return;
        }
    };

    let stride = size.w as usize * 4;
    let height = size.h.max(0) as usize;
    for p in pending {
        let wrote =
            smithay::wayland::shm::with_buffer_contents_mut(&p.buffer, |ptr, len, _spec| {
                let want = stride * height;
                if len < want || pixels.len() < want {
                    return false;
                }
                // Copied straight through, deliberately.
                //
                // A GL framebuffer read is bottom-left first, so the obvious
                // thing is to reverse the rows — and that is what this did, and
                // it produced an upside-down capture. The winit output carries
                // `Transform::Flipped180`, which the renderer has already applied
                // when compositing, and that cancels the readback's own
                // inversion. Flipping again re-introduces it.
                //
                // Which way up a buffer is cannot be reasoned about from one end
                // alone; this is what the capture actually looks like.
                unsafe {
                    std::ptr::copy_nonoverlapping(pixels.as_ptr(), ptr, stride * height);
                }
                true
            });
        match wrote {
            Ok(true) => {
                if p.with_damage {
                    // The whole output, which is permitted: the damage tracker's
                    // rectangles describe what ruster redrew, not how this buffer
                    // differs from the client's last one.
                    p.frame.damage(0, 0, size.w as u32, size.h as u32);
                }
                p.frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
                send_ready(&p.frame);
            }
            other => {
                tracing::warn!(
                    ?other,
                    "screencopy: could not write into the client's buffer"
                );
                p.frame.failed();
            }
        }
    }
}

/// Tell a client its capture is ready, with the time it was taken.
///
/// The protocol wants a `CLOCK_MONOTONIC`-ish presentation timestamp split into
/// a 64-bit seconds value across two 32-bit fields. Clients use it to order
/// frames; `grim` ignores it entirely, which is not a reason to send nonsense.
pub fn send_ready(frame: &ZwlrScreencopyFrameV1) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    frame.ready(
        (secs >> 32) as u32,
        (secs & 0xFFFF_FFFF) as u32,
        now.subsec_nanos(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_is_failed_only_after_it_has_really_waited() {
        // The number matters in both directions. Too eager and a capture on a
        // busy host is failed while its frame is on the way; never, and `grim`
        // blocks forever on a promise — which is how this session lost seven
        // minutes to a blanked screen.
        let asked = Instant::now();
        assert!(!overdue(asked, asked));
        assert!(!overdue(
            asked,
            asked + std::time::Duration::from_millis(500)
        ));
        assert!(!overdue(
            asked,
            asked + TIMEOUT - std::time::Duration::from_millis(1)
        ));
        assert!(overdue(asked, asked + TIMEOUT));
        assert!(overdue(asked, asked + std::time::Duration::from_secs(30)));
    }

    #[test]
    fn the_stride_is_the_row_a_client_must_allocate() {
        // Offered to the client in the `buffer` event and checked against what
        // it allocates, so a wrong answer here is a capture that either fails
        // for a correct client or overruns the buffer of one that trusted us.
        let data = FrameData {
            size: (1920, 1080).into(),
        };
        assert_eq!(data.stride(), 1920 * 4);
        assert_eq!(
            data.stride() as usize * 1080,
            1920 * 1080 * 4,
            "four bytes a pixel, no padding"
        );
        let odd = FrameData {
            size: (1873, 1334).into(),
        };
        assert_eq!(odd.stride(), 1873 * 4, "an odd width is not rounded up");
    }
}
