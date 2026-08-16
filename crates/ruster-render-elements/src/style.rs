use ruster_render::{Color, FontFamily};
use taffy::prelude::length;
use taffy::{AlignItems, FlexDirection, FlexWrap, JustifyContent, Position, Rect, Size};

/// The full style of one element: a taffy flexbox `Style` for layout, plus the
/// visual fields taffy does not know about (color, radius, border, typography).
/// `id` is the element's key segment, appended to its parent key during layout.
/// `Clone` powers the layout walk's mirror tree (Task 3).
#[derive(Clone)]
pub struct Style {
    /// Layout in taffy's terms.
    pub taffy: taffy::Style,
    /// Background fill. `None` draws nothing (transparent).
    pub bg: Option<Color>,
    /// Text / foreground color. `None` inherits from the parent container.
    pub fg: Option<Color>,
    /// Corner radius in physical px (used for `rect_verts` vs `rounded_rect_verts`).
    pub radius: f32,
    /// Border width in px; `0.0` draws no border.
    pub border_width: f32,
    /// Border color when `border_width > 0`.
    pub border_color: Color,
    /// Text size in px for this element's text leaves.
    pub font_size: f32,
    /// Which face text is shaped with.
    pub font_family: FontFamily,
    /// Emphasized text weight.
    pub bold: bool,
    /// This element's key segment; derived by the layout walk when absent.
    pub id: Option<String>,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            taffy: taffy::Style::default(),
            bg: None,
            fg: None,
            radius: 0.0,
            border_width: 0.0,
            border_color: Color::Default,
            font_size: 14.0,
            font_family: FontFamily::Ui,
            bold: false,
            id: None,
        }
    }
}

/// Chained builder for an element's [`Style`]. Every method mutates the style
/// and returns `&mut Self`, so calls chain up to the point where children are
/// attached (`children()` consumes the element's value) or the tree is read.
///
/// `fn style` is the accessor the concrete element type delegates to; its
/// `#[allow(clippy::should_implement_trait)]` silences clippy misreading the
/// literal name against a std trait it is not implementing.
pub trait Styled {
    /// The element's mutable style.
    #[allow(clippy::should_implement_trait)]
    fn style(&mut self) -> &mut Style;

    fn flex_col(&mut self) -> &mut Self {
        self.style().taffy.flex_direction = FlexDirection::Column;
        self
    }

    fn flex_row(&mut self) -> &mut Self {
        self.style().taffy.flex_direction = FlexDirection::Row;
        self
    }

    fn flex_wrap(&mut self) -> &mut Self {
        self.style().taffy.flex_wrap = FlexWrap::Wrap;
        self
    }

    fn flex_grow(&mut self, amount: f32) -> &mut Self {
        self.style().taffy.flex_grow = amount;
        self
    }

    fn flex_shrink(&mut self, amount: f32) -> &mut Self {
        self.style().taffy.flex_shrink = amount;
        self
    }

    fn size(&mut self, w: f32, h: f32) -> &mut Self {
        self.style().taffy.size = Size {
            width: length(w),
            height: length(h),
        };
        self
    }

    fn w(&mut self, width: f32) -> &mut Self {
        self.style().taffy.size.width = length(width);
        self
    }

    fn h(&mut self, height: f32) -> &mut Self {
        self.style().taffy.size.height = length(height);
        self
    }

    fn min_w_0(&mut self) -> &mut Self {
        self.style().taffy.min_size.width = length(0.0);
        self
    }

    /// Clamp this element's width to `width`, keeping the height free.
    ///
    /// The sibling of [`min_w_0`](Self::min_w_0): an absolute panel that would
    /// otherwise size to its content gets capped, matching the `min()`
    /// clamping the chrome's hand-built draw methods applied to their panels.
    fn max_w(&mut self, width: f32) -> &mut Self {
        self.style().taffy.max_size.width = length(width);
        self
    }

    fn gap(&mut self, gap: f32) -> &mut Self {
        self.style().taffy.gap = Size {
            width: length(gap),
            height: length(gap),
        };
        self
    }

    fn padding(&mut self, p: f32) -> &mut Self {
        self.style().taffy.padding = Rect {
            left: length(p),
            right: length(p),
            top: length(p),
            bottom: length(p),
        };
        self
    }

