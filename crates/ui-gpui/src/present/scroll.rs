//! 滚动 / 惯性 / 橡皮筋 / 视口适配 —— `ui-egui/src/present/scroll.rs` 的 GPUI 移植。
//!
//! 纯几何函数 + 挂在 [`PresentState`](super::PresentState) 上的小方法；不碰渲染与事件。

use figcraft_doc::proto::{Overflow, PrototypeSettings, ScrollPosition};
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::transform_rect_bbox;

use super::{PresentState, Scroller};

/// 拖拽结束后惯性衰减速度（1/s；滑行距离 = 速度 / decay）。
pub const DECAY_DRAG: f64 = 2.5;
/// 滚轮滑行衰减速度（1/s）。
pub const DECAY_WHEEL: f64 = 8.0;
/// 越界回弹的弹簧刚度（1/s，临界阻尼）。
const SPRING: f64 = 16.0;
/// 范围内惯性低于此速度就停（单位/秒）。
const FLING_STOP: f64 = 6.0;
/// 滚轮停下时若速度高于此值则继续滑行（屏幕点/秒）。
pub const COAST_MIN: f64 = 40.0;
/// 拖拽越界最多拉出多少（屏幕点）。
pub const RUBBER_PTS: f64 = 240.0;

impl PresentState {
    /// `t` 当前偏移（画框的滚动，或整窗的平移）。
    pub fn offset(&self, t: Scroller) -> (f64, f64) {
        match t {
            Scroller::Frame(f) => self.scroll.get(&f).copied().unwrap_or((0.0, 0.0)),
            Scroller::View => self.pan.unwrap_or((self.pan_range.0 / 2.0, 0.0)),
        }
    }

    /// `t` 能滚多远（`base`：未应用滚动偏移的文档）。
    pub fn limits(&self, base: &Document, t: Scroller) -> (f64, f64) {
        match t {
            Scroller::Frame(f) => self.range(base, f),
            Scroller::View => self.pan_range,
        }
    }

    pub fn set_offset(&mut self, t: Scroller, v: (f64, f64)) {
        if !v.0.is_finite() || !v.1.is_finite() {
            return;
        }
        match t {
            Scroller::Frame(f) => {
                if self.scroll.get(&f) != Some(&v) {
                    self.scroll.insert(f, v);
                    self.rev += 1;
                }
            }
            Scroller::View => self.pan = Some(v),
        }
    }

    /// 推进惯性；仍在动返回 true。
    pub fn step_fling(&mut self, now: f64) -> bool {
        let Some(mut f) = self.fling.take() else { return false };
        let Some(base) = self.design.as_ref() else { return false };
        let r = self.limits(base, f.target);
        let x = self.offset(f.target);
        let dt = (now - f.at).clamp(0.0, 0.25);
        f.at = now;
        let (nx, vx, done_x) = fling_axis(x.0, f.vel.0, r.0, dt, f.decay, f.bounce);
        let (ny, vy, done_y) = fling_axis(x.1, f.vel.1, r.1, dt, f.decay, f.bounce);
        self.set_offset(f.target, (nx, ny));
        f.vel = (vx, vy);
        if done_x && done_y {
            return false;
        }
        self.fling = Some(f);
        true
    }

    /// `frame` 能滚多远：被演示的画框以设备屏幕为视口，其它画框以自身为视口。
    pub fn range(&self, doc: &Document, frame: NodeId) -> (f64, f64) {
        let vp = if frame == self.current { viewport(doc, frame) } else { None };
        scroll_range_in(doc, frame, vp)
    }

    /// “Scroll to” 的目标：内侧可滚动画框由内向外把层（连同它的画框）带到左上角。
    pub fn scroll_to_layer(&self, doc: &Document, dest: NodeId, offset: [f64; 2]) -> Vec<(NodeId, (f64, f64), (f64, f64))> {
        let mut out = Vec::new();
        let mut target = dest;
        let mut off = offset.map(|v| if v.is_finite() { v } else { 0.0 });
        for f in doc.ancestors(dest) {
            let (rx, ry) = self.range(doc, f);
            let Some(tn) = doc.node(target) else { break };
            if rx <= 0.0 && ry <= 0.0 {
                continue;
            }
            let rel = doc.world_transform(f).inverse() * doc.world_transform(target);
            let b = transform_rect_bbox(rel, tn.local_rect());
            let from = self.scroll.get(&f).copied().unwrap_or((0.0, 0.0));
            let to = clamp_scroll((if rx > 0.0 { b.x0 - off[0] } else { from.0 }, if ry > 0.0 { b.y0 - off[1] } else { from.1 }), (rx, ry));
            out.push((f, from, to));
            target = f;
            off = [0.0, 0.0];
        }
        out
    }
}

