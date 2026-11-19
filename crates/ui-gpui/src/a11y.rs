//! A21：键盘区域导航 —— 上游 `ui-egui/src/a11y.rs` 在 GPUI 上可搬的那一层。
//!
//! 上游的模型是：`F6` / `Shift+F6` 在**区域**之间移动（工具条 → 左栏 → 画布 → 右栏），
//! `Tab` 在区域内的控件之间走，方向键沿工具条与图层行移动，`Escape` 回到画布，焦点环
//! 标出键盘当前所在的位置，鼠标一按就回到指针模式。
//!
//! GPUI 0.2 没有暴露 AccessKit 树，也不像 egui 那样能在下一帧取回「上一帧画过的每个控件
//! 矩形」，所以这里实现**区域级**导航：
//!
//! * `F6` / `Shift+F6`：区域循环（引擎 `focus.region` 请求的落点）；
//! * `Escape`：回到画布；
//! * `↑` / `←` 与 `↓` / `→`：在非画布区域里移动选区（`select.previous` / `select.next`，
//!   与上游「方向键沿图层行移动」的用户可见结果一致）；
//! * 活动区域画一圈强调色焦点环；指针在画布上按下即回到指针模式。
//!
//! 2026-11-16：控件级 `Tab` 与读屏树已按 gpui-pre 0.3.12 / gpui-kit 的做法补齐——
//! 不升级 GPUI、不引入 gpui-kit 依赖，把缺的语义层抄进本模块（见 `a11y/` 子模块）。

mod element;
mod platform;
mod registry;
mod tree;

pub(crate) use element::{A11yExt, Meta};

use gpui::{App, FocusHandle, Window, prelude::*};

/// App-global platform adapter, installed before the window is shown.
struct PlatformGlobal(Option<platform::PlatformA11y>);

impl gpui::Global for PlatformGlobal {}

fn ensure_platform(cx: &mut App) {
    if cx.try_global::<PlatformGlobal>().is_none() {
        cx.set_global(PlatformGlobal(None));
    }
}

/// Initialise the registry once, before the window is opened.
pub(crate) fn init(cx: &mut App) {
    registry::init(cx);
    ensure_platform(cx);
}

/// Install the Windows AccessKit adapter in a hidden window, then show it.
pub(crate) fn install(window: &Window, cx: &mut App) {
    ensure_platform(cx);
    let installed = platform::PlatformA11y::install(window);
    if let Some(adapter) = &installed {
        adapter.show();
    }
    cx.global_mut::<PlatformGlobal>().0 = installed;
}

/// Start a new frame's registry buffer.
pub(crate) fn begin_frame(cx: &mut App) {
    registry::with(cx, |reg| reg.begin_frame());
}

/// Finish the frame: publish the tree, apply focus actions, return commands
/// that assistive technology asked us to run (the caller owns the engine).
pub(crate) fn commit_frame(
    title: &str,
    focused: Option<&FocusHandle>,
    scale: f32,
    window: &mut Window,
    cx: &mut App,
) -> Vec<(accesskit::Action, String)> {
    registry::ensure(cx);
    ensure_platform(cx);
    let update = {
        let reg = &cx.global_mut::<registry::A11yGlobal>().registry;
        tree::build_update(reg, title, focused, scale)
    };
    let mut commands = Vec::new();
    if let Some(adapter) = cx.global_mut::<PlatformGlobal>().0.as_mut() {
        adapter.update(update);
        for request in adapter.drain() {
            let entry = {
                let reg = &cx.global_mut::<registry::A11yGlobal>().registry;
                reg.control(request.target_node).map(|c| (c.focus.clone(), c.command.clone()))
            };
            if let Some((Some(handle), _)) = &entry {
                window.focus(handle);
            }
            if let Some((_, Some(command))) = entry {
                commands.push((request.action, command));
            }
        }
    }
    commands
}

/// Tab / Shift+Tab inside one region, using the same control list egui walks.
/// Focus handles are stable and keyed, so this also works for stateless views.
pub(crate) fn tab_next(cx: &mut App, region: Region, step: i32, focused: Option<&FocusHandle>) -> Option<FocusHandle> {
    registry::ensure(cx);
    let global = &cx.global::<registry::A11yGlobal>().registry;
    let list = global.focusables(region);
    if list.is_empty() {
        return None;
    }
    let cur = focused.and_then(|f| list.iter().position(|c| c.focus.as_ref() == Some(f))).and_then(|i| i32::try_from(i).ok()).unwrap_or(-1);
    let next = if cur < 0 { if step > 0 { 0 } else { list.len() - 1 } } else { (cur + step).rem_euclid(list.len() as i32) as usize };
    list.get(next).and_then(|c| c.focus.clone())
}

