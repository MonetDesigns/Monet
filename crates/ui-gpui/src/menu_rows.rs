//! 菜单行的**唯一组装骨架**。
//!
//! 所有菜单共用这份骨架：分隔线、勾选槽、图标槽、标签、trail、hover 墨色、禁用态。
//! 右键 / 飞出 / palette 与各种下拉之间的差异**不是第二套组装器**，而是两个
//! [`RowBehavior`]：
//!
//! - [`card_behavior`] —— 右键 / 工具栏飞出 / 动作库 / 子菜单：子菜单展开、键盘高亮、不 dismiss；
//! - [`dropdown_behavior`] —— 各种下拉：pick 后立即 dismiss，无子菜单 / 无键盘高亮。
//!
//! 这样「行怎么画」只有一处，改间距 / 墨色 / 槽位不会再出现两边漂移。

use crate::menu_card::Item;
use crate::widgets::icon::paint_in_slot;
use crate::widgets::{MENU_TRAIL_GAP, pt, rgba, tokens};
use gpui::{AnyElement, App, MouseButton, MouseDownEvent, Window, div, prelude::*, px};
use std::rc::Rc;

pub(crate) type OnPick = Rc<dyn Fn(&str, &mut Window, &mut App)>;
pub(crate) type OnSub = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// A leading glyph owns a 24 pt grid slot; check and icon slots touch.
pub(crate) const SLOT: f32 = crate::menu_metrics::MENU_SLOT_W;
/// 卡片内边距；分隔线靠它做负边距通到卡片边缘。
pub(crate) const PAD: f32 = crate::menu_metrics::MENU_PAD;
/// 一行的圆角。
const ITEM_RADIUS: f32 = 5.0;
/// All menu rows share this text size — one constant, one place.
pub(crate) const TEXT_PT: f32 = crate::widgets::TEXT_PT;

/// The label's left inset: check slot + icon slot, or 8 pt when a row has neither.
pub(crate) fn label_x(item: &Item, has_check: bool) -> f32 {
    let marked = item.icon.is_some();
    match (has_check, marked) {
        (true, true) => SLOT * 2.0,
        (true, false) => SLOT,
        (false, true) => SLOT,
        (false, false) => 8.0,
    }
}

/// 下拉行为的关闭判定：触发按钮相对开窗锚点移动超过 1pt 就关闭。
/// 纯函数，便于单测；工具栏 flyout / 右键菜单没有 probe，不会走到这里。
pub(crate) fn trigger_moved(open: gpui::Bounds<gpui::Pixels>, live: gpui::Bounds<gpui::Pixels>) -> bool {
    (f32::from(live.origin.x) - f32::from(open.origin.x)).abs() > 1.0
        || (f32::from(live.origin.y) - f32::from(open.origin.y)).abs() > 1.0
}

/// 给一个按钮挂「下拉行为」：按钮真实矩形通过跨帧探针上报，点击时用它当锚点开菜单。
/// 这是所有左键下拉菜单唯一的开窗方式 —— 绝不从 `mouse_position()` 取锚点。
///
/// 返回 `(带探针的按钮元素, 点击时调用的开窗回调所需的一切)`；实际开窗需要 root 实体，
/// 调用处把 `on_click` 里拿到的 `rect` 交给 `open_menu_fit(anchor, rect, rows, on_pick, cx)`。
pub(crate) struct AnchoredMenu {
    /// 按钮每帧写入真实矩形的探针 cell。
    pub cell: Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// 开窗时要传给 `open_menu_fit` 的跨帧锚点。
    pub anchor: crate::widgets::dropdown::MenuAnchor,
}

impl AnchoredMenu {
    /// `key` 必须跨帧稳定（同一按钮每帧同一个值）。
    pub fn new(key: impl Into<String>) -> Self {
        let anchor = crate::widgets::dropdown::menu_anchor(key);
        Self { cell: anchor.cell(), anchor }
    }

    /// 点击那一帧的锚点矩形；首帧探针还没量到时给一个 24×24 的兜底（按钮尺寸）。
    pub fn rect(&self) -> gpui::Bounds<gpui::Pixels> {
        self.cell.get().unwrap_or_else(|| gpui::Bounds {
            origin: gpui::Point::new(px(0.0), px(0.0)),
            size: gpui::size(px(24.0), px(24.0)),
        })
    }

