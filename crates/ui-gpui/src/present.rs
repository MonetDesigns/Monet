//! Prototype 演示层 —— `ui-egui/src/present/{player,draw}.rs` 的 GPUI 移植（核心播放行为）。
//!
//! 引擎 `proto.present` / `proto.preview` 只发 `ui_requests`；这里由 `sync_ui_requests` 打开。
//! 画面用渲染层的 `figcraft_render::render_node_at(doc, frame, scale, time_ms)` 按时间求值，
//! 所以 **motion clip / 关键帧动画在演示里真的会动**（不只是静态截图）。
//! 点击画面执行当前 frame 的第一条交互（Navigate / ChangeTo / OpenOverlay / Back），
//! 过渡期间用 `figcraft_doc::anim::transitions::poses()` 摆出/淡入两张帧图。
//!
//! 与 egui 的差异：没有独立 Present 窗口；演示层覆盖窗口主体（标题栏与 PresentTab 保留）。
//! 布局 / 设备机身 / 顶栏 / 底部导航 / 缩放档 / 过渡位移都按 `ui-egui/src/present.rs` 对齐。

mod input;
mod layout;
mod scroll;

pub use input::{wants_key, Ev};
#[allow(unused_imports)]
pub use layout::{Geom, Place, RectPx};

use crate::app::CanvasApp;
use crate::canvas::render_worker::screen_image;
use crate::widgets::icon::paint as paint_icon;
use crate::widgets::{Icon, pt, rgba, tokens};
use figcraft_doc::proto::{
    Action, AnimationOp, Easing, OverlayAnchor, OverlayPosition, PrototypeSettings, Transition, Trigger,
};
use figcraft_doc::{Document, NodeId};
use figcraft_geom::Rect;
use figcraft_i18n::tr;
use gpui::{Div, ImageSource, MouseButton, SharedString, Stateful, div, img, prelude::*, px};
use std::collections::BTreeMap;
use std::time::Instant;

/// 画面缩放档（eGUI `PresentScale`）：Fit 缩到放得下、FitWidth 填满宽度、Fill 铺满、
/// Actual 100%。`Fit` 不超过 100%，其余也不超过 16 倍。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleMode {
    Fit,
    FitWidth,
    Fill,
    Actual,
}

impl ScaleMode {
    pub fn label(self) -> &'static str {
        match self {
            ScaleMode::Fit => tr("Fit"),
            ScaleMode::FitWidth => tr("Fit width"),
            ScaleMode::Fill => tr("Fill"),
            ScaleMode::Actual => tr("Actual size"),
        }
    }

    /// `Z` 轮换的下一个档位（eGUI `PresentScale::next`）。
    pub fn next(self) -> Self {
        match self {
            ScaleMode::Fit => ScaleMode::FitWidth,
            ScaleMode::FitWidth => ScaleMode::Fill,
            ScaleMode::Fill => ScaleMode::Actual,
            ScaleMode::Actual => ScaleMode::Fit,
        }
    }

    fn id(self) -> u8 {
        match self {
            ScaleMode::Fit => 0,
            ScaleMode::FitWidth => 1,
            ScaleMode::Fill => 2,
            ScaleMode::Actual => 3,
        }
    }

    fn from_id(id: u8) -> Self {
        match id {
            1 => ScaleMode::FitWidth,
            2 => ScaleMode::Fill,
            3 => ScaleMode::Actual,
            _ => ScaleMode::Fit,
        }
    }
}

/// 一个画框（动画组）挂起时记住的时钟。画框之间独立，各自持有播放头与是否在播。
/// current 画框的权威时钟直接落在 `PresentState.time_ms/playing`，这里只存 **非 current** 画框。
#[derive(Clone, Copy, Debug, PartialEq)]
struct MotionClock {
    playing: bool,
    base_ms: f64,
}

/// 对齐 desigma `animation_control` 的 op 语义，作用于一个画框的时钟。
/// `at_ms` 是该画框当前播放头，`dur_ms` 是它的总时长（>=1）。
///
/// * `Play{from_start:false}`：正在播且没到头 → 幂等不动；已到头 → 从头播；否则续播。
/// * `Play{from_start:true}` / `Restart`：从头播。
/// * `Pause`：停在播放头。
/// * `TogglePlay`：播着且没到头 → 暂停；否则播放（到头则从头）。
fn apply_animation_op(op: AnimationOp, cur: MotionClock, at_ms: f64, dur_ms: f64) -> MotionClock {
    let finished = at_ms >= dur_ms;
    let play = |base: f64| MotionClock { playing: true, base_ms: base.min(dur_ms) };
    match op {
        AnimationOp::Play { from_start: false } if cur.playing && !finished => cur,
        AnimationOp::Play { from_start: false } => play(if finished { 0.0 } else { at_ms }),
        AnimationOp::Play { from_start: true } | AnimationOp::Restart => play(0.0),
        AnimationOp::Pause => MotionClock { playing: false, base_ms: at_ms },
        AnimationOp::TogglePlay if cur.playing && !finished => MotionClock { playing: false, base_ms: at_ms },
        AnimationOp::TogglePlay => play(if finished { 0.0 } else { at_ms }),
    }
}

/// 编辑器标签栏里的演示标签（eGUI `present::chrome::PresentTab`）：每个演示独占一个 tab，
/// 切回设计文件时播放器存回 tab（`pres`），再点 tab 时取回 —— 保留播放头 / 历史 / 滚动。
#[derive(Clone, Debug)]
pub struct PresentTab {
    pub id: u64,
    pub uid: u64,
    pub path: Option<String>,
    pub page: NodeId,
    pub file: String,
    pub page_name: String,
    /// 不在屏时存放的播放器（eGUI `PresentTab.pres`）。
    pub pres: Option<PresentState>,
}

impl PresentTab {
    /// `file - page`，页面无名时只用文件名（eGUI `present_tab_name`）。
    pub fn name(&self) -> String {
        if self.page_name.is_empty() { self.file.clone() } else { format!("{} - {}", self.file, self.page_name) }
    }
}

/// 标签宽度：`clamp(140, 260)`（eGUI `present_tab_width` 的 GPUI 近似，12pt 字）。
pub fn present_tab_width(name: &str) -> f32 {
    (12.0 * 0.6 * name.chars().count() as f32 + 64.0).clamp(140.0, 260.0)
}

/// 可滚动对象：某画框的内部滚动，或整窗平移。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroller {
    Frame(NodeId),
    View,
}

/// 历史里的一步（eGUI `Visit`）：Back 用它反向播放来时的过渡。
#[derive(Clone, Copy, Debug)]
pub struct Visit {
    pub frame: NodeId,
    pub transition: Transition,
    pub dur: f64,
    pub easing: Easing,
}

/// 一个打开着的覆盖层（eGUI `OverlayState`）。
#[derive(Clone, Debug)]
pub struct OverlayState {
    pub id: NodeId,
    pub position: OverlayPosition,
    pub close_on_outside: bool,
    pub background: Option<figcraft_color::Color>,
    /// 触发层的页矩形（用于 `anchor_to` 的相对偏移）。
    pub anchor: Option<figcraft_geom::Rect>,
    pub offset: Option<[f64; 2]>,
    pub anchor_to: OverlayAnchor,
    pub opened: f64,
    pub transition: Transition,
    pub dur: f64,
    pub easing: Easing,
}

/// 惯性滑行。
#[derive(Clone, Copy, Debug)]
pub struct Fling {
    pub target: Scroller,
    pub vel: (f64, f64),
    pub decay: f64,
    pub bounce: bool,
    pub at: f64,
}

/// Scroll to 的平滑动画（eGUI `ScrollAnim`）。
#[derive(Clone, Debug)]
struct ScrollAnim {
    frames: Vec<(NodeId, (f64, f64), (f64, f64))>,
    start: f64,
    dur: f64,
    easing: Easing,
}

/// OnHover / OnPress 的临时状态：离开 / 松开时回退到按下前。
#[derive(Clone, Debug)]
struct Temp {
    rect: RectPx,
    press: bool,
    snap: Snapshot,
    transition: Transition,
    dur: f64,
    easing: Easing,
}

#[derive(Clone, Debug)]
struct Snapshot {
    current: NodeId,
    design: Option<Document>,
    overlays: Vec<OverlayState>,
    history: Vec<NodeId>,
    visits: Vec<Visit>,
    scroll: BTreeMap<NodeId, (f64, f64)>,
}

/// 一次演示的运行时状态。
#[derive(Clone, Debug)]
pub struct PresentState {
    pub start: NodeId,
    pub current: NodeId,
    pub history: Vec<NodeId>,
    visits: Vec<Visit>,
    /// 动画播放头（ms）；传给 `render_node_at` 让 motion clip 按时间求值。
    pub time_ms: f64,
    pub playing: bool,
    pub last: Instant,
    /// 导航过渡：上一帧、开始时刻、时长、类型。
    pub from: Option<NodeId>,
    pub transition_start: Instant,
    pub transition_ms: u32,
    pub transition: Transition,
    /// 过渡缓动（`poses` 要吃完缓动后的进度）。
    pub transition_easing: Easing,
    /// hotspot 提示（点空白处闪一下可点区域）——顶栏开关。
    pub hints: bool,
    pub preview: bool,
    /// 演示外壳（eGUI `PresentView`）：评论 / Inspect / 隐藏 UI。
    pub comments: bool,
    pub inspect: bool,
    pub hide_ui: bool,
    /// 画面缩放档（导航栏右侧的下拉）。
    pub scale: ScaleMode,
    /// 画不画设备机身（eGUI `PresentView.device_frame`）。
    pub device_frame: bool,
    /// 导航栏上的设备 / 缩放菜单是否展开。
    pub device_open: bool,
    pub scale_open: bool,
    /// Inspect 的目标层（点击命中的层，eGUI `presenter.inspected`）。
    pub inspected: Option<NodeId>,
    /// 待打开的外链（`Action::OpenUrl`）；由 lib 层取走并交给 GPUI。
    pub pending_url: Option<String>,
    /// 非 current 画框各自的挂起时钟；导航离开 / 进入时存取，落实「画框之间独立」。
    clocks: BTreeMap<NodeId, MotionClock>,
    /// 演示用的文档**快照**：源设计稿关掉之后预览标签仍然能打开 / 继续播
    /// （用户 2026-11-17）。源文档还在时每次切进预览都会刷新这份快照。
    pub doc: Option<std::sync::Arc<Document>>,

    // ---- eGUI `Presenter` 的运行时状态 ----
    source_ptr: usize,
    original: Option<Document>,
    design: Option<Document>,
    /// 排版后的设计（instances 播放变体交互、固定层置顶）。
    laid: Option<Document>,
    /// 显示文档（渲染 / 命中用），按 `rev` 缓存。
    disp: Option<Document>,
    disp_rev: u64,
    rev: u64,
    /// 响应式原型要把被演示画框改成的大小（eGUI `Presenter::fit`）。
    fit: Option<(f64, f64)>,
    /// 最近一次摆放的 (根, 摆放, 可视区)：把屏幕矩形换回页矩形用（eGUI `Presenter::shown`）。
    shown: Option<(NodeId, Place, RectPx)>,
    scroll_anim: Option<ScrollAnim>,
    /// 时钟（秒）：定时器 / 覆盖层动画 / 闪烁都以它为准。
    pub clock: f64,
    /// 演示开始的单调时钟（`clock` 的来源）。
    epoch: Instant,
    pub scroll: BTreeMap<NodeId, (f64, f64)>,
    pub pan: Option<(f64, f64)>,
    pub pan_range: (f64, f64),
    pub overlays: Vec<OverlayState>,
    closing: Vec<(f64, OverlayState)>,
    hovered: Vec<NodeId>,
    press: Option<(f64, f64)>,
    dragged: bool,
    drag_scroll: Option<(Scroller, (f64, f64))>,
    fling: Option<Fling>,
    wheel: Option<(Scroller, (f64, f64))>,
    temp: Option<Temp>,
    follow: Option<((f64, f64), (f64, f64), f64, f64)>,
    /// 屏幕坐标的 hotspot 矩形（供闪烁 / Inspect）。
    pub hot: Vec<(NodeId, RectPx)>,
    flash_until: f64,
    pub arrived: f64,
    delays_fired: Vec<(NodeId, usize)>,
    waiting: Vec<input::Waiting>,
    sections: std::collections::HashMap<NodeId, NodeId>,
    pub pointer: Option<(f64, f64)>,
    inside: bool,
    down: bool,
    cursor: u8,
    last_move_at: Option<Instant>,
    last_input: f64,
    /// 本帧点到的 widget 可点部分（widget, target），帧后交给插件回调（eGUI `widget_clicks`）。
    widget_clicks: Vec<(NodeId, String)>,
}

