//! 顶层帧与 section 的画布名称标签（照搬 `ui-egui/src/canvas/frame_labels.rs`）。
//!
//! 帧名贴在上边缘之上，section 名是左上角内部的 pill；组件/图标用各自的身份色。
//! `label_edge` 的几何计算与原实现**逐行一致**（含镜像帧保持正向）。
//!
//! 差异（GPUI 侧适配）：文字用 GPUI 文本元素渲染，不能像 egui 那样把已画出的图元整体旋转，
//! 因此旋转帧的标签仍放在几何算出的角点，但保持水平（记为已知限制）。
//!
//! 名称的颜色与位置按 2026-11-22 设计稿（Groups 231 / 232 / 234 / 235）：
//! 普通帧未选中 / 常态 / 悬停用同一个灰，选中才换成更深一档的强调色；组件 / 组件集一直是
//! 设计稿里那个紫。名称文字框的底边距对象上边缘 [`LABEL_GAP`]；名称行右侧留两个 16×16
//! 图标槽（[`name_slots`]，最右是开发状态，另一个是预留位，见 `docs/canvas-frame-label.md`）。

use figcraft_doc::{DevStatus, Document, NodeId, NodeKind};
use figcraft_geom::Point;
use figcraft_i18n::tr;

use super::geom::*;
use super::painter::{Painter, text_size};
use super::rulers::status_badge;
use crate::app::CanvasApp;
use crate::bidi::{ELLIPSIS, fit_with};
use crate::theme::{COMPONENT, COMPONENT_TEXT, DEV_CHANGED, DEV_COMPLETED, SLOT, Tokens, stroke};

/// 文字身份色（图标层用橙色；这里先留接口）。
pub const ICON: Color32 = Color32::from_rgb(0xFF, 0xB3, 0x4D);

/// 名称文字框底边与对象上边缘的距离（设计稿 7）。
pub const LABEL_GAP: f32 = 7.0;
/// 名称行右侧图标槽的边长（设计稿 16）。
pub const SLOT_SIZE: f32 = 16.0;
/// 最右图标槽的右边比对象右边缘多出的距离（设计稿 2）。
pub const SLOT_OUTSET: f32 = 2.0;
/// 两个图标槽之间的间距（设计稿 2）。
pub const SLOT_GAP: f32 = 2.0;
/// 名称文字与右侧图标槽之间留出的空隙（给省略号收尾）。
pub const NAME_GAP: f32 = 4.0;

/// 名称的**原始文字**（含组件前缀）是否撞上状态图标：用文字本身的实际宽度算，
/// 不能用按框宽截断后的宽度，否则长名称会把图标一直挤掉。撞上就省略**图标**（不是名称）。
fn name_hits_status(doc: &Document, id: NodeId, x: f32, x1: f32, y0: f32) -> bool {
    let Some(n) = doc.node(id) else { return false };
    let label = if matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. }) {
        format!("{COMPONENT_MARK}{}", n.name)
    } else {
        n.name.clone()
    };
    if label.is_empty() {
        return false;
    }
    // 原始文字自然宽度（不截断、不省略），从名称起点量到图标槽左边缘。
    x + text_size(&label, regular(11.0)).x >= name_slots(x1, y0)[0].min.x - NAME_GAP
}
/// 图标槽底边在对象上边缘**之上**的距离（设计稿 6）：两个槽与名称同一行，坐在对象上边缘外面。
pub const SLOT_ABOVE: f32 = 6.0;
/// 变体组「+」的顶边：对象下边缘 + 尺寸框 7 + 尺寸框高 17 + 间距 4。
pub const ADD_VARIANT_TOP: f32 = 30.0;
/// 组件 / 组件集名称的前缀：❖ 加一个空格（设计稿；改名时不显示）。
pub const COMPONENT_MARK: &str = "\u{2756} ";

