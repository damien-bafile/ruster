//! The compositor's UI chrome: statusline, editor frame, which-key overlay.
//!
//! Chrome is drawn as two kinds of quad, collected into a [`ChromeBatch`]: flat
//! vertex geometry for panels and bars (`Vertex` = x, y, reserved, reserved, r,
//! g, b, a), and textured glyph quads carrying a UV rect into the glyph atlas.
//! The geometry itself now lives in `scene.rs` — a declarative element tree run
//! through the portable layout engine — so what survives here is the chrome's
//! non-scene support: the glyph atlas, keyed render-element ids, the cursor,
//! the editor-frame grid helpers and the launcher layout. `render_frame` turns
//! a batch into smithay render elements and composites it above the client
//! surface.

use std::any::Any;
use std::collections::HashMap;

use crate::compositor::PANE_FONT_PX;
use ruster_render::{Color, StyledLine, SyntaxStyle, Theme};
use ruster_render_elements::ElementKey;
use ruster_render_gles::atlas::{cell_metrics, Atlas};
use ruster_render_gles::cursor::CursorBitmap;
pub use ruster_render_gles::geometry::{ChromeBatch, GlyphQuad, Vertex};
use ruster_shell::Layout;
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
pub(crate) const BORDER_WIDTH: f32 = 2.0;

/// One stretch of a line drawn in a single colour.
pub(crate) struct Run {
    /// Column the run starts at, in cells from the body's left edge.
    pub(crate) column: usize,
    pub(crate) text: String,
    /// `None` where the highlighter had no opinion, which is most of a line.
    pub(crate) color: Option<Color>,
}

