//! The compositor's chrome as a declarative scene.
//!
//! Each widget is built as an [`Elem`] tree with the geometry the chrome's old
//! hand-built `draw_*` methods produced. `compose` is the whole frame in
//! painter's order, and `chrome_scene` assembles one from a [`FrameInput`],
//! handing the overlay layer back separately — the launcher and the hover
//! panel, the only chrome drawn in front of the base batch.

use ruster_render::{Color, LauncherView, StyledLine, Theme, WhichKeyEntry, WhichKeyView};
use ruster_render_elements::{div, text, Elem, Styled, TextMeasurer};
use ruster_render_gles::atlas::FontFamily;
use ruster_shell::{Rect, WindowId};

use crate::chrome::gutter_width;
use crate::chrome::{
    launcher_layout, runs, severity_sign, FrameBody, HoverAnchor, TreeStatus, BORDER_WIDTH,
    FRAME_BAR_H, FRAME_PAD, SIGN_COLS,
};
use crate::compositor::PANE_FONT_PX;
use crate::render::{chrome_height, FrameInput};

/// The statusline's accent mode segment width.
const STATUSLINE_MODE_W: f32 = 64.0;

/// Measure a plain string through the scene's measurer.
///
/// The scene measures text with the same [`TextMeasurer`] the layout walk will
/// use, so the numbers that size a panel and the numbers that position its
/// glyphs come from the same source.
fn measure_width(measurer: &mut impl TextMeasurer, s: &str, size: f32, family: FontFamily) -> f32 {
    let line = StyledLine {
        text: s.to_string(),
        highlights: Vec::new(),
    };
    measurer.measure(&line, size, family).0
}

/// A chrome colour tuple back into a [`Color`], preserving the exact 8-bit
/// value the atlas bakes into a glyph cell.
///
/// `severity_sign` hands out colours as the `(f32, f32, f32, f32)` tuple the old
/// draw path consumed; converting back with the same rounding the old path's
/// `rgb8` free function applied makes the scene's text colour bit-identical to
/// the old path's.
fn tuple_color(color: (f32, f32, f32, f32)) -> Color {
    let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::Rgb(to_u8(color.0), to_u8(color.1), to_u8(color.2))
}

/// A statusline: accent mode segment, then workspace label, tree indicator and
/// the focused title.
///
/// The bar is absolute at the bottom; its three text children flow left to
/// right from `padding_left(76)`, which is where the mode segment's own box
/// ends.
pub fn statusline_elem(
    w: i32,
    h: i32,
    workspace: u32,
    focused_title: &str,
    tree: TreeStatus,
    theme: &Theme,
) -> Elem {
    let bar_h = chrome_height(h) as f32;
    let y = (h - bar_h as i32) as f32;
    let pad = (bar_h - 16.0) / 2.0;
    let title = if focused_title.is_empty() {
        "(no client)"
    } else {
        focused_title
    };

    let mut mode = div();
    mode.absolute()
        .position(0.0, 0.0)
        .size(STATUSLINE_MODE_W, bar_h)
        .bg(theme.accent)
        .padding_left((STATUSLINE_MODE_W - 16.0) / 2.0)
        .padding_top(pad);
    let mut mode_letter = text("N");
    mode_letter.font_size(16.0).fg(theme.accent_fg);
    mode.children(vec![mode_letter]);

    let mut ws = text(format!("WS {workspace}"));
    ws.font_size(16.0).fg(theme.statusline_fg);
    let mut indicator = text(tree.indicator());
    indicator.font_size(16.0).fg(theme.accent);
    let mut title_text = text(title);
    title_text.font_size(16.0).fg(theme.statusline_fg);

    let mut bar = div();
    bar.id("statusline")
        .absolute()
        .position(0.0, y)
        .size(w as f32, bar_h)
        .bg(theme.statusline_bg)
        .flex_row()
        .gap(20.0)
        .padding_left(76.0)
        .padding_top(pad)
        .children(vec![mode, ws, indicator, title_text]);
    bar
}

