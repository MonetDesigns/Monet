//! 画板自动连线（照搬 `ui-egui/src/canvas/board_connect.rs` #1597 / #1919）：
//! 选中的单个便签或形状四边各一个「+」手柄，悬停显示下一个对象落点；点击在该侧新建同类
//! 对象并由连接线接上（`board.addConnected`）；拖动可把预览带到任意位置 —— 松手在空白
//! 画布上就在那里新建并连线，松手在别的便签/形状上（section 内也算）就连起两者
//! （`board.connector`），松手在画布外或底部工具栏上什么也不做。
//!
//! 原文用 egui 的 `ctx.data` 存拖动状态；GPUI 侧放在 [`CanvasApp::connect`]。

use figcraft_doc::{BoardItem, NodeId};
use serde_json::json;

use crate::CanvasApp;
use crate::canvas::geom::*;
use crate::canvas::painter::Painter;

/// 每个「+」离对象边缘的距离（屏幕点），避开选中手柄与尺寸标签。
pub(crate) const REACH: f32 = 36.0;
const RADIUS: f32 = 9.0;
const SIDES: [&str; 4] = ["above", "right", "below", "left"];
/// 按下到抬起不超过这个位移算点击（egui `Sense::click` 的等价）。
const SLOP: f32 = 4.0;

/// 从「+」拖出的一段：对象、起始边，以及当前/起始指针位置。
#[derive(Clone, Copy)]
pub struct Drag {
    pub from: NodeId,
    pub side: &'static str,
    pub start: Pos2,
    pub pos: Pos2,
}

/// 自动连线的跨帧状态。
#[derive(Default)]
pub struct State {
    pub drag: Option<Drag>,
}

/// 手柄的目标：单个选中的便签或形状，它的页面包围盒与下一个对象的大小。
fn target(app: &CanvasApp) -> Option<(NodeId, figcraft_geom::Rect, (f64, f64))> {
    let editable = app.session.active_read_only().is_none();
    if app.session.tool.busy() || !editable || app.session.tool.tool != figcraft_engine::ToolId::Move {
        return None;
    }
    if !app.session.doc().is_ok_and(figcraft_engine::board::is_board) {
        return None;
    }
    let ds = app.session.ds().ok()?;
    let ([id], None) = (ds.selection.as_slice(), &ds.text_edit) else { return None };
    let n = ds.doc.node(*id)?;
    let next = match n.board {
        Some(BoardItem::Sticky { .. }) => (figcraft_engine::board::STICKY, figcraft_engine::board::STICKY),
        Some(BoardItem::Shape { .. }) => (n.width, n.height),
        _ => return None,
    };
    Some((*id, ds.doc.world_bounds(*id)?, next))
}

/// 是否正在从「+」拖动（选中条要暂时让开）。
pub(crate) fn dragging(app: &CanvasApp) -> bool {
    app.connect.drag.is_some()
}

/// `side` 侧「+」的中心（对象的屏幕框 `r`）。
fn handle_at(r: Rect, side: &str) -> Pos2 {
    match side {
        "above" => pos2(r.center().x, r.min.y - REACH),
        "right" => pos2(r.max.x + REACH, r.center().y),
        "below" => pos2(r.center().x, r.max.y + REACH),
        _ => pos2(r.min.x - REACH, r.center().y),
    }
}

/// `side` 侧的中点，连接线从这里离开。
fn side_middle(r: Rect, side: &str) -> Pos2 {
    match side {
        "above" => pos2(r.center().x, r.min.y),
        "right" => pos2(r.max.x, r.center().y),
        "below" => pos2(r.center().x, r.max.y),
        _ => pos2(r.min.x, r.center().y),
    }
}

/// 「+」侧边在 `board.connector` 里的名字。
fn connector_side(side: &str) -> &'static str {
    match side {
        "above" => "top",
        "below" => "bottom",
        "left" => "left",
        _ => "right",
    }
}

fn canvas_rect(app: &CanvasApp) -> Rect {
    app.canvas.rect
}