/// 名称行右侧两个 16×16 图标槽的矩形（画布坐标）：`[0]` 是最右边的开发状态槽，
/// `[1]` 是第二个槽（本轮不画内容，位置记在 `docs/canvas-frame-label.md`）。
/// 槽**坐在对象上边缘之外**：底边距上边缘 [`SLOT_ABOVE`]，与名称文字同一行。
pub fn name_slots(x1: f32, y0: f32) -> [Rect; 2] {
    let right = x1 + SLOT_OUTSET - SLOT_SIZE;
    let y = y0 - SLOT_ABOVE - SLOT_SIZE;
    let first = Rect::from_min_size(pos2(right, y), vec2(SLOT_SIZE, SLOT_SIZE));
    let second = Rect::from_min_size(pos2(right - SLOT_GAP - SLOT_SIZE, y), vec2(SLOT_SIZE, SLOT_SIZE));
    [first, second]
}

/// 变体组「+」按钮的矩形：16×16，水平居中在对象上，顶边 = 对象下边缘 + [`ADD_VARIANT_TOP`]。
pub fn add_variant_slot(center_x: f32, y1: f32) -> Rect {
    Rect::from_min_size(pos2(center_x - SLOT_SIZE / 2.0, y1 + ADD_VARIANT_TOP), vec2(SLOT_SIZE, SLOT_SIZE))
}

/// 「+」加号挂在哪个节点下方：组件集 → 它自己；组件集里的变体 → **所属组件集**
/// （所以选中变体时加号出现在整个集的下方，而不是变体小框下方）；独立组件 → 它自己
/// （点一下会变成它自己的集）。其余节点没有加号。
fn add_variant_host(doc: &Document, n: &figcraft_doc::Node) -> Option<NodeId> {
    match n.kind {
        NodeKind::ComponentSet { .. } => Some(n.id),
        NodeKind::Component { .. } => Some(figcraft_engine::components::variant_set(doc, n.id).unwrap_or(n.id)),
        _ => None,
    }
}

/// 对象四角（含旋转）在屏幕上的包围盒 `(x0, y0, x1, y1)`。
pub fn screen_bounds(app: &CanvasApp, doc: &Document, id: NodeId) -> Option<(f32, f32, f32, f32)> {
    let n = doc.node(id)?;
    let w = doc.world_transform(id);
    let at = |x: f64, y: f64| {
        let p = w * Point::new(x, y);
        app.to_screen(p.x, p.y)
    };
    let pts = [at(0.0, 0.0), at(n.width, 0.0), at(n.width, n.height), at(0.0, n.height)];
    Some((
        pts.iter().map(|p| p.x).fold(f32::MAX, f32::min),
        pts.iter().map(|p| p.y).fold(f32::MAX, f32::min),
        pts.iter().map(|p| p.x).fold(f32::MIN, f32::max),
        pts.iter().map(|p| p.y).fold(f32::MIN, f32::max),
    ))
}

/// 开发状态按钮的两个绿：常态（待开发标签）与悬停（可点出菜单）。
pub const STATUS_GREEN: Color32 = Color32::from_rgb(0x14, 0xAE, 0x5C);
pub const STATUS_GREEN_HOVER: Color32 = Color32::from_rgb(0x00, 0x99, 0x51);

/// 名称行图标槽的命中结果。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SlotHit {
    /// 开发状态槽；`true` = 该对象已经有状态（再点一次弹菜单）。
    Status(NodeId, bool),
    /// 未选中的对象、状态槽里尚未有状态：点一下直接置「待开发」（没有悬停态 / 菜单）。
    Ready(NodeId),
    /// 未选中的对象、已有状态：点一下先选中该对象、再打开菜单。
    Menu(NodeId),
    /// 变体组「+」。
    AddVariant(NodeId),
}

/// 指针所在的名称行状态槽属于哪个对象（用于让按钮和名称文字共享同一个 hover 轮廓）。
pub fn status_slot_owner(app: &CanvasApp, doc: &Document, page: NodeId, pos: Pos2) -> Option<NodeId> {
    for id in super::rulers::boards(doc, page).into_iter().rev() {
        let Some(n) = doc.node(id) else { continue };
        if !n.kind.is_frame_like() || n.is_board_atom() {
            continue;
        }
        let (x0, y0, x1, _) = screen_bounds(app, doc, id)?;
        let slot = name_slots(x1, y0)[0];
        // 原始名称撞上图标 → 图标已省略，不接 hover 轮廓。
        if name_hits_status(doc, id, x0, x1, y0) {
            continue;
        }
        if slot.contains(pos) {
            return Some(id);
        }
    }
    None
}