/// Outline every visible window, marking the focused one.
///
/// Four absolute bars per window, in the same top/bottom/left/right order the
/// old draw path emitted them. Drawn first in the batch so nothing lands under
/// the statusline.
pub fn window_borders_elem(
    windows: &[(WindowId, Rect)],
    focus: Option<WindowId>,
    scale: f64,
    theme: &Theme,
) -> Elem {
    let s = scale as f32;
    let width = (BORDER_WIDTH * s).max(1.0);
    let mut root = div();
    root.id("window-borders");
    let mut bars = Vec::new();
    for (id, rect) in windows {
        let color = if Some(*id) == focus {
            theme.border_focused
        } else {
            theme.border_unfocused
        };
        let (x, y) = (rect.x as f32 * s, rect.y as f32 * s);
        let (w, h) = (rect.w as f32 * s, rect.h as f32 * s);
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let mut top = div();
        top.absolute().position(x, y).size(w, width).bg(color);
        let mut bottom = div();
        bottom
            .absolute()
            .position(x, y + h - width)
            .size(w, width)
            .bg(color);
        let mut left = div();
        left.absolute().position(x, y).size(width, h).bg(color);
        let mut right = div();
        right
            .absolute()
            .position(x + w - width, y)
            .size(width, h)
            .bg(color);
        bars.extend([top, bottom, left, right]);
    }
    root.children(bars);
    root
}

/// The `:` line just above the statusline: sigil in the accent, the rest in the
/// text colour, flowing from `padding_left(10)`.
pub fn minibuffer_elem(
    output_w: i32,
    output_h: i32,
    line: &str,
    sigil_len: usize,
    theme: &Theme,
) -> Elem {
    let bar_h = chrome_height(output_h) as f32;
    let y = output_h as f32 - bar_h - bar_h;
    let font = (bar_h * 0.5) as u32;
    let (sigil, rest) = line.split_at(sigil_len.min(line.len()));

    let mut sigil_text = text(sigil);
    sigil_text.font_size(font as f32).fg(theme.cmdline_accent);
    let mut rest_text = text(rest);
    rest_text.font_size(font as f32).fg(theme.cmdline_fg);

    let mut bar = div();
    bar.id("minibuffer")
        .absolute()
        .position(0.0, y)
        .size(output_w as f32, bar_h)
        .bg(theme.cmdline_bg)
        .flex_row()
        .gap(0.0)
        .padding_left(10.0)
        .padding_top(bar_h * 0.25)
        .children(vec![sigil_text, rest_text]);
    bar
}

/// The which-key overlay: a panel in as many columns as it takes to show every
/// binding.
///
/// The column chunking (`per_col`, `columns`) and the width clamp are the old
/// which-key painter's, kept verbatim; taffy then sizes each column from the
/// measured text, which reproduces the old measured `widths` exactly.
pub fn whichkey_elem(output_w: i32, output_h: i32, view: &WhichKeyView, theme: &Theme) -> Elem {
    const ROW_H: f32 = 20.0;
    const PAD: f32 = 10.0;
    const GAP: f32 = 8.0;
    const COL_GAP: f32 = 18.0;

    let title_h = if view.title.is_empty() { 0.0 } else { ROW_H };
    let usable_h = (output_h as f32 * 0.8 - PAD * 2.0 - title_h).max(ROW_H);
    let per_col = ((usable_h / ROW_H) as usize).max(1);
    let columns: Vec<&[WhichKeyEntry]> = view.rows.chunks(per_col).collect();

    let mut col_elems = Vec::with_capacity(columns.len());
    for column in &columns {
        let mut col = div();
        col.flex_col();
        let mut rows = Vec::with_capacity(column.len());
        for entry in column.iter() {
            let mut key = text(entry.key.as_str());
            key.font_size(14.0).fg(theme.whichkey_key);
            let mut desc = text(entry.desc.as_str());
            desc.font_size(14.0).fg(theme.whichkey_fg);
            let mut row = div();
            row.flex_row().gap(GAP).h(ROW_H).children(vec![key, desc]);
            rows.push(row);
        }
        col.children(rows);
        col_elems.push(col);
    }

    let mut columns_div = div();
    columns_div
        .flex_row()
        .gap(COL_GAP)
        .min_w_0()
        .children(col_elems);

    let mut panel = div();
    panel
        .id("whichkey")
        .absolute()
        .position(12.0, 12.0)
        .bg(theme.whichkey_bg)
        .radius(6.0)
        .flex_col()
        .padding(PAD)
        .max_w((output_w as f32 - 24.0).max(0.0));
    if !view.title.is_empty() {
        let mut title = text(view.title.as_str());
        // The title is drawn but never contributes to the panel's width — the
        // old path sized the panel from the columns alone, and taffy would
        // otherwise widen it past that when a short binding list has a long
        // title. Zeroing the width keeps the box a plain pen position.
        title.font_size(14.0).fg(theme.whichkey_key).h(ROW_H).w(0.0);
        panel.children(vec![title, columns_div]);
    } else {
        panel.children(vec![columns_div]);
    }
    panel
}

