//! 图层轮廓（照搬 `ui-egui/src/canvas/outline.rs`）。
//!
//! hover / 选中 / 蒙版 / 布尔操作数的轮廓：椭圆、矢量、多边形、星形走**真实几何轮廓**
//! （`figcraft_render::node_outline` 展平后逐段画），其余走包围盒四边形。

use std::sync::Arc;

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::Point;

use super::geom::*;
use super::painter::Painter;
use crate::app::CanvasApp;
use crate::theme::{COMPONENT, SLOT};

pub(super) fn stroke(c: Color32, w: f32) -> Stroke {
    Stroke::new(w, c)
}

/// 一个文字层摆在画布上的排版：光标与选中的字符画在哪里。
pub(crate) struct TextOnScreen {
    pub layout: Arc<figcraft_text::TextLayout>,
    /// 文字有多少个字符。
    pub chars: usize,
    /// 排版在框内的竖直偏移（竖直对齐）。
    dy: f64,
    world: figcraft_geom::Affine,
}

impl TextOnScreen {
    /// `doc` 里图层 `id` 的文字（None：不是文字层）。
    pub(crate) fn of(doc: &Document, id: NodeId) -> Option<Self> {
        let n = doc.node(id)?;
        let NodeKind::Text(t) = &n.kind else { return None };
        let (layout, dy) = figcraft_text::layout_node(n)?;
        Some(Self { layout, chars: t.char_count(), dy, world: doc.world_transform(id) })
    }

    /// 排版点 `(x, y)` 在屏幕上的位置（沿路径排的文字也走这条路）。
    pub(crate) fn at(&self, app: &CanvasApp, x: f64, y: f64) -> Pos2 {
        let q = self.layout.warp_point(x, y);
        let p = self.world * Point::new(q.x, q.y + self.dy);
        app.to_screen(p.x, p.y)
    }

    /// 字符 `a..b` 的高亮：每行一个屏幕四边形。
    pub(crate) fn selection(&self, app: &CanvasApp, a: usize, b: usize) -> Vec<[Pos2; 4]> {
        let rects = self.layout.selection_rects(a, b);
        rects.iter().map(|r| [self.at(app, r.x0, r.y0), self.at(app, r.x1, r.y0), self.at(app, r.x1, r.y1), self.at(app, r.x0, r.y1)]).collect()
    }

    /// 第 `index` 个字符前的光标：屏幕上的上下两点。
    pub(crate) fn caret(&self, app: &CanvasApp, index: usize, zoom: f64) -> [Pos2; 2] {
        let (cx, li) = self.layout.caret(index);
        match self.layout.lines.get(li).or(self.layout.lines.last()) {
            Some(line) => [self.at(app, cx, line.top), self.at(app, cx, line.top + line.height)],
            None => {
                let p = self.at(app, 0.0, 0.0);
                [p, p + vec2(0.0, 14.0 * zoom as f32)]
            }
        }
    }

    /// 页面坐标 `p` 落在第几个字符前（和引擎 `text_index_at` 同一条命中规则）。
    pub(crate) fn index_at(&self, p: Point) -> usize {
        let local = self.world.inverse() * p;
        self.layout.hit(local.x, local.y - self.dy)
    }

    /// 第 `index` 个字符前的光标窗口矩形：IME 候选窗按这个矩形定位。
    ///
    /// `app.to_screen` 给的是画布局部坐标（`canvas.rect` 原点恒为 0），而
    /// `EntityInputHandler::bounds_for_range` 要的是窗口坐标；`origin` 就是画布
    /// 在窗口里的原点（GPUI 传进来的 `element_bounds.origin`）。
    pub(crate) fn caret_bounds(
        &self,
        app: &CanvasApp,
        index: usize,
        zoom: f64,
        origin: gpui::Point<gpui::Pixels>,
    ) -> gpui::Bounds<gpui::Pixels> {
        let [a, b] = self.caret(app, index, zoom);
        let (x0, y0, x1, y1) = (a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y));
        gpui::Bounds {
            origin: gpui::point(gpui::px(x0) + origin.x, gpui::px(y0) + origin.y),
            size: gpui::size(gpui::px((x1 - x0).max(1.0)), gpui::px((y1 - y0).max(1.0))),
        }
    }
}

