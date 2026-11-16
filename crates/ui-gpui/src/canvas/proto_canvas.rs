//! 原型连线叠加层（照搬 `ui-egui/src/canvas/proto_canvas.rs`）：
//! Prototype 标签下把有交互的图层连到它作用的 frame —— 从图层画一条曲线到目标、选中层右侧
//! 一个「+」连接手柄（拖出去新建 Navigate 交互）、点曲线打开/收起右栏的交互编辑器；
//! 以及手动定位 overlay 的 ghost（交互编辑器开着 Manual Open overlay 时，画布上显示它打开
//! 的位置，可拖动改偏移）。
//!
//! GPUI 侧没有 egui 的 `ctx.data`，跨帧的拖拽状态放在 `CanvasApp::proto` 里；
//! 编辑器的开合状态由右栏 `RightFields::proto_open` 持有，每帧同步进 `CanvasApp::proto_editor`。

use figcraft_doc::proto::{Action, MAX_ACTION_DEPTH, OverlayPosition};
use figcraft_doc::{Document, NodeId};
use serde_json::{Value, json};

use super::geom::*;
use super::painter::Painter;
use crate::app::CanvasApp;

const HANDLE_R: f32 = 7.0;
/// ghost 拖动整段算一步撤销（与原实现同一标签）。
const LABEL: &str = "Edit prototype action";

/// 从图层手柄拖出的一条连接。
#[derive(Clone, Copy)]
pub struct Drag {
    source: NodeId,
    from: Pos2,
    pos: Pos2,
}

/// Manual overlay 的编辑槽（交互 `k` 里路径为 `path` 的那个动作）。
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub node: NodeId,
    pub k: usize,
    pub path: Vec<usize>,
}

/// 正在拖动的 ghost：按下时的动作、抓住的页面点、起始/最后一次偏移。
#[derive(Clone)]
pub struct GhostDrag {
    slot: Slot,
    action: Action,
    grab: [f64; 2],
    start: [f64; 2],
    last: [f64; 2],
}

/// 原型连线的跨帧状态。
#[derive(Default)]
pub struct State {
    pub drag: Option<Drag>,
    pub ghost: Option<GhostDrag>,
}

/// 一次按下的结果。
pub enum Press {
    /// 与本模块无关，继续走画布其它输入。
    Ignored,
    /// 已消费（起拖 / 点中曲线）。
    Consumed,
    /// 关掉交互编辑器，然后继续走画布其它输入（原文最后那条分支）。
    CloseEditor,
}

struct Noodle {
    source: NodeId,
    index: usize,
    pts: [Pos2; 4],
}

/// 屏幕矩形（页面坐标的轴对齐包围盒）。
fn screen_rect(app: &CanvasApp, doc: &Document, id: NodeId) -> Option<Rect> {
    let b = doc.world_bounds(id)?;
    Some(Rect::from_min_max(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1)))
}

fn left_center(r: Rect) -> Pos2 {
    pos2(r.min.x, r.center().y)
}

fn right_center(r: Rect) -> Pos2 {
    pos2(r.max.x, r.center().y)
}

/// 从 `a`（图层）到 `b`（frame）的曲线：从朝向目标的那一侧离开。
fn curve_between(a: Rect, b: Rect) -> [Pos2; 4] {
    let right = b.center().x >= a.center().x;
    let (p0, p3) = if right { (right_center(a), left_center(b)) } else { (left_center(a), right_center(b)) };
    let s = if right { 1.0 } else { -1.0 };
    let dx = ((p3.x - p0.x).abs() * 0.5).max(40.0) * s;
    [p0, p0 + vec2(dx, 0.0), p3 - vec2(dx, 0.0), p3]
}

/// 有交互的图层，按图层树顺序。
fn sources(doc: &Document, page: NodeId) -> Vec<NodeId> {
    doc.subtree(page).into_iter().filter(|i| doc.node(*i).is_some_and(|n| !n.interactions.is_empty())).collect()
}