/// A hover panel anchored under the caret it describes, flipping above it when
/// there is no room below.
///
/// The width needs measuring *before* layout to decide the flip and the x
/// clamp, so the lines are pre-measured through the injected measurer — the
/// same source the layout walk will use.
pub fn hover_elem(
    output_w: i32,
    output_h: i32,
    anchor: HoverAnchor,
    lines: &[String],
    theme: &Theme,
    measurer: &mut impl TextMeasurer,
) -> Elem {
    const ROW_H: f32 = 18.0;
    const PAD: f32 = 8.0;

    let HoverAnchor {
        x: cell_x,
        y: cell_y,
        cell_h,
    } = anchor;
    let text_w = lines
        .iter()
        .map(|l| measure_width(measurer, l, 14.0, FontFamily::Ui))
        .fold(0.0f32, f32::max);
    let w = (text_w + PAD * 2.0).min(output_w as f32 - 8.0);
    let h = PAD * 2.0 + lines.len() as f32 * ROW_H;

    // Below the caret by default, above it when that would not fit; clamped
    // horizontally so a hover in the last column is not half off the edge.
    let below = cell_y + cell_h;
    let y = if below + h <= output_h as f32 {
        below
    } else {
        (cell_y - h).max(4.0)
    };
    let x = cell_x.min(output_w as f32 - w - 4.0).max(4.0);

    let mut panel = div();
    panel
        .id("hover")
        .absolute()
        .position(x, y)
        .size(w, h)
        .bg(theme.whichkey_bg)
        .radius(6.0)
        .flex_col()
        .padding(PAD);
    let mut line_elems = Vec::with_capacity(lines.len());
    for line in lines {
        let mut t = text(line.as_str());
        t.font_size(14.0).fg(theme.whichkey_fg).h(ROW_H);
        line_elems.push(t);
    }
    panel.children(line_elems);
    panel
}

/// The command launcher: a centered panel with a query line and the first page
/// of rows.
///
/// The old launcher painter's geometry kept verbatim, rows positioned by the
/// shared [`launcher_layout`] — the same numbers the pointer hit-testing reads
/// back — with the selection rectangle behind its row's text the way the old
/// path drew it. The panel is part of the overlay: it owns the screen while it
/// is open.
pub fn launcher_elem(
    output_w: i32,
    output_h: i32,
    view: &LauncherView,
    theme: &Theme,
    measurer: &mut impl TextMeasurer,
) -> Elem {
    const FONT: f32 = 15.0;
    const GROUP_FONT: f32 = 12.0;
    const PAD: f32 = 12.0;

    let layout = launcher_layout(output_w, output_h, view.rows.len());

    let mut panel = div();
    panel
        .id("launcher")
        .absolute()
        .position(layout.x, layout.y)
        .size(layout.w, layout.h)
        .bg(theme.whichkey_bg)
        .radius(8.0);

    let mut children: Vec<Elem> = Vec::new();

    // The query line, with its sigil accented the way the `:` prompt's is —
    // the same cue for the same thing, an editor waiting for input.
    //
    // Every child is positioned at *panel-local* coordinates: the panel is
    // absolute at (layout.x, layout.y), and taffy resolves absolute children
    // against it, so the panel's own origin supplies the layout offset. Writing
    // output coordinates (layout.x + PAD, ...) here doubled the offset and
    // dropped the query line below the panel.
    let mut sigil = text(">");
    sigil
        .absolute()
        .position(PAD, PAD)
        .font_size(FONT)
        .fg(theme.whichkey_key);
    let mut query = text(view.query.as_str());
    query
        .absolute()
        .position(
            PAD + measure_width(measurer, ">", FONT, FontFamily::Ui) + 6.0,
            PAD,
        )
        .font_size(FONT)
        .fg(theme.whichkey_fg);
    children.push(sigil);
    children.push(query);

    let mut y = layout.query_h;
    if view.rows.is_empty() {
        if !view.message.is_empty() {
            let mut message = text(view.message.as_str());
            message
                .absolute()
                .position(PAD, y)
                .font_size(FONT)
                .fg(theme.gutter);
            children.push(message);
        }
        panel.children(children);
        return panel;
    }

    for row in view.rows.iter().take(layout.visible_rows) {
        if !row.group.is_empty() {
            let mut group = text(row.group.as_str());
            group
                .absolute()
                .position(PAD, y)
                .font_size(GROUP_FONT)
                .fg(theme.gutter);
            children.push(group);
            y += layout.group_h;
        }
        if row.selected {
            let mut sel = div();
            sel.absolute()
                .position(4.0, y - 2.0)
                .size(layout.w - 8.0, layout.row_h)
                .bg(theme.selection_bg);
            children.push(sel);
        }
        let mut label = text(row.label.as_str());
        label
            .absolute()
            .position(PAD * 2.0, y)
            .font_size(FONT)
            .fg(theme.whichkey_fg);
        children.push(label);
        if !row.detail.is_empty() {
            // Right of the label rather than right-aligned to the panel: a
            // long detail then truncates against the panel edge instead of
            // colliding with the label from the other side.
            let label_w = measure_width(measurer, &row.label, FONT, FontFamily::Ui);
            let at = PAD * 2.0 + label_w + 12.0;
            if at < layout.w - PAD {
                let mut detail = text(row.detail.as_str());
                detail
                    .absolute()
                    .position(at, y + 2.0)
                    .font_size(GROUP_FONT)
                    .fg(theme.gutter);
                children.push(detail);
            }
        }
        y += layout.row_h;
    }
    panel.children(children);
    panel
}

