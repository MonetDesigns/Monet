// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
//
// 2026-11-15 用户明确授权解冻：按 参考编辑器 页面存档里的官方 CSS 重写本控件，
// 分方形 / 圆形两种色块（见 `SwatchShape`）。仅改本文件，不触碰其它冻结件。
// ============================================================================
//! Paint swatches — a direct port of `ui-egui`'s `checker`, `paint_swatch`, `swatch`,
//! `swatch_named` and `paint_name`.
//!
//! A swatch is the paint itself over a checkerboard so alpha reads. It is its own control
//! (`swatch(ui, paint, size)`), **not** a slot variant of a field: `color_field` uses it in its
//! lead slot for any paint that is not a plain solid, and other panels place one directly.
//!
//! 两种形状（用户 2026-11-15，参考 参考编辑器 存档 CSS）：方形 `chit` 左右分半、圆形 `swatch`
//! 按 45° 斜线分半；不透明的那一半恒为纯色，另一半跟着选中的透明度变化。

use crate::widgets::icon::{Icon, draw_centred};
use crate::widgets::{pt, rgba, tokens};
use figcraft_color::{Paint, PaintKind};
use figcraft_i18n::tr;
use gpui::{App, Bounds, Corners, IntoElement, Pixels, SharedString, Window, div, prelude::*};

/// `figcraft_color::Color`（f32 通道）→ 画布/GPUI 的 `Color32`。
pub(crate) fn color32(c: figcraft_color::Color) -> crate::canvas::geom::Color32 {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    crate::canvas::geom::Color32::from_rgba_unmultiplied(ch(c.r), ch(c.g), ch(c.b), ch(c.a))
}

/// Paint a checkerboard (transparency) in `bounds` — `ui-egui`'s `checker`.
pub fn checker(window: &mut Window, bounds: Bounds<Pixels>, cell: f32) {
    checker_rounded(window, bounds, cell, 0.0);
}

/// 胶囊轨道的棋盘底：左右两端按 `radius` 收圆，取色器透明度滑条用。
pub fn checker_capsule(window: &mut Window, bounds: Bounds<Pixels>, cell: f32, radius: f32) {
    checker_rounded(window, bounds, cell, radius);
}

/// [`checker`] whose corners round by `radius`, so a swatch's transparent background stays inside
/// the swatch's rounded border.
fn checker_rounded(window: &mut Window, bounds: Bounds<Pixels>, cell: f32, radius: f32) {
    let x0 = f32::from(bounds.origin.x);
    let y0 = f32::from(bounds.origin.y);
    let w = f32::from(bounds.size.width).max(1.0);
    let h = f32::from(bounds.size.height).max(1.0);
    let (x1, y1) = (x0 + w, y0 + h);
    quad(window, x0, y0, x1, y1, rgba(crate::canvas::geom::Color32::WHITE), radius);

    let cell = cell.max(1.0);
    let nx = (w / cell).ceil() as i32;
    let ny = (h / cell).ceil() as i32;
    for y in 0..ny {
        for x in 0..nx {
            if (x + y) % 2 == 0 {
                continue;
            }
            let cx0 = (x0 + x as f32 * cell).max(x0);
            let cy0 = (y0 + y as f32 * cell).max(y0);
            let cx1 = (x0 + x as f32 * cell + cell).min(x1);
            let cy1 = (y0 + y as f32 * cell + cell).min(y1);
            // The corner cells round their outer corner alone.
            let corners = Corners {
                top_left: gpui::px(if x == 0 && y == 0 { radius } else { 0.0 }),
                top_right: gpui::px(if x == nx - 1 && y == 0 { radius } else { 0.0 }),
                bottom_right: gpui::px(if x == nx - 1 && y == ny - 1 { radius } else { 0.0 }),
                bottom_left: gpui::px(if x == 0 && y == ny - 1 { radius } else { 0.0 }),
            };
            quad_corners(window, cx0, cy0, cx1, cy1, rgba(crate::canvas::geom::Color32::from_gray(0xE6)), corners);
        }
    }
}

