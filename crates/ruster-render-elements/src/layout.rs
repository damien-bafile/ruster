//! Layout scene types: the pure, GPU-free output of laying an [`Elem`] tree
//! through taffy. [`layout`] walks the tree, builds a `TaffyTree` in lockstep
//! with a mirror `SceneNode` tree, measures text leaves through the injected
//! [`TextMeasurer`], runs taffy, then reads the rects back into painter's-order
//! [`BoxNode`]s / [`TextNode`]s.

use crate::element::{Elem, ElemKind};
use crate::id::ElementKey;
use crate::style::Style;
use ruster_render::{Color, FontFamily, StyledLine};
use std::collections::HashSet;
use taffy::geometry::{Point, Size};
use taffy::prelude::length;
use taffy::style::{AvailableSpace, Position};
use taffy::tree::{NodeId, TaffyTree};
use taffy::Style as TaffyStyle;

/// How wide/tall a text leaf would be, in physical px. Injected into the layout
/// walk so text measurement is backend-agnostic.
pub trait TextMeasurer {
    fn measure(&mut self, line: &StyledLine, font_size: f32, family: FontFamily) -> (f32, f32);
}

/// A rectangle in physical px, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PxRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A painted rectangle in painter's order: a container's background or border.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxNode {
    pub rect: PxRect,
    pub radius: f32,
    pub fill: (f32, f32, f32, f32),
    pub border_width: f32,
    pub border_color: (f32, f32, f32, f32),
    pub key: ElementKey,
}

/// A text leaf positioned and styled for painting.
#[derive(Debug, Clone, PartialEq)]
pub struct TextNode {
    pub rect: PxRect,
    pub line: StyledLine,
    pub font_size: f32,
    pub family: FontFamily,
    pub fg: (f32, f32, f32, f32),
    pub bold: bool,
    pub key: ElementKey,
}

/// One frame of chrome, emitted in painter's order: every `BoxNode` before its
/// children, `TextNode`s among them. Pure data — unit-testable with no GPU.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutScene {
    pub boxes: Vec<BoxNode>,
    pub texts: Vec<TextNode>,
}

/// What one taffy node is, for the measure closure and the read-back walk.
#[derive(Debug, Clone)]
enum NodeKind {
    Container,
    Text {
        line: StyledLine,
        font_size: f32,
        family: FontFamily,
    },
}

/// The per-node context taffy stores, so its measure closure can see whether a
/// leaf is text and with what payload.
#[derive(Debug, Clone)]
struct NodeCtx {
    key: ElementKey,
    kind: NodeKind,
    parent_key: ElementKey,
}

/// The mirror tree built beside the `TaffyTree`, so the read-back walk has the
/// visual [`Style`] and the text payload taffy does not carry.
struct SceneNode {
    node: NodeId,
    ctx: NodeCtx,
    style: Style,
    children: Vec<SceneNode>,
}

