// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// 2026-11-05 补充：用户随后明确指令「重构 color_field / template 插槽隐藏 / number_field 范围 /
// 失焦提交」，本文件按该指令解冻；以上 INVARIANT 仍保持不变。
// 2026-11-27 补充：用户明确要求本控件支持多行（评论 composer 的 Shift+Enter 换行），按该指令
// 解冻；以上 INVARIANT 仍保持不变。
// ============================================================================
//! `TextEdit` — `ui-egui`'s **`text_field_edit` / `field_editor`**: the editing lifecycle only.
//!
//! It owns no shell, no slots and no layout. The caller gives it a place to live (`frame.edit` in a
//! [`super::template::Input`], or the number cell of a `Field-color`'s trail) and it paints the
//! value, takes the caret, and commits. That split is what `ui-egui` does:
//!
//! ```text
//! field()        → Input (shell + slots + layout) + text_field_edit into frame.edit
//! color_input()  → Input (shell + slots + layout) + text_field_edit into frame.edit
//!                                              + text_field_edit into frame.alpha
//! ```
//!
//! A colour field is therefore **one** `Input`, not a field containing two fields.

use crate::widgets::input::{EditOutcome, TextAlign, TextInput};
use crate::widgets::{FIELD_H, dbg_frame, pt};
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, DispatchPhase, EntityInputHandler, FocusHandle, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, UTF16Selection, Window, canvas, div, prelude::*, px,
};
use std::ops::Range;
use std::rc::Rc;

type CommitHandler = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// How a number field steps with the arrow keys / scrubs with the pointer, and the range it
/// accepts. `ui-egui`'s `number_field` never clamps at the widget — the callers do it on write —
/// but the design requires the input itself to reject out-of-range values (opacity 0..100,
/// corner ≥ 0, star points 3..60 …), so the clamp lives here in one place.
#[derive(Clone, Copy, Debug)]
pub struct NumberConfig {
    pub step: f64,
    /// Big step (Shift / ten steps) — egui's `key_steps` uses `step * 10` unless a nudge pref is set.
    pub big: f64,
    /// `Empty::Auto`: typing the word "Auto" (or "auto") commits 0.
    pub auto: bool,
    /// 允许的最小值（含）；`None` = 不限制下界。
    pub min: Option<f64>,
    /// 允许的最大值（含）；`None` = 不限制上界。
    pub max: Option<f64>,
}

impl NumberConfig {
    /// A plain config with no range (the old behaviour).
    pub fn new(step: f64) -> Self {
        Self { step, big: step * 10.0, auto: false, min: None, max: None }
    }
    /// Attach an inclusive `[min, max]` range; `None` leaves that side unbounded.
    pub fn with_range(mut self, min: Option<f64>, max: Option<f64>) -> Self {
        self.min = min;
        self.max = max;
        self
    }
    /// Clamp a value into the configured range. Non-finite input stays untouched.
    pub fn clamp(&self, v: f64) -> f64 {
        if !v.is_finite() {
            return v;
        }
        let v = self.min.map_or(v, |lo| v.max(lo));
        self.max.map_or(v, |hi| v.min(hi))
    }
}

/// The editing lifecycle of one field's value.
pub struct TextEdit {
    pub(crate) input: TextInput,
    /// The committed value, and what it was when the current draft began (`kept` in egui).
    value: String,
    kept: String,
    editing: bool,
    /// The click that starts an edit has not released yet: keep select-all laid until it does.
    select_all_pending: bool,
    /// `keep_focus`: the editor element is new on the start frame, ask for focus once it exists.
    focus_pending: bool,
    number: Option<NumberConfig>,
    /// 字段后缀是 `%`：输入里的百分号只是单位（`field_math::parse` 的 `percent_unit`），
    /// 不再代表「相对当前值的百分比」。
    percent_unit: bool,
    pub(crate) focus: FocusHandle,
    /// 失焦订阅：焦点离开就立即提交，不再依赖「下一帧恰好重绘到这个 Entity」。
    /// GPUI 会缓存未标脏的嵌套 Entity 的 prepaint（`view.rs:208-214`），把提交只挂在
    /// `render()` 里的失焦轮询上，点画布时会漏掉。
    focus_out: Option<gpui::Subscription>,
    on_commit: Option<CommitHandler>,
    /// Like `on_commit`, but receives the raw draft before number resolution. Fields whose
    /// callback parses expressions (e.g. `+10` for a Mixed vector point) need the original text:
    /// resolving it to a scalar first turns the relative edit into an absolute value.
    on_commit_draft: Option<CommitHandler>,
    /// 每改一个字符就回调一次（首页搜索这类「边打边过滤」用；`on_commit` 只在 Enter / 失焦时给）。
    on_change: Option<CommitHandler>,
    /// Scrub state: the value at drag start and the accumulated amount (egui's `scrub_id`).
    pub(crate) scrub: Option<(f64, f64)>,
    pub(crate) scrub_x: Option<Pixels>,
    last_click: Option<(std::time::Instant, Point<Pixels>, u32)>,
    /// `text_field_edit`'s left inset: `0` when the field has a lead, `8` when it has none.
    inset_left: f32,
    /// `field_editor_in`'s right pad — `8` for a field's value, `0` for a cell that can spare none.
    right_pad: f32,
    /// Right-align the value: the opacity's number sits right-aligned in its 44 pt cell.
    align: TextAlign,
    /// Explicit newlines are enabled: stack paragraphs, Shift+Enter inserts a break.
    multiline: bool,
    /// A read-only field never enters edit mode (a display-only combo).
    readonly: bool,
    /// Escape clears this field (a search box).
    search_clear: bool,
}