/// `ui-egui`'s `paint_swatch`: the paint itself on a checkerboard, a 2 pt rounding, a hairline.
pub fn paint_swatch(window: &mut Window, bounds: Bounds<Pixels>, paint: &Paint, cx: &App) {
    let t = tokens();
    let x0 = f32::from(bounds.origin.x);
    let y0 = f32::from(bounds.origin.y);
    let w = f32::from(bounds.size.width).max(1.0);
    let h = f32::from(bounds.size.height).max(1.0);
    let (x1, y1) = (x0 + w, y0 + h);

    // `checker_rounded(&clip, rect, rect.width() / 4.0, 2)`
    checker_rounded(window, bounds, w / 4.0, 2.0);

    // `left_round` / `right_round`: only the outer edges round.
    let left = Corners { top_left: gpui::px(2.0), top_right: gpui::px(0.0), bottom_right: gpui::px(0.0), bottom_left: gpui::px(2.0) };
    let right = Corners { top_left: gpui::px(0.0), top_right: gpui::px(2.0), bottom_right: gpui::px(2.0), bottom_left: gpui::px(0.0) };

    match &paint.kind {
        PaintKind::Solid { color } => {
            // Left half opaque, right half with alpha (like 参考编辑器's swatches).
            let c = rgba(color32(color.with_alpha(color.a * paint.opacity)));
            let opaque = rgba(color32(color.with_alpha(1.0)));
            let mid = x0 + w / 2.0;
            quad_corners(window, x0, y0, mid, y1, opaque, left);
            quad_corners(window, mid, y0, x1, y1, c, right);
            if paint.opacity >= 0.999 && color.a >= 0.999 {
                quad(window, x0, y0, x1, y1, opaque, 2.0);
            }
        }
        PaintKind::Gradient(g) => {
            let n = 16;
            for i in 0..n {
                let c = rgba(color32(g.sample(i as f32 / (n - 1) as f32)));
                let sx0 = x0 + w * i as f32 / n as f32;
                let sx1 = x0 + w * (i + 1) as f32 / n as f32;
                // Only the ends round their outer corners; the seam between segments stays square.
                let corners = if i == 0 {
                    left
                } else if i == n - 1 {
                    right
                } else {
                    Corners::default()
                };
                quad_corners(window, sx0, y0, sx1, y1, c, corners);
            }
        }
        PaintKind::Image(_) | PaintKind::Pattern(_) => {
            let icon = if matches!(paint.kind, PaintKind::Image(_)) { Icon::Image } else { Icon::ViewGrid };
            quad(window, x0, y0, x1, y1, rgba(crate::canvas::geom::Color32::from_gray(0xB3)), 2.0);
            let inset = 2.0;
            draw_centred(
                window,
                Bounds {
                    origin: gpui::point(gpui::px(x0 + inset), gpui::px(y0 + inset)),
                    size: gpui::size(gpui::px(w - inset * 2.0), gpui::px(h - inset * 2.0)),
                },
                icon,
                rgba(crate::canvas::geom::Color32::WHITE),
                cx,
            );
        }
    }

    // `painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, t.border), Inside)`
    let mut frame = gpui::fill(bounds, gpui::transparent_black());
    frame.corner_radii = Corners::all(gpui::px(2.0));
    frame.border_widths = gpui::Edges::all(gpui::px(1.0));
    frame.border_color = rgba(t.border).into();
    window.paint_quad(frame);
}

/// A paint as the UI names it: `"#FF0000"`, `"#FF0000 50%"`, `"Gradient"`, `"Image"`,
/// `"Pattern"` (`ui-egui`'s `paint_name`).
pub fn paint_name(paint: &Paint) -> String {
    match &paint.kind {
        PaintKind::Solid { color } => {
            let hex = color.to_hex();
            let alpha = color.a * paint.opacity;
            if alpha < 0.999 { format!("{hex} {}%", (alpha * 100.0).round()) } else { hex }
        }
        PaintKind::Gradient(_) => tr("Gradient").into(),
        PaintKind::Image(_) => tr("Image").into(),
        PaintKind::Pattern(_) => tr("Pattern").into(),
    }
}

/// 参考编辑器 的两套色块形状（用户 2026-11-15）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwatchShape {
    /// 方形 `chit`：垂直中线分左右两半，左半不透明、右半随 alpha。
    #[default]
    Square,
    /// 圆形 `swatch`：45° 斜线分两半，左下不透明、右上随 alpha。
    Circle,
}

/// A paint swatch (with a checkerboard under transparent colours) — `ui-egui`'s `swatch`.
pub fn swatch(paint: Paint, size: f32) -> Swatch {
    Swatch { paint, size, shape: SwatchShape::Square, name: None, on_click: None }
}

/// A `size` × `size` swatch of `paint` (square by default).
pub struct Swatch {
    paint: Paint,
    size: f32,
    shape: SwatchShape,
    name: Option<SharedString>,
    on_click: Option<std::rc::Rc<dyn Fn(&mut Window, &mut gpui::App)>>,
}

impl Swatch {
    /// The field the swatch belongs to (its screen-reader name in `ui-egui`).
    pub fn named(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// 选形状，默认 [`SwatchShape::Square`]。
    pub fn shape(mut self, shape: SwatchShape) -> Self {
        self.shape = shape;
        self
    }

    /// 圆形色块的快捷写法。
    pub fn circle(self) -> Self {
        self.shape(SwatchShape::Circle)
    }

    /// Handle a click (the original's `Sense::click()`).
    pub fn on_click(mut self, f: impl Fn(&mut Window, &mut gpui::App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(f));
        self
    }
}

impl IntoElement for Swatch {
    type Element = gpui::Div;