/// Focus ring around the control that has keyboard focus. Bounds come from
/// the previous frame's registry, i.e. the rectangles the user actually saw.
pub(crate) fn focus_ring(focused: Option<&FocusHandle>, cx: &mut App) -> Option<gpui::AnyElement> {
    registry::ensure(cx);
    let control = {
        let reg = &cx.global::<registry::A11yGlobal>().registry;
        reg.previous.iter().find(|c| c.focus.is_some() && c.focus.as_ref() == focused)?.clone()
    };
    let b = control.bounds;
    let t = crate::widgets::tokens();
    let x = f32::from(b.origin.x) - 2.0;
    let y = f32::from(b.origin.y) - 2.0;
    let w = f32::from(b.size.width) + 4.0;
    let h = f32::from(b.size.height) + 4.0;
    Some(
        gpui::div()
            .absolute()
            .left(gpui::px(x))
            .top(gpui::px(y))
            .w(gpui::px(w))
            .h(gpui::px(h))
            .rounded(gpui::px(6.0))
            .border_2()
            .border_color(crate::widgets::rgba(t.accent))
            .into_any_element(),
    )
}

/// 窗口的四个键盘区域（顺序即 `F6` 的顺序，同上游 `a11y::Region`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    Toolbar,
    Layers,
    Canvas,
    Properties,
}

impl Region {
    pub(crate) const ALL: [Region; 4] = [Region::Toolbar, Region::Layers, Region::Canvas, Region::Properties];

    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }

    fn from_index(i: usize) -> Region {
        Self::ALL[i % Self::ALL.len()]
    }

    /// 焦点环旁的提示文字（也用于调试与测试断言）。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Region::Toolbar => "toolbar",
            Region::Layers => "layers",
            Region::Canvas => "canvas",
            Region::Properties => "properties",
        }
    }
}

/// 键盘焦点状态。`None` = 指针模式（与上游一致：没有键盘导航就不画焦点环）。
#[derive(Debug, Default)]
pub(crate) struct KeyboardNav {
    pub region: Option<Region>,
}

impl KeyboardNav {
    /// 引擎 `focus.region` 请求的 `step`：+1 下一个区域，-1 上一个。
    /// 还没进键盘模式时从「画布之前」算起，所以 +1 落在工具条。
    pub(crate) fn step(&mut self, step: i64) {
        let n = Region::ALL.len() as i64;
        let current = self.region.map_or(-1, |r| r.index() as i64);
        // keep step overflow fix from main
        let step = step.rem_euclid(n);
        self.region = Some(Region::from_index((current + step).rem_euclid(n) as usize));
    }

    /// 回到画布（`Escape`）：只退出键盘导航，不动选区（上游同义）。
    pub(crate) fn to_canvas(&mut self) {
        self.region = Some(Region::Canvas);
    }

    /// 指针按下：回到指针模式，焦点环消失。
    pub(crate) fn pointer(&mut self) {
        self.region = None;
    }

    pub(crate) fn is_navigating(&self) -> bool {
        self.region.is_some_and(|r| r != Region::Canvas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f6_walks_the_four_regions_and_wraps() {
        let mut nav = KeyboardNav::default();
        nav.step(1);
        assert_eq!(nav.region, Some(Region::Toolbar));
        nav.step(1);
        assert_eq!(nav.region, Some(Region::Layers));
        nav.step(1);
        assert_eq!(nav.region, Some(Region::Canvas));
        nav.step(1);
        assert_eq!(nav.region, Some(Region::Properties));
        nav.step(1);
        assert_eq!(nav.region, Some(Region::Toolbar), "wraps forward");
        nav.step(-1);
        assert_eq!(nav.region, Some(Region::Properties), "wraps backward");
    }

    #[test]
    fn pointer_press_ends_keyboard_navigation_and_escape_returns_to_canvas() {
        let mut nav = KeyboardNav::default();
        nav.step(1);
        assert!(nav.is_navigating());
        nav.to_canvas();
        assert_eq!(nav.region, Some(Region::Canvas));
        assert!(!nav.is_navigating());
        nav.pointer();
        assert_eq!(nav.region, None, "pointer mode draws no focus ring");
    }
}