    /// 把探针包到按钮元素里（绝对定位、不占空间）。
    pub fn wrap<E>(&self, el: E) -> gpui::AnyElement
    where
        E: gpui::IntoElement,
        E::Element: gpui::ParentElement + gpui::Styled,
    {
        crate::popovers::with_button_bounds(el, self.cell.clone())
    }
}

/// 一行的行为：共用骨架之外**唯一**允许分叉的地方。
pub(crate) struct RowBehavior {
    /// 点击：收到该行的 pick key、行下标、是否有子菜单。
    pub on_click: Rc<dyn Fn(&str, usize, bool, &mut Window, &mut App)>,
    /// 悬停（MenuAim 的输入）；没有瞄准需求的行为传空闭包。
    pub on_hover: OnSub,
    /// 键盘选中 / 常亮高亮的行。
    pub selected: Option<usize>,
    /// 当前展开子菜单的行下标。
    pub open_sub: Option<usize>,
    /// 生成子菜单卡片（下拉行为返回 `None`）。参数是子项、行顶 y、window。
    pub on_submenu: Rc<dyn Fn(&[Item], f32, &Window) -> Option<AnyElement>>,
}

/// 卡片行为：右键 / 工具栏飞出 / Actions palette / 子菜单。
/// 有子菜单的行点击展开，其余行执行 pick；不自动关闭浮层。
/// `make_submenu` 由 `menu_card` 提供（它要递归调 `card()`）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn card_behavior(
    on_pick: OnPick,
    on_sub: OnSub,
    on_hover: OnSub,
    open_sub: Option<usize>,
    selected: Option<usize>,
    row_h: f32,
    make_submenu: Rc<dyn Fn(&[Item], f32, &Window) -> Option<AnyElement>>,
) -> RowBehavior {
    RowBehavior {
        on_click: Rc::new(move |key: &str, i: usize, has_sub: bool, window: &mut Window, cx: &mut App| {
            if has_sub {
                on_sub(i, window, cx);
            } else {
                on_pick(key, window, cx);
            }
        }),
        on_hover,
        selected,
        open_sub,
        on_submenu: make_submenu,
    }
}

/// 下拉行为：点击执行 pick 后立即 dismiss；没有子菜单 / 键盘高亮。
pub(crate) fn dropdown_behavior(
    on_pick: OnPick,
    dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
) -> RowBehavior {
    RowBehavior {
        on_click: Rc::new(move |key: &str, _i: usize, _has_sub: bool, window: &mut Window, cx: &mut App| {
            on_pick(key, window, cx);
            dismiss(window, cx);
        }),
        on_hover: Rc::new(|_, _, _| {}),
        selected: None,
        open_sub: None,
        on_submenu: Rc::new(|_, _, _| None),
    }
}