/// `frame` 内容在视口 `viewport`（页单位，自画框左上）下的滚动范围。设了视口时画框自身
/// 也算内容：画框比屏幕大就按整框滚，与它的 overflow 设置无关（Figma 顶层画框同款）。
pub fn scroll_range_in(doc: &Document, frame: NodeId, viewport: Option<(f64, f64)>) -> (f64, f64) {
    let Some(n) = doc.node(frame) else { return (0.0, 0.0) };
    if n.overflow == Overflow::None && viewport.is_none() {
        return (0.0, 0.0);
    }
    let (mut x1, mut y1) = (0.0f64, 0.0f64);
    for c in &n.children {
        let Some(cn) = doc.node(*c) else { continue };
        if !cn.visible || cn.scroll_position == ScrollPosition::Fixed {
            continue;
        }
        let b = transform_rect_bbox(cn.transform, cn.local_rect());
        x1 = x1.max(b.x1);
        y1 = y1.max(b.y1);
    }
    if let Some(l) = &n.layout {
        x1 += l.padding[1];
        y1 += l.padding[2];
    }
    if !matches!(n.overflow, Overflow::Horizontal | Overflow::Both) {
        x1 = 0.0;
    }
    if !matches!(n.overflow, Overflow::Vertical | Overflow::Both) {
        y1 = 0.0;
    }
    let (vw, vh) = match viewport {
        Some(v) => {
            x1 = x1.max(n.width);
            y1 = y1.max(n.height);
            v
        }
        None => (n.width, n.height),
    };
    let (rx, ry) = ((x1 - vw).max(0.0), (y1 - vh).max(0.0));
    (if rx.is_finite() { rx } else { 0.0 }, if ry.is_finite() { ry } else { 0.0 })
}

/// `(x, y)` 夹在 `0..=range` 内（非有限值取 0）。
pub fn clamp_scroll((x, y): (f64, f64), (rx, ry): (f64, f64)) -> (f64, f64) {
    let c = |v: f64, r: f64| if v.is_finite() && r.is_finite() { v.clamp(0.0, r.max(0.0)) } else { 0.0 };
    (c(x, rx), c(y, ry))
}

/// 一个轴上的惯性：从偏移 `x`、速度 `v`（单位/秒）在 `0..=r` 中走 `dt` 秒。
pub fn fling_axis(x: f64, v: f64, r: f64, dt: f64, decay: f64, bounce: bool) -> (f64, f64, bool) {
    if !(x.is_finite() && v.is_finite() && r.is_finite() && dt.is_finite() && decay > 0.0) || r <= 0.0 {
        return (if r.is_finite() && x.is_finite() { x.clamp(0.0, r.max(0.0)) } else { 0.0 }, 0.0, true);
    }
    let dt = dt.clamp(0.0, 1.0);
    let steps = (dt * 240.0).ceil().clamp(1.0, 64.0);
    let h = dt / steps;
    let (mut x, mut v) = (x, v.clamp(-1e6, 1e6));
    for _ in 0..steps as usize {
        if (0.0..=r).contains(&x) {
            let e = (-decay * h).exp();
            x += v * (1.0 - e) / decay;
            v *= e;
            if !bounce && !(0.0..=r).contains(&x) {
                x = x.clamp(0.0, r);
                v = 0.0;
            }
        } else {
            let end = x.clamp(0.0, r);
            let (d, e) = (x - end, (-SPRING * h).exp());
            let c = v + SPRING * d;
            x = end + (d + c * h) * e;
            v = (v - SPRING * c * h) * e;
        }
    }
    if !x.is_finite() || !v.is_finite() {
        return (0.0, 0.0, true);
    }
    let end = x.clamp(0.0, r);
    if x == end && v.abs() < FLING_STOP {
        return (x, 0.0, true);
    }
    if x != end && (x - end).abs() < 0.5 && v.abs() < FLING_STOP * 4.0 {
        return (end, 0.0, true);
    }
    (x, v, false)
}

/// 拖到偏移 `x` 时在范围 `0..=r` 中看到的位置：越界越拉越紧，最多越过 `limit`。
pub fn rubber_band(x: f64, r: f64, limit: f64) -> f64 {
    if !x.is_finite() || !r.is_finite() || r <= 0.0 || !limit.is_finite() || limit <= 0.0 {
        return if x.is_finite() && r.is_finite() { x.clamp(0.0, r.max(0.0)) } else { 0.0 };
    }
    let band = |over: f64| limit * (1.0 - 1.0 / (over * 0.55 / limit + 1.0));
    if x < 0.0 {
        -band(-x)
    } else if x > r {
        r + band(x - r)
    } else {
        x
    }
}

