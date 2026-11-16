//! 通用滑条（2026-11-16 用户要求新增，参照 gpui-kit 的 Slider）：一条胶囊轨道 + 圆形滑块，
//! 按下 / 拖动改值，可选右侧数值。颜色弹窗的色相 / 透明度、字体排版的可变轴、平滑圆角窗口
//! 共用这一份几何与拖拽逻辑；轨道视觉可以整体替换（渐变、棋盘 + 渐变）而不复制交互代码。
//!
//! 本文件是**新增**控件，不改动 `widgets/` 下任何已冻结文件的行为。

use crate::widgets::{rgba, tokens};
use gpui::prelude::*;
use gpui::{App, Bounds, Corners, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, SharedString, Window, div, px};
use std::cell::Cell;
use std::rc::Rc;

/// 轨道高度（gpui-kit 默认 6 pt）。
const TRACK_H: f32 = 6.0;
/// 滑块直径（gpui-kit 默认 12 pt）。
const THUMB: f32 = 12.0;
/// 未指定宽度时，整条滑条占满容器；数值列宽固定，避免拖动时左右跳。
const VALUE_W: f32 = 40.0;

/// 自定义轨道绘制：`frac` 是当前值在区间里的位置（0..=1），`bounds` 是整条轨道的矩形。
pub type PaintTrack = Rc<dyn Fn(&mut Window, Bounds<Pixels>, f32)>;

/// 一条滑条。构造期给值、区间与步进，`on_change` 在按下 / 拖动时按新值回调。
pub struct Slider {
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    width: Option<f32>,
    show_value: bool,
    suffix: SharedString,
    precision: usize,
    paint_track: Option<PaintTrack>,
    on_change: Option<Rc<dyn Fn(f32, &mut Window, &mut App)>>,
    /// 轨道矩形：在 canvas 的 prepaint 里写入，鼠标事件里用来把 x 映射成值。
    hit: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl Slider {
    /// `value` 会被夹到 `min..=max`。
    pub fn new(value: f32, min: f32, max: f32) -> Self {
        Self {
            value: value.clamp(min, max),
            min,
            max,
            step: 0.0,
            width: None,
            show_value: false,
            suffix: SharedString::default(),
            precision: 0,
            paint_track: None,
            on_change: None,
            hit: Rc::new(Cell::new(None)),
        }
    }

    /// 步进（例如 1 表示只取整数）；`0` = 连续。
    pub fn step(mut self, step: f32) -> Self {
        self.step = step.max(0.0);
        self
    }

    /// 固定宽度；不设时占满容器。
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }

    /// 右侧显示数值（`precision` 位小数 + `suffix`）。
    pub fn show_value(mut self) -> Self {
        self.show_value = true;
        self
    }

    pub fn suffix(mut self, suffix: impl Into<SharedString>) -> Self {
        self.suffix = suffix.into();
        self
    }

    pub fn precision(mut self, digits: usize) -> Self {
        self.precision = digits;
        self
    }

    /// 替换整个轨道视觉（色相渐变、棋盘 + 透明度渐变）；滑块仍由本控件绘制。
    pub fn paint_track(mut self, paint: impl Fn(&mut Window, Bounds<Pixels>, f32) + 'static) -> Self {
        self.paint_track = Some(Rc::new(paint));
        self
    }

    /// 值变化回调（拖动时连续触发；命令队列本身会合并同一条命令）。
    pub fn on_change(mut self, f: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }

