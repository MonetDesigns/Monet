//! 无边框「纯输入框」——重命名等就地编辑的唯一实现。
//!
//! 三处界面共用它（egui 版对应 `widgets::rename_field` / `text_field_inline`）：
//!   * 左栏 Pages 行（双击页面名），
//!   * 左栏 Layers 行（双击图层名），
//!   * 画布上顶层帧 / section 的名字标签（双击标签）。
//!
//! 它**只组合** `widgets::TextEdit`（冻结的编辑引擎：caret / 选区 / IME / Enter / 失焦提交），
//! 自己负责三件以前散落在各调用点、因而互不一致的事：
//!
//! 1. **没有边框、没有底色、没有圆角**的裸外观；高度由调用方给出，盒高固定。
//! 2. **统一的焦点生命周期**：打开后持续申请焦点直到真正拿到；只有拿过焦点之后再失焦，
//!    才算编辑结束（否则刚打开就被关掉）。
//! 3. **稳定行盒**：`TextInput` 用「当前文字的 shaped line」的 `ascent + descent` 做垂直居中，
//!    这个度量会随文字内容（空串 / 中文回退字体 / IME 组字串）变化，于是打字时基线会
//!    上下跳一两像素。这里按当帧实际显示串量出度量，给编辑器一个补偿位移，让基线恒定，
//!    与「有边框的输入框」（固定 `FIELD_H` 盒 + 同一字体）对齐。
//!
//! 该文件刻意放在 `src/` 而不是 `src/widgets/`：后者是冻结件，不得改动；本组件只调用它们。

use crate::widgets::TextEdit;
use gpui::{App, Context, Entity, IntoElement, MouseButton, Pixels, Render, SharedString, Window, div, prelude::*, px};

/// 11pt 与三处被替换的静态标签一致（egui `regular(11.0)` / GPUI `text_size(px(11.0))`）。
const FONT: f32 = 11.0;

/// 行内列表（Pages / Local styles）的行高。**这是组件唯一需要的布局参数**：
/// 传行高，组件铺满它并做一次居中减法，与静态名字那次减法字面相同。
pub const ROW_H: f32 = 24.0;
/// Layers 行的行高（比 Pages 高 8）。
pub const LAYER_H: f32 = 32.0;
/// 画布标签重命名框的高度（静态标签所在的那块框）。
pub const LABEL_H: f32 = 18.0;

/// 画布标签里，静态文字顶相对标签框顶的偏移。
///
/// 标签不是原生文本元素，而是 `painter.labels` 画出来的：整块 div 先被全局
/// `widgets::TEXT_NUDGE`（1.0）上移，div 的高度又只有 galley 那么高。行内三处没有
/// 这个 nudge，所以偏移是 0，只有标签需要它。
pub fn label_text_top() -> f32 {
    -0.5
}

