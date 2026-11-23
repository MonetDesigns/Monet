//! 通用滑条（2026-11-16 用户要求新增，参照 gpui-kit 的 Slider）：一条胶囊轨道 + 圆形滑块，
//! 按下 / 拖动改值，可选右侧数值。颜色弹窗的色相 / 透明度、字体排版的可变轴、平滑圆角窗口
//! 共用这一份几何与拖拽逻辑；轨道视觉可以整体替换（渐变、棋盘 + 渐变）而不复制交互代码。
//!
//! 本文件是**新增**控件，不改动 `widgets/` 下任何已冻结文件的行为。

use crate::widgets::{rgba, tokens};
use gpui::prelude::*;
use gpui::{App, Bounds, Corners, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, SharedString, Window, div, px};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    /// 跨帧稳定的拖拽状态：`render()` 每帧重建控件，局部 `Cell` 会在第一次
    /// `on_change` 触发重绘后丢失，导致滑条只能动一下（用户 2026-11-19 实测）。
    /// 带 `id()` 的滑条按 id 复用同一个 `Cell`。
    static DRAGS: std::cell::RefCell<HashMap<SharedString, Rc<Cell<bool>>>> =
        std::cell::RefCell::new(HashMap::new());
}

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
    /// 轨道高度：通用默认 6 pt，取色器的色相 / 透明度滑条按设计稿用 16 pt。
    track_h: f32,
    /// 滑块直径：通用默认 12 pt，取色器用 16 pt。
    thumb: f32,
    /// 整条滑条元素高度：通用默认 24 pt，取色器两条 16 pt 滑条 + 12 pt 间距 = 44 pt 行。
    row_h: f32,
    /// 滑块中心填色（取色器用它显示当前颜色预览）。
    thumb_fill: Option<gpui::Rgba>,
    /// 是否用取色器统一的圆形手柄（18pt 阴影 + 16pt 白环 + 中心色）。
    knob: bool,
    /// 跨帧稳定的拖拽 id（同一条滑条每帧都会重建，靠它复用拖拽状态）。
    id: Option<SharedString>,
    show_value: bool,
    suffix: SharedString,
    precision: usize,
    paint_track: Option<PaintTrack>,
    on_change: Option<Rc<dyn Fn(f32, &mut Window, &mut App)>>,
    /// 轨道矩形：在 canvas 的 prepaint 里写入，鼠标事件里用来把 x 映射成值。
    hit: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// CodeRabbit PR#23: the last value this drag committed. The constructor `value` is a snapshot
    /// and can lag behind while a document-driven rebuild is pending; comparing against it made the
    /// thumb snap back and swallowed moves back toward the old value.
    last: Rc<Cell<f32>>,
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
            track_h: TRACK_H,
            thumb: THUMB,
            row_h: 24.0,
            thumb_fill: None,
            knob: false,
            show_value: false,
            suffix: SharedString::default(),
            precision: 0,
            paint_track: None,
            on_change: None,
            id: None,
            hit: Rc::new(Cell::new(None)),
            last: Rc::new(Cell::new(value.clamp(min, max))),
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

    /// 轨道高度（取色器色相 / 透明度滑条传 16）。
    pub fn track_h(mut self, h: f32) -> Self {
        self.track_h = h.max(2.0);
        self
    }

    /// 滑块直径（取色器传 16）。
    pub fn thumb(mut self, d: f32) -> Self {
        self.thumb = d.max(6.0);
        self
    }

    /// 整条滑条元素高度（取色器传 16，两条 16 + 12 间距正好是设计稿的 44）。
    pub fn row_h(mut self, h: f32) -> Self {
        self.row_h = h.max(self.track_h.max(self.thumb));
        self
    }

    /// 取色器统一圆形手柄样式。
    pub fn knob(mut self) -> Self {
        self.knob = true;
        self
    }

    /// 滑块中心填色，色相滑条用它预览当前颜色。
    pub fn thumb_fill(mut self, color: impl Into<gpui::Rgba>) -> Self {
        self.thumb_fill = Some(color.into());
        self
    }

    /// 跨帧稳定的拖拽 id：同一条滑条每帧重建，靠它复用拖拽状态。
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = Some(id.into());
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
        // CodeRabbit PR#23: track whether *this* slider owns the drag. A global "left button held"
        // check let a drag started on the hue track also move the alpha track under the pointer.
        let dragging = match &self.id {
            Some(id) => DRAGS.with(|m| m.borrow_mut().entry(id.clone()).or_insert_with(|| Rc::new(Cell::new(false))).clone()),
            None => Rc::new(Cell::new(false)),
        };
        let drag_down = dragging.clone();
        let drag_win = dragging.clone();
        let drag_end = dragging.clone();
        let last_down = self.last.clone();
        let last_win = self.last.clone();
        let hit_win = hit.clone();
        let on_change = self.on_change.clone();
        let on_change_win = self.on_change.clone();
        let paint_track = self.paint_track.clone();
        let track = rgba(t.border);
        let fill = rgba(t.accent);
        let track_h = self.track_h;
        let thumb = self.thumb;
        let thumb_fill = self.thumb_fill.unwrap_or_else(|| rgba(t.panel));
        let thumb_edge = rgba(t.border_strong);
        let value_text = format!("{:.*}{}", self.precision, value, self.suffix);

        let canvas = gpui::canvas(
            move |bounds, _, _| hit.set(Some(bounds)),
            move |bounds, _, window, _| {
                let x0 = f32::from(bounds.origin.x);
                let w = f32::from(bounds.size.width).max(1.0);
                let cy = f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0;
                let track_rect = Bounds { origin: gpui::point(px(x0), px(cy - track_h / 2.0)), size: gpui::size(px(w), px(track_h)) };
                if let Some(paint) = &paint_track {
                    paint(window, track_rect, frac);
                } else {
                    window.paint_quad(gpui::fill(track_rect, track).corner_radii(pill(track_h / 2.0)));
                    let filled = (w * frac).max(track_h);
                    let fill_rect = Bounds { origin: track_rect.origin, size: gpui::size(px(filled.min(w)), px(track_h)) };
                    window.paint_quad(gpui::fill(fill_rect, fill).corner_radii(pill(track_h / 2.0)));
                }
                // 滑块：外圈 1pt 描边感的边缘 + 亮面，中心压在取值位置。
                let cx = x0 + w * frac;
                if self.knob {
                    let shadow = Bounds { origin: gpui::point(px(cx - 9.0), px(cy - 9.0)), size: gpui::size(px(18.0), px(18.0)) };
                    window.paint_quad(gpui::fill(shadow, rgba(crate::canvas::geom::Color32::from_black_alpha(20))).corner_radii(pill(9.0)));
                    let ring = Bounds { origin: gpui::point(px(cx - 8.0), px(cy - 8.0)), size: gpui::size(px(16.0), px(16.0)) };
                    window.paint_quad(gpui::fill(ring, rgba(crate::canvas::geom::Color32::WHITE)).corner_radii(pill(8.0)));
                    let center = Bounds { origin: gpui::point(px(cx - 4.0), px(cy - 4.0)), size: gpui::size(px(8.0), px(8.0)) };
                    window.paint_quad(gpui::fill(center, thumb_fill).corner_radii(pill(4.0)));
                } else {
                    let edge = Bounds { origin: gpui::point(px(cx - thumb / 2.0), px(cy - thumb / 2.0)), size: gpui::size(px(thumb), px(thumb)) };
                    window.paint_quad(gpui::fill(edge, thumb_edge).corner_radii(pill(thumb / 2.0)));
                    let inner = Bounds {
                        origin: gpui::point(px(cx - thumb / 2.0 + 1.0), px(cy - thumb / 2.0 + 1.0)),
                        size: gpui::size(px(thumb - 2.0), px(thumb - 2.0)),
                    };
                    window.paint_quad(gpui::fill(inner, thumb_fill).corner_radii(pill((thumb - 2.0) / 2.0)));
                }
                // 拖动监听挂在窗口级：指针一旦移出轨道就收不到元素的 move/up，滑块会卡在
                // 拖动状态（用户实测「滑条拖动很怪 / 放不下」）。
                {
                    let dragging = drag_win.clone();
                    let hit = hit_win.clone();
                    let last = last_win.clone();
                    let cb = on_change_win.clone();
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                        if !phase.bubble() || !dragging.get() {
                            return;
                        }
                        let Some(cb) = &cb else { return };
                        let Some(b) = hit.get() else { return };
                        let w = f32::from(b.size.width).max(1.0);
                        let f = ((f32::from(e.position.x) - f32::from(b.origin.x)) / w).clamp(0.0, 1.0);
                        let raw = min + f * (max - min);
                        let next = (if step > 0.0 { (raw / step).round() * step } else { raw }).clamp(min, max);
                        if (next - last.get()).abs() < 1e-4 {
                            return;
                        }
                        last.set(next);
                        cb(next, window, cx);
                    });
                    let dragging = drag_end.clone();
                    window.on_mouse_event(move |_e: &MouseUpEvent, phase, _, _| {
                        if phase.bubble() {
                            dragging.set(false);
                        }
                    });
                }
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
                drag_down.set(true);
                let Some(b) = hit_down.get() else { return };
                let w = f32::from(b.size.width).max(1.0);
                let f = ((f32::from(e.position.x) - f32::from(b.origin.x)) / w).clamp(0.0, 1.0);
                let raw = min + f * (max - min);
                let next = (if step > 0.0 { (raw / step).round() * step } else { raw }).clamp(min, max);
                last_down.set(next);
                if let Some(cb) = &on_change {
                    cb(next, window, cx);
                } else {
                    let _ = next;
                }
            })
            .child(canvas);

        let mut row = div().h(px(self.row_h)).flex().flex_row().items_center().gap(px(8.0));
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