/// An editor pane: title bar and a grid of buffer rows.
///
/// Per-line text is absolutely positioned from the frame's grid — the same
/// cells the pointer reads back — never by chaining advances. The id carries
/// the window it tiles, so two panes under one compose root never share a key
/// path (`push_children` rejects duplicate sibling ids).
#[allow(clippy::too_many_arguments)]
pub fn pane_elem(
    id: WindowId,
    w: i32,
    h: i32,
    lines: &[StyledLine],
    first_line: usize,
    severities: &[Option<u8>],
    title: &str,
    theme: &Theme,
) -> Elem {
    let bar_h = FRAME_BAR_H;
    let body = FrameBody::new(first_line, lines.len());
    let number_cols = gutter_width(first_line, lines.len());
    let rows = ((h as f32 - bar_h - 8.0) / body.cell_h).max(0.0) as usize;
    // Numbers start past the sign column, which is what keeps the two from
    // being drawn on top of each other.
    let numbers_x = FRAME_PAD + SIGN_COLS as f32 * body.cell_w;

    let mut root = div();
    root.id(&format!("pane:{}", id.0))
        .absolute()
        .position(0.0, 0.0)
        .size(w as f32, h as f32)
        .bg(theme.bg)
        .radius(4.0);

    let mut title_bar = div();
    title_bar
        .absolute()
        .position(0.0, 0.0)
        .size(w as f32, bar_h)
        .bg(theme.accent);
    let mut title_text = text(title);
    title_text
        .absolute()
        .position(FRAME_PAD, (bar_h - 16.0) / 2.0)
        .font_size(16.0)
        .fg(theme.accent_fg);

    let mut row_elems = Vec::new();
    for (row, line) in lines.iter().take(rows).enumerate() {
        let gy = body.y + row as f32 * body.cell_h;
        if let Some(severity) = severities.get(row).copied().flatten() {
            let (glyph, color) = severity_sign(severity, theme);
            let mut sign = text(glyph);
            sign.absolute()
                .position(FRAME_PAD, gy)
                .font_size(PANE_FONT_PX as f32)
                .font_family(FontFamily::Mono)
                .fg(tuple_color(color));
            row_elems.push(sign);
        }
        if number_cols > 0 {
            // Right-aligned, the way every editor draws line numbers: the
            // units column has to line up or the eye cannot scan it.
            let number = format!("{:>width$} ", first_line + row + 1, width = number_cols - 1);
            let mut num = text(number);
            num.absolute()
                .position(numbers_x, gy)
                .font_size(PANE_FONT_PX as f32)
                .font_family(FontFamily::Mono)
                .fg(theme.gutter);
            row_elems.push(num);
        }
        // One run per span, positioned by *cell* rather than by chaining
        // advances from the previous run — the grid is the authority.
        for run in runs(line) {
            let mut run_text = text(run.text.as_str());
            run_text
                .absolute()
                .position(body.x + run.column as f32 * body.cell_w, gy)
                .font_size(PANE_FONT_PX as f32)
                .font_family(FontFamily::Mono)
                .fg(run.color.unwrap_or(theme.fg));
            row_elems.push(run_text);
        }
    }

    let mut children = Vec::with_capacity(2 + row_elems.len());
    children.push(title_bar);
    children.push(title_text);
    children.extend(row_elems);
    root.children(children);
    root
}

