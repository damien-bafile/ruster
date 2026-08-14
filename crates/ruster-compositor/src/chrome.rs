//! The compositor's UI chrome: statusline, editor frame, which-key overlay.
//!
//! Chrome is drawn as two kinds of quad, collected into a [`ChromeBatch`]: flat
//! vertex geometry for panels and bars (`Vertex` = x, y, reserved, reserved, r,
//! g, b, a), and textured glyph quads carrying a UV rect into the glyph atlas.
//! The `draw_*` methods are pure and testable — they never touch GL — and are
//! the geometry source of truth. `render_frame` turns the batch into smithay
//! render elements and composites it above the client surface.

use std::any::Any;

use crate::compositor::PANE_FONT_PX;
use ruster_render::{Color, StyledLine, SyntaxStyle, Theme, WhichKeyEntry, WhichKeyView};
use ruster_render_gles::atlas::{cell_metrics, layout_text, layout_text_in, Atlas, FontFamily};
use ruster_render_gles::cursor::CursorBitmap;
use ruster_render_gles::geometry::{rect_verts, rounded_rect_verts, GlyphQuad, Vertex};
use ruster_shell::{Layout, Rect, WindowId};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::{Color32F, ImportMem, Renderer};
use smithay::utils::{Physical, Point, Transform};

/// What the statusline says about the container tree.
///
/// Passed in rather than reached for: `Chrome` draws, and giving it the shell
/// to interrogate would make every statusline change a reason to touch the
/// layout code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeStatus {
    /// How the split holding the focused window divides its space, which is the
    /// direction the next window will appear in.
    pub layout: Option<Layout>,
    /// Windows sharing the active workspace, floating included.
    pub windows: usize,
    /// Whether the focused window floats above the tiling.
    pub floating: bool,
}

impl TreeStatus {
    /// The short form drawn on the bar: an axis glyph, the window count, and a
    /// marker when the focused window is floating.
    ///
    /// Deliberately terse — the statusline is one row and the title needs the
    /// space. `|` and `-` read as the divider the split actually draws.
    pub fn indicator(&self) -> String {
        let axis = match self.layout {
            Some(Layout::Horizontal) => "|",
            Some(Layout::Vertical) => "-",
            None => "·",
        };
        let float = if self.floating { " ~" } else { "" };
        format!("{axis} {}{float}", self.windows)
    }
}

/// Thickness of a window border, in logical pixels.
///
/// Thin enough that covering that many pixels of the client costs nothing
/// legible, thick enough to read at a glance on a 1440p display.
const BORDER_WIDTH: f32 = 2.0;

/// One stretch of a line drawn in a single colour.
struct Run {
    /// Column the run starts at, in cells from the body's left edge.
    column: usize,
    text: String,
    /// `None` where the highlighter had no opinion, which is most of a line.
    color: Option<Color>,
}

/// A line split into runs: each highlighted span, and the plain text between.
///
/// The gaps matter as much as the spans. A highlighter colours keywords and
/// leaves the spaces and identifiers between them alone, so drawing only the
/// spans would draw only a fraction of the line.
fn runs(line: &StyledLine) -> Vec<Run> {
    let chars: Vec<char> = line.text.chars().collect();
    let mut out = Vec::new();
    let mut at = 0usize;
    // Spans are taken in order and assumed not to overlap, which is what
    // `SyntaxEngine::highlight_line` produces. One starting behind the pen is
    // skipped rather than drawn over what is already there.
    for (start, end, style) in &line.highlights {
        let (start, end) = (*start, (*end).min(chars.len()));
        if start < at || start >= end {
            continue;
        }
        if start > at {
            out.push(Run {
                column: at,
                text: chars[at..start].iter().collect(),
                color: None,
            });
        }
        out.push(Run {
            column: start,
            text: chars[start..end].iter().collect(),
            color: syntax_color(style),
        });
        at = end;
    }
    if at < chars.len() {
        out.push(Run {
            column: at,
            text: chars[at..].iter().collect(),
            color: None,
        });
    }
    out
}

/// The colour a span asks for, or `None` when it asks for nothing.
///
/// `Color::Default` is a highlighter saying "no opinion", and the pane's own
/// foreground is the opinion it defers to — resolving it to a concrete colour
/// here would hard-code the theme's text colour into every unstyled run.
fn syntax_color(style: &SyntaxStyle) -> Option<Color> {
    match style.fg {
        Color::Default => None,
        other => Some(other),
    }
}

/// The sign and colour for a diagnostic severity.
///
/// LSP counts 1 as an error and 4 as a hint. The letter carries the severity;
/// the colour is a ladder of prominence behind it — the alarm colour, ordinary
/// text, then the gutter's own dim grey for the two that are only advice.
///
/// Every one of these is a *foreground* role. The first version drew errors in
/// `mode_visual_bg`, which is a background swatch: RGB(72,50,80) as text on a
/// RGB(30,30,30) pane, measured at (55,35,70) on screen after blending. It was
/// drawn, it was correct, and it was invisible. A background colour is chosen
/// to sit behind text and can never be the right choice for the text itself.
///
/// The palette has no `diagnostic_*` roles of its own, which is what this
/// should use; adding them means the Lua override plumbing and the theme parity
/// test, and belongs with the rest of the Phase 2 theming work.
fn severity_sign(severity: u8, theme: &Theme) -> (&'static str, (f32, f32, f32, f32)) {
    match severity {
        1 => ("E", theme.accent.into()),
        2 => ("W", theme.fg.into()),
        3 => ("I", theme.gutter.into()),
        _ => ("H", theme.gutter.into()),
    }
}

/// How many cells the line-number gutter needs, including its trailing space.
///
/// Sized to the largest number that will be shown rather than to the buffer, so
/// a 10,000-line file does not reserve five columns while showing lines 1-40 —
/// and zero when there is nothing to number, so an empty pane has no gutter.
fn gutter_width(first_line: usize, shown: usize) -> usize {
    if shown == 0 {
        return 0;
    }
    let last = first_line + shown;
    last.to_string().len() + 1
}

/// The diagnostic sign's own column, left of the line numbers.
///
/// It has to be its own column. Drawing the sign at the gutter's origin and
/// then the right-aligned number at the same origin painted one over the other:
/// the numbers won, and three real diagnostics rendered as an unchanged frame.
const SIGN_COLS: usize = 1;