/// 唯一的组装骨架：把 `items` 变成行元素 + 展开的子菜单卡片。
///
/// `pick_key(item)` 由调用处给出 pick 值（下拉要区分空 key 回落到 label，卡片按 id）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble(
    items: &[Item],
    row_h: f32,
    has_check: bool,
    width: f32,
    top: f32,
    // 子菜单生成需要 window；下拉行为没有子菜单，传 `None`。
    window: Option<&Window>,
    pick_key: impl Fn(&Item) -> &str,
    behavior: RowBehavior,
) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let t = tokens();
    let mut out: Vec<AnyElement> = Vec::new();
    let mut subs: Vec<AnyElement> = Vec::new();
    let mut y = top + PAD;

    for (i, item) in items.iter().enumerate() {
        if item.separator_before {
            out.push(crate::menu_metrics::divider(width).into_any_element());
            y += crate::menu_metrics::MENU_SEP_H;
        }
        let has_sub = !item.sub.is_empty();
        let inset = label_x(item, has_check);
        let row_top = y;
        let key = pick_key(item).to_string();

        let mut row = div()
            .relative()
            .h(pt(row_h))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .rounded(pt(ITEM_RADIUS))
            .pl(pt(inset))
            .pr(pt(8.0))
            .text_size(pt(TEXT_PT))
            .when(item.disabled, |d| d.opacity(0.4));

        if !item.disabled {
            row = row.hover(|s| s.bg(rgba(t.menu_hover))).cursor_pointer();
            let hover = behavior.on_hover.clone();
            row = row.on_mouse_move(move |_e, window, cx| hover(i, window, cx));
            let click = behavior.on_click.clone();
            row = row.on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
                click(key.as_str(), i, has_sub, window, cx);
            });
        }
        if (behavior.selected == Some(i) || item.active_fill) && !item.disabled {
            row = row.bg(rgba(t.menu_hover));
        }

        // 勾选槽：24×24，字形居中；没有勾选态的菜单整列不画。
        if has_check {
            row = row.child(
                div()
                    .absolute()
                    .left(pt(0.0))
                    .w(pt(SLOT))
                    .top(pt(0.0))
                    .h(pt(row_h))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(if item.checked == Some(true) { "✓" } else { "" }),
            );
        }
        // 图标槽：紧贴勾选槽（没有勾选列就从 0 起），24×24 居中。
        if let Some(icon) = item.icon {
            let icon_x = if has_check { SLOT } else { 0.0 };
            let color = crate::menu_metrics::ink();
            row = row.child(
                div()
                    .absolute()
                    .left(pt(icon_x))
                    .w(pt(SLOT))
                    .top(pt(0.0))
                    .h(pt(row_h))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(paint_in_slot(icon, 16.0, Some(color))),
            );
        }

        row = row.child(
            div()
                .flex_1()
                .min_w(pt(0.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .child(item.label.clone()),
        );
        if has_sub {
            row = row.child(
                div()
                    .ml(pt(MENU_TRAIL_GAP))
                    .whitespace_nowrap()
                    .text_color(crate::menu_metrics::trail_ink())
                    .child("▸"),
            );
        } else if let Some(trail) = &item.trail {
            if !trail.is_empty() {
                row = row.child(
                    div()
                        .ml(pt(MENU_TRAIL_GAP))
                        .whitespace_nowrap()
                        .text_color(crate::menu_metrics::trail_ink())
                        .child(trail.clone()),
                );
            }
        }

        out.push(row.into_any_element());
        y += row_h;

        if has_sub && behavior.open_sub == Some(i) {
            if let Some(window) = window {
                if let Some(sub) = (behavior.on_submenu)(&item.sub, row_top, window) {
                    subs.push(sub);
                }
            }
        }
    }
    (out, subs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::icon::Icon;

    #[test]
    fn one_skeleton_and_two_behaviors() {
        // 骨架不关心行为：两种行为都是同一个 RowBehavior 类型的实例。
        let pick: OnPick = Rc::new(|_, _, _| {});
        let card = card_behavior(pick.clone(), Rc::new(|_, _, _| {}), Rc::new(|_, _, _| {}), None, None, 24.0, Rc::new(|_, _, _| None));
        let drop = dropdown_behavior(pick, Rc::new(|_, _| {}));
        assert!(card.selected.is_none() && drop.selected.is_none());
        assert!(card.open_sub.is_none() && drop.open_sub.is_none());
    }

    #[test]
    fn dropdown_closes_when_its_trigger_moves() {
        use gpui::{point, px, size};
        let at = gpui::Bounds { origin: point(px(100.0), px(50.0)), size: size(px(80.0), px(24.0)) };
        let same = at;
        assert!(!trigger_moved(at, same), "按钮没动就不关");
        let nudged = gpui::Bounds { origin: point(px(100.5), px(50.0)), ..at };
        assert!(!trigger_moved(at, nudged), "亚像素抖动不关");
        let shifted = gpui::Bounds { origin: point(px(120.0), px(50.0)), ..at };
        assert!(trigger_moved(at, shifted), "按钮横向移动就关");
        let dropped = gpui::Bounds { origin: point(px(100.0), px(70.0)), ..at };
        assert!(trigger_moved(at, dropped), "按钮纵向移动就关");
        let resized = gpui::Bounds { size: size(px(120.0), px(24.0)), ..at };
        assert!(!trigger_moved(at, resized), "只有宽度变化不算位置移动");
    }

    #[test]
    fn label_inset_uses_the_shared_slot_math() {
        let plain = Item::row("A");
        let marked = Item::row("B").icon(Icon::Check);
        assert_eq!(label_x(&plain, false), 8.0);
        assert_eq!(label_x(&plain, true), SLOT);
        assert_eq!(label_x(&marked, true), SLOT * 2.0);
        assert_eq!(label_x(&marked, false), SLOT);
    }
}