    pub fn render(self) -> impl IntoElement {
        let t = tokens();
        let (min, max) = (self.min, self.max);
        let span = (max - min).max(f32::EPSILON);
        let frac = ((self.value - min) / span).clamp(0.0, 1.0);
        let value = self.value;
        let step = self.step;
        let hit = self.hit.clone();
        let hit_down = hit.clone();
        let hit_move = hit.clone();
        let on_change = self.on_change.clone();
        let on_change_move = self.on_change.clone();
        let paint_track = self.paint_track.clone();
        let track = rgba(t.border);
        let fill = rgba(t.accent);
        let thumb_fill = rgba(t.panel);
        let thumb_edge = rgba(t.border_strong);
        let value_text = format!("{:.*}{}", self.precision, value, self.suffix);

        let canvas = gpui::canvas(
            move |bounds, _, _| hit.set(Some(bounds)),
            move |bounds, _, window, _| {
                let x0 = f32::from(bounds.origin.x);
                let w = f32::from(bounds.size.width).max(1.0);
                let cy = f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0;
                let track_rect = Bounds {
                    origin: gpui::point(px(x0), px(cy - TRACK_H / 2.0)),
                    size: gpui::size(px(w), px(TRACK_H)),
                };
                if let Some(paint) = &paint_track {
                    paint(window, track_rect, frac);
                } else {
                    window.paint_quad(gpui::fill(track_rect, track).corner_radii(pill(TRACK_H / 2.0)));
                    let filled = (w * frac).max(TRACK_H);
                    let fill_rect = Bounds { origin: track_rect.origin, size: gpui::size(px(filled.min(w)), px(TRACK_H)) };
                    window.paint_quad(gpui::fill(fill_rect, fill).corner_radii(pill(TRACK_H / 2.0)));
                }
                // 滑块：外圈 1pt 描边感的边缘 + 亮面，中心压在取值位置。
                let cx = x0 + w * frac;
                let edge = Bounds {
                    origin: gpui::point(px(cx - THUMB / 2.0), px(cy - THUMB / 2.0)),
                    size: gpui::size(px(THUMB), px(THUMB)),
                };
                window.paint_quad(gpui::fill(edge, thumb_edge).corner_radii(pill(THUMB / 2.0)));
                let inner = Bounds {
                    origin: gpui::point(px(cx - THUMB / 2.0 + 1.0), px(cy - THUMB / 2.0 + 1.0)),
                    size: gpui::size(px(THUMB - 2.0), px(THUMB - 2.0)),
                };
                window.paint_quad(gpui::fill(inner, thumb_fill).corner_radii(pill((THUMB - 2.0) / 2.0)));
            },
        )
        .size_full();

        // canvas 本身不带事件，交互挂在包一层的 div 上；canvas 铺满这层。
        let track_area = div()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
            let Some(b) = hit_down.get() else { return };
            let w = f32::from(b.size.width).max(1.0);
            let f = ((f32::from(e.position.x) - f32::from(b.origin.x)) / w).clamp(0.0, 1.0);
            let raw = min + f * (max - min);
            let next = (if step > 0.0 { (raw / step).round() * step } else { raw }).clamp(min, max);
            if let Some(cb) = &on_change {
                cb(next, window, cx);
            } else {
                let _ = next;
            }
        })
        .on_mouse_move(move |e: &MouseMoveEvent, window, cx| {
            if e.pressed_button != Some(MouseButton::Left) {
                return;
            }
            let Some(cb) = &on_change_move else { return };
            let Some(b) = hit_move.get() else { return };
            let w = f32::from(b.size.width).max(1.0);
            let f = ((f32::from(e.position.x) - f32::from(b.origin.x)) / w).clamp(0.0, 1.0);
            let raw = min + f * (max - min);
            let next = (if step > 0.0 { (raw / step).round() * step } else { raw }).clamp(min, max);
            // 值没变就不重复提交（命令队列里同一条命令会被合并，但拖动时每帧推送仍是浪费）。
            if (next - value).abs() < 1e-4 {
                return;
            }
            cb(next, window, cx);
        })
        .child(canvas);

        let mut row = div().h(px(24.0)).flex().flex_row().items_center().gap(px(8.0));
        if let Some(w) = self.width {
            row = row.w(px(w));
        } else {
            row = row.w_full();
        }
        row = row.child(track_area);
        if self.show_value {
            row = row.child(
                div()
                    .flex_none()
                    .w(px(VALUE_W))
                    .text_size(crate::widgets::pt(11.0))
                    .text_color(rgba(t.text_secondary))
                    .text_right()
                    .child(value_text),
            );
        }
        row
    }
}

/// 四角同半径——胶囊 / 圆形都是这个。
fn pill(r: f32) -> Corners<Pixels> {
    Corners { top_left: px(r), top_right: px(r), bottom_right: px(r), bottom_left: px(r) }
}