/// Total gutter width: the sign column plus the numbers.
///
/// One function because two callers need the same answer — [`FrameBody::new`],
/// which decides where text begins and therefore which cell a click lands on,
/// and the draw itself. Measuring the gutter twice is how the grid on screen
/// and the grid under the mouse drift apart.
fn gutter_cols(first_line: usize, shown: usize) -> usize {
    match gutter_width(first_line, shown) {
        0 => 0,
        numbers => numbers + SIGN_COLS,
    }
}

/// Height of an editor frame's title bar, in physical pixels.
const FRAME_BAR_H: f32 = 28.0;

/// Gap between an editor frame's edge and its contents, in physical pixels.
const FRAME_PAD: f32 = 6.0;

/// Where an editor frame's text starts and how big one cell is, in physical
/// pixels measured from the frame's own top-left.
///
/// The one description of that layout: [`Chrome::draw_editor_frame`] positions
/// its rows from this and the pointer reads the same numbers back out, so the
/// grid on screen and the grid under the mouse cannot be measured two ways.
/// That is the same reason `tile_under` and `geometry()` are one list — a
/// second opinion about a rectangle is how a click lands somewhere other than
/// where it looked.
///
/// `ruster_render::TextArea` answers the equivalent question for the editor's
/// windows and deliberately is not reused here: it counts in whole cells, from
/// an origin one header *row* down, which a 28px bar and a 6px pad are not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameBody {
    /// First pixel of buffer text, past the gutter.
    pub x: f32,
    /// First pixel of the first text row, past the title bar.
    pub y: f32,
    pub cell_w: f32,
    pub cell_h: f32,
}

impl FrameBody {
    /// The body of a frame showing `shown` lines starting at `first_line`.
    ///
    /// Both arguments only because the gutter is: it widens as the numbers do,
    /// which moves the first text column every time a pane scrolls past a power
    /// of ten.
    pub fn new(first_line: usize, shown: usize) -> Self {
        let (cell_w, cell_h) = cell_metrics(PANE_FONT_PX);
        FrameBody {
            x: FRAME_PAD + gutter_cols(first_line, shown) as f32 * cell_w,
            y: FRAME_BAR_H + FRAME_PAD,
            cell_w,
            cell_h,
        }
    }

    /// The (row, column) a point falls on, in cells from the body's origin.
    ///
    /// Anything above or left of the text — the title bar, the gutter, the pad
    /// — is row 0 or column 0 rather than nothing: clicking a line number means
    /// that line, and a caller that knows what is actually on those rows clamps
    /// the other end. A frame with no measurable cell is all one cell, which is
    /// the only answer that does not divide by zero.
    pub fn cell_at(&self, x: f32, y: f32) -> (usize, usize) {
        if self.cell_w <= 0.0 || self.cell_h <= 0.0 {
            return (0, 0);
        }
        let col = ((x - self.x) / self.cell_w).floor().max(0.0) as usize;
        let row = ((y - self.y) / self.cell_h).floor().max(0.0) as usize;
        (row, col)
    }
}

/// One frame's worth of chrome geometry: solid quads and textured glyph quads,
/// both in physical pixels with the origin at the output's top-left.
///
/// The two lists are appended in painter's order — a panel's background, then
/// the glyphs that sit on it — and `render_frame` reverses them into smithay's
/// front-to-back element order.
#[derive(Debug, Default)]
pub struct ChromeBatch {
    pub verts: Vec<Vertex>,
    pub glyphs: Vec<GlyphQuad>,
}

/// How much of a [`ChromeBatch`] had been drawn at some point, so a later
/// [`ChromeBatch::translate_since`] can move everything drawn after it.
#[derive(Debug, Clone, Copy)]
pub struct BatchMark {
    verts: usize,
    glyphs: usize,
}

impl ChromeBatch {
    /// Record the current end of the batch.
    pub fn mark(&self) -> BatchMark {
        BatchMark {
            verts: self.verts.len(),
            glyphs: self.glyphs.len(),
        }
    }

    /// Shift everything appended since `mark` by `(dx, dy)`. `render_frame` uses
    /// this to centre the welcome editor frame after laying it out at the origin.
    pub fn translate_since(&mut self, mark: BatchMark, dx: f32, dy: f32) {
        for v in &mut self.verts[mark.verts..] {
            v[0] += dx;
            v[1] += dy;
        }
        for g in &mut self.glyphs[mark.glyphs..] {
            g.x += dx;
            g.y += dy;
        }
    }
}

/// The compositor's UI chrome: statusline, editor frame, which-key overlay.
pub struct Chrome {
    pub atlas: Atlas,
    /// The atlas as uploaded to the GPU, with the atlas generation it was built
    /// from. Boxed as `Any` because the texture type belongs to the renderer,
    /// and the two backends composite through different ones (`GlesRenderer`
    /// nested, `MultiRenderer` on DRM) while `Chrome` itself is shared.
    texture: Option<(u64, Box<dyn Any>)>,
    /// The built-in pointer image, and its uploaded texture once a renderer has
    /// been seen. Boxed as `Any` for the same reason the atlas texture is.
    cursor: CursorBitmap,
    cursor_texture: Option<Box<dyn Any>>,
    cursor_id: Id,
    /// One render-element id per glyph slot, reused across frames.
    ///
    /// The damage tracker keys element state by id, so glyphs sharing one id
    /// would collapse into a single tracked element and damage the wrong
    /// regions. Ids are handed out by position in the batch and kept stable, so
    /// a glyph that does not move reports no damage.
    glyph_ids: Vec<Id>,
    /// The colours everything here draws in, from the user's config.
    theme: Theme,
}

impl Chrome {
    pub fn new(theme: Theme) -> Self {
        Chrome {
            atlas: Atlas::new(),
            theme,
            texture: None,
            cursor: CursorBitmap::arrow(),
            cursor_texture: None,
            cursor_id: Id::new(),
            glyph_ids: Vec::new(),
        }
    }

    /// The uploaded glyph atlas for `renderer`, re-uploading when the atlas has
    /// rasterized new glyphs since the last upload. Returns `None` if the
    /// texture cannot be imported, in which case chrome text is skipped for the
    /// frame rather than failing it.
    pub fn atlas_texture<R>(&mut self, renderer: &mut R) -> Option<R::TextureId>
    where
        R: Renderer + ImportMem,
        R::TextureId: Clone + 'static,
    {
        let generation = self.atlas.generation();
        if let Some((uploaded, cached)) = self.texture.as_ref() {
            if *uploaded == generation {
                if let Some(texture) = cached.downcast_ref::<R::TextureId>() {
                    return Some(texture.clone());
                }
            }
        }
        let size = self.atlas.texture_size as i32;
        let texture = renderer
            .import_memory(
                self.atlas.pixels(),
                Fourcc::Abgr8888,
                (size, size).into(),
                false,
            )
            .inspect_err(|_| {
                tracing::warn!("failed to upload the glyph atlas; chrome text skipped")
            })
            .ok()?;
        self.texture = Some((generation, Box::new(texture.clone())));
        Some(texture)
    }

