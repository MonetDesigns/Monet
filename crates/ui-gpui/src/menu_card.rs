//! A menu card placed at a **point** — the shared body of the canvas context menu and the bottom
//! toolbar's flyouts.
//!
//! `widgets/menu.rs::menu_popup` hangs a menu off a trigger's rect (a dropdown's field), and
//! `main_menu.rs` is the multi-level catalog with hover-expanded submenus; neither can put a card
//! at the pointer, and the flyout's rows carry a **tool icon** besides the check column, which
//! `MenuRow` has no field for. So this module owns the one row model both need:
//!
//! ```text
//! ┌ card ─ 圆角 13、内边距 8、t.menu 底、1pt t.border ─────────────┐
//! │  row (28 / 32 高, 圆角 5)                                     │
//! │  ✓ 28 列（有勾选态才留） │ 16pt 图标 │ 标签 ……… 快捷键 │ ▸   │
//! └───────────────────────────────────────────────────────────────┘
//! ```
//!
//! Geometry follows `ui-egui/src/menus.rs` (context menu: 28 pt rows) and
//! `ui-egui/src/toolbar.rs::flyout` (32 pt rows, check at 10, icon centred at 32, label from 48,
//! shortcut flush right 8). A row may open a submenu; the open one is supplied by the caller (it
//! is view state, so it survives the frame).

pub(crate) use crate::widgets::Width;
use crate::menu_rows;
use crate::widgets::icon::Icon;
use crate::widgets::{MENU_TRAIL_GAP, pt, rgba, tokens};
use gpui::prelude::*;
use gpui::{AnyElement, App, MouseButton, MouseDownEvent, SharedString, TextRun, Window, div};
use std::rc::Rc;

/// The card's inset (egui `menu_frame`: `Margin::same(8)`). The fill, the 13 pt corners and the
/// shadow come from [`crate::widgets::menu_surface`] — one definition for every popup.
const PAD: f32 = crate::menu_metrics::MENU_PAD;
/// A separator's height, shared by every menu: 4 + 1 + 4.
const SEP_H: f32 = crate::menu_metrics::MENU_SEP_H;

/// 一行菜单的模型：画布右键菜单、工具栏飞出菜单、Actions palette 共用。
/// （2026-11-06 恢复：该文件被并行编辑误删了这一段，而 `lib.rs` / `toolbar.rs` /
/// `context_menu.rs` 都仍按这套 API 调用。）
#[derive(Clone)]
pub struct Item {
    pub id: &'static str,
    /// The value handed to `OnPick` (defaults to `id`). Dynamic rows — an Actions palette
    /// component, style, plugin or widget — carry a runtime key here instead of a static id.
    pub key: SharedString,
    /// Distinguishes an explicitly-set key (even an empty one, e.g. `.key("")` to clear the
    /// brush tip) from the row() default where an empty key means "fall back to the label".
    pub key_explicit: bool,
    pub label: SharedString,
    pub checked: Option<bool>,
    /// 让这一行常亮 **hover 底色**（工具栏 flyout 用它标出当前工具）。默认 `false`：
    /// 右键菜单 / Actions palette 等其它菜单的勾选行保持原样，不因勾选而常亮。
    pub active_fill: bool,
    pub disabled: bool,
    pub icon: Option<Icon>,
    pub trail: Option<SharedString>,
    pub separator_before: bool,
    pub sub: Vec<Item>,
}

impl Item {
    pub fn new(id: &'static str, label: impl Into<SharedString>) -> Self {
        Self {
            id,
            key: SharedString::from(id),
            key_explicit: true,
            label: label.into(),
            checked: None,
            active_fill: false,
            disabled: false,
            icon: None,
            trail: None,
            separator_before: false,
            sub: Vec::new(),
        }
    }

    /// 右栏下拉/主菜单的习惯构造：只要一个标签。`id` 留空、pick 值回落到标签（对齐旧
    /// `MenuRow::new` 的语义）。统一后两侧菜单都调这里，项配置仍走同一套 builder。
    pub fn row(label: impl Into<SharedString>) -> Self {
        let mut r = Self::new("", label);
        r.key_explicit = false;
        r
    }

    /// 旧 `MenuRow::separator()` 的别名 —— 统一项模型后 `separator_before()` 保持原样，
    /// 这里只加一个同义入口，不改任何行为。
    pub fn separator(mut self) -> Self {
        self.separator_before = true;
        self
    }

    /// Override the value handed to `OnPick` (defaults to `id`).
    pub fn key(mut self, k: impl Into<SharedString>) -> Self {
        self.key = k.into();
        self.key_explicit = true;
        self
    }
    pub fn disabled(mut self, b: bool) -> Self {
        self.disabled = b;
        self
    }
    pub fn checked(mut self, c: Option<bool>) -> Self {
        self.checked = c;
        self
    }
    /// Keep this row lit with the hover fill (the toolbar flyout's current tool).
    pub fn active_fill(mut self, on: bool) -> Self {
        self.active_fill = on;
        self
    }
    pub fn icon(mut self, i: Icon) -> Self {
        self.icon = Some(i);
        self
    }
    pub fn trail(mut self, t: impl Into<SharedString>) -> Self {
        self.trail = Some(t.into());
        self
    }
    pub fn separator_before(mut self) -> Self {
        self.separator_before = true;
        self
    }
    pub fn sub(mut self, items: Vec<Item>) -> Self {
        self.sub = items;
        self
    }
}

