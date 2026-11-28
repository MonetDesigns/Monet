//! 页面之上的叠加层（照搬 `ui-egui/src/canvas/overlays.rs` 的画法与顺序）：
//! 像素网格 → 布局网格与帧标签 → hover 轮廓 → 布尔操作数 → 蒙版/帧/切图轮廓 →
//! 拖放目标 → 选中轮廓与变换框（含连接线端点、旋转原点）→ 框选 → 钢笔/橡皮筋/套索/橡皮 →
//! 吸附参考线与尺寸。

use figcraft_color::{ImagePaint, ImageScaleMode, PaintKind};
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_engine::tools::{ToolId, selection_box, selection_frame};
use figcraft_geom::Point;
use std::sync::Arc;

use super::geom::*;
use super::outline::{crisp_line, draw_outline, node_quad, outline_colour, quad, stroke};
use super::painter::{Label, Painter, text_size};
use super::rulers::fmt;
use crate::app::CanvasApp;
use crate::theme::{COMPONENT, SLOT, Tokens, doc_c32, stroke as s};
use crate::widgets::input::caret;

/// 三次贝塞尔在 `t` 处的点（GPUI 画笔没有贝塞尔图元，段要采样成折线）。
fn cubic_at(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, t: f32) -> Pos2 {
    let u = 1.0 - t;
    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    pos2(p0.x * w0 + p1.x * w1 + p2.x * w2 + p3.x * w3, p0.y * w0 + p1.y * w1 + p2.y * w2 + p3.y * w3)
}

/// 矢量编辑叠加层（照抄 `ui-egui/src/canvas/overlays.rs::vector_overlay`）：
/// 贝塞尔段（采样成折线）+ 手柄线 + 圆点（hover 放大、选中 accent）。
///
/// 用户 2026-11-12：进入矢量编辑后必须能看到每个节点的手柄、能点中它们。
fn vector_overlay(
    app: &CanvasApp,
    painter: &mut Painter,
    o: &figcraft_engine::vectoredit::EditOverlay,
    hover: Option<figcraft_engine::vectoredit::VHit>,
    accent: Color32,
) {
    use figcraft_engine::vectoredit::VHit;
    let sp = |p: &[f64; 2]| app.to_screen(p[0], p[1]);
    let hover_seg = match hover {
        Some(VHit::Segment { seg, .. }) => Some(seg),
        _ => None,
    };
    // 段：三次贝塞尔采样成折线。
    for (i, seg) in o.segments.iter().enumerate() {
        let w = if o.selected_segments.contains(&i) {
            3.0
        } else if hover_seg == Some(i) {
            2.5
        } else {
            1.5
        };
        let (p0, p1, p2, p3) = (sp(&seg[0]), sp(&seg[1]), sp(&seg[2]), sp(&seg[3]));
        let mut prev = p0;
        for step in 1..=24 {
            let q = cubic_at(p0, p1, p2, p3, step as f32 / 24.0);
            painter.line_segment([prev, q], s(accent, w));
            prev = q;
        }
    }
    // 手柄线 + 端点圆（hover 放大、选中 accent）。
    for h in &o.handles {
        let a = sp(&h.vertex);
        let b = sp(&h.handle);
        painter.line_segment([a, b], s(accent, 1.0));
        let on = o.selected_handles.contains(&(h.seg, h.start));
        let hovered = hover == Some(VHit::Handle { seg: h.seg, start: h.start });
        let r = if hovered { 4.5 } else { 3.5 };
        painter.circle_filled(b, r, if on { accent } else { Color32::WHITE });
        painter.circle_stroke(b, r, s(accent, 1.0));
    }
    // 顶点：**真正的圆点**（hover 放大、选中 accent）。egui/参考实现 用 7/9 pt 方块配
    // `CornerRadius::same(5)`，egui 会把圆角截到半边长、看起来就是圆；但 GPUI 的
    // `rect_filled` 只把 radius 原样交给 `paint_quad`（`painter.rs::corners`），
    // 7 pt 方块配 5 pt 圆角仍然是圆角方块，不会变成圆——这就是上一版「照抄 egui 写法
    // 却还是方的」的原因。钢笔态本来就走 `circle_filled`，所以这里统一用圆形绘制。
    for (i, v) in o.vertices.iter().enumerate() {
        let p = sp(v);
        let hovered = matches!(hover, Some(VHit::Vertex { index }) if index == i);
        let r = if hovered { 4.5 } else { 3.5 };
        let on = o.selected_vertices.contains(&i);
        painter.circle_filled(p, r, if on { accent } else { Color32::WHITE });
        painter.circle_stroke(p, r, s(accent, 1.0));
        // 锚点环（egui `vector_overlay`: `circle_stroke(p, 7.5, …)`）。
        if o.anchor == Some(i) {
            painter.circle_stroke(p, 7.5, s(accent, 1.0));
        }
    }
}

/// 选中/叠加层主色（与原文 `accent(painter)` 一致：页面深色用亮绿，浅色用对比绿）。
pub(super) fn accent(app: &CanvasApp) -> Color32 {
    app.accent()
}