impl TextEdit {
    pub fn new(cx: &mut App) -> Self {
        Self {
            input: TextInput::new(),
            value: String::new(),
            kept: String::new(),
            editing: false,
            select_all_pending: false,
            focus_pending: false,
            number: None,
            percent_unit: false,
            focus: cx.focus_handle(),
            focus_out: None,
            on_commit: None,
            on_commit_draft: None,
            on_change: None,
            scrub: None,
            scrub_x: None,
            last_click: None,
            inset_left: 0.0,
            right_pad: 0.0,
            align: TextAlign::Left,
            multiline: false,
            readonly: false,
            search_clear: false,
        }
    }

    pub fn build(cx: &mut App, f: impl FnOnce(&mut TextEdit, &mut App)) -> gpui::Entity<Self> {
        cx.new(|cx| {
            let mut s = Self::new(cx);
            f(&mut s, cx);
            s
        })
    }

    // ── configuration ───────────────────────────────────────────────────────────────────────
    /// Push the document's value in. A value that did not change is a no-op, and a field the user
    /// is typing into keeps its draft: `ui-egui` redraws the same number every frame without ever
    /// touching the editor's text, so a push that overwrote a live draft would be a bug there too.
    pub fn set_value(&mut self, v: impl Into<String>) {
        let v = v.into();
        // 外部推来的文档值也要先夹进范围：否则一个越界的文档值会绕过输入限制显示出来。
        // 非数值（hex、paint 名、"Mixed"）parse 失败，原样保留。
        let v = match self.number {
            Some(cfg) => v.parse::<f64>().ok().map(|n| fmt_num(cfg.clamp(n))).unwrap_or(v),
            None => v,
        };
        if v == self.value {
            return;
        }
        self.value = v;
        self.kept = self.value.clone();
        if self.editing {
            return;
        }
        self.input.buffer.set_text(self.value.clone());
    }

    /// Replace the visible draft too (mention insertion is a programmatic edit while the field
    /// is active). `set_value` deliberately keeps a live draft; this does not change that rule
    /// for document pushes.
    pub(crate) fn set_draft_text(&mut self, v: impl Into<String>) {
        let v = v.into();
        self.value = v.clone();
        self.kept = v.clone();
        self.input.buffer.set_text(v);
    }
    /// The live caret position as a character index (the mention query ends here, not at EOF).
    pub(crate) fn draft_caret(&self) -> usize {
        self.input.buffer.range().end
    }
    pub fn set_placeholder(&mut self, p: impl Into<SharedString>) {
        self.input.set_placeholder(p);
    }
    pub fn set_secret(&mut self, s: bool) {
        self.input.set_secret(s);
    }
    pub fn set_dim(&mut self, d: bool) {
        self.input.set_dim(d);
    }
    /// `text_field_edit`'s inset: `0` after a lead, `8` when the field has none.
    /// The glyph size; the inline rename editor matches the row label's size with it.
    pub fn set_font_size(&mut self, size: gpui::Pixels) {
        self.input.set_font_size(size);
    }

