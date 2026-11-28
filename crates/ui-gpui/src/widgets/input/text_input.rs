// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// 2026-11-27 补充：用户批准 P2-6「扩 TextEdit 支持 multiline」，本文件为此增加 multiline 分支；
// 默认 `multiline=false` 的单行路径与既有行为保持不变。
// ============================================================================
//! `TextInput` — Monet's most primitive, reusable single-line GPUI input.
//!
//! GPUI 0.2.1 ships no built-in `TextInput` element, so this implements the editor from scratch:
//! `WindowTextSystem::shape_line` for layout, `LineLayout::closest_index_for_x` for click-to-caret,
//! `x_for_index` for caret/selection quads, and `window.handle_input` for the platform IME.
//!
//! **All editing state lives in [`EditBuffer`]** (character indices, grapheme motion, composition).
//! This file only owns the GPUI-specific part: shaping a line, converting UTF-8 byte offsets
//! reported by `ShapedLine` to character indices, painting quads and caching the last layout for
//! hit-tests. No editing algorithm is duplicated here.

use crate::widgets::input::{EditBuffer, caret};
use crate::widgets::{TEXT_PT, pt, rgba, tokens};
use gpui::{
    App, Bounds, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Hsla, IntoElement, PaintQuad, Pixels, Point, ShapedLine, SharedString,
    TextRun, UTF16Selection, Window, canvas, fill, point, prelude::*, px, size,
};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

/// Cached shaped line + its paint bounds. Written in `prepaint`, read by pointer handlers and by
/// `EntityInputHandler::character_index_for_point` (which run later in the frame).
/// Where the glyphs sit inside the editor box. Numeric fields centre their value; the caret,
/// selection, hit-testing and the IME candidate window all have to use the *same* origin.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl TextAlign {
    fn origin_x(self, bounds: Bounds<Pixels>, text_w: Pixels) -> Pixels {
        match self {
            TextAlign::Left => bounds.origin.x,
            TextAlign::Center => bounds.origin.x + (bounds.size.width - text_w) * 0.5,
            TextAlign::Right => bounds.origin.x + bounds.size.width - text_w,
        }
    }
}

#[derive(Default)]
struct LayoutCache {
    /// Single-line shaped line (kept separate so existing callers stay byte-for-byte identical).
    line: Option<ShapedLine>,
    /// Multiline shaped lines, one per visual line.
    lines: Vec<ShapedLine>,
    /// Window-space origin of every multiline row.
    line_origins: Vec<Point<Pixels>>,
    /// Character index at which every multiline row starts.
    line_starts: Vec<usize>,
    /// Text of every multiline row (needed to map character indices to bytes).
    line_texts: Vec<String>,
    bounds: Option<Bounds<Pixels>>,
    /// X of the first glyph in window coordinates (may differ from `bounds.origin.x` when centred).
    text_x: Option<Pixels>,
    /// Whether the shaped line was the composition view (it cannot be mapped back to the buffer).
    composed: bool,
}

/// One painted row of a multiline editor.
struct PaintLine {
    line: ShapedLine,
    origin: Point<Pixels>,
    text_height: Pixels,
    selection: Option<PaintQuad>,
    preedit: Option<PaintQuad>,
}

/// State handed from `prepaint` to `paint`.
struct PaintState {
    lines: Vec<PaintLine>,
    caret: Option<PaintQuad>,
}

/// Where to paint `line`, and the line height to hand `ShapedLine::paint`.
///
/// `paint_line` draws each glyph at `origin.y + padding_top + ascent`, and `padding_top` is
/// `(line_height - ascent - descent) / 2`. Passing `line_height = ascent + descent` makes
/// `padding_top` zero, so the baseline lands exactly at `origin.y + ascent`.
///
/// The remaining question is where the baseline should go so the glyphs look centred. Rather than
/// guessing a cap ratio, ask the font: `typographic_bounds` gives the real box of a digit relative
/// to the baseline, so its centre can be put on the field's centre.
pub fn centred_line_origin(bounds: Bounds<Pixels>, line: &ShapedLine, _font_size: Pixels, _window: &Window) -> (Point<Pixels>, Pixels) {
    // Centre the line box (ascent + descent) in the field, exactly as a plain `text_size` label
    // is centred. Aligning a digit's ink box instead makes the text jump a pixel or two when a
    // label turns into an editor, which is what the inline rename was doing.
    let line_height = line.ascent + line.descent;
    let top = bounds.origin.y + (bounds.size.height - line_height) * 0.5;
    (point(bounds.origin.x, top), line_height)
}