/// 画布上所有叠加层。返回每个顶层帧/section 名称标签的屏幕矩形（页面坐标），
/// 供引擎在点击标签时选中/拖拽对应容器（`ToolState::labels`，照搬 `ui-egui`）。
pub(super) fn overlays(
    app: &CanvasApp,
    painter: &mut Painter,
    t: &Tokens,
    doc: Arc<Document>,
    renaming: Option<NodeId>,
) -> Vec<(NodeId, [Point; 4])> {
    // 文档基准由 `draw_overlays` 按「台上贴图是否属于当前文件/页面」一次性决定后传入，
    // 这里不再读 `app.canvas.tex_doc`，避免旧贴图的几何混进当前页的选框。
    let Ok(ds) = app.session.ds() else { return Vec::new() };
    let page = ds.page;
    let mut labels = Vec::new();
    let sel = ds.selection.clone();
    let zoom = ds.view.zoom;
    let view = ds.view.clone();
    let accent = accent(app);

    // ── 像素网格（508% 起，含 508% 本身）。线色按**它盖住的那块像素**算（`Color::ink`：亮底压暗 0.05 亮度、
    //    暗底往白提到 `0.8j+0.15`），所以不烘进底图，而是画成矢量线、按页面上纯色填充的边界
    //    分段取色。烘进底图会让线跟着上一帧贴图一起被缩放重采样——缩放时忽粗忽细、像在加载；
    //    矢量线永远是 1 设备像素。
    //    画板（FigJam 式白板）没有像素网格：那里只有底下的圆点点阵，放大不该冒出线。
    if view.pixel_grid_shown() && !figcraft_engine::board::is_board(&doc) {
        pixel_grid(app, painter, &doc, page);
    }

    // ── 布局网格与帧标签（顶层帧，含 section 内的帧），按 z 序。
    let focus = ds.focused();
    for c in super::rulers::boards(&doc, page).iter().filter(|c| focus.is_none_or(|f| f == **c)) {
        let Some(n) = doc.node(*c) else { continue };
        let Some(b) = doc.world_bounds(*c) else { continue };
        // 屏幕外的帧既不画网格也不画标签（标签在帧外）。
        let on_screen = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1)).expand(20.0);
        if !on_screen.intersects(app.canvas.rect) {
            continue;
        }
        if view.layout_grids && !n.grids.is_empty() && figcraft_layout::grids_shown(&doc, *c) {
            let w = doc.world_transform(*c);
            for g in n.grids.iter().filter(|g| g.visible) {
                let col = crate::theme::doc_c32(g.color);
                for line in figcraft_layout::grid_lines(g, n.width, n.height) {
                    let to_screen = |q: figcraft_geom::Point| {
                        let p = w * q;
                        app.to_screen(p.x, p.y)
                    };
                    match line {
                        figcraft_layout::GridLine::Line { a, b } => {
                            crisp_line(painter, to_screen(a), to_screen(b), 1.0, app.ppp, col);
                        }
                        figcraft_layout::GridLine::SlashLine(pts) => {
                            let v: Vec<Pos2> = pts.into_iter().map(to_screen).collect();
                            for seg in v.windows(2) {
                                crisp_line(painter, seg[0], seg[1], 1.0, app.ppp, col);
                            }
                        }
                        figcraft_layout::GridLine::Band(r) => {
                            let (a, b) = (to_screen(figcraft_geom::Point::new(r.x0, r.y0)), to_screen(figcraft_geom::Point::new(r.x1, r.y1)));
                            painter.rect_filled(Rect::from_two_pos(a, b), CornerRadius::ZERO, col);
                        }
                        figcraft_layout::GridLine::Slash(poly) => {
                            let v: Vec<Pos2> = poly.into_iter().map(to_screen).collect();
                            if v.len() >= 3 {
                                painter.add_shape(v, true, col, Stroke::NONE);
                            }
                        }
                        figcraft_layout::GridLine::Ring { center, radius, width } => {
                            let c = to_screen(center);
                            let rim = to_screen(figcraft_geom::Point::new(center.x + radius, center.y));
                            let r = (rim - c).length();
                            let scale = if radius.abs() > f64::EPSILON { r / radius as f32 } else { 1.0 };
                            painter.circle_stroke(c, r, s(col, if width > 0.0 { (width as f32 * scale).max(1.0) } else { 1.0 }));
                        }
                    }
                }
            }
        }
        // Same gate as the egui reference: instances keep their name label too, so a click on
        // it selects the instance and a double click renames it.
        let labelled = focus.is_none() && (n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. })) && !n.is_board_atom();
        if labelled && let Some(q) = super::frame_labels::draw(app, painter, t, &doc, *c, sel.contains(c), renaming == Some(*c)) {
            labels.push((*c, q.map(|p| app.to_page(p))));
        }
    }

    // ── Hover 轮廓（测量时改为用红色标出被量的框）。
    // 名称行右侧的开发状态按钮也算「悬停在这个框上」：和悬停名称文字得到同一条 hover 轮廓。
    // Focus view draws no frame labels, so its status slots are not hover targets either.
    let hover =
        ds.hover.or_else(|| app.pointer.filter(|_| ds.focused().is_none()).and_then(|p| super::frame_labels::status_slot_owner(app, &doc, page, p)));
    if let Some(h) = hover
        && !sel.contains(&h)
        && matches!(app.session.tool.tool, ToolId::Move | ToolId::Scale)
        && app.session.prefs.highlight_on_hover
        && app.session.tool.overlay.measure_box.is_none()
    {
        // 所有节点（含自动布局容器）的描边统一走源码的 draw_outline（选中色/线宽一致）。
        // 自动布局的边距/间距带不在这里画：它是 `handles` 的 hover 手柄（原实现同一处），
        // 悬停到哪条带子才亮哪条，选中时不会常驻。
        draw_outline(app, painter, &doc, h, stroke(outline_colour(&doc, h, accent), 2.0));
    }

    // ── 布尔组内部：每个操作数的轮廓，便于看清实时结果是怎么来的。
    let operands = figcraft_engine::booleans::outlined_operands(&doc, &sel, ds.vector_edit.as_ref().map(|ve| ve.id));
    for id in operands.iter().filter(|o| !sel.contains(o)) {
        draw_outline(app, painter, &doc, *id, stroke(outline_colour(&doc, *id, accent).gamma_multiply(0.6), 1.0));
    }

    // ── 蒙版轮廓（蒙版本体不绘制）、帧轮廓、切图（虚线），由 View 菜单控制。
    if view.mask_outlines || view.frame_outlines || view.show_slices {
        for id in doc.subtree(page) {
            let Some(n) = doc.node(id).filter(|n| n.visible) else { continue };
            if view.mask_outlines && n.is_mask {
                draw_outline(app, painter, &doc, id, stroke(accent, 1.0));
            } else if view.frame_outlines && n.kind.is_frame_like() {
                draw_outline(app, painter, &doc, id, stroke(t.text_secondary, 1.0));
            } else if view.show_slices
                && matches!(n.kind, NodeKind::Slice)
                && let Some(q) = node_quad(app, &doc, id)
            {
                for sh in Shape::dashed_line(&[q[0], q[1], q[2], q[3], q[0]], stroke(t.text_secondary, 1.0), 4.0, 3.0) {
                    painter.add(sh);
                }
            }
        }
    }

    // ── 拖放目标 / 插入指示 / 网格单元。
    let ov = &app.session.tool.overlay;
    if let Some(d) = ov.drop_target
        && let Some(q) = node_quad(app, &doc, d)
    {
        quad(painter, q, stroke(if figcraft_engine::components::is_slot(&doc, d) { SLOT } else { accent }, 1.0));
    }
    if let Some([x0, y0, x1, y1]) = ov.insert_line {
        painter.line_segment([app.to_screen(x0, y0), app.to_screen(x1, y1)], s(accent, 2.0));
    }
    if let Some([x0, y0, x1, y1]) = ov.drop_cell {
        let r = Rect::from_two_pos(app.to_screen(x0, y0), app.to_screen(x1, y1));
        painter.rect_filled(r, CornerRadius::same(2), accent.gamma_multiply(0.12));
        let pts = [r.min, pos2(r.max.x, r.min.y), r.max, pos2(r.min.x, r.max.y), r.min];
        for sh in Shape::dashed_line(&pts, stroke(accent, 1.5), 4.0, 3.0) {
            painter.add(sh);
        }
    }

    // ── 矢量编辑：区域斜线剖面（在段与点下面）+ 点/段/手柄。
    if let Some(ve) = ds.vector_edit.as_ref() {
        super::builder::region_hatch(app, painter);
        // 照抄 `ui-egui/src/canvas/overlays.rs:154-175`：先画段，再画手柄，最后画点。
        // hover 由引擎在指针事件里算好（`tool.overlay.vector_hover`），这里只读。
        let ov = &app.session.tool.overlay;
        let hover = ov.vector_hover.filter(|_| ov.vector_hover_on.unwrap_or(ve.id) == ve.id);
        if let Some(o) = figcraft_engine::vectoredit::overlay(&doc, ve) {
            vector_overlay(app, painter, &o, hover, accent);
        }
    }

    // ── 键盘点选光标（egui `canvas/overlays.rs` 的 `pick_outline`）：粉色 2px 描边。
    if let Some(p) = app.picked() {
        draw_outline(app, painter, &doc, p, stroke(super::handles::PINK, 2.0));
    }

    // ── 选中：每个选中层的真实轮廓 + 变换框。
    // 选中框的尺寸标签，测量标签要避开它。
    let mut size_label = None;
    let editing = ds.text_edit.is_some();
    // 运动模式播放中：选中边框隐藏（物体在这一帧已不在选中框的位置，留在原地只会造成误导；
    // gizmo / 运动路径也已在播放时藏起，这里保持一致，用户 2026-11-12）。
    if !sel.is_empty() && !editing && ds.vector_edit.is_none() && !app.motion_overlay.playing {
        if std::env::var_os("FIGCRAFT_DEBUG_OUTLINE").is_some() {
            let sb = selection_box(&doc, &sel);
            let sc = sb.map(|b| b.map(|p| app.to_presented(p.x, p.y)));
            let img = app.canvas.tex_view.as_ref().map(|v| {
                let tl = app.to_screen(v.origin.0, v.origin.1);
                let z = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
                (v.origin, v.scale, v.width, v.height, z, super::presented_image_rect(tl, v, z, app.ppp, false))
            });
            eprintln!("[selection] sel={:?} box_page={:?} box_screen={:?} shift={:?} image={:?}", sel, sb, sc, super::content_shift(app), img);
            let handles = app.session.shape_handles();
            eprintln!("[shape] {:?}", handles.iter().map(|(h, [x, y])| (format!("{h:?}"), [*x, *y], app.to_presented(*x, *y))).collect::<Vec<_>>());
            for id in &sel {
                debug_tree(app, &doc, *id, 0);
            }
        }
        let slot_sel = sel.iter().any(|i| outline_colour(&doc, *i, accent) == SLOT);
        let comp_sel = sel.iter().any(|i| outline_colour(&doc, *i, accent) == COMPONENT);
        let col = if slot_sel {
            SLOT
        } else if comp_sel {
            COMPONENT
        } else {
            accent
        };
        // 选中：只画一圈四边平直的变换框（尺寸框 + 手柄），不再沿每个物体的真实几何包边。
        // 多选和单选一样有这条框：它就是选中框，四条边永远平直（2026-11-27，用户要求）。
        if app.session.tool.tool != ToolId::ShapeBuilder
            && let Some(b) = selection_box(&doc, &sel)
            && let Some((_, w, h)) = selection_frame(&doc, &sel)
        {
            let is_line = sel.len() == 1 && matches!(sel.first().and_then(|i| doc.node(*i)).map(|n| &n.kind), Some(NodeKind::Line));
            // 选中的连接线：手柄就是它的两个端点（可拖到别的对象上）。
            let connector = figcraft_engine::board::selected_connector_ends(&doc, &sel).map(|(_, ends)| ends);
            // 尺寸标签：宽或高是 Fill / Hug 时，在对应数字后隔一个空格显示术语（右栏同款）。
            let sizing_label = |axis: figcraft_doc::Sizing| -> String {
                match axis {
                    figcraft_doc::Sizing::Fill => format!(" {}", figcraft_i18n::tr("Fill|sizing")),
                    figcraft_doc::Sizing::Hug => format!(" {}", figcraft_i18n::tr("Hug")),
                    figcraft_doc::Sizing::Fixed => String::new(),
                }
            };
            let (sw, sh) = if sel.len() == 1 {
                doc.node(sel[0]).map(|n| (n.child_layout.sizing_h, n.child_layout.sizing_v)).unwrap_or_default()
            } else {
                Default::default()
            };
            // 组件变体集：尺寸标签改显示变体数（如 "4 Variants"），不显示宽×高。
            let set_variants = if sel.len() == 1 {
                doc.node(sel[0])
                    .filter(|n| matches!(n.kind, NodeKind::ComponentSet { .. }))
                    .map(|n| n.children.iter().filter(|c| doc.node(**c).is_some_and(|cn| matches!(cn.kind, NodeKind::Component { .. }))).count())
            } else {
                None
            };
            let mut label = if let Some(count) = set_variants {
                figcraft_i18n::trn("{n} variants", count, &[])
            } else if is_line {
                fmt(w)
            } else {
                format!("{}{} × {}{}", fmt(w), sizing_label(sw), fmt(h), sizing_label(sh))
            };
            let scale = app.session.tool.overlay.scale;
            if let Some(hint) = scale {
                label = format!("{label} · {}x", fmt(hint.factor.abs()));
            }
            // 屏幕上小于手柄的层只显示轮廓（手柄仍在框外应答）。
            let tiny = figcraft_engine::tools::box_sides_px(&b, zoom).1 < figcraft_engine::tools::SMALL_BOX_PX;
            let measuring = app.session.tool.overlay.measure_box.is_some();
            let cropping = app.crop.is_some();
            let handles = if cropping
                || measuring
                || app.session.tool.dev_mode
                || connector.is_some()
                || sel.iter().any(|i| figcraft_engine::tools::locked_in(&doc, *i))
            {
                0
            } else if is_line {
                2
            } else if tiny {
                0
            } else {
                4
            };
            // 组件变体 / 组件变体集才挂「+」：独立组件没有，普通图层没有。
            let add_variant = sel.len() == 1
                && sel.first().and_then(|i| doc.node(*i)).is_some_and(|n| {
                    matches!(n.kind, NodeKind::ComponentSet { .. }) || figcraft_engine::components::variant_set(&doc, n.id).is_some()
                });
            size_label = transform_box(
                app,
                painter,
                b.map(|p| app.to_presented(p.x, p.y)),
                col,
                true,
                handles,
                (!measuring && connector.is_none()).then_some(label),
                add_variant,
            );
            if cropping {
                crop_marks(app, painter, &doc, t);
            }
            for end in connector.into_iter().flatten().filter(|_| !app.session.tool.dev_mode) {
                let c = app.to_screen(end.x, end.y);
                painter.circle_filled(c, 5.0, Color32::WHITE);
                painter.circle_stroke(c, 5.0, stroke(col, 1.5));
            }
            // 圆弧手柄 + 圆角/星形比例手柄：与原实现同处、同顺序（变换框之后、测量时隐藏）。
            if !measuring {
                arc_overlay(app, painter, col);
                shape_overlay(app, painter, col);
            }
            if app.session.tool.tool == ToolId::Scale {
                // 缩放时保持不动的点：拖拽中的支点，否则锚点。
                let pivot = scale
                    .map(|h| figcraft_geom::Point::new(h.pivot[0], h.pivot[1]))
                    .or_else(|| figcraft_engine::tools::scale_anchor_point(&doc, &sel, app.session.tool.scale_anchor));
                if let Some(p) = pivot {
                    let c = app.to_screen(p.x, p.y);
                    painter.circle_filled(c, 4.0, Color32::WHITE);
                    painter.circle_stroke(c, 4.0, stroke(col, 1.5));
                    painter.circle_filled(c, 1.5, col);
                }
            }
            // 旋转原点（Alt+R 显示时）：一个带十字的环。
            if let Some(o) = ds.origin_handle() {
                let c = app.to_screen(o.x, o.y);
                painter.circle_filled(c, 5.0, Color32::WHITE);
                painter.circle_stroke(c, 5.0, stroke(col, 1.5));
                for d in [vec2(1.0, 0.0), vec2(0.0, 1.0)] {
                    painter.line_segment([c - d * 10.0, c - d * 5.0], stroke(col, 1.5));
                    painter.line_segment([c + d * 5.0, c + d * 10.0], stroke(col, 1.5));
                }
                painter.circle_filled(c, 1.5, col);
            }
        }
    }

    // ── 形状构建器：各区域边与剖面。
    if app.session.tool.tool == ToolId::ShapeBuilder
        && let Some(arr) = figcraft_engine::shapebuilder::current(&app.session)
    {
        super::builder::builder_overlay(app, painter, t, &arr);
    }

    // ── 缩放工具拖出的区域。
    if let (Some(start), Some(p)) = (app.zoom_drag, app.pointer) {
        let area = Rect::from_two_pos(start, p);
        if area.width() > 4.0 || area.height() > 4.0 {
            painter.rect_filled(area, CornerRadius::ZERO, accent.gamma_multiply(0.1));
            painter.rect_stroke(area, CornerRadius::ZERO, stroke(accent, 1.0), StrokeKind::Inside);
        }
    }

    // ── 框选框（指针拖出的优先，其次是键盘框选 `selectArea` 建的框）。
    if let Some([x0, y0, x1, y1]) = ov.marquee.or(app.live_key_box()) {
        let r = Rect::from_two_pos(app.to_screen(x0, y0), app.to_screen(x1, y1));
        painter.rect_filled(r, CornerRadius::ZERO, accent.gamma_multiply(0.1));
        painter.rect_stroke(r, CornerRadius::ZERO, stroke(accent, 1.0), StrokeKind::Inside);
    }

    // ── 钢笔/铅笔路径。
    if !ov.pen.is_empty() {
        let pts: Vec<Pos2> = ov.pen.iter().map(|[x, y]| app.to_screen(*x, *y)).collect();
        if matches!(app.session.tool.tool, ToolId::Pencil | ToolId::Brush) {
            // 将要生成的笔画：用 `tool.drawStyle` 的颜色与粗细（缩到很小时至少 1px），
            // 与 `pencil_finish` 真正写入文档的笔触一致（参考实现 `drawing_toolbar`）。
            // 笔刷的每一段还带自己的宽度系数（`pen_widths`，来自压力），逐段画才看得出压感——
            // 以前这里是一条等宽折线，压感只在抬手后才"跳"出来。
            let tool = app.session.tool.tool;
            let style = app.session.tool.draw_style(tool).cloned().unwrap_or_default();
            // The engine knows the colour the stroke will really have (a board's ink is the pen's
            // ink colour, not the tool's drawing colour), so it wins over the style's.
            let color = crate::theme::doc_c32(ov.pen_color.unwrap_or_else(|| style.color()));
            let weight = ov.pen_weight.unwrap_or_else(|| style.weight(tool));
            let widths = &ov.pen_widths;
            if widths.len() == pts.len() && widths.iter().any(|w| (*w - 1.0).abs() > 1e-3) {
                for (i, seg) in pts.windows(2).enumerate() {
                    let (a, b) = (widths[i], widths[i + 1]);
                    let w = (((a + b) / 2.0 * weight * zoom) as f32).max(1.0);
                    painter.line_segment([seg[0], seg[1]], s(color, w));
                }
                // Round joints: segments of different widths meet with a notch otherwise.
                for (p, w) in pts.iter().zip(widths) {
                    let r = ((w * weight * zoom) as f32 / 2.0).max(0.5);
                    painter.circle_filled(*p, r, color);
                }
            } else {
                painter.add_shape(pts, false, Color32::TRANSPARENT, s(color, ((weight * zoom) as f32).max(1.0)));
            }
        } else {
            for [x0, y0, x1, y1] in &ov.pen_handles {
                bezier_handle(painter, app.to_screen(*x0, *y0), app.to_screen(*x1, *y1), 3.5, Color32::WHITE);
            }
            for p in &pts {
                painter.circle_filled(*p, 3.5, Color32::WHITE);
                painter.circle_stroke(*p, 3.5, stroke(accent, 1.0));
            }
            if let Some([x0, y0, x1, y1]) = ov.rubber {
                let (a, b) = (app.to_screen(x0, y0), app.to_screen(x1, y1));
                // 单击将生成的线段：沿上一个锚点的出方向手柄（如果有）。
                match ov.rubber_handle {
                    Some([hx, hy]) => {
                        let h = app.to_screen(hx, hy);
                        painter.add_shape(vec![a, h, b, b], false, Color32::TRANSPARENT, stroke(accent, 1.0));
                    }
                    None => {
                        painter.line_segment([a, b], stroke(accent, 1.0));
                    }
                }
            }
        }
    } else if let Some([x0, y0, x1, y1]) = ov.rubber {
        // 正在画的连接线，或拖动连接线端点：从起点到指针。
        painter.line_segment([app.to_screen(x0, y0), app.to_screen(x1, y1)], stroke(accent, 1.5));
    }

    // ── 橡皮擦笔刷（矢量编辑模式）。
    if let Some([x, y, r]) = ov.brush {
        let c = app.to_screen(x, y);
        let rs = (r * zoom) as f32;
        painter.circle_stroke(c, rs, stroke(Color32::WHITE, 3.0));
        painter.circle_stroke(c, rs, stroke(accent, 1.5));
    }

    // ── 套索（矢量编辑模式）。
    if ov.lasso.len() > 1 {
        let mut pts: Vec<Pos2> = ov.lasso.iter().map(|[x, y]| app.to_screen(*x, *y)).collect();
        if let Some(first) = pts.first().copied() {
            pts.push(first);
        }
        for sh in Shape::dashed_line(&pts, stroke(accent, 1.0), 4.0, 3.0) {
            painter.add(sh);
        }
    }

    // ── 文字编辑：光标、选中，以及输入法组合（preedit）的下划线与组合光标。
    // 组合中的文字还没进文档，用引擎给的「组合文档」排版，光标/选区才不会错位。
    let preedit = figcraft_engine::textedit::preedit_doc(&app.session);
    let edoc: &Document = preedit.as_ref().map(|(d, _, _)| d).unwrap_or(&doc);
    if let Some(te) = ds.text_edit.clone()
        && let Some(text) = super::outline::TextOnScreen::of(edoc, te.id)
    {
        if let Some(q) = node_quad(app, edoc, te.id) {
            quad(painter, q, stroke(accent, 1.0));
        }
        let (a, b) = (te.caret.min(te.anchor), te.caret.max(te.anchor));
        if a != b && preedit.is_none() {
            // 选区色与普通输入框共用同一份定义。
            let selection_colour = caret::selection(t);
            for q in text.selection(app, a, b) {
                painter.add(Shape::convex_polygon(q.to_vec(), selection_colour, Stroke::NONE));
            }
        }
        if let Some((_, (mark_start, mark_end), _)) = preedit.as_ref() {
            for r in text.layout.selection_rects(*mark_start, *mark_end) {
                // 下划线画在基线下方三分之一行高处，只覆盖组合中的字符。
                let Some(line) = text.layout.lines.iter().find(|l| (l.top - r.y0).abs() < 1e-6) else { continue };
                let x1 = text
                    .layout
                    .glyphs
                    .get(line.glyph_start..line.glyph_end)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|g| g.char_end > g.char_start && g.char_start >= *mark_start && g.char_end <= *mark_end)
                    .map(|g| g.x + g.advance)
                    .fold(r.x0, f64::max);
                let y = line.baseline + (line.top + line.height - line.baseline) * 0.35;
                painter.line_segment([text.at(app, r.x0, y), text.at(app, x1, y)], stroke(t.text, 1.0));
            }
        }
        let caret_index = preedit.as_ref().map(|(_, _, cursor)| *cursor).unwrap_or(te.caret);
        // 光标粗细、圆角、颜色和闪烁都跟普通输入框共用同一套参数。
        if app.text_caret_visible {
            let [a, b] = text.caret(app, caret_index, zoom);
            let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
            if y1 > y0 {
                let r = Rect::from_min_size(pos2(a.x, y0), vec2(caret::WIDTH, y1 - y0));
                painter.rect_filled(r, CornerRadius::same(caret::RADIUS), caret::color(t));
            }
        }
    }

    // ── 智能参考线与测量标注（标签避开尺寸标签）。
    super::measure_overlay::draw(app, painter, t, size_label);

    // ── 标尺参考线（在标尺下面）。
    super::guides::draw(app, painter, &app.guides);

    // ── 指针：自绘字形（旋转、钢笔）画在最上面。
    if let (Some(super::cursors::Shown::Drawn(g)), Some(at)) = (app.cursor, app.pointer) {
        super::cursors::paint(painter, at, g);
    }
    labels
}