    pub fn set_inset_left(&mut self, px: f32) {
        self.inset_left = px;
    }
    /// `field_editor_in`'s right pad.
    pub fn set_right_pad(&mut self, px: f32) {
        self.right_pad = px;
    }
    pub fn set_align(&mut self, a: TextAlign) {
        self.align = a;
        self.input.set_align(a);
    }
    /// Explicit newlines: the editor stacks paragraphs and Shift+Enter inserts a line break.
    /// Plain Enter still follows the existing commit path (comment composers intercept it first).
    pub fn set_multiline(&mut self, multiline: bool) {
        self.multiline = multiline;
        self.input.set_multiline(multiline);
    }
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }
    pub fn set_readonly(&mut self, b: bool) {
        self.readonly = b;
    }
    pub fn set_search_clear(&mut self, b: bool) {
        self.search_clear = b;
    }
    /// Ported from `number_field_bindable`: suffix + step (big step defaults to `step * 10`).
    pub fn set_numeric(&mut self, suffix: impl Into<SharedString>, step: f64) {
        let suffix = suffix.into();
        self.percent_unit = suffix.as_ref() == "%";
        self.input.set_suffix(suffix);
        self.number = Some(NumberConfig::new(step));
    }
    pub fn set_number_config(&mut self, cfg: NumberConfig) {
        self.number = Some(cfg);
    }
    /// Drop the numeric behaviour (a pooled slot reused by a hex / label row): no pointer scrub,
    /// no arrow stepping, no range clamp. The value it already holds is left untouched.
    pub fn clear_numeric(&mut self) {
        self.input.set_suffix("");
        self.number = None;
        self.percent_unit = false;
    }
    /// 设置允许的输入范围（含端点），并把当前值夹进去。
    /// 在 `set_numeric` / `set_number_config` **之后**调用，否则会被后写的配置覆盖。
    pub fn set_range(&mut self, min: Option<f64>, max: Option<f64>) {
        let cfg = self.number.get_or_insert_with(|| NumberConfig::new(1.0));
        cfg.min = min;
        cfg.max = max;
        let cfg = *cfg;
        if let Ok(v) = self.value.parse::<f64>() {
            let next = fmt_num(cfg.clamp(v));
            self.value = next.clone();
            self.kept = next.clone();
            if !self.editing {
                self.input.buffer.set_text(next);
            }
        }
    }
    pub fn set_on_commit(&mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) {
        self.on_commit = Some(Rc::new(f));
    }

    /// Commit callback that sees the unmodified draft (before number/expression resolution).
    pub fn set_on_commit_draft(&mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) {
        self.on_commit_draft = Some(Rc::new(f));
    }

    /// 每改一次文本就回调（含输入法逐字上屏、退格、粘贴）。和 `set_on_commit` 互不影响。
    pub fn set_on_change(&mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) {
        self.on_change = Some(Rc::new(f));
    }

    /// 把当前内容交给 `on_change`。
    fn notify_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.clear_vertical_target();
        let Some(cb) = self.on_change.clone() else { return };
        let text = self.input.buffer.text().to_string();
        cb(&text, window, cx);
    }
    pub fn value(&self) -> &str {
        &self.value
    }
    pub fn is_empty(&self) -> bool {
        self.input.buffer.is_empty()
    }
    pub fn is_readonly(&self) -> bool {
        self.readonly
    }
    /// The field takes a number: its lead drags, Arrows step, typing parses `+ - * /`.
    pub fn is_numeric(&self) -> bool {
        self.number.is_some()
    }
    /// The editor owns the keyboard this frame (the shell wears the accent hairline).
    pub fn is_active(&self, window: &Window) -> bool {
        self.editing && self.focus.is_focused(window)
    }
    /// Take the keyboard without a click — a caller that just opened a panel around this editor
    /// (the Actions palette) puts the caret in it, so typing filters instead of running canvas
    /// shortcuts.
    pub fn focus_editor(&mut self, window: &mut Window) {
        self.begin_edit(window, false);
    }

    /// 双击重命名：进入编辑的同时全选文字；再单击一次就落成单光标输入。
    pub fn focus_editor_select_all(&mut self, window: &mut Window) {
        self.begin_edit(window, true);
    }
    /// The lead's press: arm a scrub from the current value (`Sense::drag` on the grip).
    pub fn start_scrub(&mut self) {
        self.scrub = Some((self.value.parse::<f64>().unwrap_or(0.0), 0.0));
        self.scrub_x = None;
    }

    // ── edit lifecycle (egui's `text_field_edit`) ───────────────────────────────────────────
    fn begin_edit(&mut self, window: &mut Window, select_all: bool) {
        self.input.clear_vertical_target();
        let was = self.editing;
        self.editing = true;
        if !was {
            self.kept = self.value.clone();
            self.input.buffer.set_text(self.value.clone());
        }
        if select_all {
            self.input.buffer.select_all();
            self.select_all_pending = true;
        }
        // `keep_focus`: the editor element is new this frame, ask again once it exists.
        self.focus.focus(window);
        self.focus_pending = !self.focus.is_focused(window);
    }

    /// `select_all_on_focus`: lay select-all again until the pointer is released. A drag or a
    /// typed key cancels it, so a drag selects what it covers instead.
    fn tick_select_all(&mut self) {
        if self.select_all_pending && !self.input.buffer.composing() {
            self.input.buffer.select_all();
        }
    }

    /// Commit only when the draft differs from `kept` — egui: "a field left as it was commits
    /// nothing: no undo step, and a rounded display isn't written back".
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.editing {
            return false;
        }
        self.editing = false;
        self.select_all_pending = false;
        let draft = self.input.buffer.text().to_string();
        if let Some(cb) = self.on_commit_draft.clone() {
            cb(&draft, window, cx);
        }
        let value = self.resolve_number(&draft);
        let changed = value != self.kept;
        self.value = value.clone();
        self.kept = value.clone();
        self.input.buffer.set_text(value.clone());
        if !changed {
            return false;
        }
        if let Some(cb) = self.on_commit.clone() {
            cb(&value, window, cx);
        }
        true
    }

    /// 外部要求立即结束编辑并提交（例如点画布时由面板统一调用）。
    /// 没有在编辑时是 no-op，重复调用也安全。
    pub fn finish_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.commit(window, cx)
    }

    fn cancel(&mut self, window: &mut Window) {
        let saved = self.kept.clone();
        self.input.buffer.set_text(saved);
        self.editing = false;
        self.select_all_pending = false;
        window.blur();
    }

    /// egui 的 `typed`：数值、算术表达式，或 `+ - * /` 相对编辑开始时的值；百分比字段里的
    /// `%` 是单位，别的字段里 `50%` 表示一半。`Empty::Auto` 也接受 "auto" 一词。
    fn resolve_number(&self, text: &str) -> String {
        resolve_number_with(self.number, self.percent_unit, &self.kept, text)
    }

    /// ArrowUp/Down step, both while idle (`on_field`) and while typing. The stepped value
    /// stays selected so typing replaces it (egui's `selectall` temp).
    fn step_number(&mut self, dir: f64, big: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cfg) = self.number else { return };
        let by = (if big { cfg.big } else { cfg.step }) * dir;
        let base = if self.editing { self.input.buffer.text().to_string() } else { self.value.clone() };
        let cur = self.resolve_number(&base).parse::<f64>().unwrap_or(0.0);
        let next = fmt_num(cfg.clamp(crate::field_math::stepped(cur, 1.0, by)));
        self.input.buffer.set_text(next.clone());
        if self.editing {
            self.input.buffer.select_all();
        }
        self.value = next.clone();
        self.kept = next.clone();
        if let Some(cb) = self.on_commit.clone() {
            cb(&next, window, cx);
        }
        cx.notify();
    }

    /// egui's `scrub_amount` / `scrubbed`: Shift ×10, Alt ×0.1, quantised to whole steps.
    pub(crate) fn scrub_by(&mut self, dx: f64, shift: bool, alt: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cfg) = self.number else { return };
        let rate = match (shift, alt) {
            (true, false) => 10.0,
            (false, true) => 0.1,
            _ => 1.0,
        };
        let (v0, amount) = self.scrub.unwrap_or((self.value.parse::<f64>().unwrap_or(0.0), 0.0));
        let amount = amount + dx * cfg.step * rate;
        self.scrub = Some((v0, amount));
        let quantum = if alt { cfg.step * 0.1 } else { cfg.step };
        let raw = if quantum > 0.0 && quantum.is_finite() { v0 + (amount / quantum).round() * quantum } else { v0 };
        let next = fmt_num(cfg.clamp(raw));
        self.value = next.clone();
        self.input.buffer.set_text(next.clone());
        if let Some(cb) = self.on_commit.clone() {
            cb(&next, window, cx);
        }
        cx.notify();
    }
}

