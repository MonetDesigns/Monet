//! Figma 原型查看器网页界面的 GPUI 复刻。
//!
//! 布局来源：`docs/prototype-viewer-layout.md`（从 Figma 另存 HTML 的 DOM + CSS 提取）。
//! 复刻的是**网页本身的界面**（顶栏 / 左流程栏 / viewer / 底栏 / 右评论栏），
//! 不是画布里绘制的设计稿内容。
//!
//! ```text
//! cargo run -p figcraft-ui-gpui --example prototype_viewer
//! ```

use figcraft_ui_gpui::widgets::{pt, rgba, tokens};
use gpui::{
    App, Application, Bounds, Context, Render, Rgba, SharedString, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, size,
};

/// u8 RGBA → GPUI 颜色（等价于 widgets::rgba(Color32)）。
fn c(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
}

// ── 布局常量（来自 HTML inline style / CSS） ──────────────────────────────
const TOOLBAR_H: f32 = 40.0; // fullscreen override --toolbar-height
const FOOTER_H: f32 = 48.0;
const LEFT_W: f32 = 240.0; // 左侧流程栏宽度
const RIGHT_W: f32 = 260.0; // 右侧评论栏宽度（HTML 未给固定值，按 Figma 默认）
const EDGE_W: f32 = 6.0; // resize 线宽度

struct PrototypeViewer {
    left_open: bool,
    right_open: bool,
}

impl PrototypeViewer {
    fn new() -> Self {
        Self { left_open: false, right_open: false }
    }