    /// A stable render-element id for the glyph at `index` in the batch.
    pub fn glyph_id(&mut self, index: usize) -> Id {
        while self.glyph_ids.len() <= index {
            self.glyph_ids.push(Id::new());
        }
        self.glyph_ids[index].clone()
    }

    /// The built-in arrow cursor, positioned so its hotspot sits at `location`.
    ///
    /// Uploaded once and cached like the glyph atlas; the id is stable across
    /// frames so the damage tracker sees a moving element rather than a new one
    /// each frame.
    pub fn cursor_element<R>(
        &mut self,
        renderer: &mut R,
        location: Point<f64, Physical>,
    ) -> Option<TextureRenderElement<R::TextureId>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Clone + 'static,
    {
        let texture = match self.cursor_texture.as_ref() {
            Some(cached) => cached.downcast_ref::<R::TextureId>()?.clone(),
            None => {
                let texture = renderer
                    .import_memory(
                        self.cursor.pixels(),
                        Fourcc::Abgr8888,
                        (self.cursor.width, self.cursor.height).into(),
                        false,
                    )
                    .inspect_err(|_| tracing::warn!("failed to upload the cursor image"))
                    .ok()?;
                self.cursor_texture = Some(Box::new(texture.clone()));
                texture
            }
        };
        let (hx, hy) = self.cursor.hotspot;
        let origin = Point::<f64, Physical>::from((location.x - hx as f64, location.y - hy as f64));
        Some(TextureRenderElement::from_static_texture(
            self.cursor_id.clone(),
            renderer.context_id(),
            origin,
            texture,
            1,
            Transform::Normal,
            None,
            None,
            None,
            None,
            Kind::Cursor,
        ))
    }

    /// Bottom statusline: returns its height in px.
    ///
    /// Layout (left→right): an accent mode segment with the mode letter in the
    /// accent foreground, then the workspace label and the focused toplevel's
    /// title in the statusline foreground — all legible against the statusline
    /// background. Height is derived from the output via [`crate::render::chrome_height`].
    pub fn draw_statusline(
        &mut self,
        w: i32,
        h: i32,
        workspace: u32,
        focused_title: &str,
        tree: TreeStatus,
        batch: &mut ChromeBatch,
    ) -> i32 {
        let bar_h = crate::render::chrome_height(h);
        let y = (h - bar_h) as f32;
        let bar_w = w as f32;
        let bg: (f32, f32, f32, f32) = self.theme.statusline_bg.into();
        let fg: (f32, f32, f32, f32) = self.theme.statusline_fg.into();
        let accent: (f32, f32, f32, f32) = self.theme.accent.into();
        let accent_fg: (f32, f32, f32, f32) = self.theme.accent_fg.into();

        batch
            .verts
            .extend(rect_verts(0.0, y, bar_w, bar_h as f32, bg));

        // Mode segment: accent background, "N" (Normal) in the accent foreground.
        let mode_w = 64.0;
        let pad = (bar_h as f32 - 16.0) / 2.0;
        batch
            .verts
            .extend(rect_verts(0.0, y, mode_w, bar_h as f32, accent));
        self.text("N", 16, (mode_w - 16.0) / 2.0, y + pad, accent_fg, batch);

        // Workspace label + focused title in the statusline foreground.
        let ws = format!("WS {workspace}");
        let title = if focused_title.is_empty() {
            "(no client)"
        } else {
            focused_title
        };
        let cursor = mode_w + 12.0;
        let ws_w = self.text(&ws, 16, cursor, y + pad, fg, batch);
        let mut cursor = cursor + ws_w + 20.0;

        // The layout the next window will use, and how many share the screen.
        // Neither is inferable from the screen: two windows side by side look
        // the same whether the next one lands beside them or under them, and a
        // floating window looks like any other until it overlaps something.
        let indicator = tree.indicator();
        let ind_w = self.text(&indicator, 16, cursor, y + pad, accent, batch);
        cursor += ind_w + 20.0;

        self.text(title, 16, cursor, y + pad, fg, batch);

        bar_h
    }

