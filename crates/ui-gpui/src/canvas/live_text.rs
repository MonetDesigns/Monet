//! 其他人在共享文档里的文字光标（照搬 `ui-egui/src/canvas/live_text.rs`）。
//!
//! 有人在共享文档的文字层里输入时，他的光标与选中的字符用他自己的颜色画在你的那份文字上
//! （位置来自文字排版自己的光标 / 选区几何），光标上方是他的名字；你也在同一个文字层里输入
//! 时，标签换成提示语。远端光标不闪：只在画布本来就要重绘时画（presence 更新会唤醒一帧）。

use figcraft_engine::live::editing_hint;

use super::geom::*;
use super::outline::TextOnScreen;
use super::painter::Painter;
use crate::app::CanvasApp;

/// 别人的选区把字符染多深。
const SELECTION_ALPHA: f32 = 0.25;
/// 别人的光标宽度（点）。
const CARET_WIDTH: f32 = 2.0;

/// 画本页上所有正在输入文字层的人的光标与选区。
pub(crate) fn draw(app: &CanvasApp, painter: &mut Painter) {
    let live = app.session.live();
    if live.presence.is_empty() || !app.session.live_shared() {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    let mine = ds.text_edit.as_ref().map(|t| t.id);
    for (id, p) in &live.presence {
        let (Some(peer), Some(e)) = (live.peers.get(id), p.editing_text.as_deref()) else { continue };
        if ds.doc.page_of(e.node) != Some(ds.page) {
            continue;
        }
        // CodeRabbit PR#22: don't draw a remote caret/selection/label over a hidden text layer or
        // under a hidden ancestor — the peer may still be editing a layer the local user can't see.
        if !ds.doc.node(e.node).is_some_and(|n| n.visible)
            || ds.doc.ancestors(e.node).iter().any(|a| !ds.doc.node(*a).is_some_and(|n| n.visible))
        {
            continue;
        }
        let Some(text) = TextOnScreen::of(&ds.doc, e.node) else { continue };
        let fill = peer_color(&peer.color);
        // 位置按对方的字符数算（输入是完成时才同步）：夹在你这份文字的长度以内。
        let at = |i: usize| i.min(text.chars);
        if let Some((a, b)) = e.selection.map(|(a, b)| (at(a), at(b))).filter(|(a, b)| a < b) {
            for q in text.selection(app, a, b) {
                painter.add_shape(q.to_vec(), true, fill.gamma_multiply(SELECTION_ALPHA), Stroke::NONE);
            }
        }
        let [top, bottom] = text.caret(app, at(e.caret), ds.view.zoom);
        painter.line_segment([top, bottom], Stroke::new(CARET_WIDTH, fill));
        let label = if mine == Some(e.node) { editing_hint(&[peer.name.as_str()]).unwrap_or_default() } else { peer.name.clone() };
        // The higher of the caret's two ends, nudged left by half the caret and up 2 pt.
        let above = pos2(top.x.min(bottom.x) - CARET_WIDTH / 2.0, top.y.min(bottom.y) - 2.0);
        name_tag(painter, above, Align2::LEFT_BOTTOM, &label, fill);
    }
}

/// 一个人的颜色（会话给的 `#RRGGBB`），解析不了就用主题强调色。
fn peer_color(s: &str) -> Color32 {
    Color32::from_hex(s).unwrap_or_else(|| crate::theme::Tokens::light().accent)
}

/// 文字在 `fill` 上要用的颜色。
fn text_on(fill: Color32) -> Color32 {
    let l = 0.299 * f32::from(fill.0[0]) + 0.587 * f32::from(fill.0[1]) + 0.114 * f32::from(fill.0[2]);
    if l > 170.0 { Color32::from_rgb(0x1E, 0x1E, 0x1E) } else { Color32::WHITE }
}

/// 一个人颜色的名字标签，按 `align` 摆在 `anchor`（在光标上方）。
fn name_tag(painter: &mut Painter, anchor: Pos2, align: Align2, name: &str, fill: Color32) {
    let galley = painter.layout_no_wrap(name.to_string(), semibold(11.0), text_on(fill));
    let r = align.anchor_size(anchor, galley.size() + vec2(12.0, 6.0));
    painter.rect_filled(r, CornerRadius::same(4), fill);
    painter.galley(r.min + vec2(6.0, 3.0), galley, text_on(fill));
}