/// Blink state shared across every frame of one editor. The visible/hidden rhythm comes from the
/// same `caret` module the canvas uses; `started` only keeps the repaint ticker from being spawned
/// once per frame.
struct CaretBlink {
    started: std::cell::Cell<Option<std::time::Instant>>,
}

impl CaretBlink {
    fn visible(&self) -> bool {
        caret::visible_since(self.started.get(), std::time::Instant::now())
    }

    fn clear(&self) {
        self.started.set(None);
    }

    fn start(&self) {
        if self.started.get().is_none() {
            self.started.set(Some(std::time::Instant::now()));
        }
    }
}

/// A primitive single-line editor. Embed it in any view: hold a `TextInput`, render
/// [`TextInput::element`] with the view's `Entity` and focus handle, and delegate the view's
/// `EntityInputHandler` to the `input_*` methods (see `TextField` for the complete wiring).
pub struct TextInput {
    pub buffer: EditBuffer,
    cache: Rc<RefCell<LayoutCache>>,
    font_size: Pixels,
    color: Hsla,
    placeholder_color: Hsla,
    dim_color: Hsla,
    placeholder: SharedString,
    secret: bool,
    suffix: SharedString,
    /// A computed value (a Fill child's size) reads at 50% ink in egui's number fields.
    dim: bool,
    align: TextAlign,
    caret_blink: Rc<CaretBlink>,
    /// Multiline mode (Developer Mode note / code-connect template). Off by default; ordinary
    /// fields keep the frozen single-line path.
    multiline: bool,
    /// Visible rows in multiline mode.
    rows: usize,
}

impl TextInput {
    pub fn new() -> Self {
        let t = tokens();
        Self {
            buffer: EditBuffer::new(""),
            cache: Rc::new(RefCell::new(LayoutCache::default())),
            font_size: pt(TEXT_PT),
            color: rgba(t.text).into(),
            placeholder_color: rgba(t.text_tertiary).into(),
            dim_color: Hsla { h: 0.0, s: 0.0, l: 1.0, a: 0.5 },
            placeholder: SharedString::new(""),
            secret: false,
            suffix: SharedString::new(""),
            dim: false,
            align: TextAlign::Left,
            caret_blink: Rc::new(CaretBlink { started: std::cell::Cell::new(None) }),
            multiline: false,
            rows: 1,
        }
    }

    /// Set where the glyphs sit inside the editor box. All hit-testing follows this origin.
    pub fn set_align(&mut self, align: TextAlign) {
        self.align = align;
    }