impl PresentState {
    pub fn new(start: NodeId, preview: bool) -> Self {
        Self {
            start,
            current: start,
            history: Vec::new(),
            visits: Vec::new(),
            time_ms: 0.0,
            playing: false,
            last: Instant::now(),
            from: None,
            transition_start: Instant::now(),
            transition_ms: 0,
            transition: Transition::Instant,
            transition_easing: Easing::Linear,
            hints: true,
            preview,
            comments: false,
            inspect: false,
            hide_ui: false,
            scale: ScaleMode::Fit,
            device_frame: true,
            device_open: false,
            scale_open: false,
            inspected: None,
            pending_url: None,
            clocks: BTreeMap::new(),
            doc: None,
            source_ptr: 0,
            original: None,
            design: None,
            laid: None,
            disp: None,
            disp_rev: u64::MAX,
            rev: 0,
            fit: None,
            shown: None,
            scroll_anim: None,
            clock: 0.0,
            epoch: Instant::now(),
            scroll: BTreeMap::new(),
            pan: None,
            pan_range: (0.0, 0.0),
            overlays: Vec::new(),
            closing: Vec::new(),
            hovered: Vec::new(),
            press: None,
            dragged: false,
            drag_scroll: None,
            fling: None,
            wheel: None,
            temp: None,
            follow: None,
            hot: Vec::new(),
            flash_until: 0.0,
            arrived: 0.0,
            delays_fired: Vec::new(),
            waiting: Vec::new(),
            sections: std::collections::HashMap::new(),
            pointer: None,
            inside: false,
            down: false,
            cursor: 0,
            last_move_at: None,
            last_input: 0.0,
            widget_clicks: Vec::new(),
        }
    }

    /// 单调时钟（秒），定时器 / 覆盖层动画的时间基准。
    pub fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// 推进播放头 / 过渡时钟。返回是否需要继续请求动画帧。
    pub fn advance(&mut self, doc: &Document) -> bool {
        self.ensure_loaded(doc);
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64() * 1000.0;
        self.last = now;
        if self.playing {
            self.time_ms += dt;
            let base = self.laid.as_ref().unwrap_or(doc);
            // Per-frame animation group (用户 2026-11-12)：the current frame owns the timeline;
            // a frame with no entry of its own inherits its page's (settings_of does the fallback).
            let st = crate::motion::settings_of(base, self.current);
            let dur = st.duration_ms.max(1) as f64;
            if self.time_ms > dur {
                if st.looping {
                    self.time_ms %= dur;
                } else {
                    self.time_ms = dur;
                    self.playing = false;
                }
            }
        }
        let in_transition = self.from.is_some() && now.duration_since(self.transition_start).as_secs_f64() * 1000.0 < self.transition_ms as f64;
        if !in_transition {
            self.from = None;
        }
        self.playing || in_transition
    }

    /// 当前活动层：最上层覆盖层，否则被演示的画框。
    fn active_root(&self) -> NodeId {
        self.overlays.last().map_or(self.current, |o| o.id)
    }

    /// 每帧的运行时推进：时钟、过渡、惯性、等待中的鼠标触发器、覆盖层关闭动画。
    pub fn tick(&mut self, now: f64) -> bool {
        self.clock = now;
        self.step_scroll_anim(now);
        if self.laid.is_some() {
            self.step_fling(now);
            // 到期的延迟鼠标触发器：只认当前活动层（覆盖层关掉后，它的等待项作废）。
            if self.from.is_none() {
                let root = self.active_root();
                let mut due = Vec::new();
                let mut keep = Vec::new();
                for w in std::mem::take(&mut self.waiting) {
                    if w.root != root {
                        continue;
                    }
                    if w.due <= now {
                        due.push(w);
                    } else {
                        keep.push(w);
                    }
                }
                self.waiting = keep;
                for w in due {
                    self.fire_from(w.node, w.k, w.rect, now, false);
                }
            }
            self.closing.retain(|(at, o)| now - *at >= o.dur);
        }
        self.refresh_display();
        self.animating()
    }

    /// 推进 `scroll_anim`（eGUI `player()` 里同一段）。
    fn step_scroll_anim(&mut self, now: f64) {
        let Some(a) = self.scroll_anim.clone() else { return };
        let raw = ((now - a.start) / a.dur.max(1e-3)).clamp(0.0, 1.0);
        let e = a.easing.eval(raw);
        for (f, from, to) in &a.frames {
            self.scroll.insert(*f, (from.0 + (to.0 - from.0) * e, from.1 + (to.1 - from.1) * e));
        }
        self.rev += 1;
        if raw >= 1.0 {
            self.scroll_anim = None;
        }
    }

    /// 是否还需要下一帧（动画 / 过渡 / 惯性 / 覆盖层进出 / 定时器）。
    pub fn animating(&self) -> bool {
        self.playing
            || self.from.is_some()
            || self.fling.is_some()
            || self.drag_scroll.is_some()
            || self.scroll_anim.is_some()
            || !self.waiting.is_empty()
            || !self.closing.is_empty()
            || self.overlays.iter().any(|o| self.clock - o.opened < o.dur)
            // 热区提示淡出（eGUI 在这一段里每帧 request_repaint）。
            || self.clock < self.flash_until
            || self.laid.as_ref().is_some_and(|d| input::next_delay(self, d, self.active_root(), self.clock).is_some())
    }

    /// 跟随编辑器文档：首次或内容变化时拷贝；屏幕 / 覆盖层 / 滚动位置尽量保留
    /// （屏幕被删掉就回到起点）。运行时状态（变量、变体）从这次编辑重新开始。
    pub fn sync_source(&mut self, doc: &std::sync::Arc<Document>, now: f64) {
        let ptr = std::sync::Arc::as_ptr(doc) as usize;
        if self.source_ptr == ptr {
            return;
        }
        let first = self.source_ptr == 0;
        self.source_ptr = ptr;
        self.doc = Some(doc.clone());
        let copy = Document::clone(doc);
        self.original = Some(copy.clone());
        self.design = Some(copy);
        self.relayout();
        if first {
            self.arrived = now;
            return;
        }
        if !doc.contains(self.current) {
            if doc.contains(self.start) {
                self.restart(self.start, now);
            }
            return;
        }
        self.history.retain(|f| doc.contains(*f));
        self.visits.retain(|v| doc.contains(v.frame));
        self.overlays.retain(|o| doc.contains(o.id));
        self.closing.retain(|(_, o)| doc.contains(o.id));
        self.temp = None;
        self.scroll_anim = None;
        self.stop_scrolling();
        self.scroll.retain(|f, _| doc.contains(*f));
        if let Some(laid) = self.laid.as_ref() {
            let clamped: Vec<(NodeId, (f64, f64))> =
                self.scroll.iter().map(|(f, s)| (*f, scroll::clamp_scroll(*s, self.range(laid, *f)))).collect();
            self.scroll.extend(clamped);
        }
        self.rev += 1;
    }

    /// 排版：`design` →（响应式时按屏幕改尺寸）→ instances 播放变体交互 → 固定层置顶。
    fn relayout(&mut self) {
        let mut d = self.design.clone();
        if let Some(d) = d.as_mut() {
            // 响应式：被演示画框先按设备屏幕（无设备按整块可用区）改大小，约束与自动布局再回流。
            if let Some((w, h)) = self.fit {
                figcraft_engine::cmd::proto::respond(d, self.current, (w, h));
            }
            figcraft_engine::proto_run::play_interactions(d);
            scroll::raise_fixed(d);
        }
        self.laid = d;
        self.rev += 1;
    }

    /// 本帧点到的 widget 回调插件（eGUI `click_widgets`），返回给用户看的错误。
    pub fn click_widgets(&mut self, s: &figcraft_engine::Session) -> Option<String> {
        let mut error = None;
        for (id, target) in std::mem::take(&mut self.widget_clicks) {
            let Some(doc) = self.design.clone() else { break };
            match figcraft_engine::widgets::click_on(s, doc, id, &target) {
                Ok((d, _)) => {
                    self.design = Some(d);
                    self.relayout();
                }
                Err(e) => error = Some(e.to_string()),
            }
        }
        error
    }

    /// 响应式原型的目标尺寸变化时立即重排（eGUI `Presenter::respond_to`，在 layout 里调用）。
    pub(crate) fn respond_to(&mut self, fit: Option<(f64, f64)>) {
        if self.fit == fit {
            return;
        }
        self.fit = fit;
        self.relayout();
    }

    /// 动作改了 `design` 之后：文本重排、实例跟随、自动布局回流，再排版。
    fn settle(&mut self, before: &Document) {
        if let Some(d) = self.design.as_mut() {
            figcraft_engine::normalize(d, before);
        }
        self.relayout();
    }

    /// 应用滚动与设备视口后的文档（缓存到 `rev`）。
    fn refresh_display(&mut self) {
        if self.disp_rev == self.rev && self.disp.is_some() {
            return;
        }
        let Some(doc) = self.laid.as_ref() else {
            self.disp = None;
            return;
        };
        let mut d = scroll::apply_scroll(doc, &self.scroll);
        if let Some(vp) = scroll::viewport(doc, self.current) {
            scroll::fit_viewport(&mut d, self.current, vp);
        }
        self.disp = Some(d);
        self.disp_rev = self.rev;
    }

    /// 显示文档（渲染 / 命中用）。
    pub fn display(&self) -> Option<&Document> {
        self.disp.as_ref().or(self.laid.as_ref())
    }

    /// 屏幕矩形 → 页坐标矩形（eGUI `Presenter::page_rect`）。只有触发层属于当前根时才有意义。
    fn page_rect(&self, r: Option<RectPx>) -> Option<Rect> {
        let (root, place, _) = self.shown?;
        if root != self.current {
            return None;
        }
        let r = r?;
        let (x0, y0) = place.page((f64::from(r.x), f64::from(r.y)));
        let (x1, y1) = place.page((f64::from(r.x + r.w), f64::from(r.y + r.h)));
        Some(Rect::new(x0, y0, x1, y1))
    }

    fn ensure_loaded(&mut self, doc: &Document) {
        if self.design.is_none() {
            self.original = Some(doc.clone());
            self.design = Some(doc.clone());
            self.relayout();
        }
    }

    pub(crate) fn stop_scrolling(&mut self) {
        self.fling = None;
        self.wheel = None;
        self.drag_scroll = None;
    }

    pub fn cursor(&self) -> u8 {
        self.cursor
    }

    /// 点击兜底：没有命中任何子图层 hotspot 时，执行当前画框自身的第一条交互。
    #[allow(dead_code)]
    pub fn click(&mut self, doc: &Document) {
        self.ensure_loaded(doc);
        let base = self.laid.as_ref().unwrap_or(doc);
        if base.node(self.current).is_some_and(|n| !n.interactions.is_empty()) {
            let now = self.clock;
            self.fire_node(self.current, 0, now);
        }
    }

    /// 命中了子图层后执行 `node` 第 `k` 条交互。
    #[allow(dead_code)]
    pub fn fire(&mut self, doc: &Document, node: NodeId, k: usize) {
        self.ensure_loaded(doc);
        let now = self.clock;
        self.fire_node(node, k, now);
    }

    fn fire_node(&mut self, node: NodeId, k: usize, now: f64) -> bool {
        self.fire_from(node, k, None, now, true)
    }