fn noodles(app: &CanvasApp, doc: &Document, page: NodeId) -> Vec<Noodle> {
    let mut out = Vec::new();
    for id in sources(doc, page) {
        let Some(n) = doc.node(id) else { continue };
        let Some(from) = screen_rect(app, doc, id) else { continue };
        for (k, it) in n.interactions.iter().enumerate() {
            let Some(dest) = figcraft_doc::proto::walk(&it.actions).find_map(|a| a.dest()) else { continue };
            // Change-to 的目标是变体，按原样连过去。
            let Some(to) = screen_rect(app, doc, dest) else { continue };
            if doc.page_of(dest) != Some(page) {
                continue;
            }
            out.push(Noodle { source: id, index: k, pts: curve_between(from, to) });
        }
    }
    out
}

fn bezier(p: &[Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    pos2(w[0] * p[0].x + w[1] * p[1].x + w[2] * p[2].x + w[3] * p[3].x, w[0] * p[0].y + w[1] * p[1].y + w[2] * p[2].y + w[3] * p[3].y)
}

fn segment_distance(a: Pos2, b: Pos2, q: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let t = if len2 > 0.0 { ((q - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).distance(q)
}

/// 点到三次曲线的距离（采样，与原文一致）。
fn bezier_distance(p: &[Pos2; 4], q: Pos2) -> f32 {
    let mut best = f32::INFINITY;
    let mut prev = p[0];
    for i in 1..=32 {
        let cur = bezier(p, i as f32 / 32.0);
        best = best.min(segment_distance(prev, cur, q));
        prev = cur;
    }
    best
}

/// 选中层的手柄中心（右边缘、垂直居中）。
fn handles(app: &CanvasApp, doc: &Document, sel: &[NodeId]) -> Vec<(NodeId, Pos2)> {
    sel.iter()
        .take(32)
        .filter(|i| doc.node(**i).is_some_and(|n| !n.is_page()))
        .filter_map(|i| screen_rect(app, doc, *i).map(|r| (*i, right_center(r) + vec2(HANDLE_R + 4.0, 0.0))))
        .collect()
}

/// 屏幕点下最上面的顶层 frame；没有就是那里最内层的 section（连到 section 会打开它所在
/// frame 显示的最后一张图）。
fn frame_at(app: &CanvasApp, doc: &Document, page: NodeId, p: Pos2) -> Option<NodeId> {
    let pt = app.to_page(p);
    let under = |i: &NodeId| doc.world_bounds(*i).is_some_and(|b| b.contains(pt));
    doc.top_frames(page).into_iter().rev().find(under).or_else(|| doc.sections_in(page).into_iter().rev().find(under))
}

/// 采样贝塞尔画折线（GPUI 画笔没有贝塞尔图元）。
fn cubic(painter: &mut Painter, pts: &[Pos2; 4], col: Color32, w: f32) {
    let mut prev = pts[0];
    for i in 1..=32 {
        let cur = bezier(pts, i as f32 / 32.0);
        painter.line_segment([prev, cur], Stroke::new(w, col));
        prev = cur;
    }
}

fn arrow(painter: &mut Painter, pts: &[Pos2; 4], col: Color32) {
    let tip = pts[3];
    let raw = pts[3] - bezier(pts, 0.9);
    let dir = if raw.x.is_finite() && raw.y.is_finite() && raw.length() > 0.5 { raw.normalized() } else { vec2(1.0, 0.0) };
    let n = vec2(-dir.y, dir.x);
    painter.add_shape(
        vec![tip, tip - dir * 8.0 + n * 4.5, tip - dir * 8.0 - n * 4.5],
        true,
        col,
        Stroke::NONE,
    );
}

fn plus_handle(painter: &mut Painter, c: Pos2, hot: bool, accent: Color32) {
    let r = HANDLE_R + if hot { 1.0 } else { 0.0 };
    painter.circle_filled(c, r, accent);
    painter.circle_stroke(c, r, Stroke::new(1.5, Color32::WHITE));
    painter.line_segment([c - vec2(3.5, 0.0), c + vec2(3.5, 0.0)], Stroke::new(1.5, Color32::WHITE));
    painter.line_segment([c - vec2(0.0, 3.5), c + vec2(0.0, 3.5)], Stroke::new(1.5, Color32::WHITE));
}

/// Manual overlay 的编辑目标。
pub struct Manual {
    slot: Slot,
    action: Action,
    dest: NodeId,
    /// 触发图层左上角（页面坐标）。
    origin: [f64; 2],
    /// overlay 左上角相对 `origin` 的偏移；没有偏移就贴着触发层下沿。
    offset: [f64; 2],
}

/// `actions` 里第一个 Manual Open overlay（条件块里也找），路径留在 `path`。
fn find_manual(actions: &[Action], path: &mut Vec<usize>, depth: usize) -> Option<Action> {
    for (i, a) in actions.iter().enumerate() {
        path.push(i);
        if matches!(a, Action::OpenOverlay { position: OverlayPosition::Manual, .. }) {
            return Some(a.clone());
        }
        if let Action::Conditional { blocks } = a
            && depth < MAX_ACTION_DEPTH
            && let Some(found) = blocks.iter().enumerate().find_map(|(b, block)| {
                let mut inner = vec![b];
                let found = find_manual(&block.actions, &mut inner, depth + 1)?;
                path.extend(inner);
                Some(found)
            })
        {
            return Some(found);
        }
        path.pop();
    }
    None
}

/// `node` 的第 `k` 条交互里的 Manual overlay。
fn manual(doc: &Document, node: NodeId, k: usize) -> Option<Manual> {
    let it = doc.node(node)?.interactions.get(k)?;
    let mut path = Vec::new();
    let action = find_manual(&it.actions, &mut path, 0)?;
    let Action::OpenOverlay { dest, offset, .. } = &action else { return None };
    let trigger = doc.world_bounds(node)?;
    let (dest, offset) = (*dest, offset.unwrap_or([0.0, trigger.height()]));
    Some(Manual { slot: Slot { node, k, path }, action, dest, origin: [trigger.x0, trigger.y0], offset })
}

/// 编辑器开着、且在显示页上的 Manual overlay。
fn editing(app: &CanvasApp, doc: &Document, page: NodeId) -> Option<Manual> {
    let (node, k) = app.proto_editor?;
    if !app.proto_active || doc.page_of(node) != Some(page) {
        return None;
    }
    manual(doc, node, k)
}

fn screen(app: &CanvasApp, [x, y]: [f64; 2], w: f64, h: f64) -> Rect {
    Rect::from_min_max(app.to_screen(x, y), app.to_screen(x + w, y + h))
}

fn ghost_rect(app: &CanvasApp, doc: &Document, m: &Manual) -> Option<Rect> {
    let b = doc.world_bounds(m.dest)?;
    Some(screen(app, [m.origin[0] + m.offset[0], m.origin[1] + m.offset[1]], b.width(), b.height()))
}

/// 画 Manual overlay 的 ghost：frame 轮廓，accent 描边。
///
/// 原文用资产缩略图（`assets_panel::thumbnail`）铺在轮廓里；GPUI 侧资产缩略图管线还没迁移
/// （清单 A4），所以走原文缩略图缺失时的同一分支：12% accent 填充。
fn ghost_draw(app: &CanvasApp, painter: &mut Painter, doc: &Document, page: NodeId, accent: Color32) {
    let Some(m) = editing(app, doc, page) else { return };
    let Some(r) = ghost_rect(app, doc, &m) else { return };
    painter.rect_filled(r, CornerRadius::ZERO, accent.gamma_multiply(0.12));
    painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.5, accent), StrokeKind::Outside);
}

/// 画连接、手柄与正在拖出的连接。
pub(super) fn draw(app: &CanvasApp, painter: &mut Painter) {
    if !app.proto_active {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    let (doc, page, sel) = (&ds.doc, ds.page, &ds.selection);
    let accent = app.accent();
    let open = app.proto_editor;
    let hover = app.pointer;
    ghost_draw(app, painter, doc, page, accent);
    for nd in noodles(app, doc, page) {
        let selected = sel.contains(&nd.source);
        let is_open = open == Some((nd.source, nd.index));
        let near = app.proto.drag.is_none() && hover.is_some_and(|q| bezier_distance(&nd.pts, q) < 5.0);
        let col = if selected || is_open || near { accent } else { accent.gamma_multiply(0.55) };
        let w = if is_open || near {
            2.5
        } else if selected {
            2.0
        } else {
            1.5
        };
        // 选中层（或正在编辑）的连接：把目标 frame 也框出来。
        if (selected || is_open)
            && let Some(dest) = doc
                .node(nd.source)
                .and_then(|n| n.interactions.get(nd.index))
                .and_then(|it| figcraft_doc::proto::walk(&it.actions).find_map(|a| a.dest()))
            && let Some(r) = screen_rect(app, doc, dest)
        {
            painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, accent.gamma_multiply(0.7)), StrokeKind::Outside);
        }
        cubic(painter, &nd.pts, col, w);
        painter.circle_filled(nd.pts[0], 3.5, col);
        painter.circle_stroke(nd.pts[0], 3.5, Stroke::new(1.0, Color32::WHITE));
        arrow(painter, &nd.pts, col);
    }
    match app.proto.drag {
        Some(d) => {
            if let Some(f) = frame_at(app, doc, page, d.pos)
                && doc.top_level_frame(d.source) != Some(f)
                && let Some(r) = screen_rect(app, doc, f)
            {
                painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(2.0, accent), StrokeKind::Outside);
            }
            let dx = ((d.pos.x - d.from.x).abs() * 0.5).max(30.0) * if d.pos.x >= d.from.x { 1.0 } else { -1.0 };
            let pts = [d.from, d.from + vec2(dx, 0.0), d.pos - vec2(dx, 0.0), d.pos];
            cubic(painter, &pts, accent, 2.0);
            arrow(painter, &pts, accent);
            plus_handle(painter, d.from, true, accent);
        }
        None => {
            for (_, c) in handles(app, doc, sel) {
                plus_handle(painter, c, hover.is_some_and(|q| q.distance(c) <= HANDLE_R + 3.0), accent);
            }
        }
    }
}