    fn padding_x(&mut self, p: f32) -> &mut Self {
        self.style().taffy.padding.left = length(p);
        self.style().taffy.padding.right = length(p);
        self
    }

    fn padding_y(&mut self, p: f32) -> &mut Self {
        self.style().taffy.padding.top = length(p);
        self.style().taffy.padding.bottom = length(p);
        self
    }

    fn padding_left(&mut self, p: f32) -> &mut Self {
        self.style().taffy.padding.left = length(p);
        self
    }

    fn padding_right(&mut self, p: f32) -> &mut Self {
        self.style().taffy.padding.right = length(p);
        self
    }

    fn padding_top(&mut self, p: f32) -> &mut Self {
        self.style().taffy.padding.top = length(p);
        self
    }

    fn padding_bottom(&mut self, p: f32) -> &mut Self {
        self.style().taffy.padding.bottom = length(p);
        self
    }

    fn justify_center(&mut self) -> &mut Self {
        self.style().taffy.justify_content = Some(JustifyContent::CENTER);
        self
    }

    fn items_center(&mut self) -> &mut Self {
        self.style().taffy.align_items = Some(AlignItems::CENTER);
        self
    }

    fn absolute(&mut self) -> &mut Self {
        self.style().taffy.position = Position::Absolute;
        self
    }

    fn position(&mut self, x: f32, y: f32) -> &mut Self {
        self.style().taffy.inset.left = length(x);
        self.style().taffy.inset.top = length(y);
        self
    }

    fn bg(&mut self, color: Color) -> &mut Self {
        self.style().bg = Some(color);
        self
    }

    fn fg(&mut self, color: Color) -> &mut Self {
        self.style().fg = Some(color);
        self
    }

    fn radius(&mut self, radius: f32) -> &mut Self {
        self.style().radius = radius;
        self
    }

    fn border_1(&mut self) -> &mut Self {
        self.style().border_width = 1.0;
        self
    }

    fn border_color(&mut self, color: Color) -> &mut Self {
        self.style().border_color = color;
        self
    }

    fn font_size(&mut self, size: f32) -> &mut Self {
        self.style().font_size = size;
        self
    }

    fn font_family(&mut self, family: FontFamily) -> &mut Self {
        self.style().font_family = family;
        self
    }

    fn bold(&mut self) -> &mut Self {
        self.style().bold = true;
        self
    }

    fn id(&mut self, id: &str) -> &mut Self {
        self.style().id = Some(id.to_string());
        self
    }