    fn fire_from(&mut self, node: NodeId, k: usize, rect: Option<RectPx>, now: f64, live: bool) -> bool {
        let _ = live;
        let Some(it) = self
            .laid
            .as_ref()
            .or(self.design.as_ref())
            .and_then(|d| d.node(node))
            .and_then(|n| n.interactions.get(k))
            .cloned()
        else {
            return false;
        };
        let dur = it.easing.seconds(it.duration_ms);
        let temp = match it.trigger {
            Trigger::OnHover => Some(false),
            Trigger::OnPress => Some(true),
            _ => None,
        };
        let snap = if temp.is_some() && self.temp.is_none() { self.snapshot() } else { None };
        let mut did = false;
        for a in &it.actions {
            did |= self.perform(a, node, it.transition, dur, it.easing, rect, now, 0);
        }
        if did && let (Some(press), Some(snap)) = (temp, snap) {
            let rect = rect
                .or_else(|| self.hot.iter().find(|(id, _)| *id == node).map(|(_, r)| *r))
                .unwrap_or(RectPx { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
            self.temp = Some(Temp { rect, press, snap, transition: it.transition, dur, easing: it.easing });
        }
        did
    }

    /// 鼠标 enter / leave / down / up 命中：有 delay 就排队。
    fn gesture(&mut self, node: NodeId, k: usize, rect: Option<RectPx>, now: f64, root: NodeId) -> bool {
        let delay = self
            .laid
            .as_ref()
            .or(self.design.as_ref())
            .and_then(|d| d.node(node))
            .and_then(|n| n.interactions.get(k))
            .map_or(0.0, |it| it.delay_secs());
        if delay <= 0.0 {
            return self.fire_from(node, k, rect, now, true);
        }
        if !self.waiting.iter().any(|w| (w.node, w.k) == (node, k)) {
            self.waiting.push(input::Waiting { node, k, rect, root, due: now + delay });
        }
        false
    }

    fn snapshot(&self) -> Option<Snapshot> {
        let design = self.design.clone()?;
        Some(Snapshot {
            current: self.current,
            design: Some(design),
            overlays: self.overlays.clone(),
            history: self.history.clone(),
            visits: self.visits.clone(),
            scroll: self.scroll.clone(),
        })
    }

    fn revert(&mut self, now: f64) {
        let Some(t) = self.temp.take() else { return };
        self.begin(t.transition.reversed(), t.dur, t.easing, now);
        self.current = t.snap.current;
        self.design = t.snap.design;
        self.relayout();
        self.overlays = t.snap.overlays;
        self.history = t.snap.history;
        self.visits = t.snap.visits;
        self.scroll = t.snap.scroll;
        self.scroll_anim = None;
        self.follow = None;
        self.stop_scrolling();
        self.hovered.clear();
        self.rev += 1;
    }

    /// 开始一段过渡（在改状态之前调用）。
    fn begin(&mut self, transition: Transition, dur: f64, easing: Easing, _now: f64) {
        let instant = matches!(transition, Transition::Instant) || dur <= 0.0;
        self.from = (!instant).then_some(self.current);
        self.transition = transition;
        self.transition_easing = easing;
        self.transition_ms = (dur * 1000.0).round() as u32;
        self.transition_start = Instant::now();
    }

    fn arrive(&mut self, now: f64) {
        if let Some(doc) = self.laid.as_ref() {
            figcraft_engine::proto_run::remember_sections(doc, self.current, &mut self.sections);
        }
        self.arrived = now;
        self.delays_fired.clear();
        self.waiting.clear();
        self.hovered.clear();
        self.rev += 1;
    }

    /// 显示 `frame`；`preserve` 保留离开画框的滚动位置（夹到目标的可滚范围）。
    fn go_to(&mut self, frame: NodeId, tr: Transition, dur: f64, easing: Easing, now: f64, remember: bool, preserve: bool) {
        self.begin(tr, dur, easing, now);
        if remember {
            self.history.push(self.current);
            self.visits.push(Visit { frame: self.current, transition: tr, dur, easing });
        }
        self.clocks.insert(self.current, MotionClock { playing: self.playing, base_ms: self.time_ms });
        let kept = self.scroll.get(&self.current).copied().filter(|_| preserve);
        self.overlays.clear();
        self.scroll.clear();
        self.scroll_anim = None;
        self.follow = None;
        self.stop_scrolling();
        self.pan = None;
        self.temp = None;
        self.current = frame;
        self.relayout();
        if let Some(s) = kept
            && let Some(doc) = self.laid.as_ref()
        {
            let to = scroll::clamp_scroll(s, self.range(doc, frame));
            self.scroll.insert(frame, to);
        }
        let c = self.clocks.get(&frame).copied().unwrap_or(MotionClock { playing: false, base_ms: 0.0 });
        self.time_ms = c.base_ms;
        self.playing = c.playing;
        self.arrive(now);
    }

    /// 导航条 / 方向键切到相邻帧（不入历史、不保留滚动 —— eGUI `chrome::navigation_bar`）。
    pub fn go(&mut self, next: NodeId) {
        if next == self.current {
            return;
        }
        let now = self.clock;
        self.go_to(next, Transition::Instant, 0.0, Easing::Linear, now, true, false);
    }

    pub fn back(&mut self) {
        let now = self.clock;
        if self.close_overlay(now) {
            return;
        }
        let Some(v) = self.visits.pop() else { return };
        self.history.pop();
        self.go_to(v.frame, v.transition.reversed(), v.dur, v.easing, now, false, false);
    }

    /// 从头开始（顶栏 Restart）：回到编辑器里的文档。
    pub fn restart(&mut self, start: NodeId, now: f64) {
        self.start = start;
        self.current = start;
        self.sections.clear();
        self.history.clear();
        self.visits.clear();
        self.overlays.clear();
        self.closing.clear();
        self.scroll.clear();
        self.scroll_anim = None;
        self.follow = None;
        self.stop_scrolling();
        self.pan = None;
        self.from = None;
        self.temp = None;
        self.design = self.original.clone();
        self.relayout();
        self.arrive(now);
    }

    fn close_overlay(&mut self, now: f64) -> bool {
        let Some(o) = self.overlays.pop() else { return false };
        if !matches!(o.transition, Transition::Instant) && o.dur > 0.0 {
            let shown = ((now - o.opened) / o.dur).clamp(0.0, 1.0);
            self.closing.push((now - (1.0 - shown) * o.dur, o));
        }
        self.hovered.clear();
        true
    }

    fn cycle_scale(&mut self) {
        self.scale = self.scale.next();
        self.scale_open = false;
        // eGUI `set_scale`：换档后回到顶部、停掉滚动。
        self.pan = None;
        self.stop_scrolling();
    }

    /// 换设备 / 缩放档后清掉旧视口的滚动与拖拽（eGUI `reset_for_device`）。
    pub fn reset_for_device(&mut self) {
        self.pan = None;
        self.scroll.clear();
        self.scroll_anim = None;
        self.press = None;
        self.dragged = false;
        self.stop_scrolling();
        self.rev += 1;
    }

    #[allow(clippy::too_many_arguments)]
    fn perform(&mut self, a: &Action, node: NodeId, tr: Transition, dur: f64, easing: Easing, rect: Option<RectPx>, now: f64, depth: usize) -> bool {
        let Some(doc) = self.laid.clone() else { return false };
        match a {
            Action::Navigate { dest, preserve_scroll } => {
                let Some(top) = figcraft_engine::proto_run::navigate_target(&doc, *dest, &self.sections) else { return false };
                if !doc.contains(top) || (top == self.current && self.overlays.is_empty()) {
                    return false;
                }
                self.go_to(top, tr, dur, easing, now, true, *preserve_scroll);
                true
            }
            Action::Back => {
                if self.overlays.is_empty() && self.visits.is_empty() {
                    return false;
                }
                self.back();
                true
            }
            Action::ScrollTo { dest, offset } => {
                let frames = self.scroll_to_layer(&doc, *dest, *offset);
                if frames.is_empty() {
                    return false;
                }
                if matches!(tr, Transition::Instant) || dur <= 0.0 {
                    self.scroll.extend(frames.iter().map(|(f, _, to)| (*f, *to)));
                    self.rev += 1;
                } else {
                    self.scroll_anim = Some(ScrollAnim { frames, start: now, dur, easing });
                }
                true
            }
            Action::OpenOverlay { dest, position, close_on_outside, background, offset, anchor } => {
                if !doc.contains(*dest) {
                    return false;
                }
                let at = self.page_rect(rect);
                self.delays_fired.retain(|(id, _)| !doc.is_ancestor(*dest, *id));
                self.overlays.push(OverlayState {
                    id: *dest,
                    position: *position,
                    close_on_outside: *close_on_outside,
                    background: *background,
                    anchor: at,
                    offset: *offset,
                    anchor_to: *anchor,
                    opened: now,
                    transition: tr,
                    dur,
                    easing,
                });
                self.hovered.clear();
                true
            }
            Action::SwapOverlay { dest } => {
                if !doc.contains(*dest) {
                    return false;
                }
                let at = self.page_rect(rect);
                self.delays_fired.retain(|(id, _)| !doc.is_ancestor(*dest, *id));
                match self.overlays.last_mut() {
                    Some(o) => {
                        o.id = *dest;
                        o.opened = now;
                        o.transition = tr;
                        o.dur = dur;
                        o.easing = easing;
                        o.anchor = at;
                    }
                    None => self.overlays.push(OverlayState {
                        id: *dest,
                        position: OverlayPosition::Center,
                        close_on_outside: true,
                        background: None,
                        anchor: at,
                        offset: None,
                        anchor_to: OverlayAnchor::Screen,
                        opened: now,
                        transition: tr,
                        dur,
                        easing,
                    }),
                }
                self.hovered.clear();
                true
            }
            Action::CloseOverlay => self.close_overlay(now),
            Action::OpenUrl { url, .. } => {
                // eGUI 会先弹「打开链接」确认条；GPUI 侧先直接交给系统浏览器。
                if url.trim().is_empty() {
                    return false;
                }
                self.pending_url = Some(url.clone());
                true
            }
            Action::ChangeTo { dest } => {
                let Some(inst) = figcraft_engine::proto_run::change_to_target(&doc, node, *dest) else { return false };
                eprintln!("[dbg changeTo] node={node} inst={inst} dest={dest}");
                if !doc.contains(*dest) {
                    return false;
                }
                self.begin(tr, dur, easing, now);
                let Some(d) = self.design.as_mut() else { return false };
                let before = d.clone();
                dbg_boxes("pre", &before);
                dbg_tree("pre", &before, NodeId(103));
                if let Some(n) = d.node_mut(inst)
                    && let figcraft_doc::NodeKind::Instance { component, .. } = &mut n.kind
                {
                    *component = *dest;
                }
                figcraft_engine::components::sync_instance(d, inst);
                dbg_boxes("synced", d);
                self.settle(&before);
                if let Some(d) = self.design.as_ref() {
                    dbg_boxes("post", d);
                    dbg_tree("post", d, NodeId(103));
                }
                self.rev += 1;
                true
            }
            Action::SetVariable { variable, value, expr } => {
                let Some(d) = self.design.as_mut() else { return false };
                let before = d.clone();
                let set = figcraft_engine::proto_run::set_variable(d, node, variable, value, expr.as_ref());
                if set {
                    self.settle(&before);
                }
                set
            }
            Action::SetVariableMode { collection, mode } => {
                let Some(d) = self.design.as_mut() else { return false };
                let before = d.clone();
                let set = figcraft_engine::proto_run::set_mode_by_id(d, self.current, collection, mode);
                if set {
                    self.settle(&before);
                }
                set
            }
            Action::Conditional { blocks } => {
                if depth >= figcraft_doc::proto::MAX_ACTION_DEPTH {
                    return false;
                }
                let mut did = false;
                for inner in figcraft_engine::proto_run::branch(&doc, node, blocks) {
                    did |= self.perform(&inner, node, tr, dur, easing, rect, now, depth + 1);
                }
                did
            }
            Action::Animation { dest, op } => {
                let scope = doc.top_level_frame(*dest).unwrap_or(*dest);
                self.control_animation(&doc, scope, *op);
                true
            }
        }
    }

    /// 把 `op` 作用于 `dest` 所在的画框（动画组）。控制当前画框直接改 time_ms/playing；
    /// 控制其它画框只更新它的挂起时钟，等导航过去时再恢复。
    fn control_animation(&mut self, doc: &Document, scope: NodeId, op: AnimationOp) {
        let dur = crate::motion::settings_of(doc, scope).duration_ms.max(1) as f64;
        if scope == self.current {
            let cur = MotionClock { playing: self.playing, base_ms: self.time_ms };
            let next = apply_animation_op(op, cur, self.time_ms, dur);
            self.playing = next.playing;
            self.time_ms = next.base_ms;
        } else {
            let cur = self.clocks.get(&scope).copied().unwrap_or(MotionClock { playing: false, base_ms: 0.0 });
            let next = apply_animation_op(op, cur, cur.base_ms, dur);
            self.clocks.insert(scope, next);
        }
    }
}

// ---- 布局常量（eGUI Chrome）----

/// 顶栏高度（px，eGUI `Chrome::FULL.bar_h`）。
const BAR_H: f32 = 40.0;

/// 预览窗格顶栏高度（px，eGUI `Chrome::PANE.bar_h`）。
const PANE_BAR_H: f32 = 36.0;

/// 画面四周留白（eGUI `Chrome::FULL.pad`）。
const PAD: f32 = 32.0;

/// 预览窗格留白（eGUI `Chrome::PANE.pad`）。
const PANE_PAD: f32 = 12.0;

/// 预览窗格默认尺寸 / 位置（eGUI `Window::default_size 320x600`，右上角、canvas 内缩 16）。
const PANE_W: f32 = 320.0;
const PANE_H: f32 = 600.0;
const PANE_MARGIN: f32 = 16.0;

/// Inspect 面板宽度（eGUI `INSPECT_W`）。
const INSPECT_W: f32 = 280.0;

/// hotspot 闪烁时长（秒）。
const FLASH_SECS: f64 = 0.5;

fn page_settings(doc: &Document, id: NodeId) -> PrototypeSettings {
    scroll::page_settings(doc, id)
}

/// 渲染一个 frame 在当前播放头时刻的位图。
///
/// 一帧画面的位图缓存项。`key` = (文档 rev, 像素缩放, GIF 帧, 关键帧时间)。
#[derive(Clone)]
struct CachedTex {
    key: (u64, u64, u64, u64),
    img: std::sync::Arc<gpui::RenderImage>,
    bounds: figcraft_geom::Rect,
}

thread_local! {
    /// 每窗口每画框的位图缓存。演示层每帧都会调用 `render()`，没有缓存时每帧都要
    /// 重新光栅化整张画框（又慢又会让边缘在重采样里抖动）。
    static TEXTURES: std::cell::RefCell<std::collections::HashMap<(u64, NodeId), CachedTex>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// 位图像素缩放：最多到 `px`，并受 GPU 纹理边长 / 总像素上限约束（eGUI `texture_scale`）。
fn texture_scale(bounds: figcraft_geom::Rect, px: f64) -> f64 {
    const MAX_PIXELS: f64 = 16_777_216.0;
    const MAX_SIDE: f64 = 8190.0;
    let (w, h) = (bounds.width().max(1.0), bounds.height().max(1.0));
    let s = px.min(MAX_SIDE / w.max(h)).min((MAX_PIXELS / (w * h)).sqrt());
    if s.is_finite() && s > 0.0 { s } else { 1e-3 }
}

/// 取一张画框的位图（带缓存）。`px_scale` 已是设备像素比（逻辑缩放 × ppp）。
fn frame_tex(state: &PresentState, doc: &Document, id: NodeId, px_scale: f64) -> Option<CachedTex> {
    let clock = figcraft_render::images::animation_clock(doc, id, (state.clock - state.arrived).max(0.0) + 0.005);
    let key = (
        state.rev,
        px_scale.to_bits(),
        clock.map_or(0, |c| c.0),
        if state.playing { state.time_ms.to_bits() } else { 0 },
    );
    let wid = crate::windows::current();
    if let Some(c) = TEXTURES.with(|q| q.borrow().get(&(wid, id)).filter(|c| c.key == key).cloned()) {
        return Some(c);
    }
    let bounds = figcraft_render::export_bounds(doc, id)?;
    let evaluated = figcraft_doc::anim::motion::evaluate(doc, doc.top_level_frame(id).unwrap_or(id), state.time_ms);
    let px = texture_scale(bounds, px_scale);
    let pix = figcraft_render::render_node_at(&evaluated, id, px, state.time_ms)?;
    let tex = CachedTex { key, img: screen_image(&pix, doc.color_profile), bounds };
    TEXTURES.with(|q| {
        let mut q = q.borrow_mut();
        if q.len() > 24 {
            q.retain(|(w, _), _| *w != wid);
        }
        q.insert((wid, id), tex.clone());
    });
    Some(tex)
}

/// 一帧画面：按 `place` 的页→屏幕映射绝对定位，位图按设备像素光栅化、按逻辑尺寸显示。
/// `origin` 是父容器在窗口里的左上角（屏幕层或舞台）。
#[allow(clippy::too_many_arguments)]
fn frame_at(
    state: &PresentState,
    doc: &Document,
    id: NodeId,
    place: &Place,
    origin: RectPx,
    ppp: f32,
    alpha: f32,
    offset: (f32, f32),
) -> Option<gpui::AnyElement> {
    let tex = frame_tex(state, doc, id, place.scale * f64::from(ppp.max(0.25)))?;
    let r = place.rect(tex.bounds);
    // 位图尺寸 = 纹理像素数 / ppp，位置也对齐到设备像素，避免缩放采样把边缘糊掉。
    let ppp64 = f64::from(ppp.max(0.25));
    let w = ((tex.bounds.width() * place.scale * ppp64).round().max(1.0) / ppp64) as f32;
    let h = ((tex.bounds.height() * place.scale * ppp64).round().max(1.0) / ppp64) as f32;
    let snap = |v: f32| (v * ppp).round() / ppp;
    Some(
        img(ImageSource::Render(tex.img))
            .absolute()
            .left(px(snap(r.x - origin.x + offset.0)))
            .top(px(snap(r.y - origin.y + offset.1)))
            .w(px(w))
            .h(px(h))
            .opacity(alpha)
            .into_any_element(),
    )
}

/// 设备颜色（eGUI `device::shade`）。
fn shade([r, g, b]: [u8; 3], k: f32) -> gpui::Rgba {
    fn f(c: u8, k: f32) -> u8 {
        let c = f32::from(c);
        (if k >= 0.0 { c + (255.0 - c) * k } else { c * (1.0 + k) }).round().clamp(0.0, 255.0) as u8
    }
    let k = k.clamp(-1.0, 1.0);
    gpui::rgb(((f(r, k) as u32) << 16) | ((f(g, k) as u32) << 8) | f(b, k) as u32)
}

fn dev_rect(r: RectPx, area: RectPx, radius: f32, color: gpui::Rgba) -> Div {
    div()
        .absolute()
        .left(px(r.x - area.x))
        .top(px(r.y - area.y))
        .w(px(r.w))
        .h(px(r.h))
        .rounded(px(radius))
        .bg(color)
}

/// 机身：eGUI `device::device_body`。颜色带 + 黑色玻璃，Mac / Desktop 另加底座。
fn device_body(stage: Div, g: &Geom, area: RectPx) -> Div {
    let Some(d) = g.device else { return stage };
    let w = layout::upright_w(g.screen.w, g.screen.h, g.landscape);
    let (b, screen_r) = layout::bezel(d.name, w);
    let band = layout::rim(w, b);
    let rgb = g.color.map(|c| c.rgb).unwrap_or([0x2C, 0x2C, 0x2E]);
    let body = shade(rgb, 0.0);
    let edge = shade(rgb, -0.3);
    let metal = shade(rgb, 0.12);
    let glass = gpui::rgb(0x16_16_18);
    let slab = RectPx { x: g.screen.x - b, y: g.screen.y - b, w: g.screen.w + 2.0 * b, h: g.screen.h + 2.0 * b };
    let mut out = stage;
    match d.name {
        "MacBook Air" => {
            let inner = slab;
            let base = RectPx {
                x: inner.x - w * 0.08,
                y: inner.y + inner.h,
                w: inner.w + w * 0.16,
                h: (w * 0.022).max(6.0),
            };
            out = out.child(dev_rect(base, area, 4.0, metal));
            out = out.child(dev_rect(inner, area, b * 1.2, body));
            let notch = RectPx { x: base.x + base.w / 2.0 - w * 0.06, y: base.y + 1.5, w: w * 0.12, h: 3.0 };
            out = out.child(dev_rect(notch, area, 2.0, edge));
            let inset = RectPx { x: inner.x + band, y: inner.y + band, w: inner.w - 2.0 * band, h: inner.h - 2.0 * band };
            out = out.child(dev_rect(inset, area, (b * 1.2 - band).max(0.0), glass));
        }
        "Desktop" => {
            let inner = slab;
            let neck = RectPx {
                x: inner.x + inner.w / 2.0 - w * 0.06,
                y: inner.y + inner.h,
                w: w * 0.12,
                h: w * 0.08,
            };
            let foot = RectPx { x: neck.x - w * 0.06, y: neck.y + neck.h, w: neck.w + w * 0.12, h: 6.0 };
            out = out.child(dev_rect(neck, area, 0.0, metal));
            out = out.child(dev_rect(foot, area, 3.0, edge));
            out = out.child(dev_rect(inner, area, 8.0, body));
            let inset = RectPx { x: inner.x + band, y: inner.y + band, w: inner.w - 2.0 * band, h: inner.h - 2.0 * band };
            out = out.child(dev_rect(inset, area, (8.0 - band).max(0.0), glass));
        }
        _ => {
            if !d.name.starts_with("iPad") {
                for (left, f0, fh) in [(true, 0.18, 0.06), (true, 0.27, 0.1), (false, 0.24, 0.13)] {
                    let btn = if g.landscape {
                        let y = if left { slab.y + slab.h - 0.5 } else { slab.y - 2.5 };
                        RectPx { x: slab.x + slab.w * f0 as f32, y, w: slab.w * fh as f32, h: 2.5 }
                    } else {
                        let x = if left { slab.x - 2.0 } else { slab.x + slab.w };
                        RectPx { x, y: slab.y + slab.h * f0 as f32, w: 2.5, h: slab.h * fh as f32 }
                    };
                    out = out.child(dev_rect(btn, area, 1.0, edge));
                }
            }
            out = out.child(dev_rect(slab, area, screen_r + b, body));
            out = out.child(
                div()
                    .absolute()
                    .left(px(slab.x - area.x))
                    .top(px(slab.y - area.y))
                    .w(px(slab.w))
                    .h(px(slab.h))
                    .rounded(px(screen_r + b))
                    .border_1()
                    .border_color(edge),
            );
            let glass_band = b - band;
            let inset = RectPx { x: slab.x + band, y: slab.y + band, w: slab.w - 2.0 * band, h: slab.h - 2.0 * band };
            out = out.child(dev_rect(inset, area, (screen_r + glass_band).min(glass_band * 3.4).max(0.0), glass));
        }
    }
    out
}

/// 玻璃圆角遮罩：eGUI `device::device_glass`（圆角屏幕的四角 + 手机药丸）。
///
/// GPUI 0.2 的 `ContentMask` 只有矩形 bounds（`window.rs`），`rounded()+overflow_hidden()`
/// **不裁圆角**，所以屏幕内容会从设备圆角处溢出。这里按 eGUI 的做法在画面**之上**补四个
/// 角：每个 `r×r` 方块只把朝向屏幕中心的那一角做成圆角，正好盖住「方形角落减去四分之一圆」
/// 的区域。
fn device_glass(stage: Div, g: &Geom, area: RectPx) -> Div {
    let Some(d) = g.device else { return stage };
    let w = layout::upright_w(g.screen.w, g.screen.h, g.landscape);
    let (_, screen_r) = layout::bezel(d.name, w);
    let r = screen_r.min(g.screen.w / 2.0).min(g.screen.h / 2.0);
    if !(r.is_finite() && r > 0.0) {
        return stage;
    }
    let glass = gpui::rgb(0x16_16_18);
    let s = g.screen;
    // (x, y, 左上, 右上, 右下, 左下)：圆角放在朝向屏幕中心的那一角。
    let masks = [
        (s.x, s.y, false, false, true, false),
        (s.x + s.w - r, s.y, false, false, false, true),
        (s.x, s.y + s.h - r, false, true, false, false),
        (s.x + s.w - r, s.y + s.h - r, true, false, false, false),
    ];
    let mut out = stage;
    for (x, y, tl, tr, br, bl) in masks {
        let mut m = div()
            .absolute()
            .left(px(x - area.x))
            .top(px(y - area.y))
            .w(px(r))
            .h(px(r))
            .bg(glass);
        if tl {
            m = m.rounded_tl(px(r));
        }
        if tr {
            m = m.rounded_tr(px(r));
        }
        if br {
            m = m.rounded_br(px(r));
        }
        if bl {
            m = m.rounded_bl(px(r));
        }
        out = out.child(m);
    }
    out
}

// ---- 布局（eGUI `present::layout` 的 GPUI 版）----
//
// GPUI 没有「先量再画」：可用区域由窗口尺寸算出（顶栏以下的整块），这里按 eGUI 的
// Fit/FitWidth/Fill/Actual 与设备机身外扩重算一次，再以绝对坐标摆位图 —— 不再依赖
// flex 居中，过渡帧才能和当前帧严格重叠（同一 rect + 位移/透明度）。

// ---- 旧的本地布局已移到 `present/layout.rs` ----

/// 覆盖率层（全屏）：顶栏 +（设备机身 +）画面 + 过渡帧 + 底部导航（eGUI `player()`）。
pub fn render(app: &CanvasApp, state: &PresentState, vp: (f32, f32), canvas: (f32, f32, f32, f32)) -> Stateful<Div> {
    let t = tokens();
    // 演示优先用自己那份文档快照：源设计稿被关掉后，预览标签依然能打开并继续播放
    // （用户 2026-11-17）。源文档还在时快照会随切进预览刷新，所以不会看到旧内容。
    let live;
    let doc = if let Some(d) = state.display() {
        d
    } else if let Some(d) = &state.doc {
        d.as_ref()
    } else {
        let Ok(ds) = app.session.ds() else {
            return div().id("present-layer").size_full();
        };
        live = ds;
        live.doc.as_ref()
    };
    let name = doc.node(state.current).map(|n| n.name.clone()).unwrap_or_default();
    // 预览窗格（`proto.preview`）与整窗演示（`proto.present`）是两种 chrome：
    // 窗格浮在画布右上角（eGUI `present::chrome::pane`），永远 Fit、带设备机身、没有导航条。
    let pane = state.preview;
    let (cx, cy, cw, ch) = canvas;
    let (pane_x, pane_y, pane_w, pane_h) = if pane {
        let w = PANE_W.min((cw - 2.0 * PANE_MARGIN).max(220.0));
        let h = PANE_H.min((ch - 2.0 * PANE_MARGIN).max(260.0));
        ((cx + cw - w - PANE_MARGIN).max(cx + PANE_MARGIN), cy + PANE_MARGIN, w, h)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    let bar_h = if pane { PANE_BAR_H } else { BAR_H };
    let stage_y = if pane { pane_y + bar_h } else { crate::title_bar::TITLE_H + BAR_H };
    let stage_w = if pane { pane_w } else { vp.0 };
    let stage_h = if pane { (pane_h - bar_h).max(0.0) } else { (vp.1 - stage_y).max(0.0) };
    let side_open = !pane && (state.inspect || state.comments);
    let usable_w = (stage_w - if side_open { INSPECT_W } else { 0.0 }).max(1.0);
    // 舞台区域（窗口坐标）：顶栏以下、侧栏以左。输入与渲染共用这一份几何。
    let area = RectPx { x: if pane { pane_x } else { 0.0 }, y: stage_y, w: usable_w, h: stage_h };
    let g = layout::compute(state, doc, area, pane, app.ppp);
    if let Some(g) = g.as_ref() {
        GEOMS.with(|q| {
            let mut q = q.borrow_mut();
            q.retain(|(w, _)| *w != crate::windows::current());
            q.push((crate::windows::current(), g.clone()));
        });
    }
    // 舞台内一律用 area 局部坐标。
    let lx = |x: f32| x - area.x;
    let ly = |y: f32| y - area.y;
    let ppp = app.ppp;

    let mut stage = div().relative().flex_1().min_h(px(0.0)).w_full().overflow_hidden();
    if let Some(g) = g.as_ref() {
        let screen_r = g
            .device
            .map_or(0.0, |d| layout::bezel(d.name, layout::upright_w(g.screen.w, g.screen.h, g.landscape)).1);
        // 1) 设备机身（屏幕外侧的颜色带 + 黑色玻璃）。
        stage = device_body(stage, g, area);
        // 2) 屏幕层：剪到屏幕矩形；圆角由第 8 步的 `device_glass` 遮罩补齐。
        let mut layer = div()
            .absolute()
            .left(px(lx(g.screen.x)))
            .top(px(ly(g.screen.y)))
            .w(px(g.screen.w))
            .h(px(g.screen.h))
            .rounded(px(screen_r))
            .overflow_hidden();
        // 3) 画面 / 过渡。
        if let Some(from) = state.from {
            let raw = (state.transition_start.elapsed().as_secs_f64() * 1000.0 / f64::from(state.transition_ms.max(1))).clamp(0.0, 1.0);
            let eased = state.transition_easing.eval(raw);
            let size = figcraft_geom::Vec2::new(f64::from(g.screen.w), f64::from(g.screen.h));
            let poses = figcraft_doc::anim::transitions::poses(&state.transition, eased, size);
            // from 帧按自己的 page 尺寸在屏幕里居中（eGUI `Place::centered`）。
            let from_place = doc.world_bounds(from).map(|b| Place {
                page_min: (b.x0, b.y0),
                origin: (
                    f64::from(g.screen.x + (g.screen.w - (b.width() * g.scale) as f32) / 2.0),
                    f64::from(g.screen.y + (g.screen.h - (b.height() * g.scale) as f32) / 2.0),
                ),
                scale: g.scale,
            });
            let from_el = from_place.and_then(|pl| {
                frame_at(state, doc, from, &pl, g.screen, ppp, poses.from.alpha as f32, (poses.from.offset.x as f32, poses.from.offset.y as f32))
            });
            let to_el = frame_at(
                state,
                doc,
                state.current,
                &g.place,
                g.screen,
                ppp,
                poses.to.alpha as f32,
                (poses.to.offset.x as f32, poses.to.offset.y as f32),
            );
            if poses.to_on_top {
                layer = layer.children(from_el).children(to_el);
            } else {
                layer = layer.children(to_el).children(from_el);
            }
        } else if let Some(el) = frame_at(state, doc, state.current, &g.place, g.screen, ppp, 1.0, (0.0, 0.0)) {
            layer = layer.child(el);
        }
        // 4) 覆盖层 / 关闭中的覆盖层。
        let draw_overlay = |layer: Div, o: &OverlayState, pl: &Place, raw: f64| -> Div {
            let mut layer = layer;
            if let Some(bg) = o.background {
                let a = (f64::from(bg.a) * raw.clamp(0.0, 1.0) * 255.0) as u32;
                let c = ((bg.r * 255.0) as u32) << 24 | ((bg.g * 255.0) as u32) << 16 | ((bg.b * 255.0) as u32) << 8 | a;
                layer = layer.child(div().absolute().size_full().bg(gpui::rgba(c)));
            }
            let size = match doc.world_bounds(o.id) {
                Some(b) => {
                    let r = pl.rect(b);
                    figcraft_geom::Vec2::new(f64::from(r.w), f64::from(r.h))
                }
                None => figcraft_geom::Vec2::new(0.0, 0.0),
            };
            let poses = figcraft_doc::anim::transitions::poses(&o.transition, o.easing.eval(raw), size);
            match frame_at(
                state,
                doc,
                o.id,
                pl,
                g.screen,
                ppp,
                poses.to.alpha.clamp(0.0, 1.0) as f32,
                (poses.to.offset.x as f32, poses.to.offset.y as f32),
            ) {
                Some(el) => layer.child(el),
                None => layer,
            }
        };
        for (id, pl, _r) in &g.overlays {
            if let Some(o) = state.overlays.iter().find(|o| o.id == *id) {
                let raw = if o.dur > 0.0 { ((state.clock - o.opened) / o.dur).clamp(0.0, 1.0) } else { 1.0 };
                layer = draw_overlay(layer, o, pl, raw);
            }
        }
        for (at, o) in &state.closing {
            if let Some((_, pl, _)) = g.closing.iter().find(|(id, _, _)| *id == o.id) {
                let raw = 1.0 - ((state.clock - at) / o.dur.max(1e-6)).clamp(0.0, 1.0);
                layer = draw_overlay(layer, o, pl, raw);
            }
        }
        // 5) 手机镜头药丸（eGUI `device_glass`）。
        if let Some(d) = g.device
            && d.name.starts_with("iPhone")
        {
            let uw = layout::upright_w(g.screen.w, g.screen.h, g.landscape);
            let (pw, ph) = if g.landscape { (uw * 0.085, uw * 0.3) } else { (uw * 0.3, uw * 0.085) };
            let (ccx, ccy) = if g.landscape {
                (uw * 0.045, g.screen.h / 2.0)
            } else {
                (g.screen.w / 2.0, uw * 0.045)
            };
            layer = layer.child(
                div()
                    .absolute()
                    .left(px(ccx - pw / 2.0))
                    .top(px(ccy - ph / 2.0))
                    .w(px(pw))
                    .h(px(ph))
                    .rounded(px((uw * 0.0425).min(ph / 2.0)))
                    .bg(gpui::rgb(0x000000)),
            );
        }
        // 6) hotspot 提示（eGUI `draw`）：点击空白后从整不透明淡出到 0（`(flash_until-now)/FLASH_SECS`）。
        if state.hints && state.clock < state.flash_until {
            let a = ((state.flash_until - state.clock) / FLASH_SECS).clamp(0.0, 1.0) as f32;
            eprintln!(
                "[dbg flash] clock={:.3} until={:.3} a={:.3} hot={}",
                state.clock,
                state.flash_until,
                a,
                state.hot.len()
            );
            for (_, r) in &state.hot {
                if r.w <= 0.0 || r.h <= 0.0 {
                    continue;
                }
                layer = layer.child(
                    div()
                        .absolute()
                        .left(px(r.x - g.screen.x))
                        .top(px(r.y - g.screen.y))
                        .w(px(r.w))
                        .h(px(r.h))
                        .rounded(px(4.0))
                        .bg(rgba(t.accent))
                        .opacity(0.25 * a),
                );
                layer = layer.child(
                    div()
                        .absolute()
                        .left(px(r.x - g.screen.x))
                        .top(px(r.y - g.screen.y))
                        .w(px(r.w))
                        .h(px(r.h))
                        .rounded(px(4.0))
                        .border_1()
                        .border_color(rgba(t.accent))
                        .opacity(a),
                );
            }
        }
        // 7) Inspect：选中层描边。
        if state.inspect
            && let Some(id) = state.inspected
            && let Some(b) = doc.world_bounds(id)
        {
            let r = g.place.rect(b);
            layer = layer.child(
                div()
                    .absolute()
                    .left(px(r.x - g.screen.x))
                    .top(px(r.y - g.screen.y))
                    .w(px(r.w))
                    .h(px(r.h))
                    .border_1()
                    .border_color(rgba(t.accent)),
            );
        }
        stage = stage.child(layer);
        // 8) 设备玻璃圆角遮罩：在画面之上盖住方角溢出部分（eGUI `device_glass`）。
        stage = device_glass(stage, g, area);
    } else {
        stage = stage.child(
            div()
                .absolute()
                .left(px(stage_w / 2.0 - 120.0))
                .top(px(stage_h / 2.0))
                .w(px(240.0))
                .text_size(pt(13.0))
                .text_color(rgba(t.text_secondary))
                .child(tr("This frame cannot be rendered")),
        );
    }
    // 指针事件：投递给运行时（sync_present 消费）。
    stage = stage
        .when(state.cursor() == 1, |d| d.cursor_pointer())
        .when(state.cursor() == 2, |d| d.cursor(gpui::CursorStyle::ClosedHand))
        .on_mouse_down(MouseButton::Left, |e: &gpui::MouseDownEvent, _, cx| {
            cx.stop_propagation();
            input::push(Ev::Down(f64::from(e.position.x), f64::from(e.position.y)));
        })
        .on_scroll_wheel(|e: &gpui::ScrollWheelEvent, _, cx| {
            cx.stop_propagation();
            let (dx, dy) = match e.delta {
                gpui::ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                gpui::ScrollDelta::Lines(p) => (p.x * 30.0, p.y * 30.0),
            };
            let (x, y) = (f64::from(e.position.x), f64::from(e.position.y));
            input::push(Ev::Wheel(x, y, f64::from(dx), f64::from(dy)));
        });

    // ── 顶栏：eGUI `present::player::top_bar` ────────────────────────────────────
    // 左：28×28 缩略图位（画框图标）+ flow 名（semibold）+ "/" + 画框名（regular）。
    // 右：文字按钮 Close / Restart / Comments / Inspect（全窗口演示在 eGUI 里就是文字）。
    let flow_name = doc
        .page_of(state.start)
        .and_then(|pg| doc.node(pg))
        .and_then(|n| match &n.kind {
            figcraft_doc::NodeKind::Page { flows, .. } => flows.iter().find(|f| f.start == state.start).map(|f| f.name.clone()),
            _ => None,
        })
        .unwrap_or_else(|| tr("Flows").to_string());
    let title_button = |i: u8, label: SharedString, action: u8, on: bool| {
        div()
            .id(("present-bar-btn", u64::from(i)))
            .h(px(24.0))
            .px(pt(8.0))
            .flex()
            .items_center()
            .rounded(pt(5.0))
            .bg(rgba(if on { t.accent } else { t.panel }))
            .hover(|s| if on { s } else { s.bg(rgba(t.hover)) })
            .text_size(pt(11.5))
            .text_color(rgba(if on { t.accent_on } else { t.text }))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                ACTION.with(|a| a.set(action));
            })
            .child(label)
    };
    let bar = div()
        .id("present-bar")
        .flex_none()
        .h(px(BAR_H))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(6.0))
        .px(pt(10.0))
        .bg(rgba(t.panel))
        .border_b_1()
        .border_color(rgba(t.border))
        .child(
            div()
                .flex_none()
                .w(px(28.0))
                .h(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(pt(4.0))
                .bg(rgba(t.field))
                .child(paint_icon(Icon::Frame, rgba(t.text_secondary), 16.0)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(6.0))
                .overflow_hidden()
                .child(
                    div()
                        .flex_none()
                        .text_size(pt(11.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgba(t.text))
                        .child(SharedString::from(flow_name.clone())),
                )
                .child(div().flex_none().text_size(pt(11.5)).text_color(rgba(t.text_tertiary)).child("/"))
                .child(
                    div()
                        .min_w(px(0.0))
                        .text_size(pt(11.5))
                        .text_color(rgba(t.text))
                        .overflow_hidden()
                        .child(SharedString::from(name.clone())),
                ),
        )
        .child(title_button(1, tr("Inspect").into(), 6, state.inspect))
        .child(title_button(2, tr("Comments").into(), 5, state.comments))
        .child(title_button(3, tr("Restart").into(), 4, false))
        .child(title_button(4, tr("Close").into(), 3, false));

    // ── 底部导航条：eGUI `chrome::navigation_bar` ───────────────────────────────
    // 居中悬浮、bottom 18、圆角 9、padding 8/5；‹ index/N › ｜ 设备下拉 ｜ 缩放下拉。
    let page = doc.page_of(state.current).unwrap_or(state.current);
    let frames = doc.top_frames(page);
    let index = frames.iter().position(|f| *f == state.current);
    let has_prev = index.and_then(|i| i.checked_sub(1)).is_some();
    let has_next = index.and_then(|i| frames.get(i + 1)).is_some();
    let settings = page_settings(doc, state.current);
    let device_label: SharedString = if settings.device == "None" || settings.device.is_empty() {
        tr("No device").into()
    } else {
        SharedString::from(settings.device.clone())
    };
    let nav_btn = |icon: Icon, action: u8, enabled: bool| {
        div()
            .id(("present-nav-btn", u64::from(action)))
            .w(px(26.0))
            .h(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(pt(4.0))
            .hover(|s| if enabled { s.bg(rgba(t.hover)) } else { s })
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                // nav 是 stage 的子元素：不拦冒泡就会被 stage 当成「点画面」。
                cx.stop_propagation();
                if enabled {
                    ACTION.with(|a| a.set(action));
                }
            })
            .child(paint_icon(icon, rgba(if enabled { t.text } else { t.text_tertiary }), 18.0))
    };
    let menu_shell = |id: &'static str| {
        div()
            .id(id)
            .flex_none()
            .mb(px(6.0))
            .w(px(212.0))
            .max_h(px(384.0))
            .overflow_y_scroll()
            .rounded(pt(6.0))
            .bg(rgba(t.menu))
            .border_1()
            .border_color(rgba(t.border))
            .py(pt(4.0))
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    };
    let menu_item = |label: SharedString, selected: bool, pick: Box<dyn Fn(&mut gpui::Window, &mut gpui::App)>| {
        div()
            .h(px(26.0))
            .px(pt(10.0))
            .flex()
            .items_center()
            .justify_between()
            .rounded(pt(4.0))
            .text_size(pt(11.0))
            .text_color(rgba(if selected { t.text } else { t.menu_text }))
            .cursor_pointer()
            .hover(|s| s.bg(rgba(t.menu_hover)))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                pick(window, cx);
            })
            .child(label)
            .when(selected, |d| d.child(paint_icon(Icon::Check, rgba(t.accent), 14.0)))
    };

    let device_menu = if state.device_open {
        let mut list = menu_shell("present-device-menu");
        let none = menu_item(tr("No device").into(), settings.device == "None", Box::new(|_, _| SET_DEVICE.with(|d| *d.borrow_mut() = Some("None".to_string()))));
        list = list.child(none);
        for d in PrototypeSettings::DEVICES.iter() {
            let n = d.name;
            list = list.child(menu_item(
                SharedString::from(n),
                settings.device == n,
                Box::new(move |_, _| SET_DEVICE.with(|d| *d.borrow_mut() = Some(n.to_string()))),
            ));
        }
        Some(list)
    } else {
        None
    };
    let scale_menu = if state.scale_open {
        let mut list = menu_shell("present-scale-menu");
        for m in [ScaleMode::Fit, ScaleMode::FitWidth, ScaleMode::Fill, ScaleMode::Actual] {
            list = list.child(menu_item(
                SharedString::from(m.label()),
                state.scale == m,
                Box::new(move |_, _| SET_SCALE.with(|s| *s.borrow_mut() = Some(m.id()))),
            ));
        }
        Some(list)
    } else {
        None
    };

    let nav = if pane || state.hide_ui {
        div()
    } else {
        let pill = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(6.0))
            .px(pt(8.0))
            .py(pt(5.0))
            .rounded(pt(9.0))
            .bg(rgba(t.panel))
            .border_1()
            .border_color(rgba(t.border))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(nav_btn(Icon::ChevronLeft, 8, has_prev))
            .child(
                div()
                    .min_w(px(48.0))
                    .flex()
                    .justify_center()
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text))
                    .child(SharedString::from(format!("{} / {}", index.map_or(0, |i| i + 1), frames.len()))),
            )
            .child(nav_btn(Icon::ChevronRight, 9, has_next))
            .child(div().w(px(1.0)).h(px(16.0)).bg(rgba(t.border)))
            .child(
                div()
                    .id("present-device-open")
                    .h(px(24.0))
                    .px(pt(6.0))
                    .flex()
                    .items_center()
                    .gap(pt(4.0))
                    .rounded(pt(4.0))
                    .hover(|s| s.bg(rgba(t.hover)))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                        ACTION.with(|a| a.set(10));
                    })
                    .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(device_label))
                    .child(paint_icon(Icon::ChevronDown, rgba(t.text_secondary), 12.0)),
            )
            .child(
                div()
                    .id("present-scale-open")
                    .h(px(24.0))
                    .px(pt(6.0))
                    .flex()
                    .items_center()
                    .gap(pt(4.0))
                    .rounded(pt(4.0))
                    .hover(|s| s.bg(rgba(t.hover)))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                        ACTION.with(|a| a.set(11));
                    })
                    .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(state.scale.label())))
                    .child(paint_icon(Icon::ChevronDown, rgba(t.text_secondary), 12.0)),
            );
        div()
            .absolute()
            .bottom(px(18.0))
            .left(px(0.0))
            .right(px(0.0))
            .flex()
            .flex_col()
            .items_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(device_menu)
            .children(scale_menu)
            .child(pill)
    };

    // ── 侧面板：Inspect / Comments（eGUI `show()` 的右栏，宽 280）────────────────
    let side_panel = if !pane && (state.inspect || state.comments) {
        let title: SharedString = if state.inspect { tr("Inspect").into() } else { tr("Comments").into() };
        let mut body = div().flex_1().min_h(px(0.0)).flex().flex_col().px(pt(12.0)).pt(pt(10.0));
        if state.inspect {
            // eGUI `chrome.rs::inspect_panel`：图层名 + `devinspect::properties` 的只读区块。
            if let Some((id, name)) = state.inspected.and_then(|id| doc.node(id).map(|n| (id, n.name.clone()))) {
                body = body.child(
                    div()
                        .pb(pt(8.0))
                        .text_size(pt(11.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgba(t.text))
                        .child(SharedString::from(name)),
                );
                for s in figcraft_engine::devinspect::properties(doc, id) {
                    let sec = crate::panels::right::data::InspectSection {
                        title: s.title,
                        rows: s
                            .rows
                            .into_iter()
                            .map(|p| crate::panels::right::data::InspectRow { label: p.label, value: p.value, color: p.color, token: p.token })
                            .collect(),
                    };
                    body = body.child(crate::panels::right::inspect::inspect_section(&sec));
                }
            } else {
                body = body.child(
                    div()
                        .text_size(pt(11.0))
                        .text_color(rgba(t.text_secondary))
                        .child(tr("Hover a layer to see its properties")),
                );
            }
        } else {
            // eGUI `comments::panel` 的列表（筛选/排序用默认档，演示里不需要那套操作控件）。
            let me = figcraft_engine::cmd::comment::user_name(&app.session);
            let rows = crate::comments::read_rows(doc, &me, figcraft_engine::cmd::comment::Filter::Open, figcraft_engine::cmd::comment::Sort::Date, "");
            if rows.is_empty() {
                body = body.child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("No comments yet")));
            } else {
                for r in rows {
                    body = body.child(
                        div()
                            .w_full()
                            .py(pt(7.0))
                            .border_b_1()
                            .border_color(rgba(t.border))
                            .flex()
                            .flex_col()
                            .gap(pt(3.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .text_size(pt(11.0))
                                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(SharedString::from(r.author.clone())))
                                    .child(div().text_color(rgba(t.text_tertiary)).child(SharedString::from(r.ago.clone()))),
                            )
                            .child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(r.body.clone())))
                            .children((!r.meta.is_empty()).then(|| {
                                div().text_size(pt(10.0)).text_color(rgba(t.text_tertiary)).child(SharedString::from(r.meta.clone()))
                            })),
                    );
                }
            }
        }
        Some(
            div()
                .id("present-side-panel")
                .absolute()
                .right(px(0.0))
                .top(px(0.0))
                .bottom(px(0.0))
                .w(px(INSPECT_W))
                .flex()
                .flex_col()
                .bg(rgba(t.panel))
                .border_l_1()
                .border_color(rgba(t.border))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex_none()
                        .h(px(BAR_H))
                        .px(pt(12.0))
                        .flex()
                        .items_center()
                        .border_b_1()
                        .border_color(rgba(t.border))
                        .text_size(pt(11.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgba(t.text))
                        .child(title),
                )
                .child(body),
        )
    } else {
        None
    };

    // ── 预览窗格顶栏：eGUI `present::player::player`（pane = true）─────────────────
    // 左：flow 名（semibold）+ "/" + 画框名；右：Restart / Present（转整窗演示）/ Close。
    let pane_bar = if pane {
        let icon_btn = |i: u8, icon: Icon, action: u8| {
            div()
                .id(("present-pane-btn", u64::from(i)))
                .w(px(24.0))
                .h(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(pt(4.0))
                .hover(|s| s.bg(rgba(t.hover)))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    ACTION.with(|a| a.set(action));
                })
                .child(paint_icon(icon, rgba(t.text_secondary), 15.0))
        };
        Some(
            div()
                .id("present-pane-bar")
                .flex_none()
                .h(px(PANE_BAR_H))
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(6.0))
                .px(pt(10.0))
                .bg(rgba(t.panel))
                .border_b_1()
                .border_color(rgba(t.border))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(pt(6.0))
                        .overflow_hidden()
                        .child(
                            div()
                                .flex_none()
                                .text_size(pt(11.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgba(t.text))
                                .child(SharedString::from(flow_name.clone())),
                        )
                        .child(div().flex_none().text_size(pt(11.5)).text_color(rgba(t.text_tertiary)).child("/"))
                        .child(
                            div()
                                .min_w(px(0.0))
                                .text_size(pt(11.5))
                                .text_color(rgba(t.text))
                                .overflow_hidden()
                                .child(SharedString::from(name.clone())),
                        ),
                )
                .child(icon_btn(0, Icon::Rotate, 4))
                .child(icon_btn(1, Icon::Play, 12))
                .child(icon_btn(2, Icon::Close, 3)),
        )
    } else {
        None
    };

    let mut stage_el = stage;
    if !pane {
        stage_el = stage_el.children(side_panel).child(nav);
    }

    // 预览窗格：浮在画布右上角的小窗（eGUI `present::chrome::pane` 的 `Window`）。
    if pane {
        return div()
            .id("present-pane")
            .absolute()
            .left(px(pane_x))
            .top(px(pane_y))
            .w(px(pane_w))
            .h(px(pane_h))
            .rounded(px(8.0))
            .overflow_hidden()
            .border_1()
            .border_color(rgba(t.border))
            .flex()
            .flex_col()
            .bg(rgba(t.canvas))
            .children(pane_bar)
            .child(stage_el);
    }

    // eGUI 的演示覆盖 CentralPanel，标题栏（含 tab strip 的 PresentTab）仍显示在上方。
    // GPUI 这里从 TITLE_H 起铺，别把标签栏盖掉（用户 2026-11-17：标签栏搞丢了）。
    div()
        .id("present-layer")
        .absolute()
        .top(px(crate::title_bar::TITLE_H))
        .left(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .flex()
        .flex_col()
        .bg(rgba(t.canvas))
        .child(bar)
        .child(stage_el)
}

