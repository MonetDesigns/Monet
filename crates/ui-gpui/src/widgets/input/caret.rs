//! 所有文字输入共用的光标 / 选区样式与闪烁节奏。
//!
//! 普通输入框（`TextInput`）和画布文字编辑都从这里取同一份参数，避免同一个应用里
//! 光标粗细、高度、颜色、闪烁各写一套。

use crate::canvas::geom::Color32;
use crate::theme::Tokens;
use std::time::{Duration, Instant};

/// 光标宽度（屏幕 pt，不随画布缩放变粗）。
pub const WIDTH: f32 = 1.0;
/// 两端圆角；画布画笔只支持整像素圆角，统一取 1px。
pub const RADIUS: u8 = 1;
/// 闪烁周期：亮 500ms / 灭 500ms。
pub const BLINK: Duration = Duration::from_millis(500);

/// 光标色：输入框与画布都用强调色。
pub fn color(t: &Tokens) -> Color32 {
    t.accent
}

/// 选区色：暗色主题 22% 白，亮色主题 22% 黑。
pub fn selection(t: &Tokens) -> Color32 {
    if t.dark { Color32::from_white_alpha(56) } else { Color32::from_black_alpha(56) }
}

/// 从进入编辑起算，这个时刻光标是否该显示。
///
/// 没有起点时一直显示（失焦或还没开始编辑），有起点后按 `BLINK` 一半亮一半灭。
/// 用一个纯函数让输入框和画布共享同一条规则，而不是各自维护一个开关。
pub fn visible_since(started: Option<Instant>, now: Instant) -> bool {
    let Some(started) = started else { return true };
    let half = BLINK.as_millis().max(1) as u128;
    (now.saturating_duration_since(started).as_millis() / half) % 2 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blink_rhythm_is_half_on_half_off() {
        let now = Instant::now();
        assert!(visible_since(None, now), "no editing session means the caret stays visible");
        assert!(visible_since(Some(now), now));
        assert!(!visible_since(Some(now), now + BLINK));
        assert!(visible_since(Some(now), now + BLINK * 2));
    }
}