/// 在画布坐标 `pos` 上找名称行图标槽：返回命中项与它的矩形（菜单要按它定位）。
///
/// 选中的对象认状态槽 / 变体组「+」；未选中的顶层帧只认状态槽里的「三个点」——点它
/// 先选中再弹菜单（和悬停在名称文本上出现三个点是同一套交互）。
pub fn slot_at(app: &CanvasApp, doc: &Document, page: NodeId, sel: &[NodeId], pos: Pos2) -> Option<(SlotHit, Rect)> {
    for id in super::rulers::boards(doc, page) {
        let Some(n) = doc.node(id) else { continue };
        if !n.kind.is_frame_like() || n.is_board_atom() {
            continue;
        }
        let Some((x0, y0, x1, y1)) = screen_bounds(app, doc, id) else { continue };
        let status = name_slots(x1, y0)[0];
        // 原始名称撞上图标 → 图标已省略，不接点击。
        if name_hits_status(doc, id, x0, x1, y0) {
            continue;
        }
        if status.contains(pos) {
            let has = !n.dev_status.is_none();
            return Some(if sel.contains(&id) {
                (SlotHit::Status(id, has), status)
            } else if has {
                (SlotHit::Menu(id), status)
            } else {
                (SlotHit::Ready(id), status)
            });
        }
        if sel.contains(&id) && let Some(host) = add_variant_host(doc, n) {
            let (hx0, hy1, hx1) = match screen_bounds(app, doc, host) {
                Some((hx0, _, hx1, hy1)) => (hx0, hy1, hx1),
                None => (x0, y1, x1),
            };
            let add = add_variant_slot((hx0 + hx1) / 2.0, hy1);
            if add.contains(pos) {
                return Some((SlotHit::AddVariant(id), add));
            }
        }
    }
    None
}

/// 未选中 / 常态 / 悬停时的名称灰：设计稿是黑 26%（深色主题用同比例的白）。
pub fn label_gray(t: &Tokens) -> Color32 {
    if t.dark { Color32::from_white_alpha(67) } else { Color32::from_black_alpha(67) }
}

/// 名称的颜色：组件 / 组件集**一直**是设计稿里那个紫（选中与否都不变）；插槽、图标层同理
/// 用自己的身份色；普通帧未选中 / 常态 / 悬停用灰，只有选中才换成更深一档的强调色。
pub fn label_ink(app: &CanvasApp, t: &Tokens, doc: &Document, id: NodeId, selected: bool) -> Color32 {
    let Some(n) = doc.node(id) else { return label_gray(t) };
    match n.kind {
        NodeKind::Component { .. } | NodeKind::ComponentSet { .. } => COMPONENT_TEXT,
        NodeKind::Icon { .. } => ICON,
        _ if figcraft_engine::components::is_slot(doc, id) => SLOT,
        _ if selected => app.accent(),
        _ => label_gray(t),
    }
}