/// 选中椭圆的圆弧手柄（悬停/拖动中的那个略大，Start 中间带一个小点），
/// 拖动时在旁边显示读数 —— 照搬 `ui-egui/src/canvas/overlays.rs` 的 `arc_overlay`。
/// Crop mode's own marks: the crop rectangle, its corner squares, and the L-brackets / edge bars.
fn crop_marks(app: &CanvasApp, painter: &mut Painter, doc: &Document, t: &Tokens) {
    let Some((id, index)) = app.crop else { return };
    let Some(n) = doc.node(id) else { return };
    let Some(PaintKind::Image(img)) = n.fills.get(index).map(|p| &p.kind) else { return };
    if img.scale_mode != ImageScaleMode::Crop {
        return;
    }
    let Some(data) = doc.images.get(&img.image) else { return };
    let crop = img.effective_crop(f64::from(data.width), f64::from(data.height), n.width, n.height);
    let (x0, y0, x1, y1) = ImagePaint::crop_rect(crop);
    let world = doc.world_transform(id);
    let corners =
        [(x0 * n.width, y0 * n.height), (x1 * n.width, y0 * n.height), (x1 * n.width, y1 * n.height), (x0 * n.width, y1 * n.height)].map(|(x, y)| {
            let p = world * Point::new(x, y);
            app.to_screen(p.x, p.y)
        });
    let accent = accent(app);
    quad(painter, corners, stroke(Color32::WHITE, 2.0));
    quad(painter, corners, stroke(accent, 1.0));
    for p in corners {
        let r = Rect::from_center_size(p, vec2(8.0, 8.0));
        painter.rect_filled(r, CornerRadius::ZERO, Color32::WHITE);
        painter.rect_stroke(r, CornerRadius::ZERO, stroke(accent, 1.0), super::geom::StrokeKind::Inside);
    }
    const W: f32 = 5.0;
    let halo = stroke(Color32::from_black_alpha(50), W + 2.0);
    let mark = stroke(accent, W);
    let mut draw = |pts: Vec<Pos2>| {
        painter.add_shape(pts.clone(), false, Color32::TRANSPARENT, halo);
        painter.add_shape(pts, false, Color32::TRANSPARENT, mark);
    };
    for i in 0..4 {
        let (p, prev, next) = (corners[i], corners[(i + 3) % 4], corners[(i + 1) % 4]);
        let (to_prev, to_next) = ((prev - p).normalized(), (next - p).normalized());
        let at = p + (to_prev + to_next) * (W / 2.0);
        let leg = |dir: Vec2, len: f32| at + dir * 22.0_f32.min(len / 3.0);
        draw(vec![leg(to_prev, (prev - p).length()), at, leg(to_next, (next - p).length())]);
        let len = (next - p).length();
        let mid = p + (next - p) * 0.5 + to_prev * (W / 2.0);
        let half = to_next * 12.0_f32.min(len / 6.0);
        draw(vec![mid - half, mid + half]);
    }
    let _ = t;
}