/// 节点在屏幕空间的定向轮廓（四角，跟随旋转/镜像）。
pub(crate) fn node_quad(app: &CanvasApp, doc: &Document, id: NodeId) -> Option<[Pos2; 4]> {
    let n = doc.node(id)?;
    let w = doc.world_transform(id);
    let corners = [Point::new(0.0, 0.0), Point::new(n.width, 0.0), Point::new(n.width, n.height), Point::new(0.0, n.height)];
    Some(corners.map(|c| {
        let p = w * c;
        app.to_presented(p.x, p.y)
    }))
}

/// 一条闭合四边形轮廓。
pub(super) fn quad(painter: &mut Painter, q: [Pos2; 4], s: Stroke) {
    painter.add(Shape::closed_line(q.to_vec(), s));
}

/// 轮廓颜色：组件 / 组件集 / 实例（及其内部所有图层）用组件紫，其余用画布叠加色。
pub(super) fn outline_colour(doc: &Document, id: NodeId, green: Color32) -> Color32 {
    use figcraft_engine::components::{owning_component, owning_instance};
    if figcraft_engine::components::is_slot(doc, id) {
        return SLOT;
    }
    let set = matches!(doc.node(id).map(|n| &n.kind), Some(NodeKind::ComponentSet { .. }));
    if set || owning_component(doc, id).is_some() || owning_instance(doc, id).is_some() { COMPONENT } else { green }
}

