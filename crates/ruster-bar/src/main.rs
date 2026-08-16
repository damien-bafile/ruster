//! A bar, as an ordinary layer-shell client.
//!
//! ruster already draws a statusline — but it draws it *itself*, in GL, straight
//! into the frame. That is chrome, not a client, and it exercises none of the
//! `wlr-layer-shell` implementation: the two paths never meet. This is the same
//! bar as a real client, which means the protocol is exercised by ruster's own
//! software on every run rather than by whatever a user happens to have
//! installed.
//!
//! It is deliberately small. It reserves an exclusive zone and fills it, so what
//! it proves is the part that is easy to get wrong and invisible when it is:
//! that the compositor honours the zone and tiles windows *above* the bar rather
//! than underneath it. Text belongs in the version that replaces the statusline,
//! not in the one that proves the protocol.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("ruster-bar needs Wayland, which this platform does not have");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(err) = linux::run() {
        eprintln!("ruster-bar: {err}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::os::fd::AsFd;

    use wayland_client::globals::{registry_queue_init, GlobalListContents};
    use wayland_client::protocol::{
        wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_registry::WlRegistry, wl_shm::Format,
        wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_surface::WlSurface,
    };
    use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle};
    use wayland_protocols_wlr::layer_shell::v1::client::{
        zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
        zwlr_layer_surface_v1::{self, Anchor, ZwlrLayerSurfaceV1},
    };

    /// How tall the bar is, and therefore how much it takes out of the area
    /// windows are tiled into.
    const HEIGHT: u32 = 28;

    pub struct Bar {
        /// Set from the compositor's `configure`, which is the only authority on
        /// how wide the bar is: asking the output would be a second opinion, and
        /// on a scaled output a different one.
        size: (u32, u32),
        configured: bool,
        closed: bool,
        shm: WlShm,
        surface: WlSurface,
    }

    pub fn run() -> Result<(), String> {
        let conn = Connection::connect_to_env()
            .map_err(|e| format!("no Wayland display to connect to: {e}"))?;
        let (globals, mut queue) =
            registry_queue_init::<Bar>(&conn).map_err(|e| format!("registry failed: {e}"))?;
        let qh = queue.handle();

        let compositor: WlCompositor = globals
            .bind(&qh, 1..=6, ())
            .map_err(|e| format!("no wl_compositor: {e}"))?;
        let shm: WlShm = globals
            .bind(&qh, 1..=1, ())
            .map_err(|e| format!("no wl_shm: {e}"))?;
        // The one that matters. A compositor without it cannot host a bar at
        // all, which is the state ruster was in until layer-shell landed.
        let layer_shell: ZwlrLayerShellV1 = globals
            .bind(&qh, 1..=4, ())
            .map_err(|e| format!("this compositor does not speak zwlr_layer_shell_v1: {e}"))?;

        let surface = compositor.create_surface(&qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None, // whichever output the compositor chooses
            Layer::Top,
            "ruster-bar".to_string(),
            &qh,
            (),
        );
        // Anchored to three edges, so the width is the compositor's to decide
        // and only the height is ours.
        layer_surface.set_anchor(Anchor::Bottom | Anchor::Left | Anchor::Right);
        layer_surface.set_size(0, HEIGHT);
        // The whole point: this many pixels are ours, and windows are tiled in
        // what is left.
        layer_surface.set_exclusive_zone(HEIGHT as i32);
        surface.commit();

        let mut bar = Bar {
            size: (0, HEIGHT),
            configured: false,
            closed: false,
            shm,
            surface: surface.clone(),
        };

        while !bar.closed {
            queue
                .blocking_dispatch(&mut bar)
                .map_err(|e| format!("dispatch failed: {e}"))?;
        }
        Ok(())
    }

    impl Bar {
        /// Fill the bar and commit it.
        ///
        /// Two colours rather than one: a solid rectangle proves a buffer was
        /// attached, and proves nothing about *where*. An accent block at the
        /// left end gives the capture something asymmetric to read, so a bar
        /// drawn upside down or at the wrong edge is visible as such.
        fn draw(&mut self, qh: &QueueHandle<Self>) {
            let (w, h) = (self.size.0.max(1), self.size.1.max(1));
            let stride = w * 4;
            let len = (stride * h) as usize;

            let file = match tempfile(len) {
                Ok(file) => file,
                Err(err) => {
                    eprintln!("ruster-bar: could not make a buffer: {err}");
                    return;
                }
            };
            let mut map = match unsafe { memmap2::MmapMut::map_mut(&file) } {
                Ok(map) => map,
                Err(err) => {
                    eprintln!("ruster-bar: could not map the buffer: {err}");
                    return;
                }
            };
            for y in 0..h {
                for x in 0..w {
                    let i = ((y * stride) + x * 4) as usize;
                    // Argb8888 is B,G,R,A in memory on a little-endian machine.
                    let (b, g, r) = if x < h {
                        (0xf7, 0xa6, 0xcb) // the accent block, one square
                    } else {
                        (0x5a, 0x47, 0x45) // the bar itself
                    };
                    map[i] = b;
                    map[i + 1] = g;
                    map[i + 2] = r;
                    map[i + 3] = 0xff;
                }
            }

            let pool: WlShmPool = self.shm.create_pool(file.as_fd(), len as i32, qh, ());
            let buffer: WlBuffer = pool.create_buffer(
                0,
                w as i32,
                h as i32,
                stride as i32,
                Format::Argb8888,
                qh,
                (),
            );
            self.surface.attach(Some(&buffer), 0, 0);
            self.surface.damage_buffer(0, 0, w as i32, h as i32);
            self.surface.commit();
            pool.destroy();
        }
    }

    /// An anonymous file to share with the compositor.
    fn tempfile(len: usize) -> std::io::Result<std::fs::File> {
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        let path = std::path::Path::new(&dir).join(format!("ruster-bar-{}", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        // Unlinked immediately: the fd is what is shared, and leaving the name
        // behind would litter the runtime directory once per run.
        std::fs::remove_file(&path)?;
        file.set_len(len as u64)?;
        Ok(file)
    }

    impl Dispatch<ZwlrLayerSurfaceV1, ()> for Bar {
        fn event(
            state: &mut Self,
            surface: &ZwlrLayerSurfaceV1,
            event: zwlr_layer_surface_v1::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            match event {
                zwlr_layer_surface_v1::Event::Configure {
                    serial,
                    width,
                    height,
                } => {
                    // Acknowledged before drawing: a buffer attached to an
                    // unacknowledged configure is a protocol error, and the
                    // compositor is within its rights to disconnect for it.
                    surface.ack_configure(serial);
                    state.size = (width.max(1), height.max(1));
                    state.configured = true;
                    state.draw(qh);
                }
                zwlr_layer_surface_v1::Event::Closed => state.closed = true,
                _ => {}
            }
        }
    }

    impl Dispatch<WlRegistry, GlobalListContents> for Bar {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: wayland_client::protocol::wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    delegate_noop!(Bar: ignore WlCompositor);
    delegate_noop!(Bar: ignore WlSurface);
    delegate_noop!(Bar: ignore WlShm);
    delegate_noop!(Bar: ignore WlShmPool);
    delegate_noop!(Bar: ignore WlBuffer);
    delegate_noop!(Bar: ignore ZwlrLayerShellV1);
}