fn arc_overlay(app: &CanvasApp, painter: &mut Painter, col: Color32) {
    use figcraft_engine::arc_handles::ArcHandle;
    let Some(hs) = app.session.arc_handles() else { return };
    let ov = &app.session.tool.overlay;
    for h in [ArcHandle::Ratio, ArcHandle::Start, ArcHandle::Sweep] {
        let Some([x, y]) = hs.at(h) else { continue };
        let c = app.to_screen(x, y);
        let r = if ov.arc_hover == Some(h) { 5.0 } else { 4.0 };
        painter.circle_filled(c, r, Color32::WHITE);
        painter.circle_stroke(c, r, stroke(col, 1.0));
        if h == ArcHandle::Start {
            painter.circle_filled(c, 1.5, col);
        }
    }
    if let Some(hint) = &ov.arc {
        let c = app.to_screen(hint.at[0], hint.at[1]);
        pill(painter, c + vec2(12.0, 12.0), Align2::LEFT_TOP, hint.label.clone(), col);
    }
}

/// 临时诊断：选中的节点和它的子孙在页面/屏幕上的矩形。
fn debug_tree(app: &CanvasApp, doc: &Document, id: NodeId, depth: usize) {
    let n = doc.node(id);
    let wb = doc.world_bounds(id);
    let q = node_quad(app, doc, id);
    eprintln!("[tree] depth={depth} id={id:?} name={:?} kind={:?} page={:?} screen={:?}", n.map(|n| n.name.as_str()), n.map(|n| &n.kind), wb, q);
    if depth < 2
        && let Some(n) = n
    {
        for c in n.children.iter() {
            debug_tree(app, doc, *c, depth + 1);
        }
    }
}

