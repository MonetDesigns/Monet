//! 白板页的底格：点阵（缺省）、线、方格纸。换成 `figcraft_render::Dots` 交给整页渲染，
//! 画在页面底色之上、图层之下，所以平移/缩放/分带渲染都逐像素对得上。
//!
//! 点阵的档位、淡入和点径照参考实现给（档位函数在 `figcraft_engine::board_grid` 里）：
//! 缩放 50% 以下没有点、50%→100% 淡进来，点径 2 个页面单位、缩小时不低于半个设备像素。
//! 颜色不写死，由页面底色推出来（`figcraft_color::Color::mark`）。

use figcraft_color::Color;
use figcraft_engine::board_grid::{GridStyle, dot_fade, dot_step, line_spacing};
use figcraft_render::Dots;

use crate::app::CanvasApp;

/// 线之间在屏幕上最近的距离（设备像素）：再密就翻倍。
const LINE_MIN_GAP: f64 = 10.0;

/// 这一页的底格，`zoom` 是每页面单位多少设备像素（渲染尺度）。设计文件、关掉网格、或
/// 缩到点阵看不见时没有。
pub(super) fn dots(app: &CanvasApp, zoom: f64, page: [u8; 4]) -> Option<Dots> {
    if !(zoom.is_finite() && zoom > 0.0) {
        return None;
    }
    if !app.session.doc().is_ok_and(figcraft_engine::board::is_board) {
        return None;
    }
    let g = app.session.prefs.board_grid.clone().sanitized();
    if g.style == GridStyle::None {
        return None;
    }
    let page = Color::from_rgba8(page[0], page[1], page[2], 255);
    // 深浅由页面底色推：浅页压暗、深页提亮，色相留着。对比度是用户的倍数。
    let contrast = if g.contrast.is_finite() { g.contrast.clamp(0.25, 4.0) as f32 } else { 1.0 };
    if g.style == GridStyle::Dots {
        let fade = dot_fade(zoom);
        if fade <= 0.0 {
            return None;
        }
        // 点径：2 个页面单位（`size` 1 就是它），缩小时不低于半个设备像素的半径。
        let size = (2.0 * g.size).max(1.0 / zoom);
        // 点色：参考实现的白板点阵是单通道灰度，所以走 `mark_gray` —— 保住「比页面暗一步」
        // 的亮度，但不把页面的色相带进点里（否则放大看是一层褐点）。步长按 FigJam 实测：
        // #F5F5F5 底上点芯 #DBDBDB，(245-219)/255 = 0.102。
        let ink = page.mark_gray(0.102 * contrast).with_alpha(fade);
        // 点与点同色：`faded` 与 `color` 一样，`major` 只是让渲染层的强弱分档不起作用。
        return Some(Dots { step: dot_step(zoom, g.spacing), size, color: ink, faded: ink, lines: false, major: 2 });
    }
    // 线 / 方格纸：本项目自己的样式，间距按翻倍规则，方格纸每隔 `major` 条一条粗线。
    let major = if g.style == GridStyle::Graph { g.major.max(2) } else { 2 };
    let (step, fade) = line_spacing(zoom, g.spacing, major, LINE_MIN_GAP)?;
    let ink = page.mark(0.06 * contrast);
    let faint = fade * if g.style == GridStyle::Graph { 0.5 } else { 1.0 };
    Some(Dots { step, size: g.size.max(0.5) / zoom, color: ink, faded: page.mark(0.06 * contrast * faint), lines: true, major })
}