/// 开发状态按钮：
/// - **没有状态**（常态）：整块透明、没有图标、没有悬停态和菜单，点一下直接置「待开发」。
///   悬停只让下面的框显示 hover 轮廓（和悬停名称文字一样），按钮本身不起变化。
/// - **已置待开发**：常态 `STATUS_GREEN` #14AE5C + 代码图标；悬停 `STATUS_GREEN_HOVER`
///   #009951 + 三个白点，点一下弹菜单。其它状态仍用 `DEV_CHANGED` / `DEV_COMPLETED`。
fn status_slot(painter: &mut Painter, n: &figcraft_doc::Node, r: Rect, hovered: bool) {
    if n.dev_status.is_none() {
        // 未添加标签：透明、无图标、无悬停视觉（hover 轮廓由 overlays 画在对象上）。
        return;
    }
    let base = match n.dev_status {
        DevStatus::ReadyForDev if n.dev_changed => Some(DEV_CHANGED),
        DevStatus::ReadyForDev => Some(STATUS_GREEN),
        DevStatus::Completed => Some(DEV_COMPLETED),
        DevStatus::None => unreachable!(),
    };
    // 悬停「待开发」绿按钮：整块换成深一档的 #009951，图标换成三个白点。
    let fill = if hovered && n.dev_status == DevStatus::ReadyForDev && !n.dev_changed {
        Some(STATUS_GREEN_HOVER)
    } else {
        base
    };
    painter.rect_filled(r, CornerRadius::same(3), fill.unwrap());
    let ink = Color32::WHITE;
    if hovered && n.dev_status == DevStatus::ReadyForDev && !n.dev_changed {
        more_dots(painter, r, ink);
    } else {
        // 代码图标（设计稿的 `ui-code`，13×11）：左右两个尖括号。
        let (x, y) = (r.min.x, r.min.y);
        painter.add_shape(vec![pos2(x + 6.0, y + 4.5), pos2(x + 2.5, y + 8.0), pos2(x + 6.0, y + 11.5)], false, Color32::TRANSPARENT, stroke(ink, 1.0));
        painter.add_shape(vec![pos2(x + 10.0, y + 4.5), pos2(x + 13.5, y + 8.0), pos2(x + 10.0, y + 11.5)], false, Color32::TRANSPARENT, stroke(ink, 1.0));
    }
}

/// 悬停已置标签的状态按钮时显示的「…」——直接用省略号字形（和名称截断同一个字符），
/// 由画布的文字管线绘制，不手画圆点。居中排在 16×16 槽里。
fn more_dots(painter: &mut Painter, r: Rect, ink: Color32) {
    let c = r.center();
    painter.text(pos2(c.x, c.y), Align2::CENTER_CENTER, ELLIPSIS, regular(13.0), ink);
}

/// 变体组「+」按钮：组件紫底、圆角 3、白色十字（设计稿 10×10 的加号）。
fn add_variant_button(painter: &mut Painter, r: Rect) {
    painter.rect_filled(r, CornerRadius::same(3), COMPONENT);
    let c = r.center();
    let st = stroke(Color32::WHITE, 1.0);
    painter.line_segment([pos2(c.x - 5.0, c.y), pos2(c.x + 5.0, c.y)], st);
    painter.line_segment([pos2(c.x, c.y - 5.0), pos2(c.x, c.y + 5.0)], st);
}

/// 标签贴在屏幕上的位置：起点角、读取方向（弧度，屏幕顺时针）、该方向的边长。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelEdge {
    pub corner: Pos2,
    pub angle: f32,
    pub len: f32,
}

/// 本地角 (0,0)、(w,0)、(0,h) 显示在 `p00`、`p10`、`p01` 时，帧的上边缘作为标签边。
pub fn label_edge(p00: Pos2, p10: Pos2, p01: Pos2) -> LabelEdge {
    let (ex, ey) = (p10 - p00, p01 - p00);
    let len = ex.length();
    if !len.is_finite() || len <= 1e-3 {
        return LabelEdge { corner: p00, angle: 0.0, len: 0.0 };
    }
    let mirrored = ex.x * ey.y - ex.y * ey.x < 0.0;
    let reversed = mirrored && ex.x < 0.0;
    let d = if reversed { -ex } else { ex };
    let up = vec2(d.y, -d.x);
    let bottom = up.dot(ey) > 0.0;
    let corner = p00 + if reversed { ex } else { Vec2::ZERO } + if bottom { ey } else { Vec2::ZERO };
    LabelEdge { corner, angle: d.y.atan2(d.x), len }
}