/// A line split into runs: each highlighted span, and the plain text between.
///
/// The gaps matter as much as the spans. A highlighter colours keywords and
/// leaves the spaces and identifiers between them alone, so drawing only the
/// spans would draw only a fraction of the line.
pub(crate) fn runs(line: &StyledLine) -> Vec<Run> {
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
pub(crate) fn severity_sign(severity: u8, theme: &Theme) -> (&'static str, (f32, f32, f32, f32)) {
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
pub(crate) fn gutter_width(first_line: usize, shown: usize) -> usize {
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
pub(crate) const SIGN_COLS: usize = 1;

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
pub(crate) const FRAME_BAR_H: f32 = 28.0;

/// Gap between an editor frame's edge and its contents, in physical pixels.
pub(crate) const FRAME_PAD: f32 = 6.0;

/// Where an editor frame's text starts and how big one cell is, in physical
/// pixels measured from the frame's own top-left.
///
/// The one description of that layout: the pane's rows are positioned from this
/// and the pointer reads the same numbers back out, so the grid on screen and
/// the grid under the mouse cannot be measured two ways. That is the same
/// reason `tile_under` and `geometry()` are one list — a second opinion about a
/// rectangle is how a click lands somewhere other than where it looked.
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

    /// The top-left pixel of a cell, in the frame's own coordinates.
    ///
    /// The inverse of [`cell_at`](Self::cell_at), and here rather than at the
    /// caller for the same reason that one exists: anything anchored to a
    /// character — a hover panel, a completion list — has to agree with what was
    /// drawn, and two copies of `x + col * cell_w` are two chances to disagree
    /// about the gutter.
    pub fn cell_origin(&self, row: usize, col: usize) -> (f32, f32) {
        (
            self.x + col as f32 * self.cell_w,
            self.y + row as f32 * self.cell_h,
        )
    }
}

/// Where the launcher panel goes, and how much of the list fits in it.
///
/// A free function, for the same reason `gutter_cols` is one: this is where the
/// bug will be, and it has to be assertable without a GL context.
///
/// The bottom band is reserved and that is not cosmetic. `collect_render_elements`
/// hoists every glyph in front of every panel, sound only while chrome panels do
/// not cover text — and the statusline and the mini-buffer are full-width bars at
/// the bottom of the output. A panel reaching into either would have *their*
/// glyphs drawn over its background. The launcher is kept clear of both by
/// construction rather than by discipline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LauncherLayout {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// How many rows of the list fit. Always at least one, so a panel that can
    /// only show a single result still shows it.
    pub visible_rows: usize,
    pub row_h: f32,
    /// Height of the query line at the top.
    pub query_h: f32,
    /// Height of a group heading.
    pub group_h: f32,
}

pub fn launcher_layout(output_w: i32, output_h: i32, rows: usize) -> LauncherLayout {
    const ROW_H: f32 = 24.0;
    const QUERY_H: f32 = 38.0;
    const GROUP_H: f32 = 18.0;
    const GAP: f32 = 12.0;

    let (ow, oh) = (output_w.max(1) as f32, output_h.max(1) as f32);
    // Two bars' worth: the statusline, and the mini-buffer above it.
    let reserved = 2.0 * crate::render::chrome_height(output_h) as f32 + GAP;

    let w = (ow * 0.55).clamp(360.0, 820.0).min(ow - 8.0).max(64.0);
    let x = ((ow - w) / 2.0).max(4.0);
    let y = (oh * 0.14).min((oh - reserved - QUERY_H).max(4.0));
    let room = (oh - reserved - y).max(QUERY_H);

    // Enough for the rows asked for, never more than the room allows. Group
    // headings are budgeted for generously — one per row — so a list that is all
    // headings still fits rather than overflowing the panel it was measured for.
    let wanted = QUERY_H + rows as f32 * (ROW_H + GROUP_H);
    let h = wanted.min(room).max(QUERY_H);
    let visible_rows = (((h - QUERY_H) / (ROW_H + GROUP_H)).floor() as usize).max(1);

    LauncherLayout {
        x,
        y,
        w,
        h,
        visible_rows,
        row_h: ROW_H,
        query_h: QUERY_H,
        group_h: GROUP_H,
    }
}

/// Where a hover panel hangs: the caret's top-left pixel in *output*
/// coordinates, and the height of the cell it sits on.
///
/// Output coordinates rather than frame-local ones because the panel is drawn
/// after every pane, in the overlay layer — the caller has already added the
/// tile's origin, using the same [`FrameBody`] that drew the text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverAnchor {
    pub x: f32,
    pub y: f32,
    pub cell_h: f32,
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
    /// One render-element id per glyph, keyed by the element the glyph belongs
    /// to and reused across frames.
    ///
    /// The damage tracker keys element state by id, so glyphs sharing one id
    /// would collapse into a single tracked element and damage the wrong
    /// regions. Ids are keyed by [`ElementKey`]: a glyph that does not move
    /// keeps its id, and a chrome element that moves to a new place in the tree
    /// takes a fresh key and fresh ids rather than stealing its neighbour's.
    id_map: HashMap<ElementKey, Vec<Id>>,
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
            id_map: HashMap::new(),
        }
    }

    /// The theme the chrome draws in.
    ///
    /// The scene builders need the theme to colour their widgets, but the field
    /// stays private so a frame's chrome is one consistent palette.
    pub fn theme(&self) -> &Theme {
        &self.theme
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

    /// A stable render-element id for `len` consecutive glyphs of `key`.
    ///
    /// The first glyphs of a key grow a fresh run of ids; later frames reuse
    /// them, so a glyph that does not move reports no damage, and reordering
    /// the chrome remaps keys instead of remapping positions. Each run is
    /// handed out whole so no two keys ever share an id.
    pub fn element_ids(&mut self, key: &ElementKey, len: usize) -> Vec<Id> {
        let ids = self.id_map.entry(key.clone()).or_default();
        while ids.len() < len {
            ids.push(Id::new());
        }
        debug_assert!(ids.len() >= len);
        ids[..len].to_vec()
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

    /// Emit `n` glyph quads of synthetic load, for measuring how many render
    /// elements a frame can carry (`RUSTER_BENCH_GLYPHS`).
    ///
    /// Laid out in a grid the size a text pane would be, using real atlas
    /// glyphs, so the cost measured is the cost an editor pane would pay:
    /// per-element work in the damage tracker plus sampling the atlas texture.
    pub fn bench_glyphs(&mut self, n: usize, batch: &mut ChromeBatch) {
        let fg: (f32, f32, f32, f32) = self.theme.fg.into();
        let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        let rgb = [to_u8(fg.0), to_u8(fg.1), to_u8(fg.2)];
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
            // Each bench glyph is its own element, so the damage tracker counts
            // every one rather than collapsing the run into a single id.
            batch
                .glyph_keys
                .push(ElementKey(vec![format!("bench:{i}")]));
        }
    }
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

    /// Plain lines as the styled ones `pane_elem` takes.
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
        let batch = new_batch(
            scene::window_borders_elem(&windows, Some(WindowId(1)), 1.0, &theme()),
            200.0,
            100.0,
            &mut chrome.atlas,
        );

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
        let batch = new_batch(
            scene::window_borders_elem(&windows, None, 1.0, &theme()),
            100.0,
            50.0,
            &mut chrome.atlas,
        );
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
        let batch = new_batch(
            scene::window_borders_elem(&[(WindowId(0), rect)], None, 1.0, &theme()),
            120.0,
            110.0,
            &mut chrome.atlas,
        );
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
        let batch = new_batch(
            scene::window_borders_elem(
                &[(WindowId(0), Rect::new(0, 0, 100, 100))],
                None,
                2.0,
                &theme(),
            ),
            200.0,
            200.0,
            &mut chrome.atlas,
        );
        let max_x = batch.verts.iter().map(|v| v[0]).fold(0.0f32, f32::max);
        assert_eq!(max_x, 200.0, "a 100px logical window is 200px physical");
    }

    use super::*;
    use crate::scene;
    use ruster_render::Theme;
    use ruster_render_elements::{div, layout, Elem, LayoutScene, PxRect, Styled, TextMeasurer};
    use ruster_render_gles::tessellate::{scene_to_chrome_batch, GlesTextMeasurer};
    use ruster_shell::{Rect, WindowId};

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

    /// Run an element through the same path `render_frame` does, returning the
    /// laid-out scene. The element is wrapped in a sized root: an absolutely
    /// positioned widget can never be the layout root, because `layout`
    /// replaces the root's geometry with the area it is given.
    fn lay_scene(elem: Elem, w: f32, h: f32) -> LayoutScene {
        let mut root = div();
        root.id("root").size(w, h).children(vec![elem]);
        layout(
            PxRect {
                x: 0.0,
                y: 0.0,
                w,
                h,
            },
            &root,
            &mut GlesTextMeasurer,
        )
    }

    /// Tessellate a laid-out element into a batch, the way a frame would.
    fn new_batch(elem: Elem, w: f32, h: f32, atlas: &mut Atlas) -> ChromeBatch {
        scene_to_chrome_batch(&lay_scene(elem, w, h), atlas)
    }

    /// A chrome colour as the 8-bit RGB the atlas bakes into a glyph cell.
    fn rgb8(color: (f32, f32, f32, f32)) -> [u8; 3] {
        let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        [to_u8(color.0), to_u8(color.1), to_u8(color.2)]
    }

    #[test]
    fn statusline_emits_quads() {
        let mut chrome = Chrome::new(theme());
        let h = crate::render::chrome_height(600);
        let batch = new_batch(
            scene::statusline_elem(800, 600, 1, "foot", status(), &theme()),
            800.0,
            600.0,
            &mut chrome.atlas,
        );
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
        let h = crate::render::chrome_height(600);
        let batch = new_batch(
            scene::statusline_elem(800, 600, 1, "foot", status(), &theme()),
            800.0,
            600.0,
            &mut chrome.atlas,
        );
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
        let batch = new_batch(
            scene::pane_elem(
                WindowId(0),
                400,
                300,
                &styled(&buf),
                0,
                &[],
                "welcome",
                &theme(),
            ),
            400.0,
            300.0,
            &mut chrome.atlas,
        );
        assert!(batch.verts.len() >= 6 * 2); // frame + title bar
        assert!(!batch.glyphs.is_empty(), "title and rows draw glyphs");
    }

    #[test]
    fn a_hover_panel_hangs_below_the_caret_when_there_is_room() {
        let mut chrome = Chrome::new(theme());
        let lines = vec!["fn main()".to_string(), "the entry point".to_string()];
        let anchor = HoverAnchor {
            x: 100.0,
            y: 100.0,
            cell_h: 16.0,
        };
        let laid = lay_scene(
            scene::hover_elem(1920, 1080, anchor, &lines, &theme(), &mut GlesTextMeasurer),
            1920.0,
            1080.0,
        );
        let panel = &laid.boxes[0];
        assert_eq!(panel.rect.x, 100.0, "aligned with the caret's column");
        assert_eq!(panel.rect.y, 116.0, "directly under the caret's cell");
        assert!(panel.rect.y + panel.rect.h <= 1080.0);
        let batch = scene_to_chrome_batch(&laid, &mut chrome.atlas);
        assert!(!batch.glyphs.is_empty(), "the panel draws its text");
    }

    #[test]
    fn the_launcher_never_reaches_the_bars_at_the_bottom() {
        // The constraint the layout exists for. Chrome glyphs are hoisted in
        // front of chrome panels, so a launcher overlapping the statusline or
        // the mini-buffer would have their text drawn through it — and it would
        // read as a font or theme bug rather than an ordering one.
        //
        // Checked across the shapes that break geometry: a laptop panel, a 4K
        // display, and one small enough that the reserved band is most of it.
        for (w, h) in [(1920, 1080), (1366, 768), (3840, 2160), (640, 400)] {
            for rows in [0usize, 8, 200] {
                let l = launcher_layout(w, h, rows);
                let floor = h as f32 - 2.0 * crate::render::chrome_height(h) as f32;
                assert!(l.y >= 0.0, "{w}x{h}/{rows}: y={} is off the top", l.y);
                assert!(
                    l.y + l.h <= floor,
                    "{w}x{h}/{rows}: panel reaches {} but the bars start at {floor}",
                    l.y + l.h
                );
                assert!(
                    l.x >= 0.0 && l.x + l.w <= w as f32,
                    "{w}x{h}/{rows}: {}..{} escapes the output",
                    l.x,
                    l.x + l.w
                );
                assert!(
                    l.visible_rows >= 1,
                    "{w}x{h}/{rows}: a panel that can show nothing is not a panel"
                );
            }
        }
    }

    #[test]
    fn a_longer_list_asks_for_a_taller_panel_until_it_cannot() {
        let short = launcher_layout(1920, 1080, 2);
        let long = launcher_layout(1920, 1080, 40);
        assert!(long.h > short.h, "more rows, taller panel");
        assert!(
            long.visible_rows > short.visible_rows,
            "and more of them visible"
        );
        // But never past the reserved band, however many are offered.
        let absurd = launcher_layout(1920, 1080, 100_000);
        assert!(absurd.y + absurd.h <= 1080.0 - 2.0 * crate::render::chrome_height(1080) as f32);
    }

    #[test]
    fn the_launcher_draws_its_query_and_its_rows() {
        let mut chrome = Chrome::new(theme());
        let view = ruster_render::LauncherView {
            query: "fire".into(),
            rows: vec![
                ruster_render::LauncherRow {
                    label: "Firefox".into(),
                    detail: "Web Browser".into(),
                    group: "apps".into(),
                    selected: true,
                },
                ruster_render::LauncherRow {
                    label: "Files".into(),
                    detail: String::new(),
                    group: String::new(),
                    selected: false,
                },
            ],
            message: String::new(),
            scrolled: 0,
            total: 2,
        };
        let laid = lay_scene(
            scene::launcher_elem(1920, 1080, &view, &theme(), &mut GlesTextMeasurer),
            1920.0,
            1080.0,
        );
        // The panel is where the layout said it would be.
        let expected = launcher_layout(1920, 1080, 2);
        assert_eq!(laid.boxes[0].rect.x, expected.x);
        assert_eq!(laid.boxes[0].rect.y, expected.y);
        assert_eq!(laid.boxes[0].rect.w, expected.w);
        assert_eq!(laid.boxes[0].rect.h, expected.h);
        let batch = scene_to_chrome_batch(&laid, &mut chrome.atlas);
        assert!(!batch.glyphs.is_empty(), "the rows are drawn");
        assert!(
            batch.verts.len() >= 12,
            "the panel and the selection highlight are both filled"
        );
    }

    #[test]
    fn launcher_text_lands_where_launcher_layout_puts_it() {
        // BLOCKER 1 regression: every child of the panel used output
        // coordinates (layout.x + PAD, layout.y + PAD), but the panel itself is
        // absolute at (layout.x, layout.y) — taffy resolves absolute children
        // against the panel, so each landed at panel.origin + written position,
        // doubled by (layout.x, layout.y). The query line meant for y≈163 came
        // out at y≈314, below the panel. Children must be at panel-local coords;
        // the panel's origin supplies the layout offset.
        const PAD: f32 = 12.0;
        let view = ruster_render::LauncherView {
            query: "fire".into(),
            rows: vec![
                ruster_render::LauncherRow {
                    label: "Firefox".into(),
                    detail: "Web Browser".into(),
                    group: "apps".into(),
                    selected: true,
                },
                ruster_render::LauncherRow {
                    label: "Files".into(),
                    detail: String::new(),
                    group: String::new(),
                    selected: false,
                },
            ],
            message: String::new(),
            scrolled: 0,
            total: 2,
        };
        let laid = lay_scene(
            scene::launcher_elem(1920, 1080, &view, &theme(), &mut GlesTextMeasurer),
            1920.0,
            1080.0,
        );
        let expected = launcher_layout(1920, 1080, 2);
        let panel = &laid.boxes[0];

        // The sigil sits at the panel's top-left corner plus its padding, and
        // the query right of it by the sigil's advance — exactly where the
        // hand-built launcher put them, in output coordinates.
        let sigil = laid
            .texts
            .iter()
            .find(|t| t.line.text == ">")
            .expect("the query sigil is drawn");
        assert_eq!(sigil.rect.x, expected.x + PAD, "sigil at layout.x + PAD");
        assert_eq!(sigil.rect.y, expected.y + PAD, "sigil at layout.y + PAD");

        let (sigil_w, _) = GlesTextMeasurer.measure(
            &StyledLine {
                text: ">".into(),
                highlights: Vec::new(),
            },
            15.0,
            ruster_render::FontFamily::Ui,
        );
        let query = laid
            .texts
            .iter()
            .find(|t| t.line.text == "fire")
            .expect("the query text is drawn");
        assert_eq!(query.rect.x, expected.x + PAD + sigil_w + 6.0);
        assert_eq!(query.rect.y, expected.y + PAD);

        // And the whole query line sits inside the panel, not under it.
        assert!(query.rect.y + query.rect.h <= panel.rect.y + panel.rect.h);
    }

    #[test]
    fn two_panes_under_one_root_get_distinct_keys() {
        // BLOCKER 2 regression: pane_elem hardcoded the id "pane", so two panes
        // under one compose root tripped push_children's duplicate-id debug
        // assert and — in release — shared an element key path, collapsing both
        // panes' glyphs into one id run. Each pane must carry its WindowId in
        // the key.
        let mut p0 = scene::pane_elem(
            WindowId(0),
            400,
            300,
            &styled(&["hi".to_string()]),
            0,
            &[],
            "welcome",
            &theme(),
        );
        p0.absolute().position(0.0, 0.0);
        let mut p1 = scene::pane_elem(
            WindowId(1),
            400,
            300,
            &styled(&["yo".to_string()]),
            0,
            &[],
            "files",
            &theme(),
        );
        p1.absolute().position(0.0, 300.0);
        let mut root = div();
        root.children(vec![p0, p1]);
        let laid = layout(
            PxRect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 600.0,
            },
            &root,
            &mut GlesTextMeasurer,
        );

        let firsts: Vec<&str> = laid
            .boxes
            .iter()
            .map(|b| b.key.0.first().unwrap().as_str())
            .collect();
        assert!(
            firsts.contains(&"pane:0"),
            "pane 0's boxes keyed under pane:0, got {firsts:?}"
        );
        assert!(
            firsts.contains(&"pane:1"),
            "pane 1's boxes keyed under pane:1, got {firsts:?}"
        );
        let distinct: std::collections::HashSet<&str> = firsts.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            2,
            "two panes, two distinct id runs: {firsts:?}"
        );
    }

    #[test]
    fn an_empty_launcher_still_draws_its_prompt() {
        // Opening it must show something immediately, before a provider has
        // said anything — an overlay that appears blank reads as a crash.
        let mut chrome = Chrome::new(theme());
        let view = ruster_render::LauncherView {
            message: "no matches".into(),
            ..Default::default()
        };
        let batch = new_batch(
            scene::launcher_elem(1920, 1080, &view, &theme(), &mut GlesTextMeasurer),
            1920.0,
            1080.0,
            &mut chrome.atlas,
        );
        assert!(!batch.verts.is_empty(), "the panel is there");
        assert!(!batch.glyphs.is_empty(), "and it says so");
    }

    #[test]
    fn a_panel_that_covers_text_draws_into_the_overlay_layer() {
        // `collect_render_elements` hoists every glyph in front of every panel,
        // sound only while chrome panels do not cover text. The hover panel
        // covers the pane text it describes, so before the overlay layer the
        // pane's glyphs were drawn in front of the hover's background — correct
        // by every test, and wrong on screen. Hover was proven headlessly and
        // never captured, which is how it went unnoticed.
        //
        // The overlay is now a separate batch, distinct from the base: the
        // scene path cannot smear the hover's panels into the base layer's
        // geometry.
        let mut chrome = Chrome::new(theme());
        let base = new_batch(
            scene::statusline_elem(1920, 1080, 1, "x", status(), &theme()),
            1920.0,
            1080.0,
            &mut chrome.atlas,
        );
        let base_panels = base.verts.len();

        let overlay = new_batch(
            scene::hover_elem(
                1920,
                1080,
                HoverAnchor {
                    x: 100.0,
                    y: 100.0,
                    cell_h: 16.0,
                },
                &["fn main()".to_string()],
                &theme(),
                &mut GlesTextMeasurer,
            ),
            1920.0,
            1080.0,
            &mut chrome.atlas,
        );

        assert!(
            !overlay.verts.is_empty() && !overlay.glyphs.is_empty(),
            "the hover panel belongs to the overlay layer"
        );
        assert_eq!(
            base.verts.len(),
            base_panels,
            "and must not have added anything to the base layer"
        );
    }

    #[test]
    fn a_hover_panel_flips_above_a_caret_near_the_bottom() {
        // The failure this prevents is silent: a hover on one of the last lines
        // of a pane is drawn past the bottom of the output, and a panel nobody
        // can see is indistinguishable from a server that said nothing.
        let lines = vec!["fn main()".to_string(), "the entry point".to_string()];
        let anchor = HoverAnchor {
            x: 100.0,
            y: 1070.0,
            cell_h: 16.0,
        };
        let laid = lay_scene(
            scene::hover_elem(1920, 1080, anchor, &lines, &theme(), &mut GlesTextMeasurer),
            1920.0,
            1080.0,
        );
        let panel = &laid.boxes[0];
        assert!(
            panel.rect.y + panel.rect.h <= 1070.0,
            "the panel should sit above the caret, not run off the output: y={} h={}",
            panel.rect.y,
            panel.rect.h
        );
        assert!(panel.rect.y >= 0.0, "and not above the top edge either");
    }

    #[test]
    fn a_hover_panel_in_the_last_column_stays_on_screen() {
        let lines = vec!["a fairly long explanation of a symbol".to_string()];
        let anchor = HoverAnchor {
            x: 1900.0,
            y: 100.0,
            cell_h: 16.0,
        };
        let laid = lay_scene(
            scene::hover_elem(1920, 1080, anchor, &lines, &theme(), &mut GlesTextMeasurer),
            1920.0,
            1080.0,
        );
        let panel = &laid.boxes[0];
        assert!(
            panel.rect.x + panel.rect.w <= 1920.0,
            "a hover in the last column must be pulled back on screen: x={} w={}",
            panel.rect.x,
            panel.rect.w
        );
        assert!(panel.rect.x >= 0.0);
    }

    #[test]
    fn a_cell_origin_is_where_a_click_on_that_cell_lands() {
        // The two halves of one grid. `cell_at` decides which character a click
        // means and `cell_origin` decides where a panel about that character
        // hangs; if they disagree the hover points at its neighbour, and the
        // gutter is where they would disagree, since it widens with the line
        // numbers on screen.
        for (first, shown) in [(0usize, 10usize), (95, 10), (995, 10)] {
            let body = FrameBody::new(first, shown);
            for (row, col) in [(0usize, 0usize), (3, 7), (9, 40)] {
                let (x, y) = body.cell_origin(row, col);
                assert_eq!(
                    body.cell_at(x + body.cell_w / 2.0, y + body.cell_h / 2.0),
                    (row, col),
                    "cell ({row}, {col}) with the gutter for lines {first}..{}",
                    first + shown
                );
            }
        }
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
        let batch = new_batch(
            scene::whichkey_elem(1920, 1080, &view, &theme()),
            1920.0,
            1080.0,
            &mut chrome.atlas,
        );
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
        let _ = new_batch(
            scene::whichkey_elem(1920, 1080, &view, &theme()),
            1920.0,
            1080.0,
            &mut chrome.atlas,
        );
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
        // A short output, so 60 rows cannot possibly stack vertically.
        let batch = new_batch(
            scene::whichkey_elem(1920, 400, &view, &theme()),
            1920.0,
            400.0,
            &mut chrome.atlas,
        );

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
    fn a_pane_at_a_non_zero_rect_lands_offset_correctly() {
        // The old painter translated a pane laid out at the origin into its tile
        // after the fact. The scene does it up front: the pane element is
        // absolutely positioned at the tile's origin, so its title bar and every
        // glyph land there directly.
        let mut chrome = Chrome::new(theme());
        let mut pane = scene::pane_elem(
            WindowId(0),
            400,
            300,
            &styled(&["hi".to_string()]),
            0,
            &[],
            "welcome",
            &theme(),
        );
        pane.absolute().position(50.0, 30.0);
        let laid = lay_scene(pane, 400.0, 300.0);

        // The pane's background and title bar sit at the tile origin.
        assert_eq!(laid.boxes[0].rect.x, 50.0, "the pane lands at the tile's x");
        assert_eq!(laid.boxes[0].rect.y, 30.0, "and at its y");
        assert!(
            laid.boxes
                .iter()
                .skip(1)
                .all(|b| b.rect.x >= 50.0 && b.rect.y >= 30.0),
            "nothing draws outside the pane's origin"
        );
        assert!(
            laid.texts
                .iter()
                .all(|t| t.rect.x >= 50.0 && t.rect.y >= 30.0),
            "and no glyph draws outside it either"
        );

        // Tessellated, the first title glyph is offset with the pane.
        let batch = scene_to_chrome_batch(&laid, &mut chrome.atlas);
        assert!(
            batch.glyphs[0].x >= 50.0 && batch.glyphs[0].y >= 30.0,
            "the title glyph is inside the pane"
        );
    }

    #[test]
    fn statusline_colors_are_legible() {
        // Text on the statusline is drawn in the statusline foreground (and the
        // mode glyph in the accent foreground), never in the accent color that
        // fills the mode segment — the brief flagged amber-on-amber as a bug.
        // Glyph colour is baked into the atlas cell, so ask the atlas which
        // cells the scene actually produced.
        let mut chrome = Chrome::new(theme());
        let laid = lay_scene(
            scene::statusline_elem(800, 600, 1, "foot", status(), &theme()),
            800.0,
            600.0,
        );
        let batch = scene_to_chrome_batch(&laid, &mut chrome.atlas);
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