/// Lay `root` out inside `area` and emit the scene in painter's order.
///
/// The root is a synthetic viewport: its layout style is replaced by the area
/// (size, `Position::Relative`, nothing else), while its visual fields (bg,
/// radius, border) still paint — so a root with a background fills the whole
/// area. Every element below it keeps its own style verbatim.
///
/// Text leaves are measured through `measurer`; container leaves measure to
/// `Size::ZERO` and taffy sizes them from their children. Rounding is disabled
/// so the read-back rects are exactly the physical-px numbers the styles asked
/// for.
pub fn layout(area: PxRect, root: &Elem, measurer: &mut impl TextMeasurer) -> LayoutScene {
    let mut tree: TaffyTree<NodeCtx> = TaffyTree::new();
    tree.disable_rounding();

    let root_key = ElementKey::default();
    let root_style = TaffyStyle {
        size: Size {
            width: length(area.w),
            height: length(area.h),
        },
        position: Position::Relative,
        ..Default::default()
    };
    let root_node = tree
        .new_with_children(root_style, &[])
        .expect("taffy root creation cannot fail");

    let mut scene_root = SceneNode {
        node: root_node,
        ctx: NodeCtx {
            key: root_key.clone(),
            kind: NodeKind::Container,
            parent_key: root_key.clone(),
        },
        style: root.style.clone(),
        children: Vec::new(),
    };
    let root_fg = root.style.fg;
    build_children(&mut tree, root, &mut scene_root, &root_key, root_fg);

    tree.compute_layout_with_measure(
        root_node,
        Size {
            width: AvailableSpace::Definite(area.w),
            height: AvailableSpace::Definite(area.h),
        },
        |_known, _available, _node, ctx, _style| match ctx {
            Some(NodeCtx {
                kind:
                    NodeKind::Text {
                        line,
                        font_size,
                        family,
                    },
                ..
            }) => {
                let (width, height) = measurer.measure(line, *font_size, *family);
                Size { width, height }
            }
            _ => Size::ZERO,
        },
    )
    .expect("compute_layout_with_measure cannot fail on the tree we just built");

    let mut scene = LayoutScene::default();
    read_back(
        &scene_root,
        Point {
            x: area.x,
            y: area.y,
        },
        &tree,
        root_fg,
        &root_key,
        &mut scene,
    );
    scene
}

/// Build `parent`'s children into both the taffy tree and the mirror. Each
/// child's key is derived from its `.id()` or its sibling index, and `parent_fg`
/// (the parent's effective fg) is threaded down so text can inherit it.
fn build_children(
    tree: &mut TaffyTree<NodeCtx>,
    parent: &Elem,
    parent_mirror: &mut SceneNode,
    parent_key: &ElementKey,
    parent_fg: Option<Color>,
) {
    let ElemKind::Container { children } = &parent.kind else {
        unreachable!("a text leaf has no children to build")
    };
    let mut child_nodes = Vec::with_capacity(children.len());
    push_children(
        tree,
        children,
        parent_key,
        parent.style.fg.or(parent_fg),
        &mut child_nodes,
    );

    let ids: Vec<NodeId> = child_nodes.iter().map(|n| n.node).collect();
    tree.set_children(parent_mirror.node, &ids)
        .expect("set_children cannot fail");
    parent_mirror.children = child_nodes;
}

/// Build one element as a taffy node plus its mirror subtree.
fn build_node(
    tree: &mut TaffyTree<NodeCtx>,
    elem: &Elem,
    key: &ElementKey,
    parent_key: &ElementKey,
    parent_fg: Option<Color>,
) -> SceneNode {
    let inherited_fg = elem.style.fg.or(parent_fg);
    match &elem.kind {
        ElemKind::Container { children } => {
            let mut child_nodes = Vec::with_capacity(children.len());
            push_children(tree, children, key, inherited_fg, &mut child_nodes);
            let ids: Vec<NodeId> = child_nodes.iter().map(|n| n.node).collect();
            let node = tree
                .new_with_children(elem.style.taffy.clone(), &ids)
                .expect("taffy container creation cannot fail");
            SceneNode {
                node,
                ctx: NodeCtx {
                    key: key.clone(),
                    kind: NodeKind::Container,
                    parent_key: parent_key.clone(),
                },
                style: elem.style.clone(),
                children: child_nodes,
            }
        }
        ElemKind::Text { line } => {
            let node = tree
                .new_leaf_with_context(
                    elem.style.taffy.clone(),
                    NodeCtx {
                        key: key.clone(),
                        kind: NodeKind::Text {
                            line: line.clone(),
                            font_size: elem.style.font_size,
                            family: elem.style.font_family,
                        },
                        parent_key: parent_key.clone(),
                    },
                )
                .expect("taffy text-leaf creation cannot fail");
            SceneNode {
                node,
                ctx: NodeCtx {
                    key: key.clone(),
                    kind: NodeKind::Text {
                        line: line.clone(),
                        font_size: elem.style.font_size,
                        family: elem.style.font_family,
                    },
                    parent_key: parent_key.clone(),
                },
                style: elem.style.clone(),
                children: Vec::new(),
            }
        }
    }
}