/// 画 `id`（顶层帧或 section）的标签；返回它在屏幕上的四角（用于命中测试）。
pub fn draw(app: &CanvasApp, painter: &mut Painter, t: &Tokens, doc: &Document, id: NodeId, selected: bool, renaming: bool) -> Option<[Pos2; 4]> {
    let n = doc.node(id)?;
    let w = doc.world_transform(id);
    let at = |x: f64, y: f64| {
        let p = w * Point::new(x, y);
        app.to_screen(p.x, p.y)
    };
    let (p00, p01) = (at(0.0, 0.0), at(0.0, n.height));
    let edge = label_edge(p00, at(n.width, 0.0), p01);
    let tl = edge.corner;
    let col = label_ink(app, t, doc, id, selected);

    let area = if matches!(n.kind, NodeKind::Section { .. }) {
        // section 标题是左上角内部的 pill。
        let hidden = n.hides_contents();
        let fg = if selected { Color32::WHITE } else { t.text };
        let font = semibold(11.0);
        let shown = fit_with(&n.name, (edge.len - 16.0).clamp(40.0, 480.0), |s| text_size(s, font).x);
        let galley = painter.layout_no_wrap(shown, font, fg);
        let r = Rect::from_min_size(tl, galley.size() + vec2(16.0, 8.0));
        painter.rect_filled(r, CornerRadius::same(4), if selected { t.accent } else { t.hover });
        // 改名期间不画旧名字（内联输入框是透明的），pill 与矩形照旧。
        if !renaming {
            painter.galley(r.min + vec2(8.0, 4.0), galley, fg);
        }
        status_badge(painter, n, pos2(r.max.x + 6.0, r.center().y));
        let _ = hidden;
        r
    } else {
        let x = tl.x;
        // 名字画在帧上边缘之上：文字框底边距上边缘 `LABEL_GAP`。
        let font = regular(11.0);
        // 组件 / 组件集的名字前面带「❖ 」（设计稿）；改名时只显示原名（内联输入框自己不带前缀）。
        let label = if matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. }) {
            format!("{COMPONENT_MARK}{}", n.name)
        } else {
            n.name.clone()
        };
        // 宽度不设下限（`fit_with` 内部只夹到 1pt），长名字可以一路缩到只剩省略号。
        let max_w = (edge.len - (x - tl.x)).max(1.0);
        let shown = fit_with(&label, max_w, |s| text_size(s, font).x);
        // 用**原始文字**判断：撞上图标就省略图标（名称照常画）。
        let name_collides = screen_bounds(app, doc, id).is_some_and(|(_, y0, x1, _)| name_hits_status(doc, id, x, x1, y0));
        let galley = painter.layout_no_wrap(shown, font, col);
        let size = galley.size();
        let end = x + size.x;
        let top = tl.y - LABEL_GAP - size.y;
        // 命中区 = 文字框本身（顶边 = 文字顶，底边 = 上边缘上方 `LABEL_GAP`），
        // 重命名框直接用这个顶边，所以两态永远重合。
        let label_rect = Rect::from_min_max(pos2(tl.x, top), pos2(end, tl.y - LABEL_GAP));
        // 同上：几何（`area`）仍按 galley 量出来的宽度算，只是不落笔。
        if !renaming {
            painter.galley(pos2(x, top), galley, col);
        }
        // 名称行右侧的状态按钮：**原始名称文字撞上它就整个省略**（不画、不接点击、
        // 不参与 hover 轮廓）；短名称则照常显示。判断用的是未截断的文字宽度。
        if let Some((x0, y0, x1, y1)) = screen_bounds(app, doc, id) {
            if !name_collides {
                let slot = name_slots(x1, y0)[0];
                if !n.dev_status.is_none() {
                    let hover_slot = app.pointer.is_some_and(|p| slot.contains(p));
                    status_slot(painter, n, slot, hover_slot);
                }
            }
            if selected && let Some(host) = add_variant_host(doc, n) {
                let (hx0, hy1, hx1) = match screen_bounds(app, doc, host) {
                    Some((hx0, _, hx1, hy1)) => (hx0, hy1, hx1),
                    None => (x0, y1, x1),
                };
                add_variant_button(painter, add_variant_slot((hx0 + hx1) / 2.0, hy1));
            }
        }
        label_rect
    };
    if n.hides_contents() {
        let height = (p01 - p00).length();
        if edge.len > 120.0 && height > 60.0 {
            let center = at(n.width / 2.0, n.height / 2.0);
            painter.text(center, Align2::CENTER_CENTER, tr("Contents hidden"), regular(11.0), t.text_tertiary);
        }
    }
    Some([area.min, pos2(area.max.x, area.min.y), area.max, pos2(area.min.x, area.max.y)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    /// 与原文测试同款：标签边跟随旋转、镜像时保持正向。
    #[test]
    fn label_edges_follow_rotation_and_stay_upright_when_mirrored() {
        let e = label_edge(pos2(10.0, 20.0), pos2(110.0, 20.0), pos2(10.0, 70.0));
        assert!(close(e.corner, pos2(10.0, 20.0)) && e.angle == 0.0 && (e.len - 100.0).abs() < 1e-3);
        let e = label_edge(pos2(100.0, 0.0), pos2(100.0, 80.0), pos2(50.0, 0.0));
        assert!(close(e.corner, pos2(100.0, 0.0)));
        assert!((e.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        let e = label_edge(pos2(100.0, 50.0), pos2(0.0, 50.0), pos2(100.0, 0.0));
        assert!(close(e.corner, pos2(100.0, 50.0)));
        assert!((e.angle.abs() - std::f32::consts::PI).abs() < 1e-5);
        let e = label_edge(pos2(100.0, 0.0), pos2(0.0, 0.0), pos2(100.0, 50.0));
        assert!(close(e.corner, pos2(0.0, 0.0)) && e.angle.abs() < 1e-6, "{e:?}");
        let e = label_edge(pos2(0.0, 50.0), pos2(100.0, 50.0), pos2(0.0, 0.0));
        assert!(close(e.corner, pos2(0.0, 0.0)) && e.angle.abs() < 1e-6, "{e:?}");
        let e = label_edge(pos2(5.0, 5.0), pos2(5.0, 5.0), pos2(5.0, 9.0));
        assert_eq!(e, LabelEdge { corner: pos2(5.0, 5.0), angle: 0.0, len: 0.0 });
        let e = label_edge(pos2(5.0, 5.0), pos2(f32::NAN, 5.0), pos2(5.0, 9.0));
        assert_eq!(e.angle, 0.0);
    }

    /// 名称行右侧两个槽（设计稿 Groups 231 / 234 / 235）：最右槽的右边比对象右边缘多 2，
    /// 槽**底边**在对象上边缘之上 6（整块坐在对象外面，与名称同一行），两槽间距 2。
    #[test]
    fn the_name_row_slots_sit_on_the_objects_top_right() {
        let [status, reserved] = name_slots(305.0, 200.0);
        assert_eq!((status.min.x, status.min.y, status.width(), status.height()), (291.0, 178.0, 16.0, 16.0));
        assert_eq!((reserved.min.x, reserved.min.y), (273.0, 178.0));
        assert_eq!(status.min.x - reserved.max.x, 2.0, "两槽间距 2");
        assert_eq!(status.max.x, 305.0 + 2.0, "最右槽右边 = 对象右边缘 + 2");
        assert_eq!(status.max.y, 200.0 - 6.0, "槽底边 = 对象上边缘 − 6");
    }

    /// 开发状态按钮的两个绿（用户 2026-11-22 指定）：常态 #14AE5C，悬停 #009951。
    #[test]
    fn the_status_button_has_its_two_greens() {
        assert_eq!(STATUS_GREEN.to_array(), [0x14, 0xAE, 0x5C, 255]);
        assert_eq!(STATUS_GREEN_HOVER.to_array(), [0x00, 0x99, 0x51, 255]);
    }

    /// 变体组的「+」：16×16、水平居中，顶边 = 对象下边缘 + 尺寸框 9 + 框高 17 + 间距 4 = 30。
    #[test]
    fn the_add_variant_button_sits_below_the_size_pill() {
        let r = add_variant_slot(200.0, 403.0);
        assert_eq!((r.min.x, r.min.y, r.width(), r.height()), (192.0, 433.0, 16.0, 16.0));
        assert_eq!(r.center().x, 200.0);
    }
}