/// 底部悬浮工具栏在画布上的矩形（含一圈余量），避开它。
fn toolbar_rect(app: &CanvasApp) -> Option<Rect> {
    let r = canvas_rect(app);
    let groups = crate::toolbar::groups(true);
    let w = crate::toolbar::width(groups, true, false);
    let left = r.min.x + (r.width() - w) / 2.0;
    Some(Rect::from_min_max(pos2(left, r.max.y - 12.0 - crate::toolbar::PILL_H), pos2(left + w, r.max.y - 12.0)).expand(RADIUS))
}

fn on_canvas(app: &CanvasApp, p: Pos2) -> bool {
    canvas_rect(app).contains(p) && !toolbar_rect(app).is_some_and(|t| t.contains(p))
}

fn screen_box(app: &CanvasApp, b: figcraft_geom::Rect) -> Rect {
    Rect::from_min_max(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1))
}

/// 画四个「+」、悬停落点预览或拖动预览。
pub(crate) fn draw(app: &CanvasApp, painter: &mut Painter) {
    let drag = app.connect.drag;
    let Some((id, b, next)) = target(app) else { return };
    let r = screen_box(app, b);
    if r.width() < 2.0 * RADIUS || r.height() < 2.0 * RADIUS {
        return;
    }
    let accent = app.accent();
    let canvas = canvas_rect(app);
    let toolbar = toolbar_rect(app);
    let hover = app.pointer;
    let mut dashed = |painter: &mut Painter, rect: Rect, ink: Color32| {
        let corners = [rect.min, pos2(rect.max.x, rect.min.y), rect.max, pos2(rect.min.x, rect.max.y)];
        for i in 0..4 {
            dash_line(painter, corners[i], corners[(i + 1) % 4], Stroke::new(1.5, ink));
        }
    };
    for side in SIDES {
        let c = handle_at(r, side);
        if !canvas.shrink(RADIUS).contains(c) || toolbar.is_some_and(|t| t.contains(c)) {
            continue;
        }
        let hot = hover.is_some_and(|q| q.distance(c) <= RADIUS + 3.0) || drag.is_some_and(|d| d.from == id && d.side == side);
        plus_handle(painter, c, hot, accent);
        if hot && drag.is_none() {
            let Some(((x, y), ..)) = figcraft_engine::board::connected_place((b.x0, b.y0), (b.width(), b.height()), next, side) else {
                continue;
            };
            let ghost = Rect::from_min_max(app.to_screen(x, y), app.to_screen(x + next.0, y + next.1));
            painter.rect_filled(ghost, CornerRadius::same(8), accent.gamma_multiply(0.08));
            dashed(painter, ghost, accent.gamma_multiply(0.6));
            let (a, z) = match side {
                "above" => (pos2(r.center().x, r.min.y), pos2(ghost.center().x, ghost.max.y)),
                "right" => (pos2(r.max.x, r.center().y), pos2(ghost.min.x, ghost.center().y)),
                "below" => (pos2(r.center().x, r.max.y), pos2(ghost.center().x, ghost.min.y)),
                _ => (pos2(r.min.x, r.center().y), pos2(ghost.max.x, ghost.center().y)),
            };
            painter.line_segment([a, z], Stroke::new(1.5, accent.gamma_multiply(0.6)));
        }
    }
    // 拖动中：预览跟着指针走。
    if let Some(d) = drag.filter(|d| d.from == id) {
        let over = object_under(app, d.from, d.pos);
        if on_canvas(app, d.pos) {
            drag_preview(app, painter, b, next, d.side, d.pos, over, accent);
        }
    }
    let _ = &mut dashed;
}

/// 从「+」按下：开始一段拖动（点击与拖动在抬起时区分）。
pub(crate) fn press(app: &mut CanvasApp, p: Pos2) -> bool {
    if !on_canvas(app, p) {
        return false;
    }
    let Some((id, b, _)) = target(app) else { return false };
    let r = screen_box(app, b);
    let Some(side) = SIDES.into_iter().find(|side| handle_at(r, side).distance(p) <= RADIUS + 3.0) else { return false };
    app.connect.drag = Some(Drag { from: id, side, start: p, pos: p });
    true
}

/// 拖动中：预览跟随指针（指针移出画布也不断）。
pub(crate) fn drag_move(app: &mut CanvasApp, p: Pos2) -> bool {
    let Some(mut d) = app.connect.drag else { return false };
    // 选中对象变了：这一段拖动作废。
    if !target(app).is_some_and(|(id, ..)| id == d.from) {
        app.connect.drag = None;
        return false;
    }
    d.pos = p;
    app.connect.drag = Some(d);
    true
}