/// Derive each child's key (`.id()` or sibling index) and build it into `out`.
/// Duplicate `.id()`s under one parent are a GPUI contract violation: a
/// `debug_assert!` catches them in debug builds; in release the later sibling
/// simply shares the earlier one's key.
fn push_children(
    tree: &mut TaffyTree<NodeCtx>,
    children: &[Elem],
    parent_key: &ElementKey,
    inherited_fg: Option<Color>,
    out: &mut Vec<SceneNode>,
) {
    let mut seen: HashSet<&str> = HashSet::new();
    for (index, child) in children.iter().enumerate() {
        let key = child.key(parent_key, index);
        if let Some(id) = child.style.id.as_deref() {
            debug_assert!(
                seen.insert(id),
                "duplicate element id {id:?} under one parent: later siblings shadow the earlier one"
            );
        }
        out.push(build_node(tree, child, &key, parent_key, inherited_fg));
    }
}

/// Walk the mirror, accumulating each node's absolute origin from its parent and
/// emitting containers (bg/border) before their children, and text leaves among
/// them. `parent_fg` is the effective fg a text leaf inherits when it has no
/// explicit color of its own; `parent_key` is the parent's key, cross-checked
/// against each node's recorded `parent_key`.
fn read_back(
    node: &SceneNode,
    origin: Point<f32>,
    tree: &TaffyTree<NodeCtx>,
    parent_fg: Option<Color>,
    parent_key: &ElementKey,
    scene: &mut LayoutScene,
) {
    let layout = tree
        .layout(node.node)
        .expect("layout for a node we just computed");
    let abs = Point {
        x: origin.x + layout.location.x,
        y: origin.y + layout.location.y,
    };
    let rect = PxRect {
        x: abs.x,
        y: abs.y,
        w: layout.size.width,
        h: layout.size.height,
    };
    let effective_fg = node.style.fg.or(parent_fg);
    debug_assert_eq!(
        node.ctx.parent_key, *parent_key,
        "mirror key parent must match the walk's derivation"
    );

    if node.style.bg.is_some() || node.style.border_width > 0.0 {
        // A box with no background but a border still paints its border; its
        // `fill` is then ignored by the tessellator, so white is a harmless
        // placeholder.
        scene.boxes.push(BoxNode {
            rect,
            radius: node.style.radius,
            fill: node
                .style
                .bg
                .map(Into::into)
                .unwrap_or((1.0, 1.0, 1.0, 1.0)),
            border_width: node.style.border_width,
            border_color: node.style.border_color.into(),
            key: node.ctx.key.clone(),
        });
    }

    if let NodeKind::Text {
        line,
        font_size,
        family,
    } = &node.ctx.kind
    {
        scene.texts.push(TextNode {
            rect,
            line: line.clone(),
            font_size: *font_size,
            family: *family,
            fg: effective_fg.unwrap_or(Color::Default).into(),
            bold: node.style.bold,
            key: node.ctx.key.clone(),
        });
    }

    for child in &node.children {
        read_back(child, abs, tree, effective_fg, &node.ctx.key, scene);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{div, text, Elem, ElemKind, Styled};
    use crate::id::ElementKey;
    use ruster_render::{Color, FontFamily, StyledLine};

    /// Deterministic stand-in for a real text shaper: `len * 8` wide (len = char
    /// count, not bytes — the same index the draw paths use), `font_size + 4` tall.
    struct MockMeasurer;

    impl TextMeasurer for MockMeasurer {
        fn measure(
            &mut self,
            line: &StyledLine,
            font_size: f32,
            _family: FontFamily,
        ) -> (f32, f32) {
            (line.text.chars().count() as f32 * 8.0, font_size + 4.0)
        }
    }

    /// Build an owned `div()` and run `f` over it. The builder chain returns
    /// `&mut Elem`, which cannot be moved out of, so this reconstructs an owned
    /// element from the mutated one.
    fn build(f: impl FnOnce(&mut Elem)) -> Elem {
        let mut e = div();
        f(&mut e);
        e
    }

    /// Build an owned `text()` line and run `f` over it.
    fn txt(line: &str, f: impl FnOnce(&mut Elem)) -> Elem {
        let mut t = text(line);
        f(&mut t);
        t
    }

    fn layout_at(root: &Elem, area: PxRect) -> LayoutScene {
        let mut m = MockMeasurer;
        layout(area, root, &mut m)
    }

    fn find_box<'a>(scene: &'a LayoutScene, key: &ElementKey) -> &'a BoxNode {
        scene
            .boxes
            .iter()
            .find(|b| &b.key == key)
            .unwrap_or_else(|| panic!("no box with key {key:?}"))
    }

    fn find_text<'a>(scene: &'a LayoutScene, key: &ElementKey) -> &'a TextNode {
        scene
            .texts
            .iter()
            .find(|t| &t.key == key)
            .unwrap_or_else(|| panic!("no text with key {key:?}"))
    }

    /// The statusline shape Task 5's `statusline_elem` will build: a bar absolute
    /// at the bottom, an absolute mode box, and ws/indicator/title in-flow.
    fn statusline(w: f32, h: f32) -> Elem {
        let bar_h = 40.0;
        let pad = 12.0;
        let mode = build(|m| {
            m.id("mode")
                .absolute()
                .position(0.0, 0.0)
                .size(64.0, bar_h)
                .bg(Color::Rgb(69, 71, 90));
            m.padding_left(24.0).padding_top(pad);
            m.children(vec![txt("N", |t| {
                t.id("n").font_size(16.0);
            })]);
        });
        let bar = build(|b| {
            b.id("bar")
                .absolute()
                .position(0.0, h - bar_h)
                .size(w, bar_h)
                .bg(Color::Rgb(30, 30, 30));
            b.padding_left(76.0).padding_top(pad).gap(20.0);
            b.children(vec![
                mode,
                txt("ws", |t| {
                    t.id("ws").font_size(16.0);
                }),
                txt("M", |t| {
                    t.id("ind").font_size(16.0);
                }),
                txt("hello", |t| {
                    t.id("title").font_size(16.0);
                }),
            ]);
        });
        let mut root = div();
        root.children(vec![bar]);
        root
    }

    #[test]
    fn statusline_bar_mode_and_children_land_at_the_hardcoded_numbers() {
        let (w, h) = (800.0, 600.0);
        let bar_h = 40.0;
        let pad = 12.0;
        let scene = layout_at(
            &statusline(w, h),
            PxRect {
                x: 0.0,
                y: 0.0,
                w,
                h,
            },
        );

        let bar = find_box(&scene, &ElementKey(vec!["bar".into()]));
        assert_eq!(
            bar.rect,
            PxRect {
                x: 0.0,
                y: h - bar_h,
                w,
                h: bar_h
            }
        );

        let mode = find_box(&scene, &ElementKey(vec!["bar".into(), "mode".into()]));
        assert_eq!(
            mode.rect,
            PxRect {
                x: 0.0,
                y: h - bar_h,
                w: 64.0,
                h: bar_h
            }
        );

        let bar_key = ElementKey(vec!["bar".into()]);
        let ys = h - bar_h + pad;
        let n = find_text(
            &scene,
            &ElementKey(vec!["bar".into(), "mode".into(), "n".into()]),
        );
        assert_eq!(
            n.rect,
            PxRect {
                x: 24.0,
                y: ys,
                w: 8.0,
                h: 28.0
            }
        );
        let ws = find_text(&scene, &bar_key.child("ws"));
        assert_eq!(
            ws.rect,
            PxRect {
                x: 76.0,
                y: ys,
                w: 16.0,
                h: 28.0
            }
        );
        let ind = find_text(&scene, &bar_key.child("ind"));
        assert_eq!(
            ind.rect,
            PxRect {
                x: 112.0,
                y: ys,
                w: 8.0,
                h: 28.0
            }
        );
        let title = find_text(&scene, &bar_key.child("title"));
        assert_eq!(
            title.rect,
            PxRect {
                x: 140.0,
                y: ys,
                w: 40.0,
                h: 28.0
            }
        );
    }

    #[test]
    fn which_key_panel_is_sized_from_its_column_chunks() {
        let col1 = build(|c| {
            c.id("col1").flex_col().gap(4.0);
            c.bg(Color::Rgb(50, 50, 50));
            let r1 = build(|r| {
                r.id("r1").flex_row().gap(8.0).h(20.0);
                r.bg(Color::Rgb(60, 60, 60));
                r.children(vec![
                    txt("a", |t| {
                        t.id("k1");
                    }),
                    txt("first", |t| {
                        t.id("d1");
                    }),
                ]);
            });
            let r2 = build(|r| {
                r.id("r2").flex_row().gap(8.0).h(20.0);
                r.children(vec![
                    txt("bb", |t| {
                        t.id("k2");
                    }),
                    txt("second", |t| {
                        t.id("d2");
                    }),
                ]);
            });
            c.children(vec![r1, r2]);
        });
        let col2 = build(|c| {
            c.id("col2").flex_col().gap(4.0);
            let r3 = build(|r| {
                r.id("r3").flex_row().gap(8.0).h(20.0);
                r.children(vec![
                    txt("c", |t| {
                        t.id("k3");
                    }),
                    txt("x", |t| {
                        t.id("d3");
                    }),
                ]);
            });
            let r4 = build(|r| {
                r.id("r4").flex_row().gap(8.0).h(20.0);
                r.children(vec![
                    txt("dd", |t| {
                        t.id("k4");
                    }),
                    txt("y", |t| {
                        t.id("d4");
                    }),
                ]);
            });
            c.children(vec![r3, r4]);
        });
        let panel = build(|p| {
            p.id("panel")
                .absolute()
                .position(12.0, 12.0)
                .flex_col()
                .gap(8.0)
                .padding(8.0);
            p.bg(Color::Rgb(30, 30, 30));
            let cols = build(|cs| {
                cs.id("cols").flex_row().gap(8.0);
                cs.bg(Color::Rgb(40, 40, 40));
                cs.children(vec![col1, col2]);
            });
            p.children(vec![
                txt("Commands", |t| {
                    t.id("title");
                }),
                cols,
            ]);
        });
        let mut root = div();
        root.children(vec![panel]);
        let scene = layout_at(
            &root,
            PxRect {
                x: 0.0,
                y: 0.0,
                w: 400.0,
                h: 300.0,
            },
        );

        let panel = find_box(&scene, &ElementKey(vec!["panel".into()]));
        assert_eq!(
            panel.rect,
            PxRect {
                x: 12.0,
                y: 12.0,
                w: 128.0,
                h: 86.0
            }
        );

        let cols = find_box(&scene, &ElementKey(vec!["panel".into(), "cols".into()]));
        assert_eq!(
            cols.rect,
            PxRect {
                x: 20.0,
                y: 46.0,
                w: 112.0,
                h: 44.0
            }
        );

        let col1 = find_box(
            &scene,
            &ElementKey(vec!["panel".into(), "cols".into(), "col1".into()]),
        );
        assert_eq!(
            col1.rect,
            PxRect {
                x: 20.0,
                y: 46.0,
                w: 72.0,
                h: 44.0
            },
            "a column's width is the max over its rows of key_w + 8 + desc_w"
        );

        let r1 = find_box(
            &scene,
            &ElementKey(vec![
                "panel".into(),
                "cols".into(),
                "col1".into(),
                "r1".into(),
            ]),
        );
        // The row's content is 56 wide, but align-items stretch fills it to the
        // column's 72 — a real flexbox cross-axis finding, not a bug.
        assert_eq!(
            r1.rect,
            PxRect {
                x: 20.0,
                y: 46.0,
                w: 72.0,
                h: 20.0
            }
        );

        let d2 = find_text(
            &scene,
            &ElementKey(vec![
                "panel".into(),
                "cols".into(),
                "col1".into(),
                "r2".into(),
                "d2".into(),
            ]),
        );
        assert_eq!(
            d2.rect,
            PxRect {
                x: 44.0,
                y: 70.0,
                w: 48.0,
                h: 20.0
            }
        );

        let k3 = find_text(
            &scene,
            &ElementKey(vec![
                "panel".into(),
                "cols".into(),
                "col2".into(),
                "r3".into(),
                "k3".into(),
            ]),
        );
        assert_eq!(
            k3.rect,
            PxRect {
                x: 100.0,
                y: 46.0,
                w: 8.0,
                h: 20.0
            }
        );
    }

    #[test]
    fn hover_panel_below_above_and_x_clamped() {
        // w = (text_w + 16).min(output_w - 8); text_w = "hover me" = 8 chars = 64
        let w: f32 = (64.0_f32 + 16.0).min(300.0 - 8.0);
        let h: f32 = 16.0 + 2.0 * 18.0;
        let anchor = PxRect {
            x: 100.0,
            y: 200.0,
            w: 50.0,
            h: 30.0,
        };
        let clamp = |x: f32| x.min(300.0 - w);

        let make = |x: f32, y: f32| {
            let panel = build(|p| {
                p.id("panel")
                    .absolute()
                    .position(x, y)
                    .flex_col()
                    .size(w, h);
                p.bg(Color::Rgb(30, 30, 30));
                p.children(vec![
                    txt("line1", |t| {
                        t.id("l1").h(18.0);
                    }),
                    txt("line2", |t| {
                        t.id("l2").h(18.0);
                    }),
                ]);
            });
            let mut root = div();
            root.children(vec![panel]);
            root
        };
        let scene = |root: &Elem| {
            layout_at(
                root,
                PxRect {
                    x: 0.0,
                    y: 0.0,
                    w: 300.0,
                    h: 200.0,
                },
            )
        };

        // Anchor below the target: panel sits at the anchor's bottom edge.
        let below = scene(&make(clamp(anchor.x), anchor.y + anchor.h));
        let panel = find_box(&below, &ElementKey(vec!["panel".into()]));
        assert_eq!(
            panel.rect,
            PxRect {
                x: 100.0,
                y: 230.0,
                w,
                h
            }
        );
        assert_eq!(
            find_text(&below, &ElementKey(vec!["panel".into(), "l1".into()])).rect,
            PxRect {
                x: 100.0,
                y: 230.0,
                w,
                h: 18.0
            }
        );
        assert_eq!(
            find_text(&below, &ElementKey(vec!["panel".into(), "l2".into()])).rect,
            PxRect {
                x: 100.0,
                y: 248.0,
                w,
                h: 18.0
            }
        );

        // Anchor above the target: panel flips to sit above the anchor, and an
        // x that would overflow the output is clamped back.
        let above = scene(&make(clamp(290.0), anchor.y - h));
        let panel = find_box(&above, &ElementKey(vec!["panel".into()]));
        assert_eq!(
            panel.rect,
            PxRect {
                x: 220.0,
                y: 148.0,
                w,
                h
            }
        );
    }

    #[test]
    fn pane_body_runs_land_at_glyph_origins() {
        const FRAME_PAD: f32 = 6.0;
        const FRAME_BAR_H: f32 = 28.0;
        const SIGN_COLS: f32 = 1.0;
        const CELL_W: f32 = 8.0;
        let (w, h) = (500.0, 300.0);
        let body_x = FRAME_PAD + SIGN_COLS * CELL_W;
        let body_y = FRAME_BAR_H + FRAME_PAD;
        let col = 2.0;

        let body = build(|b| {
            b.id("body")
                .absolute()
                .position(body_x, body_y)
                .size(w - body_x, h - body_y);
            b.bg(Color::Rgb(10, 10, 10))
                .padding_left(col * CELL_W)
                .padding_top(0.0);
            b.children(vec![txt("fn main()", |t| {
                t.id("run").font_size(16.0).h(20.0);
            })]);
        });
        let titlebar = build(|t| {
            t.id("titlebar")
                .absolute()
                .position(0.0, 0.0)
                .size(w, FRAME_BAR_H)
                .bg(Color::Rgb(40, 40, 40));
        });
        let mut root = div();
        root.children(vec![titlebar, body]);
        let scene = layout_at(
            &root,
            PxRect {
                x: 0.0,
                y: 0.0,
                w,
                h,
            },
        );

        let titlebar = find_box(&scene, &ElementKey(vec!["titlebar".into()]));
        assert_eq!(
            titlebar.rect,
            PxRect {
                x: 0.0,
                y: 0.0,
                w,
                h: FRAME_BAR_H
            }
        );

        let body = find_box(&scene, &ElementKey(vec!["body".into()]));
        assert_eq!(
            body.rect,
            PxRect {
                x: body_x,
                y: body_y,
                w: w - body_x,
                h: h - body_y
            }
        );

        let run = find_text(&scene, &ElementKey(vec!["body".into(), "run".into()]));
        assert_eq!(
            run.rect,
            PxRect {
                x: body_x + col * CELL_W,
                y: body_y,
                w: 72.0,
                h: 20.0
            }
        );
    }

    #[test]
    fn absolute_positioning_does_not_disturb_child_geometry() {
        let content = |into: &mut Elem| {
            into.children(vec![build(|b| {
                b.id("bar")
                    .size(200.0, 40.0)
                    .bg(Color::Rgb(1, 2, 3))
                    .padding_left(8.0);
                b.children(vec![txt("hi", |t| {
                    t.id("label").font_size(16.0);
                })]);
            })]);
        };

        // Same subtree in-flow inside a viewport offset to (14, 34)...
        let mut flow_root = div();
        content(&mut flow_root);
        let flow_scene = layout_at(
            &flow_root,
            PxRect {
                x: 14.0,
                y: 34.0,
                w: 200.0,
                h: 40.0,
            },
        );

        // ...vs absolute at (14, 34) inside an un-offset viewport.
        let mut abs_root = div();
        abs_root.children(vec![build(|w| {
            w.id("wrap")
                .absolute()
                .position(14.0, 34.0)
                .size(200.0, 40.0);
            content(w);
        })]);
        let abs_scene = layout_at(
            &abs_root,
            PxRect {
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 40.0,
            },
        );

        let box_rects =
            |s: &LayoutScene| -> Vec<PxRect> { s.boxes.iter().map(|b| b.rect).collect() };
        assert_eq!(
            box_rects(&abs_scene),
            box_rects(&flow_scene),
            "absolute wrapper must not move children"
        );
        let text_rects = |s: &LayoutScene| -> Vec<(PxRect, String)> {
            s.texts
                .iter()
                .map(|t| (t.rect, t.line.text.clone()))
                .collect()
        };
        assert_eq!(
            text_rects(&abs_scene),
            text_rects(&flow_scene),
            "absolute wrapper must not move children"
        );
    }

    #[test]
    fn reordering_siblings_remaps_their_keys_at_layout_level() {
        let mut root = div();
        root.children(vec![
            txt("a", |t| {
                t.id("a").font_size(16.0);
            }),
            txt("b", |t| {
                t.id("b").font_size(16.0);
            }),
        ]);
        let keys = |root: &Elem| -> Vec<ElementKey> {
            layout_at(
                root,
                PxRect {
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 50.0,
                },
            )
            .texts
            .into_iter()
            .map(|t| t.key)
            .collect()
        };

        assert_eq!(
            keys(&root),
            vec![ElementKey(vec!["a".into()]), ElementKey(vec!["b".into()])]
        );

        let (second, first) = match &mut root.kind {
            ElemKind::Container { children } => {
                let second = children.remove(1);
                let first = children.remove(0);
                (second, first)
            }
            ElemKind::Text { .. } => unreachable!(),
        };
        root.children(vec![second, first]);

        assert_eq!(
            keys(&root),
            vec![ElementKey(vec!["b".into()]), ElementKey(vec!["a".into()])]
        );
    }
}