/// 选中形状的圆角手柄与星形的比例手柄（悬停/拖动中的那个略大），拖动时在旁边显示数值。
///
/// 照搬 `ui-egui/src/canvas/overlays.rs` 的 `shape_overlay`：几何由引擎
/// `session.shape_handles()` 给（页面坐标），这里只负责白底彩边的圆点和读数 pill。
fn shape_overlay(app: &CanvasApp, painter: &mut Painter, col: Color32) {
    use figcraft_engine::shape_handles::ShapeHandle;
    let ov = &app.session.tool.overlay;
    for (h, [x, y]) in app.session.shape_handles() {
        let r = if ov.shape_hover == Some(h) { 4.5 } else { 3.5 };
        let c = app.to_presented(x, y);
        painter.circle_filled(c, r, Color32::WHITE);
        painter.circle_stroke(c, r, stroke(col, 1.0));
    }
    if let Some(hint) = &ov.shape {
        let c = app.to_presented(hint.at[0], hint.at[1]);
        let text = match hint.handle {
            ShapeHandle::Radius { .. } => format!("{} {}", figcraft_i18n::tr("Radius"), hint.value),
            ShapeHandle::Ratio => format!("{} {}%", figcraft_i18n::tr("Ratio"), hint.value),
            ShapeHandle::Count => format!("{} {}", figcraft_i18n::tr("Count"), hint.value),
        };
        pill(painter, c + vec2(12.0, 12.0), Align2::LEFT_TOP, text, col);
    }
}

/// 贝塞尔控制点：半径 `size` 的**圆点**（egui/参考实现 的 `bezier_handle` 都是
/// `circle_filled` + `circle_stroke`，矢量编辑与钢笔绘制共用同一形状）。
fn bezier_handle(painter: &mut Painter, a: Pos2, b: Pos2, size: f32, col: Color32) {
    painter.line_segment([a, b], stroke(col.gamma_multiply(0.6), 1.0));
    painter.circle_filled(b, size, Color32::WHITE);
    painter.circle_stroke(b, size, stroke(Color32::from_gray(0), 1.0));
}

/// 选中框下方「+」的矩形：尺寸框顶边 = 对象下边缘 + 9，框高约 17，故加号顶边 = 下边缘 + 30。
/// 绘制与命中共用，保证按钮画在哪就能点在哪。
pub fn add_variant_rect(cx: f32, y1: f32) -> Rect {
    // 尺寸框顶边 = y1 + 7，高 = 13 + 上下各 2 = 17，底部再留 4 → + 顶边 y1 + 28。
    let top = y1 + 7.0 + SIZE_PILL_GLYPH_H + 4.0 + 4.0;
    super::frame_labels::add_variant_slot(cx, top - super::frame_labels::ADD_VARIANT_TOP)
}

/// 变换框（屏幕四角 tl、tr、br、bl）：`outline` 时画轮廓，前 `handles` 个角手柄，
/// 显示尺寸时在下方画尺寸 `label`（返回尺寸 pill 的矩形）。
/// 单位方向；零长度退化为 x 轴，避免旋转框手柄出现 NaN。
fn unit_edge(a: Pos2, b: Pos2) -> Vec2 {
    let d = b - a;
    let len = d.length();
    if len.is_finite() && len > 1e-4 { d / len } else { vec2(1.0, 0.0) }
}

