//! 双向文字里的名称（希伯来文、阿拉伯文等 RTL 与混排脚本）：标签、页面、图层、样式、组件和
//! 文件名。
//!
//! 上游 `ui-egui/src/bidi.rs` 把名称交给 egui 的 `LayoutJob`，由 `TextWrapping::truncate_at_width`
//! 完成两件事：按 UAX #9 排成视觉顺序、把省略号放在**视觉末端**（RTL 名称的左端）。
//!
//! GPUI 这边排版由平台承担（Windows 的 DirectWrite、macOS 的 Core Text、Linux 的
//! cosmic-text）：`shape_line` 已经按 UAX #9 双向排版、按字体 OpenType 特性连写阿拉伯字母、
//! 镜像括号，并且保持「一个字形对应一个逻辑字符」，于是光标 / 选区 / 点击偏移仍是逻辑位置。
//! 所以界面**照旧传逻辑顺序的字符串**，本模块只补上游由 `truncate_at_width` 提供的那部分：
//! 按实测宽度把逻辑尾部截掉并补一个省略号；重排由平台完成，省略号自然落到视觉末端。

use gpui::{SharedString, TextRun, Window, px};

/// 截断名称时补的省略号（与 `ui-egui` 的 `TextWrapping::truncate_at_width` 一致）。
pub const ELLIPSIS: &str = "…";

/// `name` 能放进 `max_w` 的最长逻辑前缀，放不下时补上 [`ELLIPSIS`]。
///
/// `measure` 返回一段逻辑字符串塑形后的宽度。因为塑形器会重排，RTL 名称末尾的省略号在屏幕上
/// 会出现在左端 —— 与上游 `bidi::paint_name` 的视觉结果相同。空名称返回空串；连省略号都放
/// 不下（`max_w` 小于省略号宽）时也返回空串，避免把省略号本身画出格子。
pub fn fit_with(name: &str, max_w: f32, measure: impl Fn(&str) -> f32) -> String {
    if name.is_empty() {
        return String::new();
    }
    let max_w = max_w.max(1.0);
    if measure(name) <= max_w {
        return name.to_owned();
    }
    if measure(ELLIPSIS) > max_w {
        return String::new();
    }
    let mut out = String::new();
    for ch in name.chars() {
        out.push(ch);
        let fitted = measure(&format!("{out}{ELLIPSIS}"));
        if fitted > max_w {
            out.pop();
            break;
        }
    }
    out.push_str(ELLIPSIS);
    out
}

/// [`fit_with`] 的 GPUI 版：用当前窗口的文字系统按 `font_size`（pt）实测宽度。
///
/// 字体与 `inline_input` / 各输入框同一个来源（`window.text_style().font()`），所以同一行里
/// 静态名称与编辑态的文字度量一致。
pub fn fit(window: &Window, name: &str, font_size: f32, max_w: f32) -> SharedString {
    // CodeRabbit PR#18: a document/layer name can contain a newline (file.new, unsanitised input).
    // `shape_line` asserts on line breaks in debug builds, so fold them into spaces before measuring.
    let single_line = name.replace(['\n', '\r'], " ");
    SharedString::from(fit_with(&single_line, max_w, |probe| measure(window, probe, font_size)))
}

/// 一段逻辑字符串在当前窗口文字系统、`font_size`（pt）下的塑形宽度。
pub fn measure(window: &Window, text: &str, font_size: f32) -> f32 {
    let run = TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color: gpui::Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(SharedString::from(text.to_owned()), px(font_size), &[run], None);
    f32::from(line.width)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 等宽假度量：每个字符 10 pt，省略号 10 pt。
    fn measure(text: &str) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    #[test]
    fn a_name_that_fits_is_left_whole() {
        assert_eq!(fit_with("Frame 12", 80.0, measure), "Frame 12");
        assert_eq!(fit_with("", 80.0, measure), "");
        // 正好放满也算放得下。
        assert_eq!(fit_with("abcde", 50.0, measure), "abcde");
    }

    #[test]
    fn a_long_name_is_cut_and_ends_in_an_ellipsis() {
        assert_eq!(fit_with("abcdefghij", 50.0, measure), "abcd…");
        assert!(measure(&fit_with("abcdefghij", 55.0, measure)) <= 55.0);
    }

    #[test]
    fn right_to_left_names_keep_their_logical_prefix() {
        // 上游把逻辑顺序的名称整段交给 egui；这里同样只截逻辑尾部。视觉上省略号落在左端
        // 是平台双向排版的职责，纯逻辑层不参与重排。
        let name = "שלום עולם";
        let fitted = fit_with(name, 50.0, measure);
        assert!(fitted.ends_with(ELLIPSIS), "{fitted}");
        assert!(name.starts_with(fitted.trim_end_matches(ELLIPSIS)), "{fitted}");
        assert!(measure(&fitted) <= 50.0);
    }

    #[test]
    fn no_room_for_the_ellipsis_draws_nothing() {
        assert_eq!(fit_with("abcdefghij", 5.0, measure), "");
        // 刚好放下省略号时只剩省略号（等宽上 `…` 是 10 pt），再多一个字符就放不下。
        assert_eq!(fit_with("abcdefghij", 10.0, measure), "…");
        assert_eq!(fit_with("abcdefghij", 11.0, measure), "…");
    }
}