/// 松开：点击 → 在该侧新建并开始输入；拖动 → 连到对象上，或在落点新建。
pub(crate) fn drag_end(app: &mut CanvasApp) -> bool {
    let Some(d) = app.connect.drag.take() else { return false };
    if !on_canvas(app, d.pos) {
        return true;
    }
    if d.pos.distance(d.start) <= SLOP {
        let _ = app.session.execute("board.addConnected", &json!({ "id": d.from, "side": d.side, "edit": true }));
        return true;
    }
    match object_under(app, d.from, d.pos) {
        Some(to) => {
            let _ = app.session.execute("board.connector", &json!({ "from": d.from, "to": to, "fromSide": connector_side(d.side) }));
        }
        None => {
            let q = app.to_page(d.pos);
            let _ = app.session.execute("board.addConnected", &json!({ "id": d.from, "side": d.side, "at": [q.x, q.y], "edit": true }));
        }
    }
    true
}

/// 屏幕点 `p` 下的便签/形状（section、frame 里也算），排除 `except`。
fn object_under(app: &CanvasApp, except: NodeId, p: Pos2) -> Option<NodeId> {
    if !canvas_rect(app).contains(p) {
        return None;
    }
    let ds = app.session.ds().ok()?;
    figcraft_engine::board::sticky_or_shape_at(&ds.doc, ds.page, app.to_page(p), Some(except))
}

/// 拖动预览：下一个对象以指针为中心（或框出指针下的对象），并画出从起始边的连线。
fn drag_preview(
    app: &CanvasApp,
    painter: &mut Painter,
    b: figcraft_geom::Rect,
    next: (f64, f64),
    side: &str,
    pointer: Pos2,
    over: Option<NodeId>,
    accent: Color32,
) {
    let from = screen_box(app, b);
    let to_box = match over.and_then(|o| app.session.doc().ok()?.world_bounds(o)) {
        Some(ob) => {
            let r = screen_box(app, ob);
            painter.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.5, accent), StrokeKind::Outside);
            r
        }
        None => {
            let size = app.to_screen(next.0, next.1) - app.to_screen(0.0, 0.0);
            let ghost = Rect::from_center_size(pointer, size);
            painter.rect_filled(ghost, CornerRadius::same(8), accent.gamma_multiply(0.08));
            let corners = [ghost.min, pos2(ghost.max.x, ghost.min.y), ghost.max, pos2(ghost.min.x, ghost.max.y)];
            for i in 0..4 {
                dash_line(painter, corners[i], corners[(i + 1) % 4], Stroke::new(1.5, accent.gamma_multiply(0.6)));
            }
            ghost
        }
    };
    let d = to_box.center() - from.center();
    let z = if d.x.abs() >= d.y.abs() {
        if d.x >= 0.0 { pos2(to_box.min.x, to_box.center().y) } else { pos2(to_box.max.x, to_box.center().y) }
    } else if d.y >= 0.0 {
        pos2(to_box.center().x, to_box.min.y)
    } else {
        pos2(to_box.center().x, to_box.max.y)
    };
    painter.line_segment([side_middle(from, side), z], Stroke::new(1.5, accent.gamma_multiply(0.6)));
}

/// 从一个「+」形按钮（圆 + 十字）。
fn plus_handle(painter: &mut Painter, c: Pos2, hot: bool, accent: Color32) {
    let t = crate::widgets::tokens();
    painter.circle_filled(c, RADIUS, if hot { accent } else { t.panel });
    painter.circle_stroke(c, RADIUS, Stroke::new(1.0, if hot { accent } else { t.border }));
    let ink = Stroke::new(1.5, if hot { Color32::WHITE } else { t.text_secondary });
    let arm = RADIUS * 0.45;
    painter.line_segment([pos2(c.x - arm, c.y), pos2(c.x + arm, c.y)], ink);
    painter.line_segment([pos2(c.x, c.y - arm), pos2(c.x, c.y + arm)], ink);
}