fn transform_box(
    app: &CanvasApp,
    painter: &mut Painter,
    q: [Pos2; 4],
    col: Color32,
    outline: bool,
    handles: usize,
    label: Option<String>,
    add_variant: bool,
) -> Option<Rect> {
    if outline {
        // 变换框也走同一条闭合路径描边：中心线贴住框边，只占 1 个设备像素。
        let crisp = super::outline::axis_aligned(q).is_some_and(|r| {
            // 调用方的 `q` 已经带着贴图吸附位移；这里不能再加一次。
            super::outline::outline_ring(painter, r, 1.0, app.ppp, Vec2::ZERO, col)
        });
        if !crisp {
            quad(painter, q, stroke(col, 1.0));
        }
    }
    let turned_box = super::outline::axis_aligned(q).is_none();
    for c in q.iter().take(handles) {
        if turned_box && handles == 4 {
            // 旋转框的手柄也沿框的两条边转，不能还是一枚屏幕轴对齐的小方块。
            let ex = unit_edge(q[0], q[1]);
            let ey = unit_edge(q[0], q[3]);
            let hc = [*c - ex * 4.0 - ey * 4.0, *c + ex * 4.0 - ey * 4.0, *c + ex * 4.0 + ey * 4.0, *c - ex * 4.0 + ey * 4.0];
            painter.add_shape(hc.to_vec(), true, Color32::WHITE, stroke(col, 1.0));
        } else {
            let r = Rect::from_center_size(*c, vec2(8.0, 8.0));
            painter.rect_filled(r, CornerRadius::same(1), Color32::WHITE);
            painter.rect_stroke(r, CornerRadius::same(1), stroke(col, 1.0), StrokeKind::Inside);
        }
    }
    if let Some(label) = label.filter(|_| app.session.prefs.show_dimensions) {
        // Keep the requested layer-colour background, but choose a legible foreground: a light
        // Yellow (#FFCC00) with white text was ~1.5:1. Dark ink on light fills, white on dark.
        let fg = contrasting_ink(col);
        let axis_aligned_box = super::outline::axis_aligned(q).is_some();
        let pill = if axis_aligned_box {
            let bottom = q.iter().map(|p| p.y).fold(f32::MIN, f32::max);
            let cx = q.iter().map(|p| p.x).sum::<f32>() / 4.0;
            // 尺寸框铁律：文字框顶边距对象下边缘 9；背景块距文字上下各 2、左右各 6，圆角 3。
            // （`pill_with_bg` 的 `+ vec2(pad_x*2, 4)` 与 `galley` 的 `+2` 纵向内缩即上下 2。）
            pill_with_bg(painter, pos2(cx, bottom + 7.0), Align2::CENTER_TOP, label, col, fg, col, 6.0)
        } else {
            // 旋转的框：尺寸框锚在下边中点外侧 7，并绕该点随下边一起转（文字与底色同转）。
            rotated_size_pill(painter, q, label, col, fg)
        };
        // 组件变体 / 组件变体集的「+」：紧贴在尺寸框下方（同一居中 x）。画在选中框这条路径上，
        // 因为变体在 ComponentSet 内部、不会走顶层帧标签 frame_labels::draw 那条路径。
        // 旋转的框不挂「+」：它的命中区仍是轴对齐矩形，转过去会错位。
        if add_variant && axis_aligned_box {
            let bottom = q.iter().map(|p| p.y).fold(f32::MIN, f32::max);
            let cx = q.iter().map(|p| p.x).sum::<f32>() / 4.0;
            super::frame_labels::add_variant_button(painter, add_variant_rect(cx, bottom));
        }
        return Some(pill);
    }
    None
}

/// 画布读数：`col` 底色的 pill 上白字（返回 pill 区域）。
pub(super) fn pill(painter: &mut Painter, at: Pos2, align: Align2, text: String, col: Color32) -> Rect {
    pill_with_bg(painter, at, align, text, col, Color32::WHITE, under_white(col), 4.0)
}

/// 尺寸框铁律：背景块宽度 = 文字实测宽 + 左右固定边距（各 `pad_x`），高度 = 文字高 + 上下各 2；
/// 圆角 3。文字宽度用当前字号逐字量，不写死任何总宽 —— 数字位数不同框就跟着变。
/// 尺寸框文字高：设计稿 Group 234 / Frame 232 内文字 `205x203`（11pt）实测 13，
/// 背景框 61×17 = 文字 13 + 上下各 2，圆角 3，padding 左右各 6。
const SIZE_PILL_GLYPH_H: f32 = 13.0;

pub(super) fn pill_rect(text: &str, at: Pos2, align: Align2, pad_x: f32) -> (Rect, Vec2) {
    let w = text_size(text, semibold(11.0)).x;
    // 框宽 = 文字宽 + 左右各 pad_x；框高 = 真实字形高 + 上下各 2。
    let glyph = vec2(w, SIZE_PILL_GLYPH_H);
    let size = glyph + vec2(pad_x * 2.0, 4.0);
    (align.anchor_size(at, size), glyph)
}

fn pill_with_bg(painter: &mut Painter, at: Pos2, align: Align2, text: String, _col: Color32, fg: Color32, bg: Color32, pad_x: f32) -> Rect {
    let (r, glyph) = pill_rect(&text, at, align, pad_x);
    // 背景交给 GPUI 按**真实字形宽度**画（Label.pill）：padding 精确到上下 2、左右 6，
    // 圆角 3，文字上下对称居中。不再用估算宽画框再塞字（那样留白不均、贴不紧文字外框）。
    painter.labels.push(Label {
        rect: r,
        text,
        font: semibold(11.0),
        color: fg,
        align: Align2::CENTER_CENTER,
        angle: None,
        pill: Some((bg, pad_x, 2.0)),
    });
    let _ = glyph;
    r
}