    /// A synthetic editor frame: mode-line title + buffer rows. Phase 0 shows a
    /// welcome buffer; buffer-driven content lands with the embedded editor.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_editor_frame(
        &mut self,
        w: i32,
        h: i32,
        buffer: &[StyledLine],
        first_line: usize,
        severities: &[Option<u8>],
        title: &str,
        batch: &mut ChromeBatch,
    ) {
        let bar_h = FRAME_BAR_H;
        let bg: (f32, f32, f32, f32) = self.theme.bg.into();
        let fg: (f32, f32, f32, f32) = self.theme.fg.into();
        let accent: (f32, f32, f32, f32) = self.theme.accent.into();
        let accent_fg: (f32, f32, f32, f32) = self.theme.accent_fg.into();

        batch
            .verts
            .extend(rounded_rect_verts(0.0, 0.0, w as f32, h as f32, 4.0, bg));
        batch
            .verts
            .extend(rect_verts(0.0, 0.0, w as f32, bar_h, accent));
        self.text(title, 16, FRAME_PAD, (bar_h - 16.0) / 2.0, accent_fg, batch);

        // The body is a character grid, so it is laid out on cell metrics
        // rather than a fixed chrome line height — the two are different
        // numbers, and using a chrome constant would drift a row out of
        // alignment every few lines. The old `line_h: 24` field went with it.
        let body = FrameBody::new(first_line, buffer.len());
        let number_cols = gutter_width(first_line, buffer.len());
        let rows = ((h as f32 - bar_h - 8.0) / body.cell_h).max(0.0) as usize;
        let gutter_color: (f32, f32, f32, f32) = self.theme.gutter.into();
        // Numbers start past the sign column, which is what keeps the two from
        // being drawn on top of each other.
        let numbers_x = FRAME_PAD + SIGN_COLS as f32 * body.cell_w;

        for (row, line) in buffer.iter().take(rows).enumerate() {
            let gy = body.y + row as f32 * body.cell_h;
            // A sign left of the number, in the severity's colour. One column,
            // because the gutter is already competing with the text for width
            // and a diagnostic is a pointer to a line rather than the message
            // itself.
            if let Some(severity) = severities.get(row).copied().flatten() {
                let (glyph, color) = severity_sign(severity, &self.theme);
                self.text_in(
                    glyph,
                    PANE_FONT_PX,
                    FRAME_PAD,
                    gy,
                    color,
                    FontFamily::Mono,
                    batch,
                );
            }
            if number_cols > 0 {
                // Right-aligned, the way every editor draws line numbers: the
                // units column has to line up or the eye cannot scan it.
                let number = format!("{:>width$} ", first_line + row + 1, width = number_cols - 1);
                self.text_in(
                    &number,
                    PANE_FONT_PX,
                    numbers_x,
                    gy,
                    gutter_color,
                    FontFamily::Mono,
                    batch,
                );
            }
            // One run per span, positioned by *cell* rather than by chaining
            // advances from the previous run. The runs are separate draw calls,
            // and accumulating their widths would let a rounding difference
            // slide a character out of its column — the grid is the authority,
            // exactly as it is for the click that lands on it.
            for run in runs(line) {
                self.text_in(
                    &run.text,
                    PANE_FONT_PX,
                    body.x + run.column as f32 * body.cell_w,
                    gy,
                    run.color.map(Into::into).unwrap_or(fg),
                    FontFamily::Mono,
                    batch,
                );
            }
        }
    }

    /// Bottom which-key overlay panel.
    /// Outline every visible window, marking the focused one.
    ///
    /// Without this nothing on screen says which window takes the next
    /// keystroke: a tiling compositor gives them all the same shape and the
    /// same chrome, and the only other cue — the title in the statusline —
    /// names the window without pointing at it.
    ///
    /// Drawn *over* the outermost pixels of each window rather than by insetting
    /// the layout. Insetting is what i3 does and looks better, but it would mean
    /// changing `geometry()`, `reconfigure_tiles` and `tile_under` in step; any
    /// disagreement between them puts the pointer somewhere other than where it
    /// looks, which is a bug this compositor has already had once.
    ///
    /// `scale` converts the layout's logical rectangles to the physical pixels
    /// the rest of the chrome is measured in.
    pub fn draw_window_borders(
        &mut self,
        windows: &[(WindowId, Rect)],
        focus: Option<WindowId>,
        scale: f64,
        batch: &mut ChromeBatch,
    ) {
        let s = scale as f32;
        let width = (BORDER_WIDTH * s).max(1.0);
        for (id, rect) in windows {
            let color: (f32, f32, f32, f32) = if Some(*id) == focus {
                self.theme.border_focused.into()
            } else {
                self.theme.border_unfocused.into()
            };
            let (x, y) = (rect.x as f32 * s, rect.y as f32 * s);
            let (w, h) = (rect.w as f32 * s, rect.h as f32 * s);
            if w <= 0.0 || h <= 0.0 {
                continue;
            }
            batch.verts.extend(rect_verts(x, y, w, width, color));
            batch
                .verts
                .extend(rect_verts(x, y + h - width, w, width, color));
            batch.verts.extend(rect_verts(x, y, width, h, color));
            batch
                .verts
                .extend(rect_verts(x + w - width, y, width, h, color));
        }
    }

    /// Draw the `:` line along the bottom, just above the statusline.
    ///
    /// The sigil is drawn in `cmdline_accent` and the rest in `cmdline_fg`,
    /// which is what distinguishes "waiting for a command" from "showing you a
    /// message" — the two otherwise share a row and a colour. A message has no
    /// sigil and so is drawn entirely in the text colour.
    pub fn draw_minibuffer(
        &mut self,
        output_w: i32,
        output_h: i32,
        line: &str,
        sigil_len: usize,
        batch: &mut ChromeBatch,
    ) {
        let bar_h = crate::render::chrome_height(output_h) as f32;
        let h = bar_h;
        let y = output_h as f32 - bar_h - h;
        let bg: (f32, f32, f32, f32) = self.theme.cmdline_bg.into();
        let fg: (f32, f32, f32, f32) = self.theme.cmdline_fg.into();
        let accent: (f32, f32, f32, f32) = self.theme.cmdline_accent.into();

        batch
            .verts
            .extend(rect_verts(0.0, y, output_w as f32, h, bg));
        let font = (h * 0.5) as u32;
        let ty = y + h * 0.25;
        let (sigil, rest) = line.split_at(sigil_len.min(line.len()));
        let mut x = 10.0;
        if !sigil.is_empty() {
            x += self.text(sigil, font, x, ty, accent, batch);
        }
        if !rest.is_empty() {
            self.text(rest, font, x, ty, fg, batch);
        }
    }

    /// Emit `n` glyph quads of synthetic load, for measuring how many render
    /// elements a frame can carry (`RUSTER_BENCH_GLYPHS`).
    ///
    /// Laid out in a grid the size a text pane would be, using real atlas
    /// glyphs, so the cost measured is the cost an editor pane would pay:
    /// per-element work in the damage tracker plus sampling the atlas texture.
    pub fn bench_glyphs(&mut self, n: usize, batch: &mut ChromeBatch) {
        let fg: (f32, f32, f32, f32) = self.theme.fg.into();
        let rgb = rgb8(fg);
        for i in 0..n {
            // Cycle a few characters so the atlas holds more than one cell,
            // and step positions so no two quads coincide.
            let c = (*b"miW0")[i % 4] as char;
            let g = self.atlas.glyph(14, rgb, c);
            if g.is_empty() {
                continue;
            }
            let (col, row) = (i % 80, i / 80);
            batch.glyphs.push(GlyphQuad {
                x: 20.0 + col as f32 * 9.0 + g.x,
                y: 40.0 + row as f32 * 18.0 + g.y,
                w: g.w,
                h: g.h,
                u0: g.u0,
                v0: g.v0,
                u1: g.u1,
                v1: g.v1,
            });
        }
    }

    /// Draw the which-key panel: a half-typed chord's continuations, or the
    /// whole keymap when the helper is pinned.
    ///
    /// Lays out in as many columns as it takes to show every row, because the
    /// alternative is truncating — and a shortcut reference that silently omits
    /// shortcuts is worse than none. Column widths are measured from the text
    /// rather than fixed, so a keymap of short binds does not reserve room for
    /// long ones.
    ///
    /// Takes the [`WhichKeyView`] the editor also renders, which is what lets
    /// the key and its description be drawn in different colours; they used to
    /// be one concatenated string and therefore one colour.
    pub fn draw_whichkey(
        &mut self,
        output_w: i32,
        output_h: i32,
        view: &WhichKeyView,
        batch: &mut ChromeBatch,
    ) {
        const FONT: u32 = 14;
        const ROW_H: f32 = 20.0;
        const PAD: f32 = 10.0;
        const GAP: f32 = 8.0;
        const COL_GAP: f32 = 18.0;

        let bg: (f32, f32, f32, f32) = self.theme.whichkey_bg.into();
        let fg: (f32, f32, f32, f32) = self.theme.whichkey_fg.into();
        let key_color: (f32, f32, f32, f32) = self.theme.whichkey_key.into();

        // Rows per column, from the room actually available above the
        // statusline. A laptop panel and a 1440p display do not fit the same
        // list, so this cannot be a constant.
        let title_h = if view.title.is_empty() { 0.0 } else { ROW_H };
        let usable_h = (output_h as f32 * 0.8 - PAD * 2.0 - title_h).max(ROW_H);
        let per_col = ((usable_h / ROW_H) as usize).max(1);
        let columns: Vec<&[WhichKeyEntry]> = view.rows.chunks(per_col).collect();

        // Measure each column so a narrow one costs narrow space.
        let widths: Vec<f32> = columns
            .iter()
            .map(|col| {
                col.iter()
                    .map(|e| {
                        layout_text(&e.key, FONT, None).width_px
                            + GAP
                            + layout_text(&e.desc, FONT, None).width_px
                    })
                    .fold(0.0f32, f32::max)
            })
            .collect();

        let rows_drawn = columns.first().map(|c| c.len()).unwrap_or(0);
        let h = PAD * 2.0 + title_h + rows_drawn as f32 * ROW_H;
        let w = PAD * 2.0
            + widths.iter().sum::<f32>()
            + COL_GAP * columns.len().saturating_sub(1) as f32;
        let (x, y) = (12.0, 12.0);

        batch.verts.extend(rounded_rect_verts(
            x,
            y,
            w.min(output_w as f32 - 24.0),
            h,
            6.0,
            bg,
        ));

        let mut ty = y + PAD;
        if !view.title.is_empty() {
            self.text(&view.title, FONT, x + PAD, ty, key_color, batch);
            ty += ROW_H;
        }
        let mut cx = x + PAD;
        for (col, width) in columns.iter().zip(&widths) {
            let mut row_y = ty;
            for entry in col.iter() {
                // The key in the accent, the description in the panel text, so
                // the thing you have to press is the thing that stands out.
                let advance = self.text(&entry.key, FONT, cx, row_y, key_color, batch);
                self.text(&entry.desc, FONT, cx + advance + GAP, row_y, fg, batch);
                row_y += ROW_H;
            }
            cx += width + COL_GAP;
        }
    }

    /// Lay `text` out and append one textured quad per glyph, positioned at
    /// `(x, y)` — the pen position and the top of the line box — plus the
    /// layout's per-glyph advance and the glyph's own bearing. Returns the run's
    /// advance width so callers can chain text to the right.
    ///
    /// Glyphs with no bitmap (spaces, control chars) advance the pen and draw
    /// nothing.
    fn text(
        &mut self,
        text: &str,
        font_size: u32,
        x: f32,
        y: f32,
        color: (f32, f32, f32, f32),
        batch: &mut ChromeBatch,
    ) -> f32 {
        self.text_in(text, font_size, x, y, color, FontFamily::Ui, batch)
    }

    /// As [`text`](Self::text), in a specific face.
    ///
    /// Chrome is proportional; a pane's body is not. Column alignment, the
    /// gutter and (in Stage 3) click-to-position all assume a fixed advance.
    #[allow(clippy::too_many_arguments)]
    fn text_in(
        &mut self,
        text: &str,
        font_size: u32,
        x: f32,
        y: f32,
        color: (f32, f32, f32, f32),
        family: FontFamily,
        batch: &mut ChromeBatch,
    ) -> f32 {
        let rgb = rgb8(color);
        let layout = layout_text_in(text, font_size, None, family);
        for (gx, _, c) in layout.glyphs {
            let g = self.atlas.glyph_in(font_size, rgb, c, family);
            if g.is_empty() {
                continue;
            }
            batch.glyphs.push(GlyphQuad {
                x: x + gx + g.x,
                y: y + g.y,
                w: g.w,
                h: g.h,
                u0: g.u0,
                v0: g.v0,
                u1: g.u1,
                v1: g.v1,
            });
        }
        layout.width_px
    }
}