thread_local! {
    /// 各窗口最近一帧的摆放结果（窗口坐标，按窗口 id 存，避免多窗口互相覆盖）。
    /// 输入处理在 sync 帧读取上一帧的几何。
    static GEOMS: std::cell::RefCell<Vec<(u64, Geom)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// 顶栏 / 导航按钮：3 退出、4 重播、5 评论、6 Inspect、8 上一帧、9 下一帧、
    /// 10 设备菜单、11 缩放菜单。
    pub static ACTION: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    /// 导航栏下拉里选中的设备（`proto.settings` 落地）。
    pub static SET_DEVICE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// 导航栏下拉里选中的缩放档。
    pub static SET_SCALE: std::cell::RefCell<Option<u8>> = const { std::cell::RefCell::new(None) };
}

/// 本窗口最近一帧的几何。
pub fn geom_of(wid: u64) -> Option<Geom> {
    GEOMS.with(|q| q.borrow().iter().rev().find(|(w, _)| *w == wid).map(|(_, g)| g.clone()))
}

/// 处理本帧排队的指针 / 滚轮 / 键盘事件（由 `sync_present` 调用）。
pub fn tick_input(st: &mut PresentState, now: f64) {
    let g = geom_of(crate::windows::current());
    if let Some(g) = g.as_ref() {
        st.pan_range = g.pan_range;
    }
    // 没有跟随到源文档时（设计稿不在前台），用打开预览时存下的快照初始化运行时。
    if st.disp.is_none()
        && let Some(d) = st.doc.clone()
    {
        st.sync_source(&d, st.clock);
    }
    input::tick(st, g.as_ref(), now);
}