/// `id` 所在页的原型设置（没有就用默认）。
pub fn page_settings(doc: &Document, id: NodeId) -> PrototypeSettings {
    doc.page_of(id)
        .and_then(|pg| doc.node(pg))
        .and_then(|n| match &n.kind {
            NodeKind::Page { prototype, .. } => Some(prototype.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// 顶层 `frame` 被页面设备屏幕显示的部分（页单位）。没有设备时为 None（整框显示）。
pub fn viewport(doc: &Document, frame: NodeId) -> Option<(f64, f64)> {
    let d = page_settings(doc, frame).device()?;
    let n = doc.node(frame)?;
    Some((n.width.min(d.width), n.height.min(d.height)))
}

/// 把 `frame` 改成屏幕 `(w, h)` 大小；固定层按约束贴边（拉伸约束保持尺寸）。
pub fn fit_viewport(doc: &mut Document, frame: NodeId, (w, h): (f64, f64)) {
    use figcraft_doc::props::{HConstraint, VConstraint};
    let Some(n) = doc.node(frame) else { return };
    let (dw, dh) = (w - n.width, h - n.height);
    if !dw.is_finite() || !dh.is_finite() || (dw == 0.0 && dh == 0.0) {
        return;
    }
    let fixed: Vec<NodeId> =
        n.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|c| c.scroll_position == ScrollPosition::Fixed)).collect();
    for c in fixed {
        if let Some(m) = doc.node_mut(c) {
            let dx = match m.constraints.horizontal {
                HConstraint::Right => dw,
                HConstraint::Center => dw / 2.0,
                _ => 0.0,
            };
            let dy = match m.constraints.vertical {
                VConstraint::Bottom => dh,
                VConstraint::Center => dh / 2.0,
                _ => 0.0,
            };
            m.transform = figcraft_geom::Affine::translate((dx, dy)) * m.transform;
        }
    }
    if let Some(m) = doc.node_mut(frame) {
        m.width = w;
        m.height = h;
    }
}

/// 按滚动偏移移动可滚画框的子层：固定层不动，粘性层滚到边缘后停住。
pub fn apply_scroll(doc: &Document, scroll: &std::collections::BTreeMap<NodeId, (f64, f64)>) -> Document {
    use figcraft_geom::Affine;
    let mut out = doc.clone();
    let sticky = |pos: f64, s: f64| if pos >= 0.0 && pos - s < 0.0 { -pos } else { -s };
    for (&frame, &(sx, sy)) in scroll {
        if !sx.is_finite() || !sy.is_finite() || (sx == 0.0 && sy == 0.0) {
            continue;
        }
        let Some(n) = doc.node(frame) else { continue };
        for &c in &n.children {
            let Some(cn) = doc.node(c) else { continue };
            let (dx, dy) = match cn.scroll_position {
                ScrollPosition::Fixed => continue,
                ScrollPosition::Scrolls => (-sx, -sy),
                ScrollPosition::Sticky => {
                    let b = transform_rect_bbox(cn.transform, cn.local_rect());
                    (sticky(b.x0, sx), sticky(b.y0, sy))
                }
            };
            if let Some(m) = out.node_mut(c) {
                m.transform = Affine::translate((dx, dy)) * m.transform;
            }
        }
    }
    out
}

/// 固定层要画（也要接点击）在滚动内容之上：每个画框的固定子层挪到最上，保持相对顺序。
pub fn raise_fixed(doc: &mut Document) {
    let fixed = |doc: &Document, c: &NodeId| doc.node(*c).is_some_and(|c| c.scroll_position == ScrollPosition::Fixed);
    let moves: Vec<(NodeId, Vec<NodeId>)> = doc
        .nodes()
        .filter(|n| !matches!(n.kind, NodeKind::Page { .. }) && n.children.iter().any(|c| fixed(doc, c)))
        .filter_map(|n| {
            let (top, mut order): (Vec<NodeId>, Vec<NodeId>) = n.children.iter().partition(|c| fixed(doc, c));
            order.extend(top);
            (order != n.children).then_some((n.id, order))
        })
        .collect();
    for (id, order) in moves {
        if let Some(n) = doc.node_mut(id) {
            n.children = order;
        }
    }
}

/// 屏幕点 `q` 下（`disp` 中 `root` 子树内）能滚动的画框，最内层在前。
pub fn scrollers_at(p: &PresentState, base: &Document, disp: &Document, root: NodeId, place: super::layout::Place, q: (f64, f64)) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = disp
        .subtree(root)
        .into_iter()
        .filter(|f| disp.node(*f).is_some_and(|n| n.visible && (n.overflow != Overflow::None || *f == root)))
        .filter(|f| {
            disp.world_bounds(*f).is_some_and(|b| {
                let (x, y) = (f64::from(place.pt(b.x0, b.y0).0), f64::from(place.pt(b.x0, b.y0).1));
                let (x1, y1) = (f64::from(place.pt(b.x1, b.y1).0), f64::from(place.pt(b.x1, b.y1).1));
                q.0 >= x && q.0 <= x1 && q.1 >= y && q.1 <= y1
            })
        })
        .filter(|f| p.range(base, *f) != (0.0, 0.0))
        .collect();
    out.reverse();
    out
}

/// 屏幕点 → `t` 的单位（画框为页单位，整窗为屏幕点）。
pub fn units_per_point(t: Scroller, scale: f64) -> f64 {
    match t {
        Scroller::Frame(_) => 1.0 / scale.max(1e-6),
        Scroller::View => 1.0,
    }
}