/// A chrome colour as the 8-bit RGB the atlas bakes into a glyph cell.
fn rgb8(color: (f32, f32, f32, f32)) -> [u8; 3] {
    let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    [to_u8(color.0), to_u8(color.1), to_u8(color.2)]
}

/// Convert a chrome vertex batch (physical pixels, origin top-left) into
/// smithay solid-color render elements, one per 6-vertex quad.
///
/// Each quad becomes a [`SolidColorRenderElement`] carrying its bounding box and
/// the quad's color (read from the first vertex). `render_frame` collects these
/// and composites them above the client surface in the same
/// [`OutputDamageTracker::render_output`] pass.
pub fn solid_elements_from_verts(verts: &[Vertex]) -> Vec<SolidColorRenderElement> {
    let mut elements = Vec::new();
    for quad in verts.chunks(6) {
        if quad.len() < 6 {
            break;
        }
        let mut x0 = f32::MAX;
        let mut y0 = f32::MAX;
        let mut x1 = f32::MIN;
        let mut y1 = f32::MIN;
        for v in quad {
            x0 = x0.min(v[0]);
            y0 = y0.min(v[1]);
            x1 = x1.max(v[0]);
            y1 = y1.max(v[1]);
        }
        let w = x1 - x0;
        let h = y1 - y0;
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let color = Color32F::new(quad[0][4], quad[0][5], quad[0][6], quad[0][7]);
        let buffer = SolidColorBuffer::new((w as i32, h as i32), color);
        elements.push(SolidColorRenderElement::from_buffer(
            &buffer,
            (x0 as i32, y0 as i32),
            1.0,
            1.0,
            Kind::Unspecified,
        ));
    }
    elements
}

#[cfg(test)]
mod tests {
    fn hl(fg: (u8, u8, u8)) -> SyntaxStyle {
        SyntaxStyle {
            fg: Color::Rgb(fg.0, fg.1, fg.2),
            ..SyntaxStyle::default()
        }
    }