    /// 顶部工具栏：三列。左=Figma logo+评论，中=标题，右=头像+分享原型+选项+全屏。
    fn header(&self) -> gpui::Div {
        let t = tokens();
        div()
            .flex_none()
            .h(pt(TOOLBAR_H))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(pt(12.0))
            .bg(c(0, 0, 0, 191))
            // 左列
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(12.0))
                    // Figma logo（5 色简易圆点）
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(pt(1.0))
                            .child(logo_dot(0xF2, 0x4E, 0x1E))
                            .child(logo_dot(0xA2, 0x59, 0xFF))
                            .child(logo_dot(0x1A, 0xBC, 0xFE))
                            .child(logo_dot(0x0A, 0xCF, 0x83))
                            .child(logo_dot(0xFF, 0x72, 0x62)),
                    )
                    .child(icon_btn("评论")),
            )
            // 中列：标题
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(6.0))
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text))
                    .child(SharedString::from("FigCraft-UI-ui-vars"))
                    .child(SharedString::from("⌄")),
            )
            // 右列
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(8.0))
                    .child(avatar())
                    .child(
                        div()
                            .h(pt(24.0))
                            .px(pt(12.0))
                            .flex()
                            .items_center()
                            .rounded(pt(6.0))
                            .bg(c(0x0D, 0x99, 0xF2, 255))
                            .text_size(pt(11.0))
                            .text_color(c(255, 255, 255, 255))
                            .child(SharedString::from("分享原型")),
                    )
                    .child(icon_btn("选项"))
                    .child(icon_btn("全屏")),
            )
    }

    /// 左侧流程栏（收起时宽 0，仅留 6pt resize 边）。
    fn left_sidebar(&self) -> gpui::Div {
        if !self.left_open {
            return div().flex_none().w(px(0.0)).h_full();
        }
        let t = tokens();
        div()
            .relative()
            .flex_none()
            .w(pt(LEFT_W))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgba(t.panel))
            .border_r_1()
            .border_color(rgba(t.border))
            // 头部："流程" + 关闭
            .child(
                div()
                    .flex_none()
                    .h(pt(36.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(pt(10.0))
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text))
                    .child(SharedString::from("流程"))
                    .child(SharedString::from("✕")),
            )
            // 空流程列表（HTML inline height:23px）
            .child(div().flex_none().h(pt(23.0)).border_b_1().border_color(rgba(t.border)))
            // 描述区
            .child(
                div()
                    .flex_1()
                    .p(pt(10.0))
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text_secondary))
                    .child(SharedString::from("No description")),
            )
            // resize 边（ew-resize）
            .child(
                div()
                    .absolute()
                    .right(px(-EDGE_W / 2.0))
                    .top(px(TOOLBAR_H - 40.0))
                    .w(pt(EDGE_W))
                    .h_full()
                    .cursor(gpui::CursorStyle::ResizeLeftRight),
            )
    }

    /// 中间 viewer：画布区 + 底部工具栏。
    fn viewer(&self) -> gpui::Div {
        let t = tokens();
        div()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .relative()
            .flex()
            .flex_col()
            // 画布区域（黑底），内部显示"设计稿内容"占位说明
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .w_full()
                    .relative()
                    .overflow_hidden()
                    .child(
                        div()
                            .absolute()
                            .left(pt(48.0)) // HTML inline left:47.996px
                            .top(pt(60.0)) // HTML inline top:59.9901px
                            .w(pt(680.0)) // 2093.97 缩放后
                            .h(pt(346.0)) // 1067.94 缩放后
                            .rounded(pt(2.0))
                            .border_1()
                            .border_color(rgba(t.border))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(pt(11.0))
                            .text_color(rgba(t.text_tertiary))
                            .child(SharedString::from("FigCraft design canvas")),
                    ),
            )
            // footer
            .child(
                div()
                    .flex_none()
                    .h(pt(FOOTER_H))
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(pt(12.0))
                    // 左：debug 空
                    .child(div().w(pt(80.0)))
                    // 中：stepper + 设备下拉
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(pt(10.0))
                            .text_size(pt(11.0))
                            .text_color(rgba(t.text))
                            .child(SharedString::from("1 / 1"))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(pt(4.0))
                                    .child(SharedString::from("MacBook Pro 14\""))
                                    .child(SharedString::from("⌄")),
                            ),
                    )
                    // 右：Restart pill
                    .child(
                        div()
                            .h(pt(30.0))
                            .px(pt(12.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(pt(6.0))
                            .rounded(pt(15.0))
                            .bg(c(0, 0, 0, 204))
                            .text_size(pt(11.0))
                            .text_color(rgba(t.text))
                            .child(SharedString::from("Restart"))
                            .child(SharedString::from("R")),
                    ),
            )
    }

    /// 右侧评论栏。
    fn right_sidebar(&self) -> gpui::Div {
        if !self.right_open {
            return div().flex_none().w(px(0.0)).h_full();
        }
        let t = tokens();
        div()
            .flex_none()
            .w(pt(RIGHT_W))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgba(t.panel))
            .border_l_1()
            .border_color(rgba(t.border))
            .child(
                div()
                    .flex_none()
                    .h(pt(36.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(8.0))
                    .px(pt(8.0))
                    // 搜索框
                    .child(
                        div()
                            .flex_1()
                            .h(pt(24.0))
                            .px(pt(8.0))
                            .flex()
                            .items_center()
                            .rounded(pt(4.0))
                            .bg(rgba(t.field))
                            .text_size(pt(11.0))
                            .text_color(rgba(t.text_secondary))
                            .child(SharedString::from("搜索")),
                    )
                    .child(SharedString::from("⚙"))
                    .child(SharedString::from("✕")),
            )
            .child(
                div()
                    .p(pt(12.0))
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text_secondary))
                    .child(SharedString::from("提供反馈，提出问题，或只是留下鼓励的话。点击文件中的任意位置，即可留下评论。")),
            )
    }
}

impl Render for PrototypeViewer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(c(0, 0, 0, 255))
            .child(self.header())
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .w_full()
                    .flex()
                    .flex_row()
                    .child(self.left_sidebar())
                    .child(self.viewer())
                    .child(self.right_sidebar()),
            )
    }
}

fn logo_dot(r: u8, g: u8, b: u8) -> gpui::Div {
    div()
        .size(pt(6.0))
        .rounded_full()
        .bg(c(r, g, b, 255))
}

fn icon_btn(_label: &str) -> gpui::Div {
    let t = tokens();
    div()
        .size(pt(20.0))
        .rounded(pt(4.0))
        .flex()
        .items_center()
        .justify_center()
        .text_size(pt(12.0))
        .text_color(rgba(t.text))
        .child(SharedString::from("·"))
}

fn avatar() -> gpui::Div {
    div()
        .size(pt(20.0))
        .rounded_full()
        .bg(c(0x0D, 0x99, 0xF2, 255))
}

fn main() {
    figcraft_text::scan_system_fonts();
    let app = Application::new();
    app.run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let _view = cx.open_window(options, |_window, cx| cx.new(|_cx| PrototypeViewer::new()));
    });
}