    fn into_element(self) -> gpui::Div {
        let _name = self.name;
        let on_click = self.on_click;
        let shape = self.shape;
        let size = self.size;
        let t = tokens();
        let circle = shape == SwatchShape::Circle;
        // 方形 2 pt 圆角、圆形半边长；分割线：方形垂直（90°），圆形 45°。
        let radius = if circle { size / 2.0 } else { 2.0 };
        let angle = if circle { 45.0 } else { 90.0 };
        let (from, to) = match &self.paint.kind {
            PaintKind::Solid { color } => (rgba(color32(color.with_alpha(1.0))), rgba(color32(color.with_alpha(color.a * self.paint.opacity)))),
            PaintKind::Gradient(g) => (rgba(color32(g.sample(0.0))), rgba(color32(g.sample(1.0)))),
            _ => {
                let grey = rgba(crate::canvas::geom::Color32::from_gray(0xB3));
                (grey, grey)
            }
        };

        // 容器：一个矩形 / 圆，带描边和圆角，点击也挂在它身上。
        let mut el = div().relative().size(pt(size)).rounded(pt(radius)).border_1().border_color(rgba(t.border));
        // 棋盘位图铺满容器（`img` 的 shader 按元素自身圆角裁剪）。
        el = el.child(gpui::img(gpui::ImageSource::Render(checker_texture())).absolute().top_0().left_0().size_full().rounded(pt(radius)));
        // 两半颜色：0.499/0.501 硬色标，等于左右两个矩形各分一半（圆是 45° 斜线）。
        el = el.child(div().absolute().top_0().left_0().size_full().rounded(pt(radius)).bg(gpui::linear_gradient(
            angle,
            gpui::linear_color_stop(gpui::Hsla::from(from), 0.499),
            gpui::linear_color_stop(gpui::Hsla::from(to), 0.501),
        )));
        if matches!(self.paint.kind, PaintKind::Image(_) | PaintKind::Pattern(_)) {
            let icon = if matches!(self.paint.kind, PaintKind::Image(_)) { Icon::Image } else { Icon::ViewGrid };
            el = el.child(div().absolute().size_full().flex().items_center().justify_center().child(crate::widgets::icon::paint(
                icon,
                rgba(crate::canvas::geom::Color32::WHITE),
                size * 0.7,
            )));
        }
        if let Some(f) = on_click {
            el = el.on_mouse_down(gpui::MouseButton::Left, move |_: &gpui::MouseDownEvent, window, cx| f(window, cx));
        }
        el
    }
}

/// 圆形色块的棋盘位图（4×4 格、每格 8 px），首次使用时生成并缓存。
fn checker_texture() -> std::sync::Arc<gpui::RenderImage> {
    thread_local! {
        static CHECKER: std::cell::RefCell<Option<std::sync::Arc<gpui::RenderImage>>> = const { std::cell::RefCell::new(None) };
    }
    CHECKER.with(|cell| {
        cell.borrow_mut()
            .get_or_insert_with(|| {
                let (n, px) = (4u32, 8u32);
                let side = n * px;
                let mut data = vec![0u8; (side * side * 4) as usize];
                for y in 0..side {
                    for x in 0..side {
                        let v: u8 = if ((x / px) + (y / px)) % 2 == 0 { 0xFF } else { 0xE6 };
                        let i = ((y * side + x) * 4) as usize;
                        data[i..i + 4].copy_from_slice(&[v, v, v, 0xFF]);
                    }
                }
                crate::canvas::render_worker::wrap_rgba(side, side, data)
            })
            .clone()
    })
}

/// A plain axis-aligned quad, `radius` on all four corners.
fn quad(window: &mut Window, x0: f32, y0: f32, x1: f32, y1: f32, color: gpui::Rgba, radius: f32) {
    quad_corners(window, x0, y0, x1, y1, color, Corners::all(gpui::px(radius)));
}

/// A quad with per-corner radii.
fn quad_corners(window: &mut Window, x0: f32, y0: f32, x1: f32, y1: f32, color: gpui::Rgba, corners: Corners<Pixels>) {
    let mut q = gpui::fill(
        Bounds { origin: gpui::point(gpui::px(x0), gpui::px(y0)), size: gpui::size(gpui::px((x1 - x0).max(0.0)), gpui::px((y1 - y0).max(0.0))) },
        color,
    );
    q.corner_radii = corners;
    window.paint_quad(q);
}