/// 形状类节点（椭圆/矢量/多边形/星形/布尔组）的轮廓路径，屏幕坐标。
fn outline_shape(app: &CanvasApp, doc: &Document, id: NodeId) -> Option<Vec<Vec<Pos2>>> {
    let n = doc.node(id)?;
    if !matches!(
        n.kind,
        NodeKind::Ellipse { .. } | NodeKind::Vector { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::BooleanOp { .. }
    ) {
        return None;
    }
    let (path, _) = figcraft_render::node_outline(doc, n)?;
    let w = doc.world_transform(id);
    let mut out = Vec::new();
    let mut cur: Vec<Pos2> = Vec::new();
    // 展平容差按屏幕像素给（0.5px），缩放越大越精细。
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    figcraft_geom::kurbo::flatten(path.iter(), 0.5 / zoom, |el| {
        use figcraft_geom::kurbo::PathEl::*;
        match el {
            MoveTo(p) => {
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                let q = w * p;
                cur.push(app.to_screen(q.x, q.y));
            }
            LineTo(p) => {
                let q = w * p;
                cur.push(app.to_screen(q.x, q.y));
            }
            ClosePath => {
                if let Some(f) = cur.first().copied() {
                    cur.push(f);
                }
            }
            _ => {}
        }
    });
    if cur.len() > 1 {
        out.push(cur);
    }
    Some(out)
}

fn debug_outline() -> bool {
    std::env::var_os("FIGCRAFT_DEBUG_OUTLINE").is_some()
}

/// 图层在画布上的轮廓：形状类走真实几何，其余走包围盒。
pub(super) fn draw_outline(app: &CanvasApp, painter: &mut Painter, doc: &Document, id: NodeId, s: Stroke) {
    if debug_outline() {
        let n = doc.node(id);
        let wb = doc.world_bounds(id);
        let q = node_quad(app, doc, id);
        eprintln!(
            "[outline] id={:?} name={:?} kind={:?} page_bounds={:?} screen_q={:?} shift={:?} ppp={}",
            id,
            n.map(|n| n.name.as_str()),
            n.map(|n| &n.kind),
            wb,
            q,
            Vec2::ZERO,
            app.ppp
        );
    }
    match outline_shape(app, doc, id) {
        Some(lines) => {
            for l in lines {
                painter.add(Shape::line(l, s));
            }
        }
        None => {
            if let Some(q) = node_quad(app, doc, id) {
                // 轴对齐的矩形：用一条闭合路径画直角边框，线宽原样使用调用方的值。
                // 四边和四角由同一次描边生成，不会左右分别取整。
                if let Some(r) = axis_aligned(q)
                    && outline_ring(painter, r, s.width, app.ppp, Vec2::ZERO, s.color)
                {
                    return;
                }
                quad(painter, q, s);
            }
        }
    }
}

/// 四边形若轴对齐，返回它的矩形（旋转过的不算）。
pub(super) fn axis_aligned(q: [Pos2; 4]) -> Option<Rect> {
    const EPS: f32 = 0.01;
    let [a, b, c, d] = q;
    let horizontal = (a.y - b.y).abs() < EPS && (d.y - c.y).abs() < EPS;
    let vertical = (a.x - d.x).abs() < EPS && (b.x - c.x).abs() < EPS;
    (horizontal && vertical).then(|| Rect::from_min_max(pos2(a.x.min(c.x), a.y.min(c.y)), pos2(a.x.max(c.x), a.y.max(c.y))))
}

/// 沿矩形边界画一条直角闭合环，线宽严格使用调用方给的值。
///
/// 环的四条边由同一条闭合路径一次生成，不再经过 `paint_quad` 的四边 border：
/// 后者在 1.25/1.5/1.75 这类非整数缩放下会把左边取到描边像素上、右边取到外侧，
/// 同一圈线左右不对称。路径中心线严格等于 `r`，所以四边、四角和四个手柄共用同一
/// 坐标，平移时也不会因为 floor/ceil 每隔一个像素突然跳一下。
/// 选中线宽是 1.0，悬停线宽是 2.0，原样保留。
/// `shift` 是贴图对齐设备像素带来的亚像素位移。`ppp` 只用于调试输出。
pub(super) fn outline_ring(painter: &mut Painter, r: Rect, width: f32, ppp: f32, shift: Vec2, col: Color32) -> bool {
    if !(ppp.is_finite() && ppp > 0.0 && width.is_finite() && width > 0.0) {
        return false;
    }
    let r = r.translate(shift);
    if !(r.min.x.is_finite() && r.min.y.is_finite() && r.max.x.is_finite() && r.max.y.is_finite()) {
        return false;
    }
    painter.rect_ring(r, width, col);
    if debug_outline() {
        let d = width * ppp;
        eprintln!(
            "[outline_ring] center_screen={:?}..{:?} width={} ppp={} device_width={} outer_screen={:?}..{:?}",
            (r.min.x, r.min.y),
            (r.max.x, r.max.y),
            width,
            ppp,
            d,
            (r.min.x - width / 2.0, r.min.y - width / 2.0),
            (r.max.x + width / 2.0, r.max.y + width / 2.0)
        );
    }
    true
}

/// Draw a line on whole device pixels: a 1 pt stroke at a fractional screen position otherwise
/// lands across two pixels and looks alternately thick and thin as the canvas pans.
pub(super) fn crisp_line(painter: &mut Painter, a: Pos2, b: Pos2, width: f32, ppp: f32, col: Color32) {
    if !(ppp.is_finite() && ppp > 0.0 && width.is_finite() && width > 0.0) {
        painter.line_segment([a, b], stroke(col, width));
        return;
    }
    let t = (width * ppp).round().clamp(1.0, 64.0) / ppp;
    let snap = |v: f32| (v * ppp).round() / ppp;
    if (a.x - b.x).abs() <= f32::EPSILON {
        let x = snap((a.x + b.x) * 0.5 - t * 0.5);
        let (y0, y1) = (snap(a.y.min(b.y)), snap(a.y.max(b.y)));
        painter.rect_filled(Rect::from_min_max(pos2(x, y0), pos2(x + t, y1)), CornerRadius::ZERO, col);
        return;
    }
    if (a.y - b.y).abs() <= f32::EPSILON {
        let y = snap((a.y + b.y) * 0.5 - t * 0.5);
        let (x0, x1) = (snap(a.x.min(b.x)), snap(a.x.max(b.x)));
        painter.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + t)), CornerRadius::ZERO, col);
        return;
    }
    painter.line_segment([pos2(snap(a.x), snap(a.y)), pos2(snap(b.x), snap(b.y))], stroke(col, t));
}

#[cfg(test)]
mod crisp_tests {
    use super::*;
    use crate::canvas::painter::Op;

    /// 取唯一一条闭合矩形环：`(rect, width, color)`。
    fn one_ring(p: &Painter) -> (Rect, f32, Color32) {
        assert_eq!(p.ops.len(), 1, "the outline must be one closed ring");
        match &p.ops[0] {
            Op::Ring { rect, width, color } => (*rect, *width, *color),
            other => panic!("expected one ring, got {other:?}"),
        }
    }

    /// 路径中心线必须正好落在矩形边上：四边、四角、四个手柄共用这一条线。
    fn assert_centered_on(rect: Rect, r: Rect) {
        assert!((rect.min.x - r.min.x).abs() < 1e-4, "left edge is off: {rect:?} vs {r:?}");
        assert!((rect.min.y - r.min.y).abs() < 1e-4, "top edge is off: {rect:?} vs {r:?}");
        assert!((rect.max.x - r.max.x).abs() < 1e-4, "right edge is off: {rect:?} vs {r:?}");
        assert!((rect.max.y - r.max.y).abs() < 1e-4, "bottom edge is off: {rect:?} vs {r:?}");
    }