/// 投递一个按键（窗口层捕获回调里调用）。
pub fn push_key(key: String, mods: figcraft_doc::proto::KeyMods) {
    input::push(Ev::Key(key, mods));
}

/// 窗口级指针移动（根视图 move 回调）——元素级 move 在指针移出舞台后不再触发。
pub fn push_move(x: f64, y: f64) {
    input::push(Ev::Move(x, y));
}

/// 临时调试：打印一组节点的世界矩形（present 内部布局诊断）。
fn dbg_boxes(tag: &str, d: &Document) {
    let ids = [103u64, 104, 127, 142, 143, 147, 153, 154, 177, 181, 182, 188, 195];
    for id in ids {
        match (d.world_bounds(NodeId(id)), d.node(NodeId(id))) {
            (Some(b), Some(n)) => eprintln!(
                "[dbg {tag}] {id}: ({:.1},{:.1},{:.1},{:.1}) size={:.1}x{:.1} ovr=[{}]",
                b.x0,
                b.y0,
                b.width(),
                b.height(),
                n.width,
                n.height,
                n.overrides.iter().cloned().collect::<Vec<_>>().join(",")
            ),
            _ => eprintln!("[dbg {tag}] {id}: -"),
        }
    }
}

/// 临时调试：打印子树（含 sizing 与 grid 位置）。
fn dbg_tree(tag: &str, d: &Document, root: NodeId) {
    for id in d.subtree(root) {
        let Some(n) = d.node(id) else { continue };
        let Some(b) = d.world_bounds(id) else { continue };
        let cl = &n.child_layout;
        eprintln!(
            "[dbg {tag}] {:>4} {:<8} '{}' ({:.1},{:.1},{:.1},{:.1}) h={}/{}{} v={}/{}{} grid={:?},{:?}",
            id.0,
            match n.kind {
                figcraft_doc::NodeKind::Frame { .. } => "frame",
                figcraft_doc::NodeKind::Group { .. } => "group",
                figcraft_doc::NodeKind::Instance { .. } => "instance",
                figcraft_doc::NodeKind::Text(_) => "text",
                figcraft_doc::NodeKind::Rectangle => "rect",
                figcraft_doc::NodeKind::Component { .. } => "component",
                _ => "other",
            },
            n.name,
            b.x0,
            b.y0,
            b.width(),
            b.height(),
            if cl.sizing_h == figcraft_doc::Sizing::Fill { "fill" } else if cl.sizing_h == figcraft_doc::Sizing::Hug { "hug" } else { "fix" },
            n.width,
            cl.min_w.map_or(String::new(), |v| format!(" min={v}")),
            if cl.sizing_v == figcraft_doc::Sizing::Fill { "fill" } else if cl.sizing_v == figcraft_doc::Sizing::Hug { "hug" } else { "fix" },
            n.height,
            cl.min_h.map_or(String::new(), |v| format!(" min={v}")),
            cl.grid_column,
            cl.grid_row
        );
    }
}