/// 一条虚线（GPUI 画笔没有虚线图元，按 6 / 4 的节奏画短段）。
fn dash_line(painter: &mut Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    const DASH: f32 = 6.0;
    const GAP: f32 = 4.0;
    let len = a.distance(b);
    if len <= 0.0 {
        return;
    }
    let dir = (b - a) / len;
    let mut at = 0.0;
    while at < len {
        let end = (at + DASH).min(len);
        painter.line_segment([a + dir * at, a + dir * end], stroke);
        at = end + GAP;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::BoardItem;
    use figcraft_engine::tools::ToolId;
    use serde_json::{Value, json};

    fn id(v: &Value) -> NodeId {
        NodeId(v["id"].as_u64().unwrap())
    }

    /// 一块画板、一个 160×100 的圆角矩形，缩放 100%，矩形落在画布中间。
    fn board_with_shape() -> (CanvasApp, NodeId) {
        let mut app = crate::CanvasApp::new(crate::sample_doc());
        app.session.execute("file.newBoard", &json!({})).unwrap();
        let a = id(&app.session.execute("board.shape", &json!({"shape": "roundedRectangle", "x": 0, "y": 0, "width": 160, "height": 100})).unwrap());
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
        if let Ok(ds) = app.session.ds_mut() {
            ds.view.zoom = 1.0;
            ds.view.cx = 80.0;
            ds.view.cy = 50.0;
        }
        (app, a)
    }

    fn screen_box(app: &CanvasApp, n: NodeId) -> Rect {
        let b = app.session.doc().unwrap().world_bounds(n).unwrap();
        Rect::from_min_max(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1))
    }

    /// 四个「+」分别在选中框四边外 `REACH` 处；没选中、两个对象、换了工具时都没有。
    #[test]
    fn four_handles_show_for_one_shape_only() {
        let (mut app, a) = board_with_shape();
        app.session.execute("select.set", &json!({"ids": []})).unwrap();
        assert!(target(&app).is_none(), "nothing selected");
        app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
        assert!(target(&app).is_some());
        let r = screen_box(&app, a);
        for side in SIDES {
            let h = handle_at(r, side);
            assert!(press(&mut app, h), "{side}: its handle takes the press");
            app.connect.drag = None;
        }
        assert!(!press(&mut app, pos2(r.center().x, r.center().y)), "not on a handle");
        app.session.set_tool(ToolId::Hand);
        assert!(target(&app).is_none(), "another tool");
        app.session.set_tool(ToolId::Move);
        let b = id(&app.session.execute("board.sticky", &json!({"x": 600, "y": 0})).unwrap());
        app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        assert!(target(&app).is_none(), "two layers");
    }

    /// 点「+」：该侧新建同类对象并由连接线接上，同时开始输入；一次撤销把两者都带走。
    #[test]
    fn clicking_a_handle_adds_the_next_shape_joined_and_typing() {
        let (mut app, a) = board_with_shape();
        app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
        let before = app.session.doc().unwrap().ids().count();
        let at = handle_at(screen_box(&app, a), "below");
        assert!(press(&mut app, at));
        assert!(drag_end(&mut app));
        let doc = app.session.doc().unwrap();
        let made: Vec<NodeId> = doc.ids().filter(|i| doc.node(*i).is_some_and(|n| n.board.is_some())).collect();
        let next =
            made.iter().copied().find(|i| *i != a && matches!(doc.node(*i).unwrap().board, Some(BoardItem::Shape { .. }))).expect("a new shape");
        let line = made.iter().copied().find(|i| matches!(doc.node(*i).unwrap().board, Some(BoardItem::Connector(_)))).expect("a connector");
        assert!(screen_box(&app, next).min.y > screen_box(&app, a).max.y, "below it");
        assert!(matches!(&doc.node(line).unwrap().board, Some(BoardItem::Connector(c)) if c.from.node() == Some(a) && c.to.node() == Some(next)));
        drop(doc);
        // GPUI 侧文本编辑是异步的：引擎先 request_ui，视图下一帧再开输入框。
        assert!(app.session.ui_requests.iter().any(|q| q.kind == "board.editText" && q.data["id"] == json!(next)), "typing in the new shape");
        figcraft_engine::textedit::end(&mut app.session).unwrap();
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().ids().count(), before, "undo takes the shape and its connector away");
    }

    /// 拖到空白画布：在松手处新建、居中，并由连接线接上。
    #[test]
    fn dragging_a_handle_drops_the_next_shape_where_it_is_let_go() {
        let (mut app, a) = board_with_shape();
        app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
        let from = handle_at(screen_box(&app, a), "below");
        let to = from + vec2(260.0, 180.0);
        assert!(press(&mut app, from));
        assert!(drag_move(&mut app, to));
        assert!(drag_end(&mut app));
        let doc = app.session.doc().unwrap();
        let next = doc.ids().find(|i| *i != a && matches!(doc.node(*i).unwrap().board, Some(BoardItem::Shape { .. }))).expect("a new shape");
        let line = doc.ids().find(|i| matches!(doc.node(*i).unwrap().board, Some(BoardItem::Connector(_)))).expect("a connector");
        let centre = screen_box(&app, next).center();
        assert!((centre - to).length() < 1.0, "centred where it was let go");
        assert!(matches!(&doc.node(line).unwrap().board, Some(BoardItem::Connector(c)) if c.from.node() == Some(a) && c.to.node() == Some(next)));
    }

    /// 拖到另一个形状上：只连一条线，不再新建。
    #[test]
    fn dropping_a_handle_on_another_shape_joins_them() {
        let (mut app, a) = board_with_shape();
        let b = id(&app.session.execute("board.shape", &json!({"shape": "ellipse", "x": 400, "y": 300, "width": 160, "height": 100})).unwrap());
        app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
        let shapes = |app: &CanvasApp| {
            let doc = app.session.doc().unwrap();
            doc.ids().filter(|i| matches!(doc.node(*i).unwrap().board, Some(BoardItem::Shape { .. }))).count()
        };
        assert_eq!(shapes(&app), 2);
        let from = handle_at(screen_box(&app, a), "right");
        let to = screen_box(&app, b).center();
        assert!(press(&mut app, from));
        assert!(drag_move(&mut app, to));
        assert!(drag_end(&mut app));
        assert_eq!(shapes(&app), 2, "no new shape");
        let doc = app.session.doc().unwrap();
        let line = doc.ids().find(|i| matches!(doc.node(*i).unwrap().board, Some(BoardItem::Connector(_)))).expect("a connector");
        assert!(matches!(&doc.node(line).unwrap().board, Some(BoardItem::Connector(c)) if c.from.node() == Some(a) && c.to.node() == Some(b)));
    }

    /// 松手在画布外（面板上）或底部工具栏上：什么也不做。
    #[test]
    fn letting_a_handle_go_off_the_canvas_or_over_the_toolbar_makes_nothing() {
        let (mut app, a) = board_with_shape();
        app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
        let before = app.session.doc().unwrap().ids().count();
        let from = handle_at(screen_box(&app, a), "left");
        let off = pos2(app.canvas.rect.min.x - 60.0, from.y);
        assert!(press(&mut app, from));
        assert!(drag_move(&mut app, off));
        assert!(drag_end(&mut app));
        assert_eq!(app.session.doc().unwrap().ids().count(), before, "nothing made");
        let bar = toolbar_rect(&app).expect("a toolbar rect");
        let from = handle_at(screen_box(&app, a), "below");
        assert!(press(&mut app, from));
        assert!(drag_move(&mut app, bar.center()));
        assert!(drag_end(&mut app));
        assert_eq!(app.session.doc().unwrap().ids().count(), before, "nothing made behind the toolbar");
    }

    /// 拖动中选中换到别的对象：这一段作废，之后再松手也不建东西。
    #[test]
    fn a_drag_ends_when_the_selection_moves_to_another_shape() {
        let (mut app, a) = board_with_shape();
        let b = id(&app.session.execute("board.shape", &json!({"shape": "ellipse", "x": 600, "y": 0, "width": 160, "height": 100})).unwrap());
        app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
        let before = app.session.doc().unwrap().ids().count();
        let from = handle_at(screen_box(&app, a), "below");
        let to = from + vec2(0.0, 160.0);
        assert!(press(&mut app, from));
        app.session.execute("select.set", &json!({"ids": [b]})).unwrap();
        assert!(!drag_move(&mut app, to), "the drag is over");
        app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
        assert!(!drag_end(&mut app));
        assert_eq!(app.session.doc().unwrap().ids().count(), before, "nothing made");
    }
}
