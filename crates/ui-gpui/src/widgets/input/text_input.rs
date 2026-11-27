// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// 2026-11-27 补充：用户明确要求给共享 TextEdit 增加多行能力（评论 composer 的 Shift+Enter
// 换行），本文件按该指令解冻；以上 INVARIANT 仍保持不变。
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
    line: Option<ShapedLine>,
    /// Explicit-line layouts for multiline mode. One entry per `\n`-separated paragraph.
    lines: Vec<CachedLine>,
    bounds: Option<Bounds<Pixels>>,
    /// X of the first glyph in window coordinates (may differ from `bounds.origin.x` when centred).
    text_x: Option<Pixels>,
    /// Whether the shaped line was the composition view (it cannot be mapped back to the buffer).
    composed: bool,
    multiline: bool,
    line_height: Pixels,
    /// Vertical scroll in multiline mode, kept so the caret stays inside the composer.
    scroll_y: Pixels,
}

struct CachedLine {
    line: ShapedLine,
    text: SharedString,
    /// Character index of this visual line's first character in the whole buffer.
    char_start: usize,
    /// Character index one past this visual line's last character (a `\n` or a soft wrap point is
    /// not included).
    char_end: usize,
    /// A soft-wrapped row continues into the next cached line; a paragraph-final row does not.
    wrapped: bool,
}

/// State handed from `prepaint` to `paint`.
struct PaintState {
    line: ShapedLine,
    /// Window-space point where the line starts (its baseline box top-left).
    origin: Point<Pixels>,
    /// The line's own height (ascent + descent): what centres it in the field, as egui does.
    text_height: Pixels,
    selection: Option<PaintQuad>,
    caret: Option<PaintQuad>,
    /// Underline under the live IME composition, egui draws the same in `canvas/overlays.rs`.
    preedit: Option<PaintQuad>,
}