/// 纯函数形式的 `typed`：数值或算术表达式（数字后可带单位），或以编辑开始时的值为基准的
/// `+ - * /` 与 `%` 相对编辑（`field_math`）。`percent_unit` 为真时 `%` 只是单位。
/// 抽出来是为了让「范围夹紧」可以在没有 `Entity<TextEdit>` 的单元测试里直接验证。
fn resolve_number_with(number: Option<NumberConfig>, percent_unit: bool, kept: &str, text: &str) -> String {
    let Some(cfg) = number else { return text.to_string() };
    let t = text.trim();
    // 数字框提交时只接受能解析成数字（或算术/相对编辑）的文本；空输入、中文、英文
    // 之类一律退回编辑开始时的值。否则这些字会被当成新值写回字段，提交后一直留在
    // 数字框里（用户 2026-11-26）。上游 egui 的 `typed()` 解析失败也是不提交。
    if t.is_empty() {
        return kept.to_string();
    }
    if cfg.auto && t.eq_ignore_ascii_case("auto") {
        return "0".to_string();
    }
    let cur = kept.parse::<f64>().ok();
    let parsed = crate::field_math::eval(t, cur, percent_unit);
    parsed.map(|v| fmt_num(cfg.clamp(v))).unwrap_or_else(|| kept.to_string())
}

impl Render for TextEdit {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 失焦提交：焦点一离开就由 GPUI 的 focus-out 事件直接 commit，不依赖「下一次恰好
        // 重绘到这个 Entity」。GPUI 对未标脏的嵌套 Entity 会复用缓存的 prepaint，只把提交
        // 放在 render 里轮询焦点，点画布（焦点移到画布 FocusHandle）时就会漏提交。
        if self.focus_out.is_none() {
            let focus = self.focus.clone();
            self.focus_out = Some(cx.on_focus_out(&focus, window, |this, _e, window, cx| {
                if this.finish_edit(window, cx) {
                    cx.notify();
                }
            }));
        }
        // `text_field_edit`: losing focus commits (Enter commits in key()). 保留这条兜底，
        // 供没有触发 focus-out 事件（如窗口失活）的情况。
        if self.editing && !self.focus.is_focused(window) && !self.focus_pending {
            self.commit(window, cx);
        }
        if self.focus_pending && self.focus.is_focused(window) {
            self.focus_pending = false;
        }
        let focused = self.editing && self.focus.is_focused(window);
        if focused {
            self.tick_select_all();
        }

        let entity = cx.entity();
        let focus = self.focus.clone();
        let cursor = self.editing.then_some(CursorStyle::IBeam);
        let mut el = div()
            .relative()
            .w_full()
            .h_full()
            .flex()
            .when(self.multiline, |d| d.items_start())
            .when(!self.multiline, |d| d.items_center())
            .pl(pt(self.inset_left))
            .pr(pt(self.right_pad))
            .child(self.input.element(entity, focus, focused, cursor));