/// 调试：上一帧 hotspot 矩形（`present.hot` 控制命令用）。
pub fn debug_hot() -> (Option<(f64, f64)>, Vec<u64>, Vec<(u64, f32, f32, f32, f32)>) {
    let (pos, hover) = input::PTR_DBG.with(|q| q.borrow().clone());
    let hot = input::HOT_DBG
        .with(|q| q.borrow().iter().map(|(id, r)| (id.0, r.x, r.y, r.w, r.h)).collect());
    (pos, hover.into_iter().map(|n| n.0).collect(), hot)
}

/// 窗口级松开（元素级 up 可能收不到）。
pub fn push_up(x: f64, y: f64) {
    input::push(Ev::Up(x, y));
}

/// 窗口级按下（调试/合成指针用，与元素级 down 等价）。
pub fn push_down(x: f64, y: f64) {
    input::push(Ev::Down(x, y));
}

/// 取走导航栏下拉的选择（由 lib.rs 的 sync 帧落地命令）。
pub fn take_device_choice() -> Option<String> {
    SET_DEVICE.with(|d| d.borrow_mut().take())
}

pub fn take_scale_choice() -> Option<ScaleMode> {
    SET_SCALE.with(|s| s.borrow_mut().take().map(ScaleMode::from_id))
}