/// 旋转框最靠下的边：按**外法线朝下**判断，不能用中点 y（旋转 30° 时竖边中点可能比下边中点更低）。
fn bottom_edge(q: [Pos2; 4]) -> (Pos2, Pos2) {
    let center = pos2(q.iter().map(|p| p.x).sum::<f32>() / 4.0, q.iter().map(|p| p.y).sum::<f32>() / 4.0);
    let mut best = (q[3], q[2], f32::MIN);
    for (a, b) in [(q[0], q[1]), (q[1], q[2]), (q[2], q[3]), (q[3], q[0])] {
        let d = b - a;
        let len = d.length();
        if !(len.is_finite() && len > 1e-4) {
            continue;
        }
        let d = d / len;
        let mid = pos2((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
        let mut n = vec2(-d.y, d.x);
        if (mid - center).dot(n) < 0.0 {
            n = -n;
        }
        if n.y > best.2 {
            best = (a, b, n.y);
        }
    }
    (best.0, best.1)
}

/// 旋转后的尺寸框：锚在框的下边中点向外 7，文字和底色都随下边角度一起转。
/// 返回它的轴对齐外接矩形（测量标签避让用）。
fn rotated_size_pill(painter: &mut Painter, q: [Pos2; 4], text: String, bg: Color32, fg: Color32) -> Rect {
    let mut bottom = bottom_edge(q);
    // 读数从左到右，文字才正着写；换算出的角度永远在 (-90°, 90°]。
    if bottom.0.x > bottom.1.x {
        bottom = (bottom.1, bottom.0);
    }
    let (bl, br) = bottom;
    let delta = br - bl;
    let len = delta.length();
    let d = if len.is_finite() && len > 1e-4 { delta / len } else { vec2(1.0, 0.0) };
    // 朝框外（屏幕下方）的法线；本地 +y 向下。
    let n = vec2(-d.y, d.x);
    let angle = d.y.atan2(d.x);
    let mid = pos2((bl.x + br.x) * 0.5, (bl.y + br.y) * 0.5);
    // 尺寸框铁律同水平版：文字顶边距对象下边缘 7，左右各 6、上下各 2，圆角 3。
    let glyph = text_size(&text, semibold(11.0));
    let size = glyph + vec2(12.0, 4.0);
    // 尺寸框**以底边中点为中心**，左右沿边各伸一半；只把顶边压到中点外 7，
    // 不能再沿 `d` 平移半个宽度，否则整个框会跑到右下角（用户 2026-11-27）。
    let center = mid + n * (7.0 + size.y * 0.5);
    painter.rounded_quad_filled(center, d, n, size, 3.0, bg);
    painter.labels.push(Label {
        rect: Rect::from_center_size(center, size),
        text,
        font: semibold(11.0),
        color: fg,
        align: Align2::CENTER_CENTER,
        angle: Some(angle),
        pill: None,
    });
    let corners = [
        center - d * (size.x * 0.5) - n * (size.y * 0.5),
        center + d * (size.x * 0.5) - n * (size.y * 0.5),
        center + d * (size.x * 0.5) + n * (size.y * 0.5),
        center - d * (size.x * 0.5) + n * (size.y * 0.5),
    ];
    let (x0, y0) = (corners.iter().map(|p| p.x).fold(f32::MAX, f32::min), corners.iter().map(|p| p.y).fold(f32::MAX, f32::min));
    let (x1, y1) = (corners.iter().map(|p| p.x).fold(f32::MIN, f32::max), corners.iter().map(|p| p.y).fold(f32::MIN, f32::max));
    Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
}

/// WCAG relative-luminance split: near-black ink on light fills, white on dark fills.
fn contrasting_ink(bg: Color32) -> Color32 {
    let [r, g, b, _] = bg.to_array();
    let lin = |v: u8| {
        let c = f32::from(v) / 255.0;
        if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let lum = 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
    if lum > 0.35 { Color32::from_black_alpha(230) } else { Color32::WHITE }
}

/// 白字下面的底色需要压暗才够对比（对应 `theme::under_white`）。
fn under_white(col: Color32) -> Color32 {
    let d = |v: u8| (f32::from(v) * 0.55) as u8;
    Color32::from_rgb(d(col.0[0]), d(col.0[1]), d(col.0[2]))
}

/// 一条闭合四边形轮廓（供其它模块用）。
pub(super) fn _quad(painter: &mut Painter, q: [Pos2; 4], st: Stroke) {
    quad(painter, q, st);
}

#[cfg(test)]
mod size_pill_tests {
    use super::pill_rect;
    use crate::canvas::geom::{Align2, pos2};

    /// 尺寸框铁律：距对象下边缘 7，框高 = 文字 13 + 上下各 2 = 17，
    /// 宽 = 文字宽 + 左右各 6，圆角 3。
    #[test]
    fn size_pill_obeys_the_7_2_6_3_rule() {
        let (r, glyph) = super::pill_rect("110 × 116", pos2(0.0, 0.0), Align2::LEFT_TOP, 6.0);
        assert_eq!(r.height(), 17.0); // 13 字形 + 2 + 2（设计稿 Frame 232）
        assert_eq!(r.width(), glyph.x + 12.0); // 左右各 6
    }

    /// 尺寸框宽度 = 文字实测宽 + 左右各 6，高度 = 文字高 + 上下各 2；位数变化框跟着变。
    #[test]
    fn size_pill_width_follows_text_with_fixed_margins() {
        let (short, _) = pill_rect("1 × 1", pos2(0.0, 0.0), Align2::CENTER_TOP, 6.0);
        let (long, _) = pill_rect("1000 × 1000", pos2(0.0, 0.0), Align2::CENTER_TOP, 6.0);
        assert!(long.width() > short.width() + 20.0, "{} vs {}", long.width(), short.width());
        assert_eq!(short.height(), long.height()); // 上下边距固定 2
    }
}

#[cfg(test)]
mod text_edit_colour_tests {
    use super::*;
    use crate::app::{CanvasApp, sample_doc};
    use crate::canvas::painter::Op;
    use figcraft_engine::textedit;

    /// 画布文字编辑的选区/光标必须和普通输入框同一套颜色，
    /// 不能再用画布强调色和正文墨色。
    #[test]
    fn text_selection_and_caret_match_the_input_field_colours() {
        let mut app = CanvasApp::new(sample_doc());

        app.canvas.rect = Rect { min: pos2(0.0, 0.0), max: pos2(800.0, 600.0) };

        let v = app.session.execute("text.create", &serde_json::json!({"text": "Hello 世界", "x": 100.0, "y": 100.0})).expect("create text");
        let id: NodeId = serde_json::from_value(v["id"].clone()).expect("text id");

        if let Ok(ds) = app.session.ds_mut()
            && let Some(NodeKind::Text(t)) = ds.doc_mut().node_mut(id).map(|n| &mut n.kind)
        {
            t.style.font_family = "MiSans".into();
        }

        textedit::begin(&mut app.session, id, Some(0), false).expect("begin");
        textedit::set_caret(&mut app.session, 5, true).expect("select");

        let doc = app.session.ds().expect("session").doc.clone();
        let Some(text) = crate::canvas::outline::TextOnScreen::of(&doc, id) else { return };
        if text.layout.glyphs.is_empty() {
            return; // no fonts installed: geometry is not testable
        }

        let t = Tokens::dark();
        let mut painter = Painter::new(app.canvas.rect);
        let _ = overlays(&app, &mut painter, &t, doc, None);

        let selection = painter.ops.iter().find_map(|op| match op {
            Op::Poly { fill, .. } if fill.a() > 0 => Some(*fill),
            _ => None,
        });
        assert_eq!(selection, Some(Color32::from_white_alpha(56)), "dark theme selection must be 22% white like the input field");

        let caret = painter.ops.iter().find_map(|op| match op {
            Op::Rect { radius, fill, .. } if *fill == t.accent && *radius == CornerRadius::same(caret::RADIUS) => Some(*fill),
            _ => None,
        });
        assert_eq!(caret, Some(t.accent), "caret must use the same accent and rounded 1pt shape as the input field");
    }
}

#[cfg(test)]
mod selection_shape_tests {
    use super::*;
    use crate::app::CanvasApp;
    use crate::canvas::painter::Op;
    use figcraft_doc::{Corners, Document, NodeKind, Sizing};
    use serde_json::json;

    fn app_with(kind: NodeKind, rotation: f64) -> (CanvasApp, NodeId) {
        let mut doc = Document::new("selection-shape");
        let page = doc.first_page().expect("page");
        let id = doc.create(page, kind, "Layer", 100.0, 100.0, 200.0, 100.0).expect("layer");
        let mut app = CanvasApp::new(doc);
        app.canvas.rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(900.0, 700.0));
        if rotation != 0.0 {
            app.session.execute("node.set", &json!({"id": id, "props": {"rotation": rotation}})).expect("rotate");
        }
        let _ = app.session.set_selection(vec![id]);
        (app, id)
    }

    fn paint(app: &CanvasApp) -> Painter {
        let t = Tokens::dark();
        let doc = app.session.ds().expect("session").doc.clone();
        let mut painter = Painter::new(app.canvas.rect);
        let _ = overlays(app, &mut painter, &t, doc, None);
        painter
    }

    /// A rounded layer gets **one** straight box: no outline traces its corners or geometry.
    #[test]
    fn a_rounded_layer_gets_one_straight_box() {
        let (mut app, id) = app_with(NodeKind::Rectangle, 0.0);
        app.session.ds_mut().expect("session").doc_mut().node_mut(id).expect("layer").corners = Corners { radii: [30.0; 4], ..Default::default() };
        let painter = paint(&app);
        let rings: Vec<_> = painter
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Ring { width, .. } if (*width - 1.0).abs() < 1e-6 => Some(()),
                _ => None,
            })
            .collect();
        assert_eq!(rings.len(), 1, "the selection box is one straight ring");
        assert!(!painter.ops.iter().any(|op| matches!(op, Op::Poly { .. })), "no shape-hugging outline");
    }

    /// The size label's Fill term is the short word, never the raw disambiguated key.
    #[test]
    fn the_size_label_names_fill_short() {
        let (mut app, id) = app_with(NodeKind::Rectangle, 0.0);
        app.session.ds_mut().expect("session").doc_mut().node_mut(id).expect("layer").child_layout.sizing_h = Sizing::Fill;
        let painter = paint(&app);
        let size = painter.labels.iter().find(|l| l.pill.is_some()).expect("size label");
        assert!(size.text.contains("Fill") || size.text.contains("填充"), "{}", size.text);
        assert!(!size.text.contains('|'), "the raw key leaked: {}", size.text);
    }

    /// Two layers that share a rotation still get a page-square multi box.
    #[test]
    fn a_multi_selection_never_turns() {
        let (mut app, first) = app_with(NodeKind::Rectangle, 30.0);
        let page = app.session.ds().expect("session").doc.first_page().expect("page");
        let second =
            app.session.ds_mut().expect("session").doc_mut().create(page, NodeKind::Rectangle, "Second", 260.0, 260.0, 120.0, 80.0).expect("layer");
        app.session.execute("node.set", &json!({"id": second, "props": {"rotation": 30}})).expect("rotate");
        let _ = app.session.set_selection(vec![first, second]);
        let painter = paint(&app);
        let rings: Vec<_> = painter
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Ring { width, .. } if (*width - 1.0).abs() < 1e-6 => Some(()),
                _ => None,
            })
            .collect();
        assert_eq!(rings.len(), 1, "one page-square multi box");
        assert!(!painter.ops.iter().any(|op| matches!(op, Op::Poly { .. })), "a turned box would be a quad");
    }

    /// A rotated layer turns its size label with the bottom edge: anchored below, rotated by 30°.
    #[test]
    fn a_rotated_layer_turns_its_size_label() {
        let (app, id) = app_with(NodeKind::Rectangle, 30.0);
        let painter = paint(&app);
        let label = painter.labels.iter().find(|l| l.angle.is_some()).expect("rotated size label");
        let angle = label.angle.expect("angle");
        // The label must stay upright: ±30°, never a 180°-flipped 150°.
        assert!((angle.to_degrees().abs() - 30.0).abs() < 0.01, "{}", angle.to_degrees());
        // And it must sit on the **lowest edge midpoint**, not float inside the box.
        let doc = app.session.ds().expect("session").doc.clone();
        let q = figcraft_engine::tools::selection_box(&doc, &[id]).expect("box").map(|p| app.to_presented(p.x, p.y));
        let mut edge = bottom_edge(q);
        if edge.0.x > edge.1.x {
            edge = (edge.1, edge.0);
        }
        let (bl, br) = edge;
        let d = (br - bl) / (br - bl).length();
        let n = vec2(-d.y, d.x);
        let mid = pos2((bl.x + br.x) * 0.5, (bl.y + br.y) * 0.5);
        let expected = mid + n * (7.0 + label.rect.height() * 0.5);
        assert!((label.rect.center() - expected).length() < 0.01, "{:?} vs {expected:?}", label.rect.center());
        assert!((label.rect.center().x - (mid.x + n.x * (7.0 + label.rect.height() * 0.5))).abs() < 0.01);
    }

    /// The four corner handles of a rotated box are themselves turned with its edges.
    #[test]
    fn a_rotated_layer_turns_its_handles() {
        let (app, id) = app_with(NodeKind::Rectangle, 30.0);
        let painter = paint(&app);
        let white_quads: Vec<_> = painter
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Poly { points, fill, .. } if points.len() == 4 && *fill == Color32::WHITE => Some(points),
                _ => None,
            })
            .collect();
        assert_eq!(white_quads.len(), 4, "four turned handles");
        let doc = app.session.ds().expect("session").doc.clone();
        let q = figcraft_engine::tools::selection_box(&doc, &[id]).expect("box").map(|p| app.to_presented(p.x, p.y));
        for quad in white_quads {
            let center = pos2(quad.iter().map(|p| p.x).sum::<f32>() / 4.0, quad.iter().map(|p| p.y).sum::<f32>() / 4.0);
            assert!(q.iter().any(|corner| (center - *corner).length() < 0.01), "{center:?} not on {q:?}");
        }
    }

    /// A rotated frame turns its name with the top edge as well.
    #[test]
    fn a_rotated_frame_turns_its_name() {
        let (app, _) = app_with(NodeKind::Frame, 30.0);
        let painter = paint(&app);
        let name = painter.labels.iter().find(|l| l.text == "Layer").expect("frame name");
        assert!(name.angle.is_some(), "the frame name stays horizontal");
        assert!((name.angle.expect("angle").to_degrees().abs() - 30.0).abs() < 0.01);
    }
}