    /// Escape hatch: run `f` over the raw taffy `Style` for any taffy field
    /// without a named setter above (e.g. `max_size`). A plain `fn` pointer so
    /// it never captures.
    fn taffy_style(&mut self, f: fn(&mut taffy::Style)) -> &mut Self {
        f(&mut self.style().taffy);
        self
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::div;
    use taffy::prelude::{
        Dimension, LengthPercentage, LengthPercentageAuto, Position as TaffyPosition,
    };

    #[test]
    fn size_sets_the_taffy_size_in_physical_px() {
        let mut e = div();
        e.size(100.0, 200.0);
        assert_eq!(
            e.style().taffy.size,
            Size {
                width: Dimension::length(100.0),
                height: Dimension::length(200.0),
            }
        );
    }

    #[test]
    fn w_and_h_set_each_axis_independently() {
        let mut e = div();
        e.w(80.0).h(24.0);
        assert_eq!(e.style().taffy.size.width, Dimension::length(80.0));
        assert_eq!(e.style().taffy.size.height, Dimension::length(24.0));
    }

    #[test]
    fn absolute_and_position_land_on_taffy_position_and_inset() {
        let mut e = div();
        e.absolute().position(5.0, 7.0);
        assert_eq!(e.style().taffy.position, TaffyPosition::Absolute);
        assert_eq!(
            e.style().taffy.inset.left,
            LengthPercentageAuto::length(5.0)
        );
        assert_eq!(e.style().taffy.inset.top, LengthPercentageAuto::length(7.0));
    }

    #[test]
    fn gap_sets_both_axes() {
        let mut e = div();
        e.gap(8.0);
        assert_eq!(
            e.style().taffy.gap,
            Size {
                width: LengthPercentage::length(8.0),
                height: LengthPercentage::length(8.0),
            }
        );
    }

    #[test]
    fn padding_sets_all_four_sides() {
        let mut e = div();
        e.padding(4.0);
        let l = |p| LengthPercentage::length(p);
        assert_eq!(
            e.style().taffy.padding,
            Rect {
                left: l(4.0),
                right: l(4.0),
                top: l(4.0),
                bottom: l(4.0),
            }
        );
    }

    #[test]
    fn per_axis_and_per_side_padding_target_only_their_sides() {
        let mut e = div();
        e.padding_x(3.0).padding_y(5.0).padding_left(1.0);
        let p = e.style().taffy.padding;
        let l = |p| LengthPercentage::length(p);
        assert_eq!(p.left, l(1.0));
        assert_eq!(p.right, l(3.0));
        assert_eq!(p.top, l(5.0));
        assert_eq!(p.bottom, l(5.0));
    }

    #[test]
    fn flex_helpers_set_direction_wrap_grow_shrink() {
        let mut e = div();
        e.flex_col().flex_wrap().flex_grow(1.0).flex_shrink(2.0);
        assert_eq!(e.style().taffy.flex_direction, FlexDirection::Column);
        assert_eq!(e.style().taffy.flex_wrap, FlexWrap::Wrap);
        assert_eq!(e.style().taffy.flex_grow, 1.0);
        assert_eq!(e.style().taffy.flex_shrink, 2.0);

        e.flex_row();
        assert_eq!(e.style().taffy.flex_direction, FlexDirection::Row);
    }

    #[test]
    fn justify_and_items_center_set_the_flex_alignment() {
        let mut e = div();
        e.justify_center().items_center();
        assert_eq!(
            e.style().taffy.justify_content,
            Some(JustifyContent::CENTER)
        );
        assert_eq!(e.style().taffy.align_items, Some(AlignItems::CENTER));
    }

    #[test]
    fn min_w_0_sets_the_min_width_to_zero() {
        let mut e = div();
        e.min_w_0();
        assert_eq!(e.style().taffy.min_size.width, Dimension::length(0.0));
    }

    #[test]
    fn visual_fields_land_on_style_not_taffy() {
        let mut e = div();
        e.bg(Color::Rgb(1, 2, 3))
            .fg(Color::Rgb(4, 5, 6))
            .radius(6.0)
            .border_1()
            .border_color(Color::Rgb(7, 8, 9))
            .font_size(16.0)
            .font_family(FontFamily::Mono)
            .bold();
        let s = e.style();
        assert_eq!(s.bg, Some(Color::Rgb(1, 2, 3)));
        assert_eq!(s.fg, Some(Color::Rgb(4, 5, 6)));
        assert_eq!(s.radius, 6.0);
        assert_eq!(s.border_width, 1.0);
        assert_eq!(s.border_color, Color::Rgb(7, 8, 9));
        assert_eq!(s.font_size, 16.0);
        assert_eq!(s.font_family, FontFamily::Mono);
        assert!(s.bold);
    }

    #[test]
    fn id_lands_on_style_id() {
        let mut e = div();
        assert_eq!(e.style().id, None);
        e.id("pane");
        assert_eq!(e.style().id, Some("pane".to_string()));
    }

    #[test]
    fn taffy_style_runs_the_closure_over_the_raw_taffy_style() {
        let mut e = div();
        e.taffy_style(|s| {
            s.max_size = Size {
                width: Dimension::length(300.0),
                height: Dimension::length(200.0),
            };
        });
        assert_eq!(e.style().taffy.max_size.width, Dimension::length(300.0));
        assert_eq!(e.style().taffy.max_size.height, Dimension::length(200.0));
    }

    #[test]
    fn style_defaults() {
        let s = Style::default();
        assert_eq!(s.bg, None);
        assert_eq!(s.fg, None);
        assert_eq!(s.radius, 0.0);
        assert_eq!(s.border_width, 0.0);
        assert_eq!(s.border_color, Color::Default);
        assert_eq!(s.font_size, 14.0);
        assert_eq!(s.font_family, FontFamily::Ui);
        assert!(!s.bold);
        assert_eq!(s.id, None);
    }
}