/// 静态名字的绘制：**与编辑态完全同一段代码**（同一个 `ShapedLine::paint`、同一个居中式）。
///
/// 这是「进入 / 退出编辑文字上下跳」的根治办法。以前静态名字是 `div().child(name)` 交给
/// GPUI 的文本元素画，编辑态是 `TextInput` 的 canvas 画 —— 两条管线的行盒取整不同，
/// 无论怎么调参数都会在某个行高上差 1 像素（用户 2026-11-16 连着抓到三次）。
/// 带外框的大输入框从来没有这个问题，就是因为它自始至终只有**一个** `TextEdit`。
///
/// 这里让静态文字也走 canvas，居中式与 `TextInput::centred_line_origin` 逐字相同：
/// `bounds.origin.y + (bounds.height - (ascent + descent)) / 2`。两态因此同源。
pub fn static_text(text: SharedString, color: gpui::Hsla, height: f32) -> impl IntoElement {
    let prepaint_text = text.clone();
    div().w_full().h(px(height)).child(
        gpui::canvas(
            move |bounds, window, _cx| {
                // A22：按格子实测宽度截断，省略号落在视觉末端（上游 `bidi::paint_name` 的
                // `max_w`）。名称始终保持逻辑顺序，双向排版由平台塑形器负责。
                let fitted = crate::bidi::fit(window, &prepaint_text, FONT, f32::from(bounds.size.width));
                // 与 `TextInput` 塑形时**同一份输入**：同一个字体来源、同一个字号、
                // 同一个 `shape_line` 调用形式（末位 `force_width` 传 `None`）。
                let run = gpui::TextRun {
                    len: fitted.len(),
                    font: window.text_style().font(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(fitted, px(FONT), &[run], None);
                let ink = line.ascent + line.descent;
                let top = bounds.origin.y + (bounds.size.height - ink) * 0.5;
                (line, gpui::point(bounds.origin.x, top), ink)
            },
            |_bounds, (line, origin, ink), window, cx| {
                let _ = line.paint(origin, ink, window, cx);
            },
        )
        .size_full(),
    )
}

/// 编辑态那一行文字的度量：`(宽度, ascent + descent)`。
///
/// 字体必须取自 `window.text_style().font()`（和 `TextInput` 塑造文字时完全同一个 source），
/// 字号固定 11。用 `TextStyle::default()` 的字体族会得到另一套度量（11pt 下 13.97 而不是
/// 14.586），重命名框会偏宽、垂直补偿也会差半像素。
pub fn text_measure(window: &Window, text: SharedString) -> (Pixels, Pixels) {
    let run = gpui::TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color: gpui::Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(text, px(FONT), &[run], None);
    (line.width, line.ascent + line.descent)
}

/// 一次就地编辑的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditState {
    /// 还在编辑（含等待焦点落地的开头几帧）。
    Editing,
    /// 拿到过焦点后又失焦：调用方应关掉这次重命名。
    Ended,
}

/// 无边框内联输入框。视觉上没有 shell，只有一层盒 + 裸 `TextEdit`。
pub struct InlineInput {
    edit: Entity<TextEdit>,
    /// 外层盒高（pt）：必须盖住被替换的静态名字，供行内布局使用。
    height: f32,
    /// 只服务画布标签：标签静态文字带 `TEXT_NUDGE` 与「div 高 = galley 高」这两件事，
    /// 行内列表传 0。见 [`label_text_top`]。
    text_top: f32,
    /// 焦点曾经落到这个 editor 上。
    was_active: bool,
}

impl InlineInput {
    /// 每次重命名都新建一个：复用同一个 `TextEdit` 会留下上一次的草稿
    /// （`set_value` 在编辑态不会动 buffer），导致「第二次双击改成了第一个名字」。
    ///
    /// `height` 传**所在行的行高**（[`ROW_H`] / [`LAYER_H`] / [`LABEL_H`]），组件铺满它；
    /// 不要传「文字行盒高度」之类的中间量 —— 组件内部只做一次
    /// `(行高 - 文字墨迹高) / 2` 的居中，与静态名字的算法逐字相同。
    /// `text_top` 只给画布标签用，行内列表传 `0.0`。
    pub fn build(
        cx: &mut App,
        height: f32,
        text_top: f32,
        value: impl Into<String>,
        on_commit: impl Fn(&str, &mut Window, &mut App) + 'static,
    ) -> Entity<Self> {
        let value = value.into();
        cx.new(|cx| {
            let edit = TextEdit::build(cx, |te, _| {
                te.set_inset_left(0.0);
                te.set_right_pad(0.0);
                te.set_font_size(px(FONT));
                // 空缓冲区默认显示空串，空串的 shaped line 度量与有字时不同；
                // 一个不可见的空格让「删光了」也保持同样的行盒。
                te.set_placeholder(" ");
                te.set_value(value);
                te.set_on_commit(on_commit);
            });
            Self { edit, height, text_top, was_active: false }
        })
    }

    /// 点击别处 / 提交草稿：与 `widgets::TextField` 的失焦提交同源。
    pub fn finish_edit(&self, window: &mut Window, cx: &mut App) {
        self.edit.update(cx, |te, cx| {
            te.finish_edit(window, cx);
        });
    }

    /// 当前是否持有键盘（Local styles 用它判断改名是否结束）。
    /// 写成关联函数，调用点不必先 `read(cx)` 再借 `cx` 给 `is_active`。
    pub fn is_active(entity: &Entity<Self>, window: &Window, cx: &App) -> bool {
        entity.read(cx).edit.read(cx).is_active(window)
    }

    /// 打开一次就地编辑：先收掉上一次编辑（`TextEdit::set_value` 在编辑态不会改 buffer，
    /// 直接复用会留下旧草稿），再预填新值并聚焦。关联函数形式避开 `read` / `update`
    /// 对 `cx` 的借用冲突。
    pub fn begin_edit(entity: &Entity<Self>, value: impl Into<String>, window: &mut Window, cx: &mut App) {
        entity.update(cx, |this, cx| {
            this.edit.update(cx, |te, tcx| {
                te.finish_edit(window, tcx);
                te.set_value(value);
                // 双击进入时全选文字；再单击一次就落成单光标输入（用户 2026-11-15）。
                te.focus_editor_select_all(window);
            });
            this.was_active = false;
        });
    }

    /// 每帧在渲染前调用：申请焦点、判断是否已经结束。
    pub fn poll_active(&mut self, window: &mut Window, cx: &mut Context<Self>) -> EditState {
        // 只在「还没激活过」时抢焦点：一旦激活过，失焦就代表用户点了别处。未改动的重命名
        // 在失焦时 `TextEdit::commit` 不产生回调，这里若把焦点再抢回来，就会把重命名重新
        // 打开、把后续输入全部截走（点另一个输入框即复现）。
        if !self.was_active && !self.edit.read(cx).is_active(window) {
            // 双击进入编辑态的第一件事就是全选文字（用户 2026-11-15）：
            // Pages / Layers / 画布标签都走这条路径（begin_inline_rename 新建 editor 后靠这里
            // 抢焦点），必须与 Local styles 的 `InlineInput::begin_edit` 一样 select-all，
            // 否则三处是单光标、一处是全选，行为不统一。再单击一次由 TextEdit 自己的
            // mouse handler 落成单光标（count=1 且 already editing → move_to）。
            self.edit.update(cx, |te, _| te.focus_editor_select_all(window));
        }
        let active = self.edit.read(cx).is_active(window);
        if active {
            self.was_active = true;
        }
        if !active && self.was_active { EditState::Ended } else { EditState::Editing }
    }
}

/// 行盒补偿量（pt）：把「随内容变化的基线」拉回参考基线。
///
/// `TextInput` 绘制时 `centred_line_origin` 给出
/// `origin.y = box.y + (H - (A + D)) / 2`，`paint_line` 的基线是 `origin.y + A`
/// （`line_height == A + D`，padding 为 0）。要把基线锁在参考值
/// `box.y + (H - (A0 + D0)) / 2 + A0`，只需把整个编辑器盒子下移
/// `((A + D) - (A0 + D0)) / 2 + (A0 - A)`。
pub fn stable_top_offset(ascent: f32, descent: f32, ref_ascent: f32, ref_descent: f32) -> f32 {
    ((ascent + descent) - (ref_ascent + ref_descent)) * 0.5 + (ref_ascent - ascent)
}

/// 画布标签重命名框的矩形（egui `frame_labels::rename_field`），纯几何、可回归测试。
///
/// `min_x / min_y / max_x` 是标签四角经 `to_screen` 得到的**画布局部坐标**；本项目
/// `app.canvas.rect` 原点恒为 0，绝对定位的浮层挂在整窗根视图上，因此必须补 `origin_*`。
/// 调用方给的 `text_w` 是 11pt 下真实量算出的名字宽度（不再是字符数估算）。
///
/// `y` 就是标签命中区的**顶边**（= 静态 galley 的顶边，`frame_labels` 把名字画在这里），
/// 高度则是 [`LABEL_H`]（18 = 静态行盒），编辑框才能和静态文字重合。
pub fn label_rename_rect(min_x: f32, min_y: f32, max_x: f32, text_w: f32, origin_x: f32, origin_y: f32) -> (f32, f32, f32) {
    let w = (text_w + 24.0).max(max_x - min_x).clamp(80.0, 480.0);
    (min_x + origin_x, min_y + origin_y, w)
}

/// 标签 quad 缺失时的兜底位置（帧左上角上方 20pt），同样要补画布窗口原点。
pub fn label_rename_fallback(x: f32, y: f32, text_w: f32, origin_x: f32, origin_y: f32) -> (f32, f32, f32) {
    let w = (text_w + 24.0).clamp(80.0, 480.0);
    (x + origin_x, y - 20.0 + origin_y, w)
}

impl Render for InlineInput {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // `height` 就是**所在行的行高**，组件自己铺满它。`TextInput` 的居中公式是
        // `(盒高 - (ascent + descent)) / 2`，静态名字的落笔位置是
        // `(行高 - (ascent + descent)) / 2` —— 两个式子是**同一个减法**，只要盒高等于行高，
        // 结果必然逐像素相同（包括亚像素取整）。
        //
        // 以前这里让盒高取「静态文本的行盒高」（18）再靠父容器 `items_center` 二次居中：
        // 数学上等值，但要经过两次取整，行高 24 与 32 的行会落到不同的物理像素上 ——
        // 那正是「同一个组件，列表往下跳、标签往上跳」的来源。调用方现在只需给行高，
        // 不需要也不应该传任何像素偏移（画布标签除外，见 `label_text_top`）。
        div()
            .relative()
            .w_full()
            .h(px(self.height))
            // 点输入框自己不能冒泡到根视图的「点别处就提交」。
            .on_mouse_down(MouseButton::Left, |_e, _w, cx| cx.stop_propagation())
            .child(div().absolute().top(px(self.text_top)).left_0().w_full().h_full().child(self.edit.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_metrics_need_no_correction() {
        assert_eq!(stable_top_offset(9.0, 3.0, 9.0, 3.0), 0.0);
    }

    /// 关键回归：字体度量变了，补偿后的基线必须逐像素不动。
    /// baseline = box.y + (H - (A+D))/2 + A + drift，H 取 24。
    #[test]
    fn corrected_baseline_is_constant_across_font_metrics() {
        let h = 24.0;
        let (a0, d0) = (9.0, 3.0);
        let expected = (h - (a0 + d0)) * 0.5 + a0;
        for (a, d) in [(11.0, 3.0), (8.0, 2.0), (10.0, 4.0), (0.0, 0.0), (13.0, 5.0)] {
            let drift = stable_top_offset(a, d, a0, d0);
            let baseline = (h - (a + d)) * 0.5 + a + drift;
            assert!((baseline - expected).abs() < 1e-4, "metrics ({a}, {d}) moved the baseline by {}", baseline - expected);
        }
    }

    /// 画布标签输入框位置：必须补上 dock 给的画布窗口原点（原来漏了，输入框整体偏到左栏）。
    #[test]
    fn label_rename_rect_adds_the_canvas_window_origin() {
        // 标签命中区：x 100…300、顶边 224（底边由 `LABEL_GAP` 决定，这里只用顶边）。
        let (x, y, w) = label_rename_rect(100.0, 224.0, 300.0, 60.0, 300.0, 40.0);
        assert_eq!((x, y, w), (400.0, 264.0, 200.0));
        // 同一标签在原点为 0 的画布里，x/y 正好少掉轨道 + 左栏 + 标题栏。
        assert_eq!(label_rename_rect(100.0, 224.0, 300.0, 60.0, 0.0, 0.0), (100.0, 224.0, 200.0));
    }

    /// 宽度逐项照抄 egui：`(text_w + 24).max(area.width).clamp(80, 480)`。
    #[test]
    fn label_rename_rect_width_uses_measured_text_and_label_bounds() {
        assert_eq!(label_rename_rect(0.0, 30.0, 10.0, 2.0, 0.0, 0.0).2, 80.0, "短名字至少 80pt");
        assert_eq!(label_rename_rect(0.0, 30.0, 400.0, 2.0, 0.0, 0.0).2, 400.0, "标签比文字宽时取标签宽");
        assert_eq!(label_rename_rect(0.0, 30.0, 10.0, 900.0, 0.0, 0.0).2, 480.0, "长名字夹到 480pt");
        assert_eq!(label_rename_rect(0.0, 30.0, 10.0, 100.0, 0.0, 0.0).2, 124.0);
    }

    #[test]
    fn label_rename_fallback_also_adds_the_origin() {
        assert_eq!(label_rename_fallback(50.0, 200.0, 60.0, 300.0, 40.0), (350.0, 220.0, 84.0));
    }

    /// 组件只吃「行高」这一个布局事实（用户 2026-11-16：不许让每个调用点各自校准像素）。
    /// 这三个数必须与各行的实际行高一致，否则进入编辑时文字会离开静态位置。
    #[test]
    fn the_widget_is_driven_by_row_heights_only() {
        assert_eq!(ROW_H, 24.0, "Pages / Local styles 行高");
        assert_eq!(LAYER_H, 32.0, "Layers 行高");
        assert_eq!(LABEL_H, 18.0, "画布标签框高");
    }
}