    fn line(text: &str, spans: Vec<(usize, usize, SyntaxStyle)>) -> StyledLine {
        StyledLine {
            text: text.to_string(),
            highlights: spans,
        }
    }

    #[test]
    fn the_text_between_spans_is_drawn_too() {
        // A highlighter colours keywords and leaves what is between them alone.
        // Drawing only the spans would draw a fraction of the line — and the
        // fraction would still look plausible, which is worse.
        // Two spans with plain text before, between and after them — the
        // earlier version of this test put a span at column 0 and so never
        // exercised an interior gap at all. It passed with the gap runs
        // deleted entirely.
        let runs = runs(&line(
            "let x = 1;",
            vec![(0, 3, hl((1, 2, 3))), (8, 9, hl((3, 2, 1)))],
        ));
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "let x = 1;", "every character must be drawn once");
        let coloured: Vec<&str> = runs
            .iter()
            .filter(|r| r.color.is_some())
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(coloured, ["let", "1"], "only the spans are coloured");
        let plain: String = runs
            .iter()
            .filter(|r| r.color.is_none())
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(
            plain, " x = ;",
            "and everything between them is still drawn"
        );
    }

    #[test]
    fn every_run_starts_at_the_column_it_occupies() {
        // Runs are positioned by cell, not by chaining advances: separate draw
        // calls whose widths were accumulated would let a rounding difference
        // slide a character out of its column.
        let runs = runs(&line(
            "fn main() {}",
            vec![(0, 2, hl((1, 0, 0))), (3, 7, hl((0, 1, 0)))],
        ));
        for run in &runs {
            let expected: String = "fn main() {}"
                .chars()
                .skip(run.column)
                .take(run.text.chars().count())
                .collect();
            assert_eq!(run.text, expected, "run at column {}", run.column);
        }
    }

    #[test]
    fn a_line_with_no_spans_is_one_plain_run() {
        let runs = runs(&line("plain text", Vec::new()));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].column, 0);
        assert!(runs[0].color.is_none());
    }

    #[test]
    fn an_empty_line_produces_nothing_to_draw() {
        assert!(runs(&line("", Vec::new())).is_empty());
    }

    #[test]
    fn a_span_that_overlaps_an_earlier_one_is_skipped() {
        // The engine produces ordered, non-overlapping spans. If one ever
        // arrived behind the pen, drawing it would paint over text already
        // placed — and duplicate the characters underneath.
        let runs = runs(&line(
            "abcdef",
            vec![(0, 4, hl((1, 0, 0))), (2, 5, hl((0, 1, 0)))],
        ));
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "abcdef", "no character drawn twice");
    }

    #[test]
    fn a_span_with_no_colour_of_its_own_defers_to_the_pane() {
        // `Color::Default` is the highlighter saying "no opinion". Resolving it
        // here would bake the theme's text colour into the run and stop the
        // pane's own foreground from applying.
        assert_eq!(syntax_color(&SyntaxStyle::default()), None);
        assert_eq!(syntax_color(&hl((9, 9, 9))), Some(Color::Rgb(9, 9, 9)));
    }

    /// Plain lines as the styled ones `draw_editor_frame` now takes.
    fn styled(lines: &[String]) -> Vec<StyledLine> {
        lines
            .iter()
            .map(|text| StyledLine {
                text: text.clone(),
                highlights: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn the_focused_window_is_bordered_differently_from_the_others() {
        // The whole point: with every window the same shape and the same
        // chrome, the border is the only thing that says which one takes the
        // next keystroke.
        let mut chrome = Chrome::new(Theme::default());
        let windows = vec![
            (WindowId(0), Rect::new(0, 0, 100, 100)),
            (WindowId(1), Rect::new(100, 0, 100, 100)),
        ];
        let mut batch = ChromeBatch::default();
        chrome.draw_window_borders(&windows, Some(WindowId(1)), 1.0, &mut batch);

        // Four edges per window, two triangles each, three vertices a triangle.
        assert_eq!(batch.verts.len(), 2 * 4 * 6);
        let colors: std::collections::BTreeSet<[u32; 4]> = batch
            .verts
            .iter()
            .map(|v| [v[4], v[5], v[6], v[7]].map(f32::to_bits))
            .collect();
        assert_eq!(
            colors.len(),
            2,
            "focused and unfocused must not come out the same colour"
        );
    }

    #[test]
    fn an_unfocused_workspace_gets_borders_in_one_colour() {
        // Nothing is focused while a workspace with no focusable window is on
        // screen, and every border being the "focused" one would be a lie.
        let mut chrome = Chrome::new(Theme::default());
        let windows = vec![
            (WindowId(0), Rect::new(0, 0, 50, 50)),
            (WindowId(1), Rect::new(50, 0, 50, 50)),
        ];
        let mut batch = ChromeBatch::default();
        chrome.draw_window_borders(&windows, None, 1.0, &mut batch);
        let colors: std::collections::BTreeSet<[u32; 4]> = batch
            .verts
            .iter()
            .map(|v| [v[4], v[5], v[6], v[7]].map(f32::to_bits))
            .collect();
        assert_eq!(colors.len(), 1);
    }

    #[test]
    fn a_border_stays_inside_the_window_it_outlines() {
        // Drawn over the client's outer pixels rather than by insetting the
        // layout, so a border that strayed outside would land on the neighbour.
        let mut chrome = Chrome::new(Theme::default());
        let rect = Rect::new(10, 20, 100, 80);
        let mut batch = ChromeBatch::default();
        chrome.draw_window_borders(&[(WindowId(0), rect)], None, 1.0, &mut batch);
        for v in &batch.verts {
            assert!(
                v[0] >= 10.0 && v[0] <= 110.0,
                "x {} escaped the window",
                v[0]
            );
            assert!(
                v[1] >= 20.0 && v[1] <= 100.0,
                "y {} escaped the window",
                v[1]
            );
        }
    }

    #[test]
    fn borders_scale_with_the_output() {
        // Chrome is measured in physical pixels and the layout in logical ones,
        // so a 2x display would otherwise get a border at half the size and in
        // the wrong place.
        let mut chrome = Chrome::new(Theme::default());
        let mut batch = ChromeBatch::default();
        chrome.draw_window_borders(
            &[(WindowId(0), Rect::new(0, 0, 100, 100))],
            None,
            2.0,
            &mut batch,
        );
        let max_x = batch.verts.iter().map(|v| v[0]).fold(0.0f32, f32::max);
        assert_eq!(max_x, 200.0, "a 100px logical window is 200px physical");
    }

    use super::*;
    use ruster_render::Theme;

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

    #[test]
    fn statusline_emits_quads() {
        let mut chrome = Chrome::new(theme());
        let mut batch = ChromeBatch::default();
        let h = chrome.draw_statusline(800, 600, 1, "foot", status(), &mut batch);
        assert!(h > 0);
        assert!(batch.verts.len() >= 6);
        // every vertex within the bar band
        for v in &batch.verts {
            assert!(v[1] >= 600.0 - h as f32 - 1.0 && v[1] <= 600.0 + 1.0);
        }
    }

    #[test]
    fn the_indicator_says_which_way_the_next_window_goes() {
        // Two windows side by side look identical whether the next one lands
        // beside them or under them, so the axis has to be stated.
        let across = TreeStatus {
            layout: Some(Layout::Horizontal),
            windows: 2,
            floating: false,
        };
        assert_eq!(across.indicator(), "| 2");

        let down = TreeStatus {
            layout: Some(Layout::Vertical),
            ..across
        };
        assert_eq!(down.indicator(), "- 2");
    }

    #[test]
    fn a_lone_window_has_no_split_to_report() {
        let alone = TreeStatus {
            layout: None,
            windows: 1,
            floating: false,
        };
        assert_eq!(alone.indicator(), "· 1");
    }

    #[test]
    fn a_floating_window_is_marked() {
        // A float looks like any other window until it overlaps something.
        let floating = TreeStatus {
            layout: None,
            windows: 3,
            floating: true,
        };
        assert_eq!(floating.indicator(), "· 3 ~");
    }

    #[test]
    fn statusline_draws_its_text_as_glyphs() {
        // The mode letter, the workspace label and the focused title all reach
        // the batch as glyph quads inside the bar — not as solid blocks, and not
        // dropped on the floor.
        let mut chrome = Chrome::new(theme());
        let mut batch = ChromeBatch::default();
        let h = chrome.draw_statusline(800, 600, 1, "foot", status(), &mut batch);
        assert!(
            batch.glyphs.len() >= "N".len() + "WS 1".len(),
            "expected a glyph per drawn character, got {}",
            batch.glyphs.len()
        );
        for g in &batch.glyphs {
            assert!(g.w > 0.0 && g.h > 0.0, "glyph quads carry a bitmap");
            assert!(
                g.y >= 600.0 - h as f32 - 1.0 && g.y + g.h <= 600.0 + 1.0,
                "glyph at y={} escapes the {h}px bar",
                g.y
            );
        }
    }

    #[test]
    fn editor_frame_renders_title_and_lines() {
        let mut chrome = Chrome::new(theme());
        let buf: Vec<String> = (0..20).map(|i| format!("line {i}")).collect();
        let mut batch = ChromeBatch::default();
        chrome.draw_editor_frame(400, 300, &styled(&buf), 0, &[], "welcome", &mut batch);
        assert!(batch.verts.len() >= 6 * 2); // frame + title bar
        assert!(!batch.glyphs.is_empty(), "title and rows draw glyphs");
    }

    #[test]
    fn whichkey_panel_renders_its_view() {
        let mut chrome = Chrome::new(theme());
        let view = ruster_render::WhichKeyView {
            title: "M-w".into(),
            rows: vec![
                ruster_render::WhichKeyEntry {
                    key: "h".into(),
                    desc: "focus left".into(),
                },
                ruster_render::WhichKeyEntry {
                    key: "l".into(),
                    desc: "focus right".into(),
                },
            ],
            anim: 1.0,
        };
        let mut batch = ChromeBatch::default();
        chrome.draw_whichkey(1920, 1080, &view, &mut batch);
        assert!(!batch.verts.is_empty());
        assert!(!batch.glyphs.is_empty());
    }

    #[test]
    fn whichkey_draws_the_key_and_its_description_in_different_colours() {
        // They were one concatenated string and therefore one colour, so the
        // key you have to press did not stand out from the sentence about it.
        let mut chrome = Chrome::new(theme());
        let view = ruster_render::WhichKeyView {
            title: String::new(),
            rows: vec![ruster_render::WhichKeyEntry {
                key: "h".into(),
                desc: "focus left".into(),
            }],
            anim: 1.0,
        };
        let mut batch = ChromeBatch::default();
        chrome.draw_whichkey(1920, 1080, &view, &mut batch);
        let theme = theme();
        assert_ne!(
            theme.whichkey_key, theme.whichkey_fg,
            "the test is meaningless if the two roles are the same colour"
        );
        // The atlas bakes colour into each glyph cell, so both colours having
        // been rasterized is the evidence that both were used.
        assert!(chrome
            .atlas
            .contains(14, rgb8(theme.whichkey_key.into()), 'h'));
        assert!(chrome
            .atlas
            .contains(14, rgb8(theme.whichkey_fg.into()), 'f'));
    }

    #[test]
    fn a_keymap_too_tall_for_the_screen_wraps_into_columns() {
        // Truncating was the alternative, and a shortcut reference that
        // silently omits shortcuts is worse than none.
        let mut chrome = Chrome::new(theme());
        let rows: Vec<ruster_render::WhichKeyEntry> = (0..60)
            .map(|n| ruster_render::WhichKeyEntry {
                key: format!("M-{n}"),
                desc: format!("action {n}"),
            })
            .collect();
        let view = ruster_render::WhichKeyView {
            title: "keys".into(),
            rows,
            anim: 1.0,
        };
        let mut batch = ChromeBatch::default();
        // A short output, so 60 rows cannot possibly stack vertically.
        chrome.draw_whichkey(1920, 400, &view, &mut batch);

        let xs: Vec<f32> = batch.glyphs.iter().map(|g| g.x).collect();
        let min_x = xs.iter().copied().fold(f32::MAX, f32::min);
        let max_x = xs.iter().copied().fold(0.0f32, f32::max);
        assert!(
            max_x > min_x + 200.0,
            "60 rows in 400px of height must occupy more than one column"
        );
        let bottom = batch.glyphs.iter().map(|g| g.y).fold(0.0f32, f32::max);
        assert!(bottom < 400.0, "and must stay on the screen, got {bottom}");
    }

    #[test]
    fn translate_since_moves_both_panels_and_glyphs() {
        let mut chrome = Chrome::new(theme());
        let mut batch = ChromeBatch::default();
        chrome.draw_editor_frame(
            400,
            300,
            &styled(&["hi".to_string()]),
            0,
            &[],
            "welcome",
            &mut batch,
        );
        let (vert, glyph) = (batch.verts[0], batch.glyphs[0]);

        let mark = batch.mark();
        chrome.draw_editor_frame(
            400,
            300,
            &styled(&["hi".to_string()]),
            0,
            &[],
            "welcome",
            &mut batch,
        );
        batch.translate_since(mark, 10.0, 20.0);

        // Everything before the mark stays put; everything after it shifts.
        assert_eq!(batch.verts[0], vert);
        assert_eq!(batch.glyphs[0], glyph);
        let moved_vert = batch.verts[vert_count(&batch)];
        assert_eq!(
            (moved_vert[0], moved_vert[1]),
            (vert[0] + 10.0, vert[1] + 20.0)
        );
    }

    fn vert_count(batch: &ChromeBatch) -> usize {
        batch.verts.len() / 2
    }

    #[test]
    fn statusline_colors_are_legible() {
        // Text on the statusline is drawn in the statusline foreground (and the
        // mode glyph in the accent foreground), never in the accent color that
        // fills the mode segment — the brief flagged amber-on-amber as a bug.
        // Glyph colour is baked into the atlas cell, so ask the atlas which
        // cells the draw actually produced.
        let mut chrome = Chrome::new(theme());
        let mut batch = ChromeBatch::default();
        let _ = chrome.draw_statusline(800, 600, 1, "foot", status(), &mut batch);
        let quad = |i: usize| &batch.verts[i * 6..i * 6 + 6];
        assert!(
            quad(0).iter().all(|v| v[4] == 69.0 / 255.0),
            "bar = statusline_bg"
        );
        assert!(
            quad(1).iter().all(|v| v[4] == 243.0 / 255.0),
            "mode segment = accent"
        );

        let accent = [243, 139, 168];
        let accent_fg = [30, 30, 30];
        let statusline_fg = [205, 214, 244];
        assert!(
            chrome.atlas.contains(16, accent_fg, 'N'),
            "the mode letter is drawn in the accent foreground"
        );
        assert!(
            chrome.atlas.contains(16, statusline_fg, 'W'),
            "the workspace label is drawn in the statusline foreground"
        );
        for c in "NWS1foot".chars() {
            assert!(
                !chrome.atlas.contains(16, accent, c),
                "no statusline text is drawn in the accent fill color ({c})"
            );
        }
    }
    /// The diagnostic sign and the line numbers must not share a cell.
    ///
    /// They did: both were drawn at the gutter's origin, so the number was
    /// painted over the sign and three real rust-analyzer diagnostics rendered
    /// as a frame identical to one with none. Nothing caught it, because
    /// `line_severities` was correct — the data reached the draw and the draw
    /// threw it away.
    #[test]
    fn the_sign_column_is_not_the_number_column() {
        let (cell_w, _) = cell_metrics(PANE_FONT_PX);
        let numbers_x = FRAME_PAD + SIGN_COLS as f32 * cell_w;
        assert!(
            numbers_x >= FRAME_PAD + cell_w,
            "the numbers start at {numbers_x}, on top of the sign at {FRAME_PAD}"
        );
    }

    /// And the text must start clear of both, or the sign column would be
    /// stolen from the first character of every line instead.
    #[test]
    fn text_starts_past_the_whole_gutter() {
        let (cell_w, _) = cell_metrics(PANE_FONT_PX);
        let body = FrameBody::new(0, 6);
        let numbers_end = FRAME_PAD + gutter_cols(0, 6) as f32 * cell_w;
        assert_eq!(body.x, numbers_end);
        assert!(
            body.x >= FRAME_PAD + (SIGN_COLS as f32 + 1.0) * cell_w,
            "text at {} leaves no room for a sign plus a digit",
            body.x
        );
    }

    /// An empty pane has no numbers, so it has no sign column either — a lone
    /// blank column indented every empty buffer for nothing.
    #[test]
    fn an_empty_pane_has_no_gutter_at_all() {
        assert_eq!(gutter_cols(0, 0), 0);
        assert_eq!(FrameBody::new(0, 0).x, FRAME_PAD);
    }

    /// The sign column is a constant overhead, not one that grows: only the
    /// numbers widen as a pane scrolls past a power of ten.
    #[test]
    fn only_the_numbers_widen_with_the_line_count() {
        assert_eq!(gutter_cols(0, 9) + 1, gutter_cols(0, 10));
        assert_eq!(
            gutter_cols(0, 9) - gutter_width(0, 9),
            gutter_cols(0, 1000) - gutter_width(0, 1000)
        );
    }

    /// A diagnostic sign has to be legible against the pane it is drawn on.
    ///
    /// This is the test that was missing. `line_severities` was unit-tested and
    /// correct, the draw was reached with the right data, and errors still came
    /// out as RGB(55,35,70) on RGB(30,30,30) — because the colour was
    /// `mode_visual_bg`, a background swatch. Nothing compared the two.
    #[test]
    fn every_severity_sign_is_legible_on_the_pane_background() {
        let theme = Theme::default();
        let (br, bg_, bb) = match theme.bg {
            Color::Rgb(r, g, b) => (r as f32, g as f32, b as f32),
            other => panic!("expected an rgb background, got {other:?}"),
        };
        // Relative luminance, then WCAG contrast. A sign is a small glyph, so
        // 3:1 is the floor rather than the 4.5:1 body text would want.
        fn lum(c: f32) -> f32 {
            let c = c / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        let rel = |r: f32, g: f32, b: f32| 0.2126 * lum(r) + 0.7152 * lum(g) + 0.0722 * lum(b);
        let back = rel(br, bg_, bb);
        for severity in 1u8..=4 {
            let (glyph, (r, g, b, _)) = severity_sign(severity, &theme);
            let front = rel(r * 255.0, g * 255.0, b * 255.0);
            let (hi, lo) = if front > back {
                (front, back)
            } else {
                (back, front)
            };
            let contrast = (hi + 0.05) / (lo + 0.05);
            assert!(
                contrast >= 3.0,
                "severity {severity} draws {glyph} at contrast {contrast:.2}:1 against the \
                 pane background — below the 3:1 floor, which is how an error sign gets \
                 drawn every frame and never seen"
            );
        }
    }

    /// Errors must not look like warnings. The letter distinguishes them, but a
    /// reader scanning a gutter reads colour first.
    #[test]
    fn an_error_does_not_share_a_colour_with_a_warning() {
        let theme = Theme::default();
        assert_ne!(
            severity_sign(1, &theme).1,
            severity_sign(2, &theme).1,
            "an error and a warning are the two a reader most needs to tell apart"
        );
    }
}