/// 选中一项（送命令 id）与展开子菜单（送行下标）的回调。
pub type OnPick = Rc<dyn Fn(&str, &mut Window, &mut App)>;
pub type OnSub = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// 卡片内容高度：上下各 8pt 内边距 + 每行行高 + 每条分隔线 9pt。
pub(crate) fn height(items: &[Item], row_h: f32) -> f32 {
    2.0 * PAD + items.len() as f32 * row_h + items.iter().filter(|i| i.separator_before).count() as f32 * SEP_H
}

/// 最宽一行的文字宽度（标签内缩 + 标签 + 可选的快捷键列）。
fn widest(items: &[Item], has_check: bool, window: &Window) -> f32 {
    let measure = |s: &SharedString| -> f32 {
        let run = TextRun {
            len: s.len(),
            font: window.text_style().font(),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        // 测量必须和渲染用同一个字号（共用骨架 `menu_rows::TEXT_PT`）：两边各写一份就会漂移。
        f32::from(window.text_system().shape_line(s.clone(), pt(menu_rows::TEXT_PT), &[run], None).width)
    };
    items
        .iter()
        .map(|i| {
            let mut w = menu_rows::label_x(i, has_check) + measure(&i.label) + 10.0;
            if let Some(trail) = &i.trail {
                if !trail.is_empty() {
                    w += MENU_TRAIL_GAP + measure(trail) + 16.0;
                }
            }
            w
        })
        .fold(0.0f32, f32::max)
        .max(120.0)
}

/// 整卡宽度（最宽行 + 两侧内边距）：给「向上飞出」的调用方用 —— 它得先知道宽度才能把卡片
/// 水平夹进窗口（`card` 自己只做纵向防溢出）。与 `card` 内部的取值完全同源。
pub(crate) fn fit_width(items: &[Item], window: &Window) -> f32 {
    let has_check = items.iter().any(|i| i.checked.is_some());
    widest(items, has_check, window) + 2.0 * PAD
}

/// The rows alone, with no card chrome (the Actions palette puts them under its search field).
///
/// Returns the rows **and** any open submenu cards as two separate vecs: a scrollable card puts the
/// rows inside its scroll viewport, while the submenus stay direct children of the surface (their
/// `absolute` placement is in window coordinates and must not scroll with the rows).
#[allow(clippy::too_many_arguments)]
pub(crate) fn rows(
    items: &[Item],
    row_h: f32,
    has_check: bool,
    open_sub: Option<usize>,
    // The keyboard-highlighted row (palette Up/Down); `None` for menus that have no keyboard.
    selected: Option<usize>,
    // The highlighted row inside the expanded submenu; only the context menu sets this.
    sub_selected: Option<usize>,
    left: f32,
    top: f32,
    width: f32,
    on_pick: OnPick,
    on_sub: OnSub,
    // 悬停某一行时回调（`MenuAim` 的输入）：子菜单的展开 / 收起由通用状态机裁决，
    // 不再靠点击切换。工具栏飞出 / palette 传空闭包即可。
    on_hover: OnSub,
    // Kept for call sites' readability; every menu is the dark surface now.
    _menu_surface: bool,
    window: &Window,
    vh: f32,
) -> (Vec<AnyElement>, Vec<AnyElement>) {
    // 组装骨架完全共用；这里只挂「卡片行为」：子菜单展开、键盘高亮、点击不关闭浮层。
    let make_sub = {
        let (on_pick, on_sub) = (on_pick.clone(), on_sub.clone());
        Rc::new(move |sub: &[Item], row_top: f32, window: &Window| {
            Some(card_selected(
                left + width + 4.0,
                row_top - menu_rows::PAD,
                Width::Min(width),
                row_h,
                sub,
                None,
                sub_selected,
                None,
                on_pick.clone(),
                on_sub.clone(),
                Rc::new(|_, _, _| {}),
                window,
                vh,
            ))
        })
    };
    let behavior = menu_rows::card_behavior(on_pick, on_sub, on_hover, open_sub, selected, row_h, make_sub);
    menu_rows::assemble(items, row_h, has_check, width, top, Some(window), |item| item.key.as_ref(), behavior)
}

/// The card at `(left, top)` (window coordinates).
///
/// `sizing` is the shared `Menu` control's width policy (`用户 2026-11-06`): `Width::Min(f)` 固定
/// 下限但内容更宽时以最宽那项为准；`Width::Fit` 就是最宽那项；`Width::Anchor` 跟随触发控件。
/// 宽度恒为确定值，所以每行的 `w_full` 都填充到同一宽度。
///
/// When `top + content` would run past the viewport bottom, the card pins its bottom to `vh` and
/// scrolls internally — a 24 pt affordance (a centred 24×24 down chevron) replaces the 8 pt bottom
/// padding, and the scrollbar is hidden; none of the items ever show a drag thumb.
#[allow(clippy::too_many_arguments)]
pub(crate) fn card(
    left: f32,
    top: f32,
    sizing: Width,
    row_h: f32,
    items: &[Item],
    open_sub: Option<usize>,
    on_pick: OnPick,
    on_sub: OnSub,
    on_hover: OnSub,
    window: &Window,
    vh: f32,
) -> AnyElement {
    card_selected(left, top, sizing, row_h, items, open_sub, None, None, on_pick, on_sub, on_hover, window, vh)
}

/// [`card`] with a keyboard-highlighted row (`selected`): the context menu opened via
/// Shift+F10 / Menu uses this so Up/Down visibly move before Enter runs a row.
#[allow(clippy::too_many_arguments)]
pub(crate) fn card_selected(
    left: f32,
    top: f32,
    sizing: Width,
    row_h: f32,
    items: &[Item],
    open_sub: Option<usize>,
    selected: Option<usize>,
    sub_selected: Option<usize>,
    on_pick: OnPick,
    on_sub: OnSub,
    on_hover: OnSub,
    window: &Window,
    vh: f32,
) -> AnyElement {
    let has_check = items.iter().any(|r| r.checked.is_some());
    let content_w = widest(items, has_check, window);
    let rows_h = height(items, row_h) - 2.0 * PAD;
    let w = match sizing {
        Width::Fit => content_w + 2.0 * PAD,
        Width::Min(f) => (content_w + 2.0 * PAD).max(f),
        Width::Anchor => (content_w + 2.0 * PAD).max(48.0),
    };
    let (row_els, sub_els) = rows(items, row_h, has_check, open_sub, selected, sub_selected, left, top, w, on_pick, on_sub, on_hover, true, window, vh);
    // 唯一的浮层控件：宽度三档、锚点跟随、上下防溢出与滚动全在 `Menu::show` 里。
    crate::widgets::menu(crate::widgets::at_point(left, top))
        .gap(0.0)
        .width(sizing)
        .viewport(0.0, vh) // left 已由调用方按当前窗口算好
        .content(content_w, rows_h, row_h)
        .tag(format!("card-{}-{}", left as i32, top as i32))
        .extras(sub_els)
        .show(|| div().flex().flex_col().children(row_els).into_any_element())
        .unwrap_or_else(|| div().into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The card's height must be predictable from the rows: 8 + 8 chrome, shared 24 pt rows,
    /// and one shared 9 pt divider.
    #[test]
    fn card_height_is_chrome_plus_rows() {
        let rows = vec![Item::new("a", "A"), Item::new("b", "B"), Item::new("c", "C").separator_before()];
        let h = crate::menu_metrics::MENU_ROW_H;
        assert_eq!(height(&rows, h), 2.0 * PAD + 3.0 * h + SEP_H);
        assert_eq!(height(&[], h), 2.0 * PAD);
    }

    /// 标签内缩由共用骨架算，卡片这一侧不再有一份自己的规则。
    #[test]
    fn label_inset_follows_the_check_and_icon_columns() {
        let plain = Item::new("a", "A");
        assert_eq!(menu_rows::label_x(&plain, false), 8.0);
        assert_eq!(menu_rows::label_x(&plain, true), menu_rows::SLOT);
        let tool = Item::new("b", "B").icon(Icon::Move);
        assert_eq!(menu_rows::label_x(&tool, true), menu_rows::SLOT * 2.0);
    }

    /// 统一后：下拉 / 右键 / 工具栏 / 主菜单共用这一个 Item。`row()` 是旧 MenuRow::new 的
    /// 兼容入口（pick 回落到 label），`icon()` 走统一图标枚举，
    /// `checked()` / `separator()` 是单选勾选与分隔线 —— sides 菜单就靠这三项配置。
    #[test]
    fn one_item_model_carries_all_menu_config() {
        let r = Item::row("Custom").key("custom").icon(Icon::Check).checked(Some(true)).separator();
        assert_eq!(r.label, "Custom");
        assert_eq!(r.key, "custom");
        assert_eq!(r.icon, Some(Icon::Check));
        assert_eq!(r.checked, Some(true));
        assert!(r.separator_before);
        assert_eq!(menu_rows::label_x(&r, true), menu_rows::SLOT * 2.0, "带图标的行也要给图标列留位");
        // 普通 row() 的 pick key 为空 → 渲染器回落到 label，等价旧 None 语义。
        let plain = Item::row("All");
        assert!(plain.key.is_empty());
        assert!(plain.icon.is_none());
    }
}