        // ── keyboard: `track_focus` registers the handle in the element tree, which is what
        //    routes keystrokes here. ──────────────────────────────────────────────────────────
        el = el.track_focus(&self.focus);

        // ── scrub: window-level move/up, so it keeps tracking outside the field ────────────
        if self.scrub.is_some() {
            let entity = cx.entity();
            el = el.child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        let mover = entity.clone();
                        window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble {
                                return;
                            }
                            let shift = window.modifiers().shift;
                            let alt = window.modifiers().alt;
                            mover.update(cx, |this, cx| {
                                // 窗口外松手：没有 `on_mouse_up` 会送到这里，下一帧 move 才带
                                // 着「按键已松开」回来。立刻结束 scrub，否则回窗后还会继续乱跳。
                                if e.pressed_button != Some(MouseButton::Left) {
                                    this.scrub = None;
                                    this.scrub_x = None;
                                    cx.notify();
                                    return;
                                }
                                if this.scrub.is_none() {
                                    return;
                                }
                                match this.scrub_x {
                                    Some(prev) => {
                                        let dx = f64::from(e.position.x - prev);
                                        if dx != 0.0 {
                                            this.scrub_x = Some(e.position.x);
                                            this.scrub_by(dx, shift, alt, window, cx);
                                        }
                                    }
                                    // First move after the press seeds the origin (no jump).
                                    None => this.scrub_x = Some(e.position.x),
                                }
                            });
                        });
                        let upper = entity.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                            if phase != DispatchPhase::Capture {
                                return;
                            }
                            upper.update(cx, |this, cx| {
                                if this.scrub.take().is_some() {
                                    this.scrub_x = None;
                                    cx.notify();
                                }
                            });
                        });
                    },
                )
                // 绝对定位：这块 canvas 只是用来挂 window 级 move/up 监听的，
                // 绝不能成为 flex 的第二个 item —— 否则一按下「%」输入框就被挤走，
                // 右对齐的数字会「跑到前面」，松手又弹回来（用户 2026-11-15）。
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h_full(),
            );
        }

        // ── pointer: click positions the caret, drag selects, double-click selects a word,
        //    triple-click selects the line ───────────────────────────────────────────────────
        el = el
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    if this.readonly {
                        // 只读字段不算「点进输入框」：让事件继续冒泡，由根视图统一提交别的草稿。
                        return;
                    }
                    // 点输入框自己 = 进入/继续编辑，**不要**让根视图把这次按下当成「点别处」
                    // 而把编辑提交掉（否则刚点进来就被收走，表现为点不进编辑态）。
                    cx.stop_propagation();
                    if this.select_all_pending {
                        // 双击进入编辑的**第二下**：这一下属于「打开编辑」的手势，不是定位光标
                        // 的点击。以前它会走下面的定位逻辑把刚做的全选清掉，于是双击后没有
                        // 选区、还得再点一次（用户 2026-11-16）。这里保持全选并消化掉标记。
                        this.select_all_pending = false;
                        return;
                    }
                    let now = std::time::Instant::now();
                    let count = match this.last_click {
                        Some((at, p, n))
                            if now.duration_since(at).as_millis() < 400
                                && (p.x - e.position.x).abs() < px(6.0)
                                && (p.y - e.position.y).abs() < px(6.0) =>
                        {
                            n + 1
                        }
                        _ => 1,
                    };
                    this.last_click = Some((now, e.position, count));
                    let idx = this.input.index_for_point(e.position);
                    match count {
                        1 => {
                            let starting = !this.editing;
                            this.begin_edit(window, starting && !e.modifiers.shift);
                            if !starting {
                                if e.modifiers.shift {
                                    this.input.buffer.select_to(idx);
                                } else {
                                    this.input.buffer.move_to(idx);
                                }
                            }
                            this.input.buffer.set_selecting(true);
                        }
                        2 => {
                            this.begin_edit(window, false);
                            this.input.buffer.select_word(idx);
                            this.select_all_pending = false;
                        }
                        _ => {
                            this.begin_edit(window, false);
                            this.input.buffer.select_line();
                            this.select_all_pending = false;
                        }
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _window, cx| {
                if this.scrub.is_some() {
                    return;
                }
                if this.input.buffer.is_selecting() {
                    let idx = this.input.index_for_point(e.position);
                    this.input.buffer.select_to(idx);
                    if !this.input.buffer.is_all_selected() {
                        this.select_all_pending = false;
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _window, cx| {
                    this.input.buffer.set_selecting(false);
                    this.select_all_pending = false;
                    this.scrub = None;
                    this.scrub_x = None;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _window, cx| {
                    this.input.buffer.set_selecting(false);
                    this.select_all_pending = false;
                    this.scrub = None;
                    this.scrub_x = None;
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let m = &e.keystroke.modifiers;
                let ctrl = m.platform || m.control;
                // GPUI names the arrows `up`/`down`/`left`/`right`; `EditBuffer` and egui use
                // `arrowup`… Without this the arrow branch never matched.
                let key = match e.keystroke.key.as_str() {
                    "left" => "arrowleft",
                    "right" => "arrowright",
                    "up" => "arrowup",
                    "down" => "arrowdown",
                    k => k,
                };
                // A run of Up/Down keeps the column it started from. Every other key ends that
                // run, so the next vertical move records a fresh x; Home/End read the current
                // wrap-boundary affinity themselves.
                if key != "arrowup" && key != "arrowdown" && key != "home" && key != "end" {
                    this.input.clear_vertical_target();
                }
                if ctrl {
                    match key {
                        "a" => this.input.buffer.select_all(),
                        "c" => {
                            let r = this.input.buffer.range();
                            let s: String = this.input.buffer.text().chars().skip(r.start).take(r.end - r.start).collect();
                            if !s.is_empty() {
                                cx.write_to_clipboard(ClipboardItem::new_string(s));
                            }
                        }
                        "x" => {
                            let r = this.input.buffer.range();
                            let s: String = this.input.buffer.text().chars().skip(r.start).take(r.end - r.start).collect();
                            if !s.is_empty() {
                                cx.write_to_clipboard(ClipboardItem::new_string(s));
                                this.input.buffer.insert("");
                                this.notify_change(window, cx);
                            }
                        }
                        "v" => {
                            let text = cx.read_from_clipboard().and_then(|i| i.text().map(|s| s.to_string()));
                            if let Some(s) = text {
                                this.input.buffer.insert(&s);
                                this.notify_change(window, cx);
                            }
                        }
                        _ => return,
                    }
                    this.select_all_pending = false;
                    cx.notify();
                    return;
                }
                // Arrow stepping for number fields (egui's `arrow_step`), idle or typing.
                if this.number.is_some() {
                    match key {
                        "arrowup" => {
                            this.step_number(1.0, m.shift, window, cx);
                            return;
                        }
                        "arrowdown" => {
                            this.step_number(-1.0, m.shift, window, cx);
                            return;
                        }
                        _ => {}
                    }
                }
                // Escape clears a search box (and closes its popover in egui), else cancels.
                if key == "escape" && this.search_clear {
                    this.input.buffer.set_text("");
                    this.value.clear();
                    this.kept.clear();
                    this.editing = false;
                    window.blur();
                    if let Some(cb) = this.on_commit.clone() {
                        cb("", window, cx);
                    }
                    cx.notify();
                    return;
                }
                // Multiline editing: Shift+Enter inserts a paragraph break, arrows/home/end follow
                // the visual paragraphs rather than the whole buffer.
                if this.multiline {
                    match key {
                        "enter" if m.shift => {
                            this.input.buffer.insert("\n");
                            this.select_all_pending = false;
                            this.notify_change(window, cx);
                            cx.notify();
                            return;
                        }
                        "arrowup" | "arrowdown" => {
                            let dir = if key == "arrowup" { -1 } else { 1 };
                            if let Some(target) = this.input.vertical_target(this.input.buffer.caret(), dir) {
                                if m.shift {
                                    this.input.buffer.select_to(target);
                                } else {
                                    this.input.buffer.move_to(target);
                                }
                                this.select_all_pending = false;
                                cx.notify();
                            }
                            return;
                        }
                        "home" | "end" => {
                            if let Some((start, end)) = this.input.line_bounds(this.input.buffer.caret()) {
                                let target = if key == "home" { start } else { end };
                                if m.shift {
                                    this.input.buffer.select_to(target);
                                } else {
                                    this.input.buffer.move_to(target);
                                }
                                this.select_all_pending = false;
                                cx.notify();
                            }
                            return;
                        }
                        _ => {}
                    }
                }
                // Printable characters belong to the platform input handler.
                if let Some(ch) = e.keystroke.key_char.as_ref().and_then(|s| s.chars().next()).filter(|c| *c >= ' ') {
                    if !this.editing {
                        this.begin_edit(window, false);
                        this.input.buffer.set_text("");
                        this.input.buffer.key(&ch.to_string(), false, false);
                        this.select_all_pending = false;
                        cx.notify();
                    }
                    return;
                }
                if !this.editing {
                    let ch = e.keystroke.key_char.as_ref().and_then(|s| s.chars().next()).filter(|c| *c >= ' ');
                    if ch.is_some() {
                        this.begin_edit(window, false);
                        this.input.buffer.set_text("");
                    }
                }
                let word = m.alt;
                let ch = e.keystroke.key_char.as_ref().and_then(|s| s.chars().next()).filter(|c| *c >= ' ');
                let typed = ch.map(|c| c.to_string()).unwrap_or_else(|| key.to_string());
                match this.input.buffer.key(&typed, m.shift, word) {
                    EditOutcome::Submit => {
                        this.commit(window, cx);
                        window.blur();
                    }
                    EditOutcome::Cancel => this.cancel(window),
                    EditOutcome::Changed => {
                        this.select_all_pending = false;
                        this.notify_change(window, cx);
                        cx.notify();
                    }
                    EditOutcome::None => {}
                }
            }));
        dbg_frame(el, 0xFF453A)
    }
}

impl EntityInputHandler for TextEdit {
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        self.input.input_text_for_range(range, adjusted)
    }
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        self.input.input_selected_range()
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.input.input_marked_range()
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.input.input_unmark();
        cx.notify();
    }
    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input.input_replace(range, text);
        self.select_all_pending = false;
        self.notify_change(window, cx);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input.input_replace_and_mark(range, new_text, selected);
        self.select_all_pending = false;
        self.notify_change(window, cx);
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.input.input_bounds_for_range(range, element_bounds)
    }
    fn character_index_for_point(&mut self, point: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        self.input.input_character_index_for_point(point)
    }
}