/// 画布按下时调用（Prototype 标签、非空格/缩放键）：ghost → 手柄 → 曲线 → 收编辑器，
/// 与原文 `proto_canvas::input` 同一优先级。
pub fn press(app: &mut CanvasApp, pos: Pos2) -> Press {
    if !app.proto_active {
        app.proto.drag = None;
        end_ghost(app);
        return Press::Ignored;
    }
    // ghost：按在它上面就开始拖。
    if app.proto.ghost.is_none() && ghost_press(app, pos) {
        return Press::Consumed;
    }
    let Ok(ds) = app.session.ds() else { return Press::Ignored };
    let (doc, page, sel) = (ds.doc.clone(), ds.page, ds.selection.clone());
    let over_handle = handles(app, &doc, &sel).into_iter().find(|(_, c)| c.distance(pos) <= HANDLE_R + 3.0);
    if let Some((source, from)) = over_handle {
        app.proto.drag = Some(Drag { source, from, pos });
        return Press::Consumed;
    }
    let hit = noodles(app, &doc, page)
        .into_iter()
        .filter(|n| bezier_distance(&n.pts, pos) < 5.0)
        .min_by(|a, b| bezier_distance(&a.pts, pos).total_cmp(&bezier_distance(&b.pts, pos)));
    if let Some(n) = hit {
        app.session.set_selection(vec![n.source]);
        app.proto_editor = Some((n.source, n.index));
        return Press::Consumed;
    }
    if app.proto_editor.is_some() {
        app.proto_editor = None;
        return Press::CloseEditor;
    }
    Press::Ignored
}

