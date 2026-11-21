//! 所有菜单共用的一套规格（用户 2026-11-14）：行高 24、分隔线通边、白色百分比墨色。
//!
//! 画布右键菜单、工具栏飞出菜单、Actions palette、主菜单、下拉菜单都读这里，不再各自写
//! 28/32/8/9 或 `t.border` / `t.menu_text`。卡片左右各有 [`MENU_PAD`] 的内边距，所以分隔线
//! 必须从内容区左边缘往左挪一个 `MENU_PAD`、再按**卡片完整宽度**画，才能真正通到两边。

use crate::canvas::geom::Color32;
use crate::widgets::{pt, rgba};
use gpui::{IntoElement, Rgba, div, prelude::*};

/// Every menu row, in points.
pub(crate) const MENU_ROW_H: f32 = 24.0;
/// A separator's total height: 4 pt above + 1 pt rule + 4 pt below.
pub(crate) const MENU_SEP_H: f32 = 9.0;
/// The card's inset; the divider cancels it with a negative margin.
pub(crate) const MENU_PAD: f32 = 8.0;
/// Every leading glyph (check / icon) owns a 24 pt grid slot; adjacent slots have no gap.
pub(crate) const MENU_SLOT_W: f32 = 24.0;

fn white(pct: f32) -> Rgba {
    rgba(Color32::from_white_alpha((255.0 * pct).round() as u8))
}

// Every menu is the dark surface (`t.menu`), so its ink is always white — including the actions
// palette, which used to sit on a light card and read dark (user 2026-11-18: one surface only).

/// Labels, check marks and leading icons: white 90 %.
pub(crate) fn ink() -> Rgba {
    white(0.9)
}

/// Trailing shortcut hints and the submenu chevron: white 70 %.
pub(crate) fn trail_ink() -> Rgba {
    white(0.7)
}

/// Dividers: white 10 %.
pub(crate) fn divider_ink() -> Rgba {
    white(0.1)
}

/// A 1 pt rule that reaches the card's left and right edges. `card_w` is the **full** card
/// width (including both paddings); the line starts one `MENU_PAD` to the left of the content
/// box, so a parent padding can never leave a gap at either end.
pub(crate) fn divider(card_w: f32) -> impl IntoElement {
    div()
        .h(pt(MENU_SEP_H))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .child(div().ml(pt(-MENU_PAD)).w(pt(card_w)).h(pt(1.0)).flex_none().bg(divider_ink()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ink_lives_only_in_the_shared_surface() {
        // 所有菜单行的默认文字颜色必须由通用 `menu_surface()` 一处注入：
        // 主菜单 / 下拉 / 工具栏飞出 / 右键 / palette 都继承它。调用处再写一次
        // 就会出现「三种菜单三种亮度」——2026-11-19 的回归就是这个原因。
        fn code(src: &str) -> String {
            src.lines().filter(|l| !l.trim_start().starts_with("//")).collect()
        }
        let surface = code(include_str!("widgets/menu_popup.rs"));
        assert!(surface.contains("fn menu_surface"));
        assert!(surface.contains("text_color(crate::menu_metrics::ink())"), "统一墨色必须写在通用 menu_surface() 内");

        for (name, src) in [
            ("menu_card", include_str!("menu_card.rs")),
            ("main_menu", include_str!("main_menu.rs")),
            ("widgets/menu", include_str!("widgets/menu.rs")),
        ] {
            let src = code(src);
            assert!(!src.contains("t.menu_text"), "{name} 不能自己挑文字颜色");
            assert!(!src.contains("t.menu_dim"), "{name} 不能自己挑图标颜色");
            assert!(!src.contains("text_color(crate::menu_metrics::ink())"), "{name} 不要再设默认墨色，继承通用控件");
        }

        let toolbar = include_str!("toolbar.rs");
        let more = toolbar
            .split("fn vector_more_card")
            .nth(1)
            .unwrap_or_default()
            .split("\nfn ")
            .next()
            .unwrap_or_default()
            .split("\npub(crate) struct ")
            .next()
            .unwrap_or_default();
        let more = code(more);
        assert!(!more.contains("t.menu_text"), "工具栏飞出菜单不能自己挑文字颜色");
        assert!(!more.contains("t.menu_dim"), "工具栏飞出菜单不能自己挑快捷键颜色");
        assert!(!more.contains("text_color(crate::menu_metrics::ink())"), "工具栏飞出菜单要继承通用控件墨色");
    }

    #[test]
    fn every_menu_row_is_24_and_the_palette_is_white_percentages() {
        assert_eq!(MENU_ROW_H, 24.0);
        assert_eq!(MENU_SEP_H, 9.0);
        // 卡片的水平内边距必须留在 `menu_surface()` 上：选中/hover 高亮画在整个 w_full
        // 行盒上，把它挪到行内会让色块顶到卡片左右边缘（用户 2026-11-14 的回归）。
        assert_eq!(MENU_PAD, 8.0);
        assert_eq!(MENU_SLOT_W, 24.0, "check and icon slots are 24 pt and touch");
        assert!((ink().a - 0.9).abs() < 0.01);
        assert!((trail_ink().a - 0.7).abs() < 0.01);
        assert!((divider_ink().a - 0.1).abs() < 0.01);
        assert_eq!(ink().r, 1.0);
    }
}