/// One frame of chrome in painter's order: the given pieces inside a root the
/// size of the output.
///
/// The root has no background — the old path drew none, and adding one would
/// change every pixel on screen.
pub fn compose(pieces: Vec<Elem>, _theme: &Theme, w: f32, h: f32) -> Elem {
    let mut root = div();
    root.size(w, h).children(pieces);
    root
}

/// Assemble the whole frame's chrome as one scene, in the order the old draw
/// path emitted it: window borders, statusline, mini-buffer, which-key, then
/// the editor panes. The overlay layer comes back separately, drawn in front
/// of the base batch — the launcher first, then the hover panel, so a hover on
/// text under the launcher still renders above it.
pub fn chrome_scene(
    frame: &FrameInput,
    theme: &Theme,
    measurer: &mut impl TextMeasurer,
) -> (Elem, Option<Elem>) {
    let size = frame
        .output
        .current_mode()
        .map(|mode| mode.size)
        .unwrap_or_default();
    let scale = frame.output.current_scale().fractional_scale();
    let (w, h) = (size.w as f32, size.h as f32);

    let mut pieces: Vec<Elem> = Vec::new();
    pieces.push(window_borders_elem(
        frame.geometry,
        frame.focus,
        scale,
        theme,
    ));
    pieces.push(statusline_elem(
        size.w,
        size.h,
        frame.workspace,
        frame.focused_title,
        frame.tree_status,
        theme,
    ));

    if let Some(mb) = frame.minibuffer {
        pieces.push(minibuffer_elem(
            size.w,
            size.h,
            &mb.display(),
            mb.sigil_len(),
            theme,
        ));
    }

    if let Some(view) = &frame.whichkey {
        pieces.push(whichkey_elem(size.w, size.h, view, theme));
    }

    // Editor panes, at the rectangles the layout gave them. Chrome is measured
    // in physical pixels and the layout in logical ones, the same conversion
    // the window-borders elements do. The hover anchor is resolved here because the
    // gutter — and therefore the first text column — depends on which lines
    // that pane is showing.
    let chrome_scale = scale as f32;
    let mut hover_at: Option<(HoverAnchor, &[String])> = None;
    for (id, rect) in frame.geometry {
        let Some(pane) = frame.panes.get(id) else {
            continue;
        };
        let Some(doc) = frame.buffers.get(pane.doc) else {
            continue;
        };
        let (first_line, lines) = pane.visible_lines(&doc.buffer);
        let extension = doc
            .file_path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let lines = frame.highlights.borrow_mut().styled_lines(
            pane.doc,
            extension,
            &doc.buffer,
            first_line,
            &lines,
        );
        let severities =
            pane.line_severities(frame.lsp.diagnostics(pane.doc), first_line, lines.len());
        // Only while the line it describes is actually on screen.
        if let Some(hover) = frame.hover.filter(|h| h.pane == *id) {
            if (first_line..first_line + lines.len()).contains(&hover.row) {
                let body = FrameBody::new(first_line, lines.len());
                let (bx, by) = body.cell_origin(hover.row - first_line, hover.col);
                hover_at = Some((
                    HoverAnchor {
                        x: rect.x as f32 * chrome_scale + bx,
                        y: rect.y as f32 * chrome_scale + by,
                        cell_h: body.cell_h,
                    },
                    &hover.lines,
                ));
            }
        }
        let mut pane_el = pane_elem(
            *id,
            (rect.w as f32 * chrome_scale) as i32,
            (rect.h as f32 * chrome_scale) as i32,
            &lines,
            first_line,
            &severities,
            &doc.name,
            theme,
        );
        pane_el
            .absolute()
            .position(rect.x as f32 * chrome_scale, rect.y as f32 * chrome_scale);
        pieces.push(pane_el);
    }

    let base = compose(pieces, theme, w, h);
    let mut overlay_pieces: Vec<Elem> = Vec::new();
    if let Some(view) = &frame.launcher {
        overlay_pieces.push(launcher_elem(size.w, size.h, view, theme, measurer));
    }
    if let Some((anchor, lines)) = hover_at {
        overlay_pieces.push(hover_elem(size.w, size.h, anchor, lines, theme, measurer));
    }
    let overlay = if overlay_pieces.is_empty() {
        None
    } else {
        Some(compose(overlay_pieces, theme, w, h))
    };
    (base, overlay)
}