/// 连线拖动中：跟随指针，松开时落成一条 `proto.interaction.add`。
/// 返回是否还在拖 / 刚结束（用于续帧与通知）。
pub fn drag_move(app: &mut CanvasApp, pos: Pos2) -> bool {
    let Some(mut d) = app.proto.drag else { return false };
    d.pos = pos;
    app.proto.drag = Some(d);
    true
}

/// 松开：目标是别的顶层 frame 就新建交互并打开编辑器，返回要打开的槽。
pub fn drag_end(app: &mut CanvasApp) -> Option<(NodeId, usize)> {
    let d = app.proto.drag.take()?;
    let Ok(ds) = app.session.ds() else { return None };
    let (doc, page) = (ds.doc.clone(), ds.page);
    let dest = frame_at(app, &doc, page, d.pos)?;
    if doc.top_level_frame(d.source) == Some(dest) {
        return None;
    }
    let r = app.session.execute("proto.interaction.add", &json!({"ids": [d.source], "dest": dest})).ok()?;
    let k = r.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    Some((d.source, k))
}

/// ghost 按下：开始一步撤销并记录抓取点。
fn ghost_press(app: &mut CanvasApp, p: Pos2) -> bool {
    let Ok(ds) = app.session.ds() else { return false };
    let (doc, page) = (ds.doc.clone(), ds.page);
    let Some(m) = editing(app, &doc, page).filter(|m| ghost_rect(app, &doc, m).is_some_and(|r| r.contains(p))) else {
        return false;
    };
    if app.session.begin_interaction(LABEL).is_err() {
        return false;
    }
    let at = app.to_page(p);
    app.proto.ghost = Some(GhostDrag {
        slot: m.slot,
        action: m.action,
        grab: [at.x, at.y],
        start: m.offset,
        last: m.offset,
    });
    true
}