    /// 选中框与悬停框的线宽必须原样保留：选中 1.0，悬停 2.0。
    #[test]
    fn selected_and_hover_widths_are_kept() {
        let ppp = 1.75f32;
        for width in [1.0f32, 2.0] {
            let mut p = Painter::new(Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 200.0)));
            let r = Rect::from_min_max(pos2(10.37, 20.61), pos2(110.83, 80.42));
            assert!(outline_ring(&mut p, r, width, ppp, Vec2::ZERO, Color32::WHITE));
            let (rect, got, color) = one_ring(&p);
            assert_eq!(color, Color32::WHITE);
            assert!((got - width).abs() < 1e-6, "requested {width}, got {got}");
            assert_centered_on(rect, r);
        }
    }

    /// 画布平移到任何小数坐标时，中心线都严格等于物体边缘，框不会整像素地跳。
    #[test]
    fn the_ring_hugs_the_content_while_the_canvas_pans() {
        let ppp = 1.75f32;
        let base = Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 80.0));
        for step in 0..40 {
            let off = step as f32 / 40.0;
            let r = base.translate(vec2(off, off * 0.37));
            let mut p = Painter::new(Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 200.0)));
            assert!(outline_ring(&mut p, r, 1.0, ppp, Vec2::ZERO, Color32::WHITE), "offset {off}");
            let (rect, width, _) = one_ring(&p);
            assert_centered_on(rect, r);
            assert!((width - 1.0).abs() < 1e-6, "offset {off}: width changed to {width}");
        }
    }

    /// 贴图吸附位移必须带着整圈一起走：位移后中心线落在位移后的边缘上。
    #[test]
    fn the_ring_follows_the_content_shift() {
        let ppp = 2.0f32;
        let mut p = Painter::new(Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 200.0)));
        let r = Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 80.0));
        let shift = vec2(0.5, 0.5);
        assert!(outline_ring(&mut p, r, 2.0, ppp, shift, Color32::WHITE));
        let (rect, width, _) = one_ring(&p);
        assert_centered_on(rect, r.translate(shift));
        assert!((width - 2.0).abs() < 1e-6, "hover width must stay 2.0");
    }

    /// 旋转过的四边形不做吸附，退回描路径（否则会画成轴对齐的框）。
    #[test]
    fn a_rotated_quad_is_not_taken_for_a_rectangle() {
        let square = [pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(10.0, 10.0), pos2(0.0, 10.0)];
        assert!(axis_aligned(square).is_some());
        let turned = [pos2(0.0, 5.0), pos2(5.0, 0.0), pos2(10.0, 5.0), pos2(5.0, 10.0)];
        assert!(axis_aligned(turned).is_none());
    }

    /// 非法缩放/线宽不画（返回 false，调用方退回描路径）。
    #[test]
    fn absurd_scales_fall_back() {
        let mut p = Painter::new(Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0)));
        let r = Rect::from_min_max(pos2(1.0, 1.0), pos2(9.0, 9.0));
        assert!(!outline_ring(&mut p, r, 1.0, 0.0, Vec2::ZERO, Color32::WHITE));
        assert!(!outline_ring(&mut p, r, 1.0, f32::NAN, Vec2::ZERO, Color32::WHITE));
        assert!(!outline_ring(&mut p, r, 0.0, 1.0, Vec2::ZERO, Color32::WHITE));
        assert!(p.ops.is_empty());
    }

    /// Grid lines and guides must stay the same thickness at fractional pan offsets.
    #[test]
    fn a_crisp_line_lands_on_whole_device_pixels() {
        let ppp = 1.75f32;
        let mut p = Painter::new(Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 200.0)));
        crisp_line(&mut p, pos2(10.37, 4.0), pos2(10.37, 90.0), 1.0, ppp, Color32::WHITE);
        crisp_line(&mut p, pos2(4.0, 20.61), pos2(90.0, 20.61), 1.0, ppp, Color32::WHITE);
        let q: Vec<Rect> = p
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Rect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(q.len(), 2);
        let t = (1.0f32 * ppp).round() / ppp;
        for band in &q {
            for v in [band.min.x, band.min.y, band.max.x, band.max.y] {
                let d = v * ppp;
                assert!((d - d.round()).abs() < 1e-3, "{v} is not on a device pixel");
            }
            let thick = (band.max.x - band.min.x).min(band.max.y - band.min.y);
            assert!((thick - t).abs() < 1e-4, "thickness {thick} is not {t}");
        }
    }
}