    pub fn set_placeholder(&mut self, p: impl Into<SharedString>) {
        self.placeholder = p.into();
    }
    pub fn set_secret(&mut self, s: bool) {
        self.secret = s;
    }
    pub fn set_suffix(&mut self, s: impl Into<SharedString>) {
        self.suffix = s.into();
    }
    pub fn set_dim(&mut self, d: bool) {
        self.dim = d;
    }
    /// Enable/disable multiline editing. `rows = 0` restores the frozen single-line editor; any
    /// positive value lays out that many 24 pt rows and lets `EditBuffer` keep line breaks.
    pub fn set_multiline(&mut self, rows: usize) {
        let was = self.multiline;
        self.multiline = rows > 0;
        self.rows = rows.max(1);
        self.buffer.set_multiline(self.multiline);
        if was && !self.multiline {
            // Re-enter the single-line invariant: a pooled field must not keep a line break from
            // its previous multiline use. Only on the transition, so repeated per-frame resets
            // do not touch a live draft.
            let text = self.buffer.text().replace('\n', " ");
            self.buffer.set_text(text);
        }
    }
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }
    /// The glyph size. The inline rename editor sets this to match the label it replaces:
    /// two different sizes mean two different line boxes, and centring each one in the row
    /// leaves the baselines a pixel or two apart.
    pub fn set_font_size(&mut self, size: Pixels) {
        self.font_size = size;
    }

    /// Caret colour is shared by every editor; the third parameter is kept for API compatibility.
    pub fn set_colors(&mut self, text: Hsla, placeholder: Hsla, _caret: Hsla) {
        self.color = text;
        self.placeholder_color = placeholder;
    }

    /// What the line shows: composed text (IME), masked text, the value + suffix while not
    /// focused, or the placeholder when empty. The composition view always wins.
    fn display(&self, focused: bool) -> (SharedString, bool) {
        let c = self.buffer.composed();
        if c.composing {
            return (SharedString::from(c.text), true);
        }
        if self.buffer.is_empty() {
            return (self.placeholder.clone(), false);
        }
        let shown = if self.secret { "•".repeat(self.buffer.len()) } else { c.text };
        let suffix = if !focused && !self.multiline { self.suffix.as_ref() } else { "" };
        (SharedString::from(format!("{}{}", shown, suffix)), false)
    }

    /// Point (window coords) → character index. Outside the editor it clamps to the ends.
    pub fn index_for_point(&self, p: Point<Pixels>) -> usize {
        let cache = self.cache.borrow();
        let Some(bounds) = cache.bounds else { return 0 };
        if self.multiline {
            let Some((row, origin, line)) = self.multiline_row_at(&cache, p.y) else {
                return self.buffer.len();
            };
            let x = p.x - origin.x;
            let local = if x <= px(0.0) { 0 } else { self.buffer.from_byte(line.closest_index_for_x(x)) };
            let start = cache.line_starts.get(row).copied().unwrap_or(0);
            return (start + local).min(self.buffer.len());
        }
        let text_x = cache.text_x.unwrap_or(bounds.origin.x);
        if p.x <= text_x {
            return 0;
        }
        let byte = match cache.line.as_ref().map(|l| l.closest_index_for_x(p.x - text_x)) {
            Some(b) => b,
            None => return self.buffer.len(),
        };
        self.buffer.from_byte(byte)
    }

    /// Which multiline row contains `y`, with its origin and shaped line. Clamps above/below.
    fn multiline_row_at<'a>(&self, cache: &'a LayoutCache, y: Pixels) -> Option<(usize, Point<Pixels>, &'a ShapedLine)> {
        if cache.line_origins.is_empty() {
            return None;
        }
        let h = cache.bounds.map(|b| self.row_height(&b)).unwrap_or(px(24.0));
        let i = cache
            .line_origins
            .iter()
            .position(|o| y >= o.y && y < o.y + h)
            .unwrap_or_else(|| if y < cache.line_origins[0].y { 0 } else { cache.line_origins.len() - 1 });
        Some((i, *cache.line_origins.get(i)?, cache.lines.get(i)?))
    }

    fn row_height(&self, bounds: &Bounds<Pixels>) -> Pixels {
        if self.multiline { bounds.size.height / self.rows.max(1) as f32 } else { bounds.size.height }
    }

    // ── EntityInputHandler delegation (no editing logic duplicated) ─────────────────────────
    pub fn input_text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>) -> Option<String> {
        self.buffer.text_for_utf16(range, adjusted)
    }
    pub fn input_selected_range(&mut self) -> Option<UTF16Selection> {
        Some(self.buffer.selected_utf16())
    }
    pub fn input_marked_range(&self) -> Option<Range<usize>> {
        self.buffer.marked_utf16()
    }
    pub fn input_unmark(&mut self) {
        self.buffer.commit_preedit();
    }
    pub fn input_replace(&mut self, range: Option<Range<usize>>, text: &str) {
        self.buffer.replace_utf16(range, text);
    }
    pub fn input_replace_and_mark(&mut self, range: Option<Range<usize>>, new_text: &str, selected: Option<Range<usize>>) {
        self.buffer.replace_and_mark_utf16(range, new_text, selected);
    }
    pub fn input_bounds_for_range(&self, range: Range<usize>, element_bounds: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        let cache = self.cache.borrow();
        if cache.composed {
            return Some(element_bounds);
        }
        if self.multiline {
            let ci = self.buffer.utf16_to_char(range.start);
            let row = self.buffer.line_index(ci);
            let line = cache.lines.get(row)?;
            let origin = *cache.line_origins.get(row)?;
            let text = cache.line_texts.get(row)?;
            let local_start = ci.saturating_sub(cache.line_starts.get(row).copied().unwrap_or(0));
            let local_end = self.buffer.utf16_to_char(range.end).saturating_sub(cache.line_starts.get(row).copied().unwrap_or(0));
            let x0 = line.x_for_index(byte_at(text, local_start));
            let x1 = line.x_for_index(byte_at(text, local_end.min(text.chars().count())));
            return Some(Bounds { origin: point(origin.x + x0, origin.y), size: size((x1 - x0).max(px(1.0)), self.row_height(&element_bounds)) });
        }
        let line = cache.line.as_ref()?;
        if cache.composed {
            return Some(element_bounds);
        }
        let x0 = line.x_for_index(self.buffer.to_byte(self.buffer.utf16_to_char(range.start)));
        let x1 = line.x_for_index(self.buffer.to_byte(self.buffer.utf16_to_char(range.end)));
        // The platform passes the element bounds; the glyphs may start further right when centred.
        let shift = cache.text_x.unwrap_or(element_bounds.origin.x) - cache.bounds.map_or(element_bounds.origin.x, |b| b.origin.x);
        Some(Bounds { origin: point(element_bounds.origin.x + shift + x0, element_bounds.origin.y), size: size(x1 - x0, element_bounds.size.height) })
    }
    pub fn input_character_index_for_point(&self, p: Point<Pixels>) -> Option<usize> {
        let usable = {
            let cache = self.cache.borrow();
            cache.bounds.is_some() && !cache.composed
        };
        if !usable {
            return None; // composition view has no stable mapping back to the committed text
        }
        Some(self.buffer.char_to_utf16(self.index_for_point(p)))
    }

    /// The editor's canvas element. Registers `handle_input` in `paint` (paint-only API), draws
    /// selection behind the glyphs, the caret on top, and an underline under a live composition.
    pub fn element<V: EntityInputHandler + 'static>(
        &self,
        entity: Entity<V>,
        focus: FocusHandle,
        focused: bool,
        cursor: Option<gpui::CursorStyle>,
    ) -> impl IntoElement {
        let (display, is_composed) = self.display(focused);
        let blink = self.caret_blink.clone();
        let ticker_started = Rc::new(std::cell::Cell::new(false));
        let prepaint_ticker = ticker_started.clone();
        let blink_entity = entity.clone();
        let font_size = self.font_size;
        // ⛔ 颜色**每帧从 `tokens()` 重新取**，不要用构造时存下来的 `self.color`：
        // `TextInput::new()` 只在实体创建时跑一次，那一刻主题还没设定（线程局部还是默认
        // 暗色），存下来的颜色会让输入框永远停在白字上，切主题也不变。
        //
        // 输入框里的**值**用「90% 墨」（亮色主题 = 90% 黑 #1A1A1A，比正文 1E1E1E 更实一点）；
        // 暗色主题仍是纯白。占位符用 text_tertiary，dim（计算值）用 50% 墨。
        let t = tokens();
        let ink = if t.dark { rgba(t.text) } else { rgba(crate::canvas::geom::Color32::from_rgb(0x1A, 0x1A, 0x1A)) };
        let dim_ink = if t.dark {
            rgba(crate::canvas::geom::Color32::from_white_alpha(128))
        } else {
            rgba(crate::canvas::geom::Color32::from_black_alpha(128))
        };
        let color: Hsla = if self.buffer.is_empty() {
            rgba(t.text_tertiary)
        } else if self.dim {
            dim_ink
        } else {
            ink
        }
        .into();
        // 光标与选区色统一从 `caret` 取，画布文字编辑用的是同一份。
        let caret_color: Hsla = rgba(caret::color(&t)).into();
        let selection_color: Hsla = rgba(caret::selection(&t)).into();
        let align = self.align;
        let multiline = self.multiline;
        let rows = self.rows.max(1);
        let c = self.buffer.composed();
        let sel_range = if c.composing { c.mark_start..c.mark_end } else { self.buffer.range() };
        let caret_char = c.caret;
        let mark_range = c.composing.then_some((c.mark_start, c.mark_end));
        let cache = self.cache.clone();

        let prepaint = move |bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App| {
            let ticker_started = prepaint_ticker.clone();
            let source = display.to_string();
            let raw_lines: Vec<&str> = source.split('\n').collect();
            let row_h = if multiline { bounds.size.height / rows as f32 } else { bounds.size.height };
            // Keep the caret's row visible when the text has more lines than the box shows.
            let caret_row_guess = line_row(&source, caret_char);
            let first_line = if multiline && raw_lines.len() > rows { caret_row_guess.saturating_sub(rows - 1) } else { 0 };
            let last_line = (first_line + if multiline { rows } else { 1 }).min(raw_lines.len());
            let mut paint_lines = Vec::new();
            let mut cache_lines = Vec::new();
            let mut line_origins = Vec::new();
            let mut line_starts = Vec::new();
            let mut line_texts = Vec::new();
            let mut caret_quad = None;
            let mut char_cursor = char_index_of_row(&source, first_line);
            for idx in first_line..last_line {
                let raw = raw_lines[idx];
                let line_text = SharedString::from(raw.to_string());
                let run =
                    TextRun { len: raw.len(), font: window.text_style().font(), color, background_color: None, underline: None, strikethrough: None };
                let line = window.text_system().shape_line(line_text.clone(), font_size, &[run], None);
                let text_height = line.ascent + line.descent;
                let row_top = if multiline { bounds.origin.y + (idx - first_line) as f32 * row_h } else { bounds.origin.y };
                let text_top =
                    if multiline { row_top + (row_h - text_height) * 0.5 } else { centred_line_origin(bounds, &line, font_size, window).0.y };
                let text_x = if multiline { bounds.origin.x } else { centred_line_origin(bounds, &line, font_size, window).0.x };
                let line_start = char_cursor;
                let line_len = raw.chars().count();
                let line_end = line_start + line_len;

                let selection = if sel_range.start < line_end && sel_range.end > line_start {
                    let a = sel_range.start.saturating_sub(line_start).min(line_len);
                    let b = sel_range.end.saturating_sub(line_start).min(line_len);
                    let x0 = line.x_for_index(byte_at(raw, a));
                    let x1 = line.x_for_index(byte_at(raw, b));
                    Some(fill(Bounds { origin: point(text_x + x0, text_top), size: size(x1 - x0, text_height) }, selection_color))
                } else {
                    None
                };
                let preedit = mark_range.and_then(|(a, b)| {
                    if a >= line_end || b <= line_start {
                        return None;
                    }
                    let a = a.saturating_sub(line_start).min(line_len);
                    let b = b.saturating_sub(line_start).min(line_len);
                    let x0 = line.x_for_index(byte_at(raw, a));
                    let x1 = line.x_for_index(byte_at(raw, b));
                    Some(fill(Bounds { origin: point(text_x + x0, text_top + text_height - px(1.0)), size: size(x1 - x0, px(1.0)) }, color))
                });
                let is_caret_line = caret_char >= line_start && caret_char <= line_end;
                if is_caret_line && focused && blink.visible() && sel_range.start == sel_range.end {
                    let local = caret_char.saturating_sub(line_start).min(line_len);
                    let x = line.x_for_index(byte_at(raw, local));
                    let mut quad = fill(Bounds { origin: point(text_x + x, text_top), size: size(px(caret::WIDTH), text_height) }, caret_color);
                    quad.corner_radii = gpui::Corners::all(px(caret::RADIUS as f32));
                    caret_quad = Some(quad);
                }
                line_origins.push(point(text_x, row_top));
                line_starts.push(line_start);
                line_texts.push(raw.to_string());
                cache_lines.push(line.clone());
                paint_lines.push(PaintLine { line, origin: point(text_x, text_top), text_height, selection, preedit });
                char_cursor = line_end + 1; // the `\n` between rows
            }
            // Blink: one ticker per editor, 500 ms on / 500 ms off (gpui-kit's `CursorBlink`
            // default), restarted whenever the field gains focus and reset to visible when it
            // loses it.
            if !focused {
                blink.clear();
            } else {
                blink.start();
                if !ticker_started.get() {
                    ticker_started.set(true);
                    let e = blink_entity.clone();
                    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                        loop {
                            cx.background_executor().timer(caret::BLINK).await;
                            if e.update(cx, |_, cx| cx.notify()).is_err() {
                                break;
                            }
                        }
                    })
                    .detach();
                }
            }
            if std::env::var_os("FC_DEBUG_TEXT").is_some() {
                eprintln!("[text] bounds_y={} h={} multiline={} rows={}", bounds.origin.y, bounds.size.height, multiline, rows);
            }
            let text_x = if multiline { Some(bounds.origin.x) } else { cache_lines.first().map(|l| align.origin_x(bounds, l.width)) };
            *cache.borrow_mut() = LayoutCache {
                line: if multiline { None } else { cache_lines.first().cloned() },
                lines: cache_lines,
                line_origins,
                line_starts,
                line_texts,
                bounds: Some(bounds),
                text_x,
                composed: is_composed,
            };
            PaintState { lines: paint_lines, caret: caret_quad }
        };

        let paint = move |bounds: Bounds<Pixels>, state: PaintState, window: &mut Window, cx: &mut App| {
            window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
            let PaintState { lines, caret } = state;
            for line in lines {
                if let Some(quad) = line.selection {
                    window.paint_quad(quad);
                }
                let _ = line.line.paint(line.origin, line.text_height, window, cx);
                if let Some(quad) = line.preedit {
                    window.paint_quad(quad);
                }
            }
            if let Some(quad) = caret {
                window.paint_quad(quad);
            }
        };
        let el = canvas(prepaint, paint).size_full();
        match cursor {
            Some(c) => el.cursor(c),
            None => el,
        }
    }
}