/// 把 ghost 挪到「抓取点在 `p` 下」，动作的 X/Y 跟着走。
fn ghost_move(app: &mut CanvasApp, p: Pos2) {
    let Some(mut d) = app.proto.ghost.take() else { return };
    let at = app.to_page(p);
    let o = [
        (d.start[0] + at.x - d.grab[0]).round(),
        (d.start[1] + at.y - d.grab[1]).round(),
    ]
    .map(|v| v.clamp(-100_000.0, 100_000.0));
    if o != d.last {
        let mut action = d.action.clone();
        if let Action::OpenOverlay { offset, .. } = &mut action {
            *offset = Some(o);
        }
        let mut params = json!({ "ids": [d.slot.node], "index": d.slot.k, "path": d.slot.path });
        if let Value::Object(x) = &mut params {
            x.insert("action".into(), serde_json::to_value(&action).unwrap_or(Value::Null));
        }
        let _ = app.session.execute("proto.action.set", &params);
        d.last = o;
    }
    app.proto.ghost = Some(d);
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::proto::ConditionalBlock;

    fn overlay(offset: [f64; 2]) -> Action {
        Action::OpenOverlay {
            dest: NodeId(7),
            position: OverlayPosition::Manual,
            close_on_outside: true,
            background: None,
            offset: Some(offset),
            anchor: Default::default(),
        }
    }

    /// 条件块里的 Manual overlay 也要找到，路径指到它（原文同一条测试）。
    #[test]
    fn manual_overlay_inside_conditional() {
        let actions = vec![
            Action::Back,
            Action::Conditional {
                blocks: vec![ConditionalBlock {
                    condition: Default::default(),
                    actions: vec![Action::CloseOverlay, overlay([4.0, 6.0])],
                }],
            },
        ];
        let mut path = Vec::new();
        let found = find_manual(&actions, &mut path, 0).expect("要能找到条件块里的 Manual overlay");
        assert!(matches!(found, Action::OpenOverlay { .. }));
        assert_eq!(path, vec![1, 0, 1]);
    }

    /// 自动定位的 overlay 不算 ghost。
    #[test]
    fn auto_overlay_is_not_manual() {
        let mut action = overlay([0.0, 0.0]);
        if let Action::OpenOverlay { position, .. } = &mut action {
            *position = OverlayPosition::Center;
        }
        let mut path = Vec::new();
        assert!(find_manual(&[Action::Back, action], &mut path, 0).is_none());
    }
}

/// ghost 拖动中：跟随指针。返回是否仍占着手势。
pub fn ghost_drag_move(app: &mut CanvasApp, pos: Pos2) -> bool {
    if app.proto.ghost.is_none() {
        return false;
    }
    ghost_move(app, pos);
    true
}

/// 结束 ghost 拖动，整段算一步撤销。
pub fn end_ghost(app: &mut CanvasApp) {
    if app.proto.ghost.take().is_some() {
        let _ = app.session.commit_interaction();
    }
}