/// 调试：设定演示缩放档（0 Fit / 1 Fit width / 2 Fill / 3 Actual size）。
pub fn set_scale(id: u8) {
    SET_SCALE.with(|s| *s.borrow_mut() = Some(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::new("present-test")
    }

    #[test]
    fn playback_advances_and_wraps_at_the_page_duration() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::anim::motion::TimelineSettings;
        // D3：loop=false 到头停帧，回绕只发生在 looping=true。这里显式设一个循环页。
        let mut d = doc();
        let page = d.first_page().unwrap();
        let _ = d.create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 5.0, 5.0).unwrap();
        d.timelines.insert(page, TimelineSettings { duration_ms: 2000, fps: 30, looping: true });
        let mut p = PresentState::new(page, false);
        p.playing = true;
        p.last = Instant::now() - std::time::Duration::from_millis(120);
        assert!(p.advance(&d), "播放中要续帧");
        assert!(p.time_ms > 0.0);
        p.time_ms = 1999.0;
        p.last = Instant::now() - std::time::Duration::from_millis(10);
        p.advance(&d);
        assert!(p.time_ms < 2000.0, "循环页超过时长后回绕");
        p.playing = false;
        assert!(!p.advance(&d));
    }

    /// D3 Control：one-shot（loop=false）播到头停在末帧并退出播放态，不再续帧。
    #[test]
    fn one_shot_stops_at_the_end_and_stops_requesting_frames() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::anim::motion::TimelineSettings;
        let mut d = Document::new("present-one-shot");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
        d.timelines.insert(frame, TimelineSettings { duration_ms: 500, fps: 60, looping: false });
        let mut p = PresentState::new(frame, false);
        p.playing = true;
        // 第一步：小步进，离末帧还远 → 仍要续帧。
        p.time_ms = 100.0;
        p.last = Instant::now() - std::time::Duration::from_millis(40);
        assert!(p.advance(&d), "到达末帧前仍要续帧");
        assert!(p.time_ms > 100.0 && p.time_ms < 500.0, "播放头在末帧之前推进");
        // 第二步：跨过末帧 → 停在 500ms 并退出播放态。
        p.time_ms = 480.0;
        p.last = Instant::now() - std::time::Duration::from_millis(40);
        p.advance(&d);
        assert!((p.time_ms - 500.0).abs() < 1e-6, "停在画框末帧 500ms");
        assert!(!p.playing, "one-shot 播完退出播放态");
        assert!(!p.advance(&d), "停帧后不再请求动画帧");
    }

    /// M1（用户 2026-11-12）：演示播放按**当前画框自己的**时间轴时长回绕（画框没设才回退
    /// 页面），不再无条件用整页时长。这是 per-frame 动画组在演示侧的落点。
    #[test]
    fn playback_wraps_at_the_frame_duration_not_the_page() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::anim::motion::TimelineSettings;
        let mut d = Document::new("present-frame");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
        d.timelines.insert(page, TimelineSettings { duration_ms: 2000, fps: 30, looping: false });
        d.timelines.insert(frame, TimelineSettings { duration_ms: 500, fps: 60, looping: true });
        let mut p = PresentState::new(frame, false);
        p.playing = true;
        p.time_ms = 480.0;
        p.last = Instant::now() - std::time::Duration::from_millis(40);
        p.advance(&d);
        assert!(p.time_ms < 500.0, "必须按画框的 500ms 回绕，而不是页面的 2000ms（否则会跑到 ~520）");
    }

    /// M1 地基：motion::evaluate 把画框子树的关键帧烘焙进 scratch doc，frame_image 依赖它让
    /// 演示里的组件真的随时间动（而不只是 GIF）。这里验证求值链对当前画框作用域成立。
    #[test]
    fn evaluate_bakes_the_frames_subtree_at_time() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::anim::motion::{AnimProperty, AnimValue, Ease};
        let mut d = Document::new("present-eval");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
        let dot = d.create(frame, NodeKind::Rectangle, "Dot", 2.0, 2.0, 8.0, 8.0).unwrap();
        let n = d.node_mut(dot).unwrap();
        let tr = n.motion.ensure_track(AnimProperty::Opacity);
        tr.set_key(0, AnimValue::Scalar(1.0), Some(Ease::bezier(0.0, 0.0, 1.0, 1.0)));
        tr.set_key(500, AnimValue::Scalar(0.0), None);
        let at0 = figcraft_doc::anim::motion::evaluate(&d, frame, 0.0);
        let at250 = figcraft_doc::anim::motion::evaluate(&d, frame, 250.0);
        let o0 = at0.node(dot).unwrap().opacity as f64;
        let o250 = at250.node(dot).unwrap().opacity as f64;
        assert!((o0 - 1.0).abs() < 1e-3, "t=0 不透明度 1");
        assert!(o250 < 0.9 && o250 > 0.1, "t=250 关键帧被求值成中间值，演示里会动");
    }

    #[test]
    fn back_returns_to_the_previous_frame() {
        let mut p = PresentState::new(figcraft_doc::NodeId(1), false);
        p.history.push(figcraft_doc::NodeId(1));
        p.current = figcraft_doc::NodeId(2);
        p.back();
        assert_eq!(p.current, figcraft_doc::NodeId(1));
        assert_eq!(p.from, Some(figcraft_doc::NodeId(2)));
    }

    // ---- D3 Control：Action::Animation 作用于画框时钟 ----

    fn clock(playing: bool, base_ms: f64) -> MotionClock {
        MotionClock { playing, base_ms }
    }

    #[test]
    fn animation_op_resume_is_idempotent_while_playing() {
        // Play{from_start:false} 正在播且没到头 → 幂等不动（desigma 语义）。
        let cur = clock(true, 250.0);
        assert_eq!(apply_animation_op(AnimationOp::RESUME, cur, 250.0, 1000.0), cur);
    }

    #[test]
    fn animation_op_resume_restarts_a_finished_one_shot() {
        // 到头的一次性动画再 resume → 从头播。
        let cur = clock(false, 1000.0);
        let next = apply_animation_op(AnimationOp::RESUME, cur, 1000.0, 1000.0);
        assert!(next.playing && (next.base_ms - 0.0).abs() < 1e-6);
    }

    #[test]
    fn animation_op_from_start_and_restart_rewind() {
        let cur = clock(true, 700.0);
        for op in [AnimationOp::Play { from_start: true }, AnimationOp::Restart] {
            let next = apply_animation_op(op, cur, 700.0, 1000.0);
            assert!(next.playing, "{op:?} 应处于播放态");
            assert!((next.base_ms - 0.0).abs() < 1e-6, "{op:?} 应回到 0");
        }
    }

    #[test]
    fn animation_op_pause_keeps_the_playhead() {
        let cur = clock(true, 333.0);
        let next = apply_animation_op(AnimationOp::Pause, cur, 333.0, 1000.0);
        assert!(!next.playing && (next.base_ms - 333.0).abs() < 1e-6);
    }

    #[test]
    fn animation_op_toggle_play_flips_both_ways() {
        // 播着 → 暂停在播放头。
        let playing = clock(true, 400.0);
        let paused = apply_animation_op(AnimationOp::TogglePlay, playing, 400.0, 1000.0);
        assert!(!paused.playing && (paused.base_ms - 400.0).abs() < 1e-6);
        // 暂停 → 从播放头续播。
        let resumed = apply_animation_op(AnimationOp::TogglePlay, paused, 400.0, 1000.0);
        assert!(resumed.playing && (resumed.base_ms - 400.0).abs() < 1e-6);
    }

    /// 造一个带 `Action::Animation` 点击交互的画框；dest 落在画框内（验证按整框作用域算）。
    fn frame_with_animation_click(op: AnimationOp) -> (Document, figcraft_doc::NodeId) {
        use figcraft_doc::NodeKind;
        use figcraft_doc::proto::{Easing, Interaction, Trigger};
        let mut d = Document::new("present-control");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
        let btn = d.create(frame, NodeKind::Rectangle, "Btn", 0.0, 0.0, 10.0, 10.0).unwrap();
        // 既有 Present 架构：`click()` 读当前画框自身的 `interactions.first()`，故交互挂在画框上；
        // dest 指向子图层 btn，验证 control_animation 用 top_level_frame 把作用域解析回整框。
        d.get_mut(frame).unwrap().interactions.push(Interaction {
            trigger: Trigger::OnClick,
            actions: vec![Action::Animation { dest: btn, op }],
            transition: Transition::Instant,
            duration_ms: 300,
            easing: Easing::EaseOut,
            delay_ms: 0,
        });
        d.timelines.insert(frame, figcraft_doc::anim::motion::TimelineSettings { duration_ms: 1000, fps: 60, looping: false });
        (d, frame)
    }

    #[test]
    fn click_dispatches_animation_op_to_the_current_frame() {
        let (d, frame) = frame_with_animation_click(AnimationOp::Play { from_start: true });
        let mut p = PresentState::new(frame, false);
        p.time_ms = 800.0; // 一个正在播、已过半的画框
        p.playing = true;
        p.click(&d);
        assert!(p.playing, "Play 后处于播放态");
        assert!((p.time_ms - 0.0).abs() < 1e-6, "from_start 把当前画框播放头拉回 0");
        assert_eq!(p.current, frame, "控制类动作不导航");
    }

    #[test]
    fn click_toggle_play_pauses_without_navigating() {
        let (d, frame) = frame_with_animation_click(AnimationOp::TogglePlay);
        let mut p = PresentState::new(frame, false);
        p.time_ms = 250.0;
        p.playing = true;
        p.click(&d);
        assert!(!p.playing, "TogglePlay 暂停正在播的画框");
        assert!((p.time_ms - 250.0).abs() < 1e-6, "暂停保留播放头");
    }

    /// 画框之间独立：dest 指向另一个画框时只更新它的挂起时钟，当前画框与播放态不受影响。
    #[test]
    fn animation_of_another_frame_is_kept_in_its_own_clock() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::proto::{Easing, Interaction, Trigger};
        let mut d = Document::new("present-two-frames");
        let page = d.first_page().unwrap();
        let a = d.create(page, NodeKind::Frame, "A", 0.0, 0.0, 100.0, 100.0).unwrap();
        let b = d.create(page, NodeKind::Frame, "B", 200.0, 0.0, 100.0, 100.0).unwrap();
        for f in [a, b] {
            d.timelines.insert(f, figcraft_doc::anim::motion::TimelineSettings { duration_ms: 1000, fps: 60, looping: false });
        }
        d.get_mut(a).unwrap().interactions.push(Interaction {
            trigger: Trigger::OnClick,
            actions: vec![Action::Animation { dest: b, op: AnimationOp::Play { from_start: true } }],
            transition: Transition::Instant,
            duration_ms: 300,
            easing: Easing::EaseOut,
            delay_ms: 0,
        });
        let mut p = PresentState::new(a, false);
        p.time_ms = 120.0;
        p.playing = true;
        p.click(&d);
        // 当前画框 A 的时钟没被 B 的控制影响。
        assert!(p.playing && (p.time_ms - 120.0).abs() < 1e-6, "控制 B 不动 A 的播放头");
        assert_eq!(p.current, a, "不导航");
        // B 的挂起时钟被置成「从头播」。
        assert_eq!(p.clocks.get(&b), Some(&clock(true, 0.0)), "B 独立记住自己的时钟");
    }

    /// 导航到另一个画框时恢复它挂起的时钟（画框之间独立、来回不丢播放头）。
    #[test]
    fn navigation_restores_a_frames_suspended_clock() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::proto::{Easing, Interaction, Trigger};
        let mut d = Document::new("present-nav-clock");
        let page = d.first_page().unwrap();
        let a = d.create(page, NodeKind::Frame, "A", 0.0, 0.0, 100.0, 100.0).unwrap();
        let b = d.create(page, NodeKind::Frame, "B", 200.0, 0.0, 100.0, 100.0).unwrap();
        for f in [a, b] {
            d.timelines.insert(f, figcraft_doc::anim::motion::TimelineSettings { duration_ms: 1000, fps: 60, looping: false });
        }
        // B 的动画被预先起播、此刻挂起在 600ms。
        let mut p = PresentState::new(a, false);
        p.clocks.insert(b, clock(true, 600.0));
        d.get_mut(a).unwrap().interactions.push(Interaction {
            trigger: Trigger::OnClick,
            actions: vec![Action::Navigate { dest: b, preserve_scroll: false }],
            transition: Transition::Instant,
            duration_ms: 300,
            easing: Easing::EaseOut,
            delay_ms: 0,
        });
        p.click(&d);
        assert_eq!(p.current, b, "导航到 B");
        assert!(p.playing && (p.time_ms - 600.0).abs() < 1e-6, "恢复 B 挂起的播放头与播放态");
    }

    // ---- 摆放 / 滚动 / 覆盖层 / 定时器 ----

    #[test]
    fn place_maps_page_bounds_to_screen() {
        let pl = Place { page_min: (100.0, 50.0), origin: (300.0, 320.0), scale: 2.0 };
        // 页左上角 → 屏幕原点；页内 (50, 25) → 屏幕 (100, 50) 偏移。
        assert_eq!(pl.pt(100.0, 50.0), (300.0, 320.0));
        assert_eq!(pl.pt(150.0, 75.0), (400.0, 370.0));
        // 反投影回页坐标。
        assert_eq!(pl.page((400.0, 370.0)), (150.0, 75.0));
        let r = pl.rect(figcraft_geom::Rect::new(100.0, 50.0, 200.0, 100.0));
        assert_eq!((r.x, r.y, r.w, r.h), (300.0, 320.0, 200.0, 100.0));
    }

    #[test]
    fn clamp_scroll_stays_in_range_and_rubber_band_resists() {
        assert_eq!(scroll::clamp_scroll((50.0, -10.0), (100.0, 20.0)), (50.0, 0.0));
        assert_eq!(scroll::clamp_scroll((150.0, 30.0), (100.0, 20.0)), (100.0, 20.0));
        // 越界越拉越紧，但永远到不了 limit。
        let (below, above) = (scroll::rubber_band(-100.0, 200.0, 80.0), scroll::rubber_band(300.0, 200.0, 80.0));
        assert!(below < 0.0 && below > -80.0, "上越界有阻尼");
        assert!(above > 200.0 && above < 280.0, "下越界有阻尼");
        assert_eq!(scroll::rubber_band(50.0, 200.0, 80.0), 50.0, "范围内原样");
    }

    #[test]
    fn fling_settles_inside_the_range() {
        // 高速冲向端点：弹性越界后回到 [0, 100]。
        let (mut x, mut v, mut done) = (90.0, 1000.0, false);
        for _ in 0..240 {
            (x, v, done) = scroll::fling_axis(x, v, 100.0, 1.0 / 60.0, 2.5, true);
            if done {
                break;
            }
        }
        assert!(done, "惯性必须停下");
        assert!(x >= 0.0 && x <= 100.0, "停在可滚范围内");
        // 范围为零时立刻结束。
        assert!(scroll::fling_axis(10.0, 500.0, 0.0, 1.0 / 60.0, 2.5, true).2);
    }

    #[test]
    fn close_overlay_pops_the_topmost_and_reports_whether_anything_closed() {
        let mut p = PresentState::new(figcraft_doc::NodeId(1), false);
        assert!(!p.close_overlay(0.0), "没有覆盖层时不消费 Back");
        p.overlays.push(OverlayState {
            id: figcraft_doc::NodeId(2),
            position: OverlayPosition::Center,
            close_on_outside: true,
            background: None,
            anchor: None,
            offset: None,
            anchor_to: OverlayAnchor::Screen,
            opened: 0.0,
            transition: Transition::Instant,
            dur: 0.0,
            easing: Easing::EaseOut,
        });
        assert!(p.close_overlay(0.0));
        assert!(p.overlays.is_empty(), "关闭最上层覆盖层");
    }

    #[test]
    fn after_delay_fires_once_after_its_deadline() {
        use figcraft_doc::proto::Interaction;
        let mut d = Document::new("delay");
        let page = d.first_page().unwrap();
        let frame = d.create(page, figcraft_doc::NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
        d.get_mut(frame).unwrap().interactions.push(Interaction {
            trigger: Trigger::AfterDelay { ms: 500 },
            actions: vec![],
            transition: Transition::Instant,
            duration_ms: 300,
            easing: Easing::EaseOut,
            delay_ms: 0,
        });
        let mut p = PresentState::new(frame, false);
        p.arrived = 10.0;
        let (_, due) = input::next_delay(&p, &d, frame, 10.2).unwrap();
        assert!((due - 0.3).abs() < 1e-9, "500ms 定时器还剩 300ms");
        p.delays_fired.push((frame, 0));
        assert!(input::next_delay(&p, &d, frame, 10.2).is_none(), "已触发的不再排队");
    }
}