/// Character index → UTF-8 byte offset in an arbitrary (possibly composed) string.
fn byte_at(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

/// Zero-based row containing character `ci` in `text`.
fn line_row(text: &str, ci: usize) -> usize {
    text.chars().take(ci).filter(|c| *c == '\n').count()
}

/// Character index at which row `row` starts in `text`.
fn char_index_of_row(text: &str, row: usize) -> usize {
    if row == 0 {
        return 0;
    }
    let mut seen = 0;
    for (i, c) in text.chars().enumerate() {
        if c == '\n' {
            seen += 1;
            if seen == row {
                return i + 1;
            }
        }
    }
    text.chars().count()
}

impl Default for TextInput {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_at_stays_on_char_boundaries() {
        let s = "a中😀z";
        assert_eq!(byte_at(s, 0), 0);
        assert_eq!(byte_at(s, 1), 1);
        assert_eq!(byte_at(s, 2), 4);
        assert_eq!(byte_at(s, 3), 8);
        assert_eq!(byte_at(s, 99), s.len());
    }

    #[test]
    fn masked_and_suffix_displays_never_touch_the_buffer() {
        let mut i = TextInput::new();
        i.buffer.set_text("12");
        i.set_suffix("%");
        assert_eq!(i.display(false).0.as_ref(), "12%");
        assert_eq!(i.display(true).0.as_ref(), "12");
        assert_eq!(i.buffer.text(), "12");
        i.set_secret(true);
        assert_eq!(i.display(false).0.as_ref(), "••%");
    }

    #[test]
    fn composition_view_wins_over_suffix_and_mask() {
        let mut i = TextInput::new();
        i.buffer.set_text("ab");
        i.buffer.move_to(1);
        i.set_secret(true);
        i.set_suffix("%");
        i.buffer.set_preedit("中", 1);
        assert_eq!(i.display(false).0.as_ref(), "a中b");
    }

    #[test]
    fn multiline_mode_keeps_breaks_and_ignores_suffix() {
        let mut i = TextInput::new();
        i.set_multiline(2);
        i.buffer.set_text("one\ntwo");
        i.set_suffix("%");
        assert!(i.is_multiline());
        assert_eq!(i.display(false).0.as_ref(), "one\ntwo");
        assert_eq!(i.buffer.line_count(), 2);
        i.set_multiline(0);
        assert!(!i.is_multiline());
        assert_eq!(i.buffer.text(), "one two", "pooled reuse removes the line break");
    }
}