/// State handed from `prepaint` to `paint` in multiline mode.
struct MultilinePaintState {
    lines: Vec<(ShapedLine, Point<Pixels>)>,
    line_height: Pixels,
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
    preedit: Option<PaintQuad>,
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
    /// Shape every `\n`-separated paragraph on its own line. Single-line fields keep the old path.
    multiline: bool,
    caret_blink: Rc<CaretBlink>,
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
            multiline: false,
            caret_blink: Rc::new(CaretBlink { started: std::cell::Cell::new(None) }),
        }
    }

    /// Enable multiline editing: paragraphs stack, long rows soft-wrap, and pasted newlines stay.
    pub fn set_multiline(&mut self, multiline: bool) {
        self.multiline = multiline;
        self.buffer.set_multiline(multiline);
    }

    pub fn is_multiline(&self) -> bool {
        self.multiline
    }

    /// Test hook: how many visual rows the last paint cached, and where it scrolled.
    #[cfg(test)]
    pub(crate) fn visual_rows_for_test(&self) -> (usize, Pixels) {
        let cache = self.cache.borrow();
        (cache.lines.len(), cache.scroll_y)
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
        let suffix = if !focused { self.suffix.as_ref() } else { "" };
        (SharedString::from(format!("{}{}", shown, suffix)), false)
    }

    /// Point (window coords) → character index. Outside the editor it clamps to the ends.
    pub fn index_for_point(&self, p: Point<Pixels>) -> usize {
        let cache = self.cache.borrow();
        let Some(bounds) = cache.bounds else { return 0 };
        let text_x = cache.text_x.unwrap_or(bounds.origin.x);
        if cache.multiline && !cache.lines.is_empty() {
            let line_ix = if p.y <= bounds.origin.y {
                0
            } else {
                (((p.y - bounds.origin.y + cache.scroll_y) / cache.line_height) as usize).min(cache.lines.len() - 1)
            };
            let line = &cache.lines[line_ix];
            if p.x <= text_x {
                return line.char_start;
            }
            let byte = line.line.closest_index_for_x(p.x - text_x);
            return line.char_start + line.text.as_ref()[..byte].chars().count();
        }
        if p.x <= text_x {
            return 0;
        }
        let byte = match cache.line.as_ref().map(|l| l.closest_index_for_x(p.x - text_x)) {
            Some(b) => b,
            None => return self.buffer.len(),
        };
        self.buffer.from_byte(byte)
    }

    /// The character range of the visual line containing `caret`. Used by Home/End in a
    /// multiline editor; single-line callers keep the buffer-wide behaviour.
    pub(crate) fn line_bounds(&self, caret: usize) -> Option<(usize, usize)> {
        let cache = self.cache.borrow();
        if !cache.multiline || cache.lines.is_empty() {
            return None;
        }
        let ix = line_index_for_char(&cache.lines, caret);
        let line = &cache.lines[ix];
        Some((line.char_start, line.char_end))
    }

    /// Move `caret` one visual line up/down while keeping the x position as close as possible.
    pub(crate) fn vertical_target(&self, caret: usize, dir: i32) -> Option<usize> {
        let cache = self.cache.borrow();
        if !cache.multiline || cache.lines.is_empty() {
            return None;
        }
        let ix = line_index_for_char(&cache.lines, caret);
        let line = &cache.lines[ix];
        let local = caret.saturating_sub(line.char_start).min(line.char_end - line.char_start);
        let x = line.line.x_for_index(byte_at(&line.text, local));
        let target_ix = (ix as i32 + dir).clamp(0, cache.lines.len() as i32 - 1) as usize;
        if target_ix == ix {
            return Some(caret);
        }
        let target = &cache.lines[target_ix];
        let byte = target.line.closest_index_for_x(x);
        Some(target.char_start + target.text.as_ref()[..byte].chars().count())
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
        if cache.multiline && !cache.lines.is_empty() {
            if cache.composed {
                return Some(element_bounds);
            }
            let start = self.buffer.utf16_to_char(range.start);
            let end = self.buffer.utf16_to_char(range.end);
            let ix = line_index_for_char(&cache.lines, start);
            let line = &cache.lines[ix];
            let local_start = start.saturating_sub(line.char_start).min(line.char_end - line.char_start);
            let local_end = end.saturating_sub(line.char_start).min(line.char_end - line.char_start);
            let x0 = line.line.x_for_index(byte_at(&line.text, local_start));
            let x1 = line.line.x_for_index(byte_at(&line.text, local_end));
            let top = cache.bounds.map_or(element_bounds.origin.y, |b| b.origin.y) + cache.line_height * ix as f32 - cache.scroll_y;
            // The platform passes the element bounds; keep the horizontal shift used by glyphs.
            let text_x = cache.text_x.unwrap_or(element_bounds.origin.x);
            let shift = text_x - cache.bounds.map_or(element_bounds.origin.x, |b| b.origin.x);
            return Some(Bounds { origin: point(element_bounds.origin.x + shift + x0, top), size: size((x1 - x0).max(px(1.0)), cache.line_height) });
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
        let cache = self.cache.borrow();
        if cache.multiline && !cache.lines.is_empty() {
            if cache.composed {
                return None; // composition view has no stable mapping back to the committed text
            }
            let ci = self.index_for_point(p);
            return Some(self.buffer.char_to_utf16(ci));
        }
        let bounds = cache.bounds.as_ref()?;
        let line = cache.line.as_ref()?;
        if cache.composed {
            return None; // composition view has no stable mapping back to the committed text
        }
        let text_x = cache.text_x.unwrap_or(bounds.origin.x);
        let ci = self.buffer.from_byte(line.closest_index_for_x(p.x - text_x));
        Some(self.buffer.char_to_utf16(ci))
    }

    /// The editor's canvas element. Registers `handle_input` in `paint` (paint-only API), draws
    /// selection behind the glyphs, the caret on top, and an underline under a live composition.
    pub fn element<V: EntityInputHandler + 'static>(
        &self,
        entity: Entity<V>,
        focus: FocusHandle,
        focused: bool,
        cursor: Option<gpui::CursorStyle>,
    ) -> gpui::AnyElement {
        if self.multiline {
            return self.multiline_element(entity, focus, focused, cursor).into_any_element();
        }
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
        let c = self.buffer.composed();
        let sel_range = if c.composing { c.mark_start..c.mark_end } else { self.buffer.range() };
        let sel_a = byte_at(&c.text, sel_range.start);
        let sel_b = byte_at(&c.text, sel_range.end);
        let caret_byte = byte_at(&c.text, c.caret);
        let preedit_bytes = c.composing.then(|| (byte_at(&c.text, c.mark_start), byte_at(&c.text, c.mark_end)));
        let cache = self.cache.clone();

        let prepaint = move |bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App| {
            let ticker_started = prepaint_ticker.clone();
            let run = TextRun {
                len: display.as_ref().len(),
                font: window.text_style().font(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window.text_system().shape_line(display.clone(), font_size, &[run], None);
            let text_x = align.origin_x(bounds, line.width);
            // 选区与光标用**同一条行盒**（`ascent + descent`），并且和文字同顶。
            // 以前选区取 `bounds.size.height`（整个输入框的高度），高亮块比字形高一整截，
            // 在 24pt 行里尤其明显；gpui-kit 的 Input 也是按行盒画选区与光标的。
            let (line_origin, text_height) = centred_line_origin(bounds, &line, font_size, window);
            let text_top = line_origin.y;
            let selection = if sel_a != sel_b {
                let x0 = line.x_for_index(sel_a);
                let x1 = line.x_for_index(sel_b);
                Some(fill(Bounds { origin: point(text_x + x0, text_top), size: size(x1 - x0, text_height) }, selection_color))
            } else {
                None
            };
            let preedit = preedit_bytes.map(|(a, b)| {
                let x0 = line.x_for_index(a);
                let x1 = line.x_for_index(b);
                fill(Bounds { origin: point(text_x + x0, text_top + text_height - px(1.0)), size: size(x1 - x0, px(1.0)) }, color)
            });
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
            let caret_visible = blink.visible();
            // The caret shares the line box with the glyphs: same top, same height — both are
            // taken from `centred_line_origin` above.
            let caret_quad = if focused && caret_visible && sel_a == sel_b {
                let x = line.x_for_index(caret_byte);
                // 1 pt wide with a 0.5 pt radius: both ends are round, so the caret reads as a
                // pill rather than a hairline rectangle.
                let mut quad = fill(Bounds { origin: point(text_x + x, line_origin.y), size: size(px(caret::WIDTH), text_height) }, caret_color);
                quad.corner_radii = gpui::Corners::all(px(caret::RADIUS as f32));
                Some(quad)
            } else {
                None
            };
            if std::env::var_os("FC_DEBUG_TEXT").is_some() {
                eprintln!(
                    "[text] bounds_y={} h={} ascent={} descent={} top={} h_line={}",
                    bounds.origin.y, bounds.size.height, line.ascent, line.descent, text_top, text_height
                );
            }
            *cache.borrow_mut() =
                LayoutCache { line: Some(line.clone()), bounds: Some(bounds), text_x: Some(text_x), composed: is_composed, ..Default::default() };
            PaintState { line, origin: point(text_x, text_top), text_height, selection, caret: caret_quad, preedit }
        };

        let paint = move |bounds: Bounds<Pixels>, state: PaintState, window: &mut Window, cx: &mut App| {
            window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
            if let Some(quad) = state.selection {
                window.paint_quad(quad);
            }
            let _ = state.line.paint(state.origin, state.text_height, window, cx);
            if let Some(quad) = state.preedit {
                window.paint_quad(quad);
            }
            if let Some(quad) = state.caret {
                window.paint_quad(quad);
            }
        };
        let el = canvas(prepaint, paint).size_full();
        match cursor {
            Some(c) => el.cursor(c).into_any_element(),
            None => el.into_any_element(),
        }
    }

    /// Multiline rendering. One shaped line per explicit `\n` paragraph; the parent composer gives
    /// the editor its height and clips lines beyond it. No soft wrapping, so caret/selection
    /// geometry stays byte-accurate per paragraph.
    fn multiline_element<V: EntityInputHandler + 'static>(
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
        let caret_color: Hsla = rgba(caret::color(&t)).into();
        let selection_color: Hsla = rgba(caret::selection(&t)).into();
        let c = self.buffer.composed();
        let sel_range = if c.composing { c.mark_start..c.mark_end } else { self.buffer.range() };
        let sel_a = sel_range.start;
        let sel_b = sel_range.end;
        let caret = c.caret;
        let preedit = c.composing.then(|| (c.mark_start, c.mark_end));
        let cache = self.cache.clone();

        let prepaint = move |bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App| {
            let ticker_started = prepaint_ticker.clone();
            let previous_scroll = cache.borrow().scroll_y;
            let font = window.text_style().font();
            let mut cached = Vec::new();
            let mut char_start = 0;
            let mut line_height = font_size * 1.35;
            let text_top = bounds.origin.y;
            let text_x = bounds.origin.x;
            let wrap_width = bounds.size.width.max(px(1.0));
            for (ix, part) in display.split('\n').enumerate() {
                let chars: Vec<char> = part.chars().collect();
                if chars.is_empty() {
                    let text = SharedString::new("");
                    let run = TextRun { len: 0, font: font.clone(), color, background_color: None, underline: None, strikethrough: None };
                    let line = window.text_system().shape_line(text.clone(), font_size, &[run], None);
                    if ix == 0 {
                        line_height = (line.ascent + line.descent).max(font_size * 1.35);
                    }
                    cached.push(CachedLine { line, text, char_start, char_end: char_start, wrapped: false });
                    // The paragraph break itself is not part of any visual row.
                    char_start += 1;
                    continue;
                }
                let full_text = SharedString::from(part.to_string());
                let full_run =
                    TextRun { len: full_text.len(), font: font.clone(), color, background_color: None, underline: None, strikethrough: None };
                let full = window.text_system().shape_line(full_text.clone(), font_size, &[full_run], None);
                if ix == 0 {
                    line_height = (full.ascent + full.descent).max(font_size * 1.35);
                }
                let mut start = 0;
                while start < chars.len() {
                    let byte_start = byte_at(part, start);
                    let x0 = full.x_for_index(byte_start);
                    let mut fit = chars.len();
                    for end in start + 1..=chars.len() {
                        if full.x_for_index(byte_at(part, end)) - x0 > wrap_width {
                            fit = end.saturating_sub(1);
                            break;
                        }
                    }
                    // Prefer a word boundary: a trailing space stays on the row it belongs to.
                    if fit < chars.len() {
                        if let Some(space) = chars[start..fit].iter().rposition(|c| c.is_whitespace() || *c == '-') {
                            if space > 0 {
                                fit = start + space + 1;
                            }
                        }
                        fit = fit.max(start + 1);
                    }
                    let text: String = chars[start..fit].iter().collect();
                    let len = text.len();
                    let run = TextRun { len, font: font.clone(), color, background_color: None, underline: None, strikethrough: None };
                    let line = window.text_system().shape_line(SharedString::from(text.clone()), font_size, &[run], None);
                    let wrapped = fit < chars.len();
                    cached.push(CachedLine {
                        line,
                        text: SharedString::from(text),
                        char_start: char_start + start,
                        char_end: char_start + fit,
                        wrapped,
                    });
                    start = fit;
                }
                char_start += chars.len() + 1;
            }
            // A fixed-height composer clips long comments. Keep the caret line inside the visible
            // box instead of letting the user type past the bottom edge blindly.
            let total_h = line_height * cached.len() as f32;
            let visible_h = bounds.size.height.max(px(1.0));
            let mut scroll_y = previous_scroll;
            if total_h > visible_h {
                let caret_ix = line_index_for_char(&cached, caret);
                let caret_top = line_height * caret_ix as f32;
                if caret_top < scroll_y {
                    scroll_y = caret_top;
                } else if caret_top + line_height > scroll_y + visible_h {
                    scroll_y = caret_top + line_height - visible_h;
                }
                scroll_y = scroll_y.max(px(0.0)).min(total_h - visible_h);
            } else {
                scroll_y = px(0.0);
            }
            let lines: Vec<(ShapedLine, Point<Pixels>)> = cached
                .iter()
                .enumerate()
                .map(|(ix, line)| (line.line.clone(), point(text_x, text_top + line_height * ix as f32 - scroll_y)))
                .collect();
            let mut selection = Vec::new();
            for line in &cached {
                let start = sel_a.max(line.char_start).min(line.char_end);
                let end = sel_b.max(line.char_start).min(line.char_end);
                if start < end {
                    let x0 = line.line.x_for_index(byte_at(&line.text, start - line.char_start));
                    let x1 = line.line.x_for_index(byte_at(&line.text, end - line.char_start));
                    selection.push(fill(
                        Bounds {
                            origin: point(text_x + x0, text_top + line_height * line_index_for_char(&cached, line.char_start) as f32 - scroll_y),
                            size: size((x1 - x0).max(px(1.0)), line_height),
                        },
                        selection_color,
                    ));
                }
            }
            let caret_quad = if focused && blink.visible() && sel_a == sel_b {
                let ix = line_index_for_char(&cached, caret);
                let line = &cached[ix];
                let local = caret.saturating_sub(line.char_start).min(line.char_end - line.char_start);
                let x = line.line.x_for_index(byte_at(&line.text, local));
                let mut quad = fill(
                    Bounds { origin: point(text_x + x, text_top + line_height * ix as f32 - scroll_y), size: size(px(caret::WIDTH), line_height) },
                    caret_color,
                );
                quad.corner_radii = gpui::Corners::all(px(caret::RADIUS as f32));
                Some(quad)
            } else {
                None
            };
            let preedit = preedit.and_then(|(a, b)| {
                let ix = line_index_for_char(&cached, a);
                let line = &cached[ix];
                let a = a.saturating_sub(line.char_start).min(line.char_end - line.char_start);
                let b = b.saturating_sub(line.char_start).min(line.char_end - line.char_start);
                let x0 = line.line.x_for_index(byte_at(&line.text, a));
                let x1 = line.line.x_for_index(byte_at(&line.text, b));
                Some(fill(
                    Bounds {
                        origin: point(text_x + x0, text_top + line_height * ix as f32 + line_height - px(1.0) - scroll_y),
                        size: size((x1 - x0).max(px(1.0)), px(1.0)),
                    },
                    color,
                ))
            });
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
            *cache.borrow_mut() = LayoutCache {
                line: None,
                lines: cached,
                bounds: Some(bounds),
                text_x: Some(text_x),
                composed: is_composed,
                multiline: true,
                line_height,
                scroll_y,
            };
            MultilinePaintState { lines, line_height, selection, caret: caret_quad, preedit }
        };

        let paint = move |bounds: Bounds<Pixels>, state: MultilinePaintState, window: &mut Window, cx: &mut App| {
            window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
            for quad in state.selection {
                window.paint_quad(quad);
            }
            for (line, origin) in state.lines {
                let _ = line.paint(origin, state.line_height, window, cx);
            }
            if let Some(quad) = state.preedit {
                window.paint_quad(quad);
            }
            if let Some(quad) = state.caret {
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

/// Which visual row contains `caret`. At a soft-wrap boundary the caret belongs to the row that
/// starts there, while at a paragraph-final boundary (the `\n` is not part of the row) it belongs
/// to the row that just ended.
fn line_index_for_char(lines: &[CachedLine], caret: usize) -> usize {
    lines.iter().position(|l| caret < l.char_end || (caret == l.char_end && !l.wrapped)).unwrap_or(lines.len() - 1)
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
}