/// `fmt_num` from `ui-egui`: two decimals, `-0` and float noise removed, non-finite → empty.
pub fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r}") }
}

/// `FIELD_H` is the row height the editor fills; kept here so callers can size a cell.
pub const EDIT_H: f32 = FIELD_H;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_num_matches_egui() {
        assert_eq!(fmt_num(0.0), "0");
        assert_eq!(fmt_num(-0.0001), "0");
        assert_eq!(fmt_num(12.3456), "12.35");
        assert!(fmt_num(f64::NAN).is_empty());
    }

    /// 输入侧范围：百分比 0..100、尺寸 ≥0、点数 3..60，未配置时与原行为一致。
    #[test]
    fn number_range_clamps_both_sides() {
        let pct = NumberConfig::new(1.0).with_range(Some(0.0), Some(100.0));
        assert_eq!(pct.clamp(-12.0), 0.0);
        assert_eq!(pct.clamp(250.0), 100.0);
        assert_eq!(pct.clamp(37.5), 37.5);

        let nonneg = NumberConfig::new(1.0).with_range(Some(0.0), None);
        assert_eq!(nonneg.clamp(-8.0), 0.0);
        assert_eq!(nonneg.clamp(1000.0), 1000.0);

        let points = NumberConfig::new(1.0).with_range(Some(3.0), Some(60.0));
        assert_eq!(points.clamp(1.0), 3.0);
        assert_eq!(points.clamp(99.0), 60.0);

        let plain = NumberConfig::new(1.0);
        assert_eq!(plain.clamp(-999.0), -999.0);
        assert_eq!(plain.clamp(f64::INFINITY), f64::INFINITY);
    }

    /// 打字（含 `+ - * /` 表达式）最终也要落在范围内，而不是只在写回文档时夹紧。
    #[test]
    fn resolve_number_applies_the_range() {
        let cfg = NumberConfig::new(1.0).with_range(Some(0.0), Some(100.0));
        let s = |v: &str, kept: &str, number| resolve_number_with(number, false, kept, v);
        assert_eq!(s("250", "100", Some(cfg)), "100");
        assert_eq!(s("-150", "100", Some(cfg)), "0");
        // 上游 `field_math`：前导 `-` 是数值的负号（"-30" 就是 -30），相对编辑只认 `+ * /`。
        assert_eq!(s("-30", "100", Some(cfg)), "0");
        assert_eq!(s("+50", "80", Some(cfg)), "100");
        assert_eq!(s("*3", "80", Some(cfg)), "100");
        assert_eq!(s("/2", "80", Some(cfg)), "40");
        // 解析失败（除零、只有空格、中文/英文）提交时退回原值，不能把文字留在数字框里。
        assert_eq!(s("/0", "80", Some(cfg)), "80");
        assert_eq!(s("  ", "80", Some(cfg)), "80");
        assert_eq!(s("中文", "80", Some(cfg)), "80");
        assert_eq!(s("hello", "80", Some(cfg)), "80");
        assert_eq!(s("250", "100", None), "250");
        // A22 之外的字段通用行为：算术与百分比相对编辑。
        assert_eq!(s("(10+5)*2", "10", Some(cfg)), "30");
        assert_eq!(s("30px + 12", "30", Some(cfg)), "42");
        assert_eq!(s("50%", "80", Some(cfg)), "40");
        assert_eq!(s("+10%", "80", Some(cfg)), "88");
        assert_eq!(resolve_number_with(Some(cfg), true, "80", "50%"), "50");
    }

    /// 用户报的「数字框输入中文/英文，提交后文字不消失」：提交必须把草稿退回原值。
    #[gpui::test]
    async fn a_numeric_field_reverts_text_that_is_not_a_number_on_commit(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_window, cx| {
            let mut field = TextEdit::new(cx);
            field.set_value("80");
            field.set_numeric("", 1.0);
            field
        });

        view.update_in(window, |field, window, cx| {
            field.begin_edit(window, false);
            field.input.buffer.set_text("中文");
            field.commit(window, cx);
            assert_eq!(field.value(), "80", "the old number must survive");
            assert_eq!(field.input.buffer.text(), "80", "the text must disappear from the field");
        });
    }

    /// 评论 composer：多行模式下 Shift+Enter 必须真的在缓冲区里插入段落，而不是提交或忽略。
    #[gpui::test]
    async fn shift_enter_inserts_a_paragraph_break(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_window, cx| {
            let mut field = TextEdit::new(cx);
            field.set_value("first");
            field.set_multiline(true);
            field
        });

        view.update_in(window, |field, window, _cx| {
            field.begin_edit(window, false);
            window.focus(&field.focus);
        });
        window.simulate_keystrokes("shift-enter");

        view.update(window, |field, _| {
            assert_eq!(field.input.buffer.text(), "first\n", "Shift+Enter must insert a newline");
            assert!(field.is_multiline());
        });
    }

    /// 上下键按视觉段落走：从第二行末尾上行到第一行末尾，而不是整段文本的首尾。
    #[gpui::test]
    async fn up_moves_to_the_previous_visual_paragraph(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_window, cx| {
            let mut field = TextEdit::new(cx);
            field.set_value("ab\ncdef");
            field.set_multiline(true);
            field
        });
        window.simulate_resize(gpui::Size::new(px(300.0), px(90.0)));
        view.update_in(window, |field, window, _cx| {
            field.begin_edit(window, false);
            window.focus(&field.focus);
        });
        window.simulate_keystrokes("up");

        view.update(window, |field, _| {
            assert_eq!(field.input.buffer.range().end, 2, "the caret should land at the end of 'ab'");
        });
    }

    /// Shift+Up twice must keep extending from the active caret, not jump back to the selection
    /// end that the first step left behind.
    #[gpui::test]
    async fn repeated_shift_up_extends_the_selection(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_window, cx| {
            let mut field = TextEdit::new(cx);
            field.set_value("aa\naa\naa");
            field.set_multiline(true);
            field.input.buffer.move_to(8);
            field
        });
        window.simulate_resize(gpui::Size::new(px(300.0), px(90.0)));
        view.update_in(window, |field, window, _cx| {
            field.begin_edit(window, false);
            window.focus(&field.focus);
        });
        window.simulate_keystrokes("shift-up");
        view.update(window, |field, _| {
            assert_eq!(field.input.buffer.range(), 5..8, "the first Shift+Up reaches the previous paragraph");
            assert_eq!(field.input.buffer.caret(), 5, "the active caret moves even though the anchor stays at 8");
        });
        window.simulate_keystrokes("shift-up");
        view.update(window, |field, _| {
            assert_eq!(field.input.buffer.range(), 2..8, "the second Shift+Up must keep extending upward");
        });
    }

    /// Consecutive Up/Down presses keep the column they started from even when an intermediate
    /// paragraph is shorter.
    #[gpui::test]
    async fn consecutive_vertical_moves_keep_the_preferred_column(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_window, cx| {
            let mut field = TextEdit::new(cx);
            field.set_value("mmmmmm\nm\nmmmmmm");
            field.set_multiline(true);
            field.input.buffer.move_to(15);
            field
        });
        window.simulate_resize(gpui::Size::new(px(300.0), px(90.0)));
        view.update_in(window, |field, window, _cx| {
            field.begin_edit(window, false);
            window.focus(&field.focus);
        });
        window.simulate_keystrokes("up");
        view.update(window, |field, _| assert_eq!(field.input.buffer.caret(), 8, "the first Up clamps to the short row"));
        window.simulate_keystrokes("up");
        view.update(window, |field, _| assert_eq!(field.input.buffer.caret(), 6, "the second Up must return to the original column"));
    }

    /// At the outer rows, Up/Down still move to the document's start/end instead of doing nothing.
    #[gpui::test]
    async fn vertical_moves_reach_the_document_boundaries(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_window, cx| {
            let mut field = TextEdit::new(cx);
            field.set_value("a\nb");
            field.set_multiline(true);
            field
        });
        window.simulate_resize(gpui::Size::new(px(300.0), px(90.0)));
        view.update_in(window, |field, window, _cx| {
            field.begin_edit(window, false);
            field.input.buffer.move_to(1);
            window.focus(&field.focus);
        });
        window.simulate_keystrokes("up");
        view.update(window, |field, _| assert_eq!(field.input.buffer.caret(), 0, "Up on the first row reaches the document start"));
        window.simulate_keystrokes("down");
        window.simulate_keystrokes("down");
        view.update(window, |field, _| assert_eq!(field.input.buffer.caret(), 3, "Down on the last row reaches the document end"));
    }

    /// 固定高度的 composer 也要真的能看全内容：长行软换行，行数超过可视高度时跟随光标滚动。
    #[gpui::test]
    async fn long_lines_wrap_and_the_caret_scrolls_inside_the_box(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_window, cx| {
            let mut field = TextEdit::new(cx);
            field.set_value("one two three four five six seven eight nine ten\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine");
            field.set_multiline(true);
            field
        });
        window.simulate_resize(gpui::Size::new(px(140.0), px(72.0)));
        view.update_in(window, |field, window, _cx| {
            field.begin_edit(window, false);
            window.focus(&field.focus);
        });
        // Force a paint so the cache reflects this window's narrow, short composer.
        window.simulate_keystrokes("end");

        view.update(window, |field, _| {
            let (rows, scroll_y) = field.input.visual_rows_for_test();
            assert!(scroll_y > px(0.0), "the caret row must stay inside the 72 px composer");
            assert!(rows > 9, "the long paragraph must wrap into extra visual rows");
        });
    }
}
