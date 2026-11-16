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
//! 控件级 `Tab` 遍历与读屏树依赖 AccessKit，这个 GPUI 版本给不了；见计划表 A21 的说明。

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