#[cfg(test)]
mod add_variant_rect_tests {
    use super::add_variant_rect;

    /// 加号 = 尺寸框下方 4；与绘制、命中共用同一几何。
    #[test]
    fn add_variant_rect_sits_below_the_size_pill() {
        let r = add_variant_rect(200.0, 403.0);
        assert_eq!((r.min.x, r.min.y, r.width(), r.height()), (192.0, 431.0, 16.0, 16.0));
    }
}

// ── 像素网格 ────────────────────────────────────────────────────────────────

/// 把段表里 `[a, b)` 这段换成 `c`（其余段原样保留）。
fn ink_span(segs: &mut Vec<(f64, f64, Color32)>, a: f64, b: f64, c: Color32) {
    if !(b > a) {
        return;
    }
    let mut out = Vec::with_capacity(segs.len() + 2);
    for &(s, e, col) in segs.iter() {
        if b <= s || a >= e {
            out.push((s, e, col));
            continue;
        }
        if s < a {
            out.push((s, a, col));
        }
        out.push((a.max(s), b.min(e), c));
        if e > b {
            out.push((b, e, col));
        }
    }
    *segs = out;
}

/// 画布像素网格：每文档单位一条 1 设备像素的线，颜色取它盖住的那块像素（`Color::ink`）。
///
/// 这里只认**纯色填充**的节点当底：渐变/图片/半透明按下面那层算（近似），文字按页底算。
/// 每条线先是一整段页底墨色，再按节点 z 序把落在它上面的区间换成节点自己的墨色。
fn pixel_grid(app: &CanvasApp, painter: &mut Painter, doc: &Document, page: NodeId) {
    let r = app.canvas.rect;
    let p0 = app.to_page(r.min);
    let p1 = app.to_page(r.max);
    let hair = 1.0 / app.ppp.max(1.0);
    let ground = match doc.node(page).map(|n| &n.kind) {
        Some(NodeKind::Page { background, .. }) => doc_c32(background.ink()),
        _ => Color32::WHITE,
    };
    // 每个「纯色填充」节点的页坐标矩形 + 墨色，按绘制顺序（后画的盖前面的）。
    let mut marks: Vec<(figcraft_geom::Rect, Color32)> = Vec::new();
    for id in doc.subtree(page) {
        if id == page {
            continue;
        }
        let Some(n) = doc.node(id) else { continue };
        if !n.visible || n.opacity <= 0.01 {
            continue;
        }
        if matches!(n.kind, NodeKind::Text(_)) {
            continue;
        }
        let Some(c) = n.fills.iter().find_map(|f| f.solid_color()) else { continue };
        let Some(b) = doc.world_bounds(id) else { continue };
        if !(b.x0 < b.x1 && b.y0 < b.y1) {
            continue;
        }
        marks.push((b, doc_c32(c.ink())));
        if marks.len() > 4096 {
            break;
        }
    }

    // 竖线
    let mut k = p0.x.floor();
    while k <= p1.x {
        let x = k;
        let sx = app.to_screen(x, 0.0).x;
        let mut segs: Vec<(f64, f64, Color32)> = vec![(p0.y, p1.y, ground)];
        for (b, c) in marks.iter() {
            if x < b.x0 || x >= b.x1 || b.y1 <= p0.y || b.y0 >= p1.y {
                continue;
            }
            ink_span(&mut segs, b.y0.max(p0.y), b.y1.min(p1.y), *c);
        }
        for (a, b, c) in segs {
            if b <= a {
                continue;
            }
            let (ya, yb) = (app.to_screen(0.0, a).y, app.to_screen(0.0, b).y);
            crisp_line(painter, pos2(sx, ya), pos2(sx, yb), hair, app.ppp, c);
        }
        k += 1.0;
    }

    // 横线
    let mut k = p0.y.floor();
    while k <= p1.y {
        let y = k;
        let sy = app.to_screen(0.0, y).y;
        let mut segs: Vec<(f64, f64, Color32)> = vec![(p0.x, p1.x, ground)];
        for (b, c) in marks.iter() {
            if y < b.y0 || y >= b.y1 || b.x1 <= p0.x || b.x0 >= p1.x {
                continue;
            }
            ink_span(&mut segs, b.x0.max(p0.x), b.x1.min(p1.x), *c);
        }
        for (a, b, c) in segs {
            if b <= a {
                continue;
            }
            let (xa, xb) = (app.to_screen(a, 0.0).x, app.to_screen(b, 0.0).x);
            crisp_line(painter, pos2(xa, sy), pos2(xb, sy), hair, app.ppp, c);
        }
        k += 1.0;
    }
}
