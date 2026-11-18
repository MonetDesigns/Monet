//! 顶部 40pt 标签条的 GPUI 视图层（P0-1 第 2 步）。
//!
//! 几何 / 动画状态全在 `tabs.rs`（照搬上游 831 行可测部分）；这里只负责把上游
//! `ui-egui/src/tabs.rs::tab_bar` 的**可见行为**用 GPUI Kit 拼出来：
//! Home、文件标签、宽度收缩/溢出、新标签、主题切换、溢出菜单、X / 中键关闭、拖拽重排插入线。
//!
//! 明确**不在本轮**的（对比报告里已标好阶段，避免半成品假完成）：
//! - 演示标签组 / present 状态机（P2）；
//! - 拖离成新窗口（P2：多窗口）；窗口按钮 / 无边框拖窗 / 双击最大化 / 8 向边缘缩放在
//!   P0-2 用 `WindowControlArea` 落地（**更正**：GPUI 0.2.1 有等价能力，不是框架差异）；
//! - ~~未保存关闭三问~~（**A18 已接，2026-11-10**）：关闭走 `file.close {index}`，引擎对脏文档
//!   发 `dialog.unsaved`，由 `dialogs.rs` 的通用三问对话框处理；Alt+close 才带 `force:true` 直接丢弃；
//! - live / 断连标记（P0-2）、a11y（AccessKit，P4）、Home 首页内容（P1）。
//!
//! 坐标口径：`tab_strip` 是**窗口标题栏区域的 child**（kit `TitleBar::new().child(...)`），
//! 它的 (0,0) 就是窗口原点；标签 x、拖拽落点、溢出菜单锚点都直接用窗口坐标。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    App, Bounds, ClickEvent, Div, Entity, MouseButton, MouseDownEvent, Pixels, Point, SharedString, Stateful, Window, WindowControlArea, div,
    prelude::*, px, size,
};
use serde_json::json;

use crate::panels::{Cmd, Cmds};
use crate::tabs::{
    BAR_BUTTON, BAR_ICON, CLOSE_ICON, HEIGHT, LIVE_MARK_WIDTH, LiveBadge, NEW_TAB_MARGIN_X, OVERFLOW_WIDTH, TAB_BUTTON, TAB_BUTTON_RADIUS,
    TAB_CLOSE_RIGHT, TAB_FONT, TAB_GAP, TAB_LABEL_MAX, TAB_PAD_X, TabMotion, VisibleTab, live_badge, preferred_width, tab_drop_position,
    tab_layout,
};
use crate::title_bar;
use crate::widgets::icon::{self, Icon};
use crate::widgets::{MenuRow, pt, rgba, tokens};
use crate::{CanvasView, panels};

/// 一个文档标签的数据快照：每帧从 `Session.docs` 现读，照右栏 `PanelData` 的模式。
#[derive(Clone, Debug)]
pub(crate) struct TabDoc {
    /// `Session.docs` 里的真实下标（`doc.switch` / `file.close` 都用它；重排一律按 uid）。
    pub index: usize,
    pub uid: u64,
    pub name: String,
    pub board: bool,
    pub dirty: bool,
    /// 上游 `live_mark` / `live_lost`：这个文档自己的 live 会话，每帧从 Session 现读。
    pub live: LiveBadge,
}

/// 这个窗口该显示的文档：**按 `Windows` 归属过滤**（A16）。
///
/// 每个 OS 窗口挂同一个 `CanvasView`，所以「有哪些文档」是共享的，但「这个窗口显示哪几个」
/// 由 `windows.of(uid)` 决定：主窗口显示不属于任何额外窗口的文档，额外窗口只显示自己 `docs`
/// 里的。拖出标签后归属立刻变化，于是**原窗口的标签消失、新窗口只有那一个**。
pub(crate) fn read_docs(
    session: &figcraft_engine::Session,
    windows: &crate::windows::Windows,
    current: u64,
    hidden: &[u64],
) -> Vec<TabDoc> {
    session
        .docs
        .iter()
        .enumerate()
        .filter(|(_, d)| windows.of(d.uid) == current && !hidden.contains(&d.uid))
        .map(|(index, d)| {
            // `live_of` 给的是「这个 uid 的会话」，但会话还带着它同步的 `doc_uid`；
            // 两个条件都满足才在标签上画徽标（上游 tabs.rs:688/694 的 filter）。
            let live = session.live_of(d.uid);
            TabDoc {
                index,
                uid: d.uid,
                name: d.doc.name.clone(),
                board: d.doc.kind == figcraft_doc::FileKind::Board,
                dirty: d.is_dirty(),
                live: live_badge(live.shared(), live.disconnected(), live.doc_uid, d.uid),
            }
        })
        .collect()
}

/// 标签条上的一个条目：文件文档或演示标签。
///
/// ⚠️ 这是标签条唯一的「身份」类型：排布、溢出、拖拽、点击、关闭都按它分发，
/// 不再给演示标签单开一套几何 / 行为（用户 2026-11-17：演示标签必须进原来的标签系统）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabKey {
    Doc { index: usize },
    Present { id: u64 },
    /// 「+」打开的空白标签页（内容 = 首页那套创建/导入入口 + 当前文件），窗口私有。
    New { id: u64 },
}

/// 标签条的**稳定身份**：`TabKey::Doc` 里的 `Session.docs` 下标会随增删漂移，
/// 不能存进跨帧的顺序表；这里存 uid / 演示 id（用户 2026-11-17：统一顺序）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabId {
    Doc(u64),
    Present(u64),
    New(u64),
}

impl TabKey {
    fn is_doc(self) -> bool {
        matches!(self, TabKey::Doc { .. })
    }
}

/// 拖拽状态由**跨帧共享的 Rc** 持有，而不是在 `drag_router` 的回调里 `Entity::update`：
/// 根视图的 `cx.listener` 已经借出了 `CanvasView`，回调里再 `me.update(cx, …)` 会二次借用同一
/// entity 并 panic。现有 `panels/extra.rs::dev-split` 也是用共享状态 + `wake()` 这个模式。
#[derive(Clone, Default)]
pub(crate) struct TabDragState(Rc<RefCell<DragInner>>);

#[derive(Default)]
struct DragInner {
    current: Option<TabDrag>,
    /// 本帧可见标签的窗口 x 区间（`tab_drop_position` 的输入）。
    visible: Vec<(TabKey, (f32, f32))>,
    /// 本帧算出的落点；松手时**原样提交**：判定与画面必须同源，
    /// 否则「看到了让位、松手却回原位」。`usize` 是同类标签里的目标槽位
    /// （文档 = 目标文档下标，演示 = 目标槽位）。
    to: Option<(TabKey, usize)>,
    /// 本帧被拖标签是否已整体拖出标签条（左右越界）→ 松手变成新窗口（仅文档）。
    detach: bool,
    /// 标签条可见区间的窗口 x 范围（`tabs_x0 .. tabs_x0 + tabs_w`）。
    /// 松手时用指针**当场**判越界：只依赖渲染帧的 `detach` 会漏掉「指针移出后立刻松开、
    /// 中间没来得及重绘」这一拍，表现成拖出去却只是重排（用户 2026-11-17）。
    strip: (f32, f32),
}

struct TabDrag {
    from: TabKey,
    start: Point<Pixels>,
    pointer: Point<Pixels>,
    /// 超过 3pt 死区才算拖拽，否则这一下仍是「点击切换标签」。
    moved: bool,
}

impl TabDragState {
    fn begin(&self, from: TabKey, at: Point<Pixels>) {
        *self.0.borrow_mut() = DragInner {
            current: Some(TabDrag { from, start: at, pointer: at, moved: false }),
            visible: Vec::new(),
            to: None,
            detach: false,
            strip: (0.0, f32::MAX),
        };
    }
    fn set_strip(&self, x0: f32, x1: f32) {
        self.0.borrow_mut().strip = (x0, x1);
    }
    fn set_visible(&self, visible: Vec<(TabKey, (f32, f32))>) {
        self.0.borrow_mut().visible = visible;
    }
    fn set_to(&self, to: Option<(TabKey, usize)>) {
        self.0.borrow_mut().to = to;
    }
    fn set_detach(&self, detach: bool) {
        self.0.borrow_mut().detach = detach;
    }
    /// 拖拽中的 `(被拖标签, 指针相对按下点的 x 位移, 指针 x)`；没超过 3pt 死区时是 `None`
    /// （这时仍然是「点击切换」而不是拖拽）。
    fn offset(&self) -> Option<(TabKey, f32, f32)> {
        let b = self.0.borrow();
        let d = b.current.as_ref()?;
        if !d.moved {
            return None;
        }
        Some((d.from, f32::from(d.pointer.x) - f32::from(d.start.x), f32::from(d.pointer.x)))
    }
}

/// 标签条上、由拖拽回调**延迟提交**的操作。
///
/// ⛔ 拖拽回调（`drag_router` 的 on_mouse_up）里**绝对不能** `Entity::update`：那是二次租借
/// 同一个 `CanvasView`，GPUI 直接 panic → 进程 abort。表现就是「拖出窗口时闪退」
/// （用户 2026-11-17）。回调只把操作记进共享队列，由 `CanvasView::render` 在持有
/// `&mut self` 的那一帧直接应用（`CanvasView::apply_tab_ops`）。
#[derive(Clone, Copy, Debug)]
pub(crate) enum TabOp {
    /// 标签挪到本窗口统一顺序的第 `slot` 位：`slot` 是「除被拖标签和空白页之外」的目标槽位，
    /// 文档和演示标签共用一条顺序（用户 2026-11-17：两类标签按统一顺序自由换位）。
    Reorder { id: TabId, slot: usize },
    /// 演示标签拖出成独立窗口：把 tab 连同播放器交给新窗口（见 `Windows::pending_present`）。
    DetachPresent { id: u64, uid: u64 },
}

/// 跨帧的标签条状态；`home` / `motion` / 拖拽都由 `CanvasView` 持有。
#[derive(Clone)]
pub(crate) struct TabState {
    pub home: bool,
    pub motion: TabMotion,
    pub drag: TabDragState,
    /// 本窗口标签条的统一顺序（文档 / 演示 / 空白混排）。窗口私有，关掉的标签下一帧自动剔除。
    pub order: Vec<TabId>,
    /// 设计标签已关、但数据被预览标签接管的文档 uid：不画设计标签，切进预览时照常绑定
    /// （用户 2026-11-17：关掉设计稿后把数据直接移交给预览）。
    pub hidden: Vec<u64>,
    /// 拖拽回调记下的待应用操作（见 [`TabOp`]）。
    pub pending: Rc<RefCell<Vec<TabOp>>>,
    started: Option<Instant>,
}

impl Default for TabState {
    fn default() -> Self {
        Self {
            home: false,
            motion: TabMotion::default(),
            drag: TabDragState::default(),
            order: Vec::new(),
            hidden: Vec::new(),
            pending: Default::default(),
            started: None,
        }
    }
}

impl TabState {
    fn now(&mut self) -> f64 {
        self.started.get_or_insert_with(Instant::now).elapsed().as_secs_f64()
    }
}

/// 上游 `icons::file_mark(FileKind)`：Design → 钢笔尖，Board → 网格。
fn file_mark(board: bool) -> Icon {
    if board { Icon::ViewGrid } else { Icon::Pen }
}

/// 标签名字的宽度上限：可用宽度扣掉 72pt 的图标 / 间距留白（上游 `paint_name` 的 `max_w`）。
fn name_width(width: f32) -> f32 {
    (width - 72.0).max(0.0)
}

/// 标签条上的一次拖拽：注册到窗口级 `drag_router`（项目铁律：GPUI 0.2.1 无鼠标捕获，
/// 跨元素拖拽必须走覆盖整窗的根 `on_mouse_move` / `on_mouse_up`）。
fn begin_tab_drag(state: &TabDragState, cmds: &Cmds, pending: &Rc<RefCell<Vec<TabOp>>>, from: TabKey, uid: u64, at: Point<Pixels>) {
    state.begin(from, at);
    let on_move = state.clone();
    let on_end = state.clone();
    let wake = cmds.clone();
    let wake_end = cmds.clone();
    let commit = pending.clone();
    crate::drag_router::begin(
        "tab",
        move |pos, _pressed, _window, _cx| {
            if let Some(d) = on_move.0.borrow_mut().current.as_mut() {
                let dx = f32::from(pos.x) - f32::from(d.start.x);
                let dy = f32::from(pos.y) - f32::from(d.start.y);
                d.pointer = pos;
                if dx.hypot(dy) > 3.0 {
                    d.moved = true;
                }
            }
            wake.wake();
        },
        move |_window, _cx| {
            let (from, pointer_x, pointer_y, to, detach, strip, moved) = {
                let b = on_end.0.borrow();
                let Some(d) = b.current.as_ref() else { return };
                (d.from, f32::from(d.pointer.x), f32::from(d.pointer.y), b.to, b.detach, b.strip, d.moved)
            };
            on_end.0.borrow_mut().current = None;
            // A16：标签拖出标签条下方 2 行，或**左右整体拖出标签条**，都脱离成独立 OS 窗口
            // （用户 2026-11-16：向左/向右超出范围也要变新窗口）。演示标签同样可以拖出去，
            // 由新窗口接管它的源文档与播放器（用户 2026-11-17）。
            // `detach` 来自渲染帧，`pointer_x` 越界是当场判定，两者取或。
            let out = pointer_x < strip.0 || pointer_x > strip.1;
            if moved && (pointer_y > HEIGHT * 3.0 || detach || out) {
                match from {
                    TabKey::Doc { index } => {
                        wake_end.push(Cmd { id: "window.new", params: Some(json!({ "index": index })) });
                    }
                    TabKey::Present { id } => {
                        commit.borrow_mut().push(TabOp::DetachPresent { id, uid });
                    }
                    // 「+」空白页不参与拖拽（见 `on_mouse_down` 的过滤）。
                    TabKey::New { .. } => {}
                }
                wake_end.wake();
                return;
            }
            // 落点用**渲染时算好的那个**（见 `tab_strip`）：拖拽画面和松手结果必须一致。
            // 文档和演示标签共用一条统一顺序，所以不再按「同类」过滤；重排记进共享队列，
            // 下一帧由 `CanvasView::apply_tab_ops` 直接改状态（⛔ 不能在这里 `Entity::update`，会 panic 闪退）。
            if moved && let Some((_, slot)) = to {
                let id = match from {
                    TabKey::Doc { .. } => TabId::Doc(uid),
                    TabKey::Present { id } => TabId::Present(id),
                    TabKey::New { .. } => return,
                };
                commit.borrow_mut().push(TabOp::Reorder { id, slot });
            }
            wake_end.wake();
        },
    );
}

/// 溢出菜单：所有文档（含未显示的）都能从这里切换，勾选当前文档。
fn open_overflow_menu(
    me: &Entity<CanvasView>,
    _cmds: &Cmds,
    docs: &[TabDoc],
    present: &[(u64, String, bool, u64)],
    active: usize,
    home: bool,
    at_x: f32,
    cx: &mut App,
) {
    // 文档和演示标签共用这个溢出菜单；演示行加 ▶ 前缀，避免与同名文档行撞 label。
    let mut labels: Vec<(String, Option<u64>, u64)> =
        docs.iter().map(|d| (if d.dirty { format!("{} •", d.name) } else { d.name.clone() }, Some(d.uid), 0)).collect();
    for (id, name, _, _) in present {
        labels.push((format!("▶ {name}"), None, *id));
    }
    let rows: Vec<MenuRow> = labels
        .iter()
        .map(|(label, uid, pid)| {
            let checked = match uid {
                Some(_) => !home && docs.iter().any(|d| d.index == active && Some(d.uid) == *uid),
                None => present.iter().any(|(id, _, on, _)| *on && id == pid),
            };
            MenuRow::new(label.clone()).checked(Some(checked))
        })
        .collect();
    let me_pick = me.clone();
    let on_pick: Rc<dyn Fn(&str, &mut Window, &mut App)> = Rc::new(move |label: &str, window, cx| {
        let Some((_, uid, pid)) = labels.iter().find(|(l, _, _)| l == label) else { return };
        me_pick.update(cx, |cv, cx| {
            cv.bind_window_document(window, true, cx);
            match uid {
                Some(uid) => cv.select_doc(*uid, cx),
                None => {
                    cv.switch_present_tab(*pid);
                    cx.notify();
                }
            }
        });
    });
    // 通用 Menu 自己按「锚点底边 + 4pt」定位；给 40pt 高的锚点正好落在标签条下方 4pt。
    let anchor = Bounds { origin: Point::new(px(at_x), px(0.0)), size: size(px(OVERFLOW_WIDTH), px(HEIGHT)) };
    me.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), rows, on_pick, cx));
}

/// Figma 桌面壳标签页的颜色语义 → 项目现有 theme token 的映射（1:1 语义，不写死色值）。
///
/// `desktop_shell.css` 用的是 `--color-*` 语义变量，这里逐条对到 `theme::Tokens`：
///   `--color-text`            → t.text           （主文字）
///   `--color-text-secondary`  → t.text_secondary （次文字 / 普通标签文字）
///   `--color-icon-brand`      → t.accent         （hover/active 图标）
///   `--color-bghovertransparent`→ t.hover        （标签 hover 底）
///   `--color-bg`              → t.panel          （active 标签不透明底）
///   `--color-bordertranslucent`→ t.border        （标签分隔线 / active 底线）
/// 按钮的 hover/active 分别用 t.hover / t.pressed。这样换主题 / 换品牌色时标签自动跟随。
struct TabInk {
    text: crate::canvas::geom::Color32,
    secondary: crate::canvas::geom::Color32,
    brand: crate::canvas::geom::Color32,
    hover: crate::canvas::geom::Color32,
    pressed: crate::canvas::geom::Color32,
    panel: crate::canvas::geom::Color32,
    border: crate::canvas::geom::Color32,
}

fn tab_ink() -> TabInk {
    let t = tokens();
    TabInk { text: t.text, secondary: t.text_secondary, brand: t.accent, hover: t.hover, pressed: t.pressed, panel: t.panel, border: t.border }
}

/// 一条 32pt 方形按钮（新标签 / 主题）：上游 `BAR_BUTTON=32` / `BAR_ICON=20`。
/// `IconButton` 内部把字形锁在 16pt，这里按 #1729 的规范用 Kit 的 `icon::paint` 画 20pt。
fn bar_button(icon: Icon, t: crate::theme::Tokens, id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .w(pt(BAR_BUTTON))
        .h(pt(BAR_BUTTON))
        .flex()
        .items_center()
        .justify_center()
        .rounded(pt(5.0))
        .hover(move |s| s.bg(rgba(t.hover)))
        .child(icon::paint(icon, rgba(t.icon), BAR_ICON))
}

/// Figma `.tab_bar--newTabButtonUI3`：24×24、圆角 5、左右外边距 7，
/// 图标 text-secondary、hover 变 text 并铺 hover 底、active 铺 pressed 底。
/// 颜色全部走 tab_ink() 映射，不写死色值。
fn new_tab_button(id: &'static str) -> Stateful<Div> {
    let ink = tab_ink();
    let (normal, hover) = (ink.secondary, ink.hover);
    let ink_text = ink.text;
    div()
        .id(id)
        .flex_none()
        .w(pt(TAB_BUTTON))
        .h(pt(TAB_BUTTON))
        .mx(pt(NEW_TAB_MARGIN_X))
        .flex()
        .items_center()
        .justify_center()
        .rounded(pt(TAB_BUTTON_RADIUS))
        .hover(move |s| s.bg(rgba(hover)).text_color(rgba(ink_text)))
        .child(icon::paint(Icon::Plus, rgba(normal), TAB_BUTTON - 8.0))
}

/// 画 40pt 标签条，并把本帧的可见标签矩形同步进拖拽状态。
#[allow(clippy::too_many_arguments)]
pub(crate) fn tab_strip(
    state: &mut TabState,
    window: &Window,
    docs: &[TabDoc],
    active: usize,
    // 演示标签（eGUI `app.ui.presents`）：`(id, 显示名, 是否在屏, 源文档 uid)`，画在文件 tab 之后。
    present_tabs: &[(u64, String, bool, u64)],
    // 「+」空白标签页的 id 列表 与 当前激活项。
    new_tabs: &[u64],
    new_active: Option<u64>,
    dark: bool,
    reduce_motion: bool,
    // 标题栏壳留给 TabBar 内容的宽度（已扣掉左留白与右侧窗口按钮）。
    content_w: f32,
    cmds: &Cmds,
    me: &Entity<CanvasView>,
    // 必须是**具体类型**：Rust 2024 的 `impl Trait` 会捕获 `&mut state` 的生命周期，
    // 根视图在后面还要借 `self.tabs` / `self.render_left_panel(...)`，会直接 E0502/E0499。
) -> Stateful<Div> {
    let t = tokens();
    // Figma 标签页语义色 → 项目 token（见 tab_ink）。逐条映射，换主题自动跟随。
    let ink = tab_ink();
    let now = state.now();
    state.motion.begin_frame(crate::windows::current(), docs.iter().map(|d| d.uid), now, reduce_motion);
    // 不再做新标签的渐显：点 `+` 时新标签正好落在指针底下，半透明会一直挂着，
    // 直到鼠标移开触发重绘才恢复（用户 2026-11-16）。标签一律立刻完整显示。
    // 上游还会排除「正在演示」的情况；GPUI 的 present 是覆盖层、不是标签（P2）。
    let home_active = state.home || docs.is_empty();

    // 窗口标题栏壳（`title_bar.rs`）负责左留白 8pt 和右侧 3×46pt 窗口按钮；这里只画
    // TabBar 内容。x 一律用**窗口坐标**（拖拽落点、溢出菜单锚点都是窗口坐标），
    // 标签用相对容器的绝对坐标摆，落差是 `tabs_x0`。
    let content_x0 = title_bar::LEFT_PAD;
    // 少 1pt 的分隔线宽度：标题栏这条竖线要和下面左竖栏的右边框（在 `RAIL_W - 1 .. RAIL_W`）
    // 严丝合缝对齐，否则两条线错开 1pt（用户 2026-11-16）。
    let home_w = (panels::RAIL_W - content_x0 - 1.0).max(0.0);
    let mut x = content_x0 + home_w;
    // 上游 `room = rect.max.x - buttons_w - 70 - x - 34`；`content_w` 已扣掉壳的两侧，
    // 于是等价 `content_w - 70 - home_w - 34`（新标签 / 主题 / 空白区留位）。
    let room = (content_w - 70.0 - home_w - 34.0).max(0.0);
    // ── 统一标签模型（用户 2026-11-17）────────────────────────────────────────────
    // 文件标签和演示标签是**同一种标签**：同一个条目列表、同一套宽度收缩 / 溢出 / 拖拽 /
    // 关闭 / 激活态。演示 tab 不再单开一套几何与回调（之前它什么原有行为都没有）。
    #[derive(Clone)]
    struct StripTab {
        key: TabKey,
        /// 文档 uid（演示标签为 0）；点击切换按 uid，不按下标。
        uid: u64,
        name: String,
        board: bool,
        dirty: bool,
        live: LiveBadge,
        /// 文档：本窗口当前文档；演示：正在演示的那个。
        active: bool,
        on_screen: bool,
    }
    impl StripTab {
        fn id(&self) -> TabId {
            match self.key {
                TabKey::Doc { .. } => TabId::Doc(self.uid),
                TabKey::Present { id } => TabId::Present(id),
                TabKey::New { id } => TabId::New(id),
            }
        }
    }
    let mut entries: Vec<StripTab> = Vec::with_capacity(docs.len() + present_tabs.len());
    for d in docs {
        entries.push(StripTab {
            key: TabKey::Doc { index: d.index },
            uid: d.uid,
            name: d.name.clone(),
            board: d.board,
            dirty: d.dirty,
            live: d.live,
            active: !state.home && new_active.is_none() && d.index == active,
            on_screen: false,
        });
    }
    for (id, name, on_screen, uid) in present_tabs {
        entries.push(StripTab {
            key: TabKey::Present { id: *id },
            uid: *uid,
            name: name.clone(),
            board: false,
            dirty: false,
            live: LiveBadge::default(),
            active: false,
            on_screen: *on_screen,
        });
    }
    for id in new_tabs {
        entries.push(StripTab {
            key: TabKey::New { id: *id },
            uid: 0,
            name: "New Tab".to_string(),
            board: false,
            dirty: false,
            live: LiveBadge::default(),
            active: !state.home && new_active == Some(*id),
            on_screen: false,
        });
    }
    // ── 统一顺序（用户 2026-11-17）────────────────────────────────────────────
    // 文档、演示、空白页在**同一条**顺序里混排：拖预览标签经过设计标签时也要能换位。
    // 每帧先按上帧顺序挑出仍存在的项，再按默认顺序（文档 → 演示 → 空白）追加新出现的。
    let mut ordered: Vec<StripTab> = Vec::with_capacity(entries.len());
    let mut rest = entries;
    for id in state.order.drain(..) {
        if let Some(i) = rest.iter().position(|e| e.id() == id) {
            ordered.push(rest.remove(i));
        }
    }
    ordered.append(&mut rest);
    state.order = ordered.iter().map(|e| e.id()).collect();
    let entries = ordered;

    let preferred: Vec<f32> = entries
        .iter()
        .map(|e| match e.key {
            TabKey::Doc { .. } | TabKey::New { .. } => preferred_width(&e.name, e.live.visible(), e.dirty),
            TabKey::Present { .. } => crate::present::present_tab_width(&e.name),
        })
        .collect();
    let active_tab = entries.iter().position(|e| e.active || e.on_screen);
    let layout = tab_layout(&preferred, room, active_tab);

    // Home 键（active / hover 底色 + 32pt 图标：图标本身等比放大，线宽在 SVG 里按比例
    // 退回去保持视觉粗细不变，用户 2026-11-16）。
    let me_home = me.clone();
    let home_ink = tab_ink();
    let home_normal = home_ink.secondary;
    let home_hot = home_ink.text;
    let home = div()
        .id("tab-home")
        .flex_none()
        .w(pt(home_w))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        // Home 的盒子右边缘要让给 1pt 分隔线，所以内容再右缩 `content_x0`：
        // 这样图标中心落在整条左竖栏（0..RAIL_W）的中心，而不是盒子中心（偏右 4pt）。
        .pr(pt(content_x0))
        .group("tab-home-grp")
        .when(home_active, move |d| d.bg(rgba(t.pressed)))
        .hover(move |s| s.bg(rgba(t.hover)))
        .on_click(move |_e: &ClickEvent, window, cx: &mut App| {
            me_home.update(cx, |cv, cx| {
                // ⛔ 先绑定本窗口：`show_home` 改的是**窗口私有**状态，不绑定就会写进
                // 另一个窗口的 pane（用户 2026-11-16：两个窗口一起切首页）。
                cv.bind_window_document(window, true, cx);
                cv.show_home();
                cx.notify();
            });
        })
        // Figma `.tab--fileBrowserTab`：图标普通 text-secondary，hover/active 变 text。
        // svg 的显式色不继承父 text_color，用双叠层 + group_hover 显隐。
        .child(
            div()
                .relative()
                .flex_none()
                .w(pt(32.0))
                .h(pt(32.0))
                .child(icon::paint(Icon::Home, rgba(if home_active { home_hot } else { home_normal }), 32.0))
                .when(!home_active, |d| {
                    d.child(
                        div()
                            .absolute()
                            .inset_0()
                            .opacity(0.0)
                            .group_hover("tab-home-grp", |s| s.opacity(1.0))
                            .child(icon::paint(Icon::Home, rgba(home_hot), 32.0)),
                    )
                }),
        )
        .into_any_element();

    // 这是 kit `TitleBar::new().child(...)` 的 child：高度 / 底色 / 窗口按钮都由壳负责。
    let mut strip = div()
        .id("tab-strip")
        .flex_1()
        .min_w(pt(0.0))
        .h_full()
        .flex()
        .flex_row()
        .items_center()
        .relative()
        .child(home)
        .child(div().flex_none().w(pt(1.0)).h_full().bg(rgba(t.border)));

    // ── 标签排布 ────────────────────────────────────────────────────────────────
    // 先算每个可见标签的**自然位置**（TabKey, x, 宽）。拖拽重排时标签要互相让位，
    // flex 表达不了「B 整体挪到 A 前面」，所以标签统一放进一个相对容器里按绝对坐标摆。
    let tabs_x0 = x;
    let mut natural: Vec<(TabKey, f32, f32)> = Vec::with_capacity(layout.widths.len());
    let mut nx = tabs_x0;
    for (slot, &w) in layout.widths.iter().enumerate() {
        natural.push((entries[layout.first + slot].key, nx, w));
        // 每个标签自带右侧 1pt 分隔线，宽度已经算进间距里。
        nx += w + 1.0;
    }
    let tabs_w = (nx - tabs_x0).max(0.0);
    state.drag.set_visible(natural.iter().map(|(k, x0, w)| (*k, (*x0, *x0 + *w))).collect());
    state.drag.set_strip(tabs_x0, tabs_x0 + tabs_w);

    // 拖拽中：被拖的标签跟着指针走，其余标签按「插槽」依次让位（用户 2026-11-16）。
    // 判定口径：向右拖时看被拖标签的**右边缘**、向左拖时看**左边缘**（前缘）是否盖住邻居
    // 中点——盖住一半就让位，邻居整体滑到本标签原来的位置（用户 2026-11-17）。
    // 文档和演示标签走同一条顺序，可以互相换位。
    // 松手时提交的就是这里算出的落点（见 `begin_tab_drag` 的 on_end）。
    let (dragged, ghost_x, display_x) = match state.drag.offset() {
        Some((from, dx, _)) if natural.iter().any(|(k, _, _)| *k == from) => {
            let drag_w = natural.iter().find(|(k, _, _)| *k == from).map_or(0.0, |(_, _, w)| *w);
            let natural_x = natural.iter().find(|(k, _, _)| *k == from).map_or(tabs_x0, |(_, x0, _)| *x0);
            // 空白页是临时入口，不参与让位，永远留在末尾。
            let others: Vec<(TabKey, f32, f32)> = natural
                .iter()
                .copied()
                .filter(|(k, _, _)| *k != from && !matches!(k, TabKey::New { .. }))
                .collect();
            // 不再把幽灵夹在标签条内：夹住会让第一/最后一项在两端「拖不动」。
            let ghost = natural_x + dx;
            let edge = if dx >= 0.0 { ghost + drag_w } else { ghost };
            let slot = others.partition_point(|(_, x0, w)| x0 + w / 2.0 < edge);
            let target = if slot < others.len() { Some(others[slot].0) } else { others.last().map(|(k, _, _)| *k) };
            state.drag.set_to(target.map(|t| (t, slot)));
            // 整个标签左右拖出标签条 → 松手脱离成新窗口（用户 2026-11-16；仅文档）。
            state.drag.set_detach(from.is_doc() && (ghost + drag_w < tabs_x0 || ghost > tabs_x0 + tabs_w));
            // 让位后整条标签从 `tabs_x0` 重新顺序摆放：总宽度不变，末尾的空白页位置不受影响。
            let mut display: Vec<(TabKey, f32)> = natural.iter().map(|(k, x0, _)| (*k, *x0)).collect();
            let set = |display: &mut Vec<(TabKey, f32)>, key: TabKey, x: f32| {
                if let Some(d) = display.iter_mut().find(|(k, _)| *k == key) {
                    d.1 = x;
                }
            };
            let mut pos = tabs_x0;
            let mut placed = false;
            for (i, (key, _, w)) in others.iter().enumerate() {
                if i == slot {
                    // 空出来的这一格就是松手后的落点。
                    set(&mut display, from, pos);
                    pos += drag_w + 1.0;
                    placed = true;
                }
                set(&mut display, *key, pos);
                pos += *w + 1.0;
            }
            if !placed {
                set(&mut display, from, pos);
            }
            (Some(from), ghost, display)
        }
        _ => {
            state.drag.set_to(None);
            state.drag.set_detach(false);
            (None, 0.0, natural.iter().map(|(k, x0, _)| (*k, *x0)).collect())
        }
    };

    let mut elements: Vec<(TabKey, gpui::AnyElement)> = Vec::with_capacity(natural.len());
    for (slot, &w) in layout.widths.iter().enumerate() {
        let e = entries[layout.first + slot].clone();
        let key = e.key;
        let is_active = e.active || e.on_screen;
        // Figma 按钮态色（Copy 的 Color32），供下面的 hover/active 闭包 move。
        let ink_hover = ink.hover;
        let ink_panel = ink.panel;
        let ink_text = ink.text;
        // Figma hover 联动：body 作为 group，图标与文字用 group_hover 变蓝/变深。
        let tab_group = SharedString::from(format!("tabgrp-{key:?}"));
        let tab_x = if Some(key) == dragged { ghost_x } else { display_x.iter().find(|(k, _)| *k == key).map_or(tabs_x0, |(_, x)| *x) };

        let me_switch = me.clone();
        let me_down = me.clone();
        let me_close_present = me.clone();
        let cmds_close = cmds.clone();
        let cmds_middle = cmds.clone();
        let drag = state.drag.clone();
        let drag_pending = state.pending.clone();
        let drag_cmds = cmds.clone();
        let uid = e.uid;
        let doc_index = match key {
            TabKey::Doc { index } => index,
            TabKey::Present { .. } | TabKey::New { .. } => 0,
        };
        // `w` 含右侧 1pt 分隔线：body + close 只能占 `w - 1`（close 按 Figma 24pt）。
        let body_w = (w - TAB_BUTTON - TAB_CLOSE_RIGHT - 1.0).max(0.0);
        // 上游 `bidi::paint_name` 的可用宽度也要扣掉 live 标记与未保存点，否则名字会压到徽标上。
        let name_w = (w - if e.live.visible() { LIVE_MARK_WIDTH } else { 0.0 } - if e.dirty { 12.0 } else { 0.0 }).max(0.0);
        let name = crate::bidi::fit(window, &e.name, 12.0, name_width(name_w));
        let badge = e.live;

        let mut body = div()
            .flex()
            .flex_row()
            .items_center()
            .h_full()
            .w(pt(body_w))
            .min_w(pt(0.0))
            .pl(pt(TAB_PAD_X))
            .gap(pt(TAB_GAP))
            .overflow_hidden()
            // 文档 body 的 id 必须先于 on_click 设置（Stateful 才有 on_click）。
            .id(SharedString::from(format!("tab-body-{key:?}")))
            .group(tab_group.clone())
            .on_click(move |_e: &ClickEvent, window, cx: &mut App| {
                me_switch.update(cx, |cv, cx| {
                    cv.bind_window_document(window, true, cx);
                    match key {
                        // 直接按 uid 切（不排队）：多窗口增删文档后 `session.docs` 下标会漂移。
                        TabKey::Doc { .. } => cv.select_doc(uid, cx),
                        // 演示标签：切到它所属的文件 + 恢复播放器。
                        TabKey::Present { id } => {
                            cv.switch_present_tab(id);
                            cx.notify();
                        }
                        // 「+」空白标签页：内容就是首页（创建 / 导入 + 当前文件）。
                        TabKey::New { id } => {
                            cv.select_new_tab(id);
                            cx.notify();
                        }
                    }
                });
            })
            .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx: &mut App| {
                // 「+」空白页不是可重排的实体，别让它进入拖拽模型。
                if !matches!(key, TabKey::New { .. }) {
                    // 空白页是临时入口：一按别的标签（哪怕接着就要拖出去）就把它关掉，
                    // 并立刻切到被按的标签上（用户 2026-11-17 的设计要求）。
                    me_down.update(cx, |cv, cx| {
                        cv.bind_window_document(window, true, cx);
                        match key {
                            TabKey::Doc { .. } => cv.select_doc(uid, cx),
                            TabKey::Present { id } => cv.switch_present_tab(id),
                            TabKey::New { .. } => {}
                        }
                        cx.notify();
                    });
                    begin_tab_drag(&drag, &drag_cmds, &drag_pending, key, uid, e.position);
                }
            });
        // A16：标签右键的 "Open in new window"（egui `tabs.rs` 同款菜单项）。仅文档。
        if key.is_doc() {
            body = body.on_mouse_down(MouseButton::Right, {
                let me_menu = me.clone();
                let cmds_menu = cmds.clone();
                move |e: &MouseDownEvent, _window, cx: &mut App| {
                    // 额外窗口里多一条「塞回主窗口」：拖出去之后没有办法拖回来（GPUI 没有跨窗口
                    // 拖拽），所以给一个显式入口。
                    let extra = crate::windows::current() != 0;
                    let mut rows = vec![MenuRow::new("Open in new window")];
                    if extra {
                        rows.push(MenuRow::new("Move to main window"));
                    }
                    let cmds_pick = cmds_menu.clone();
                    let on_pick: Rc<dyn Fn(&str, &mut Window, &mut App)> = Rc::new(move |label: &str, _window, _cx| {
                        let id = if label == "Move to main window" { "window.merge" } else { "window.new" };
                        cmds_pick.push(Cmd { id, params: Some(json!({ "index": doc_index })) });
                    });
                    let anchor = Bounds { origin: e.position, size: size(px(1.0), px(1.0)) };
                    me_menu.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), rows, on_pick, cx));
                }
            });
        }
        let glyph = match key {
            TabKey::Doc { .. } => file_mark(e.board),
            TabKey::Present { .. } => Icon::Play,
            TabKey::New { .. } => Icon::Plus,
        };
        body = body
            // Figma `.tab--tab`：图标普通 = text-secondary，active = icon-brand；
            // 非 active 标签 hover 时图标也变品牌蓝（叠一层 group_hover 显隐）。
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w(pt(BAR_ICON))
                    .h(pt(BAR_ICON))
                    .child(icon::paint(glyph, rgba(if is_active { ink.brand } else { ink.secondary }), BAR_ICON))
                    .when(!is_active, |d| {
                        d.child(
                            div()
                                .absolute()
                                .inset_0()
                                .opacity(0.0)
                                .group_hover(tab_group.clone(), |s| s.opacity(1.0))
                                .child(icon::paint(glyph, rgba(ink.brand), BAR_ICON)),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(pt(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(pt(TAB_FONT))
                    .text_color(rgba(if is_active { ink.text } else { ink.secondary }))
                    .when(!is_active, |d| d.group_hover(tab_group.clone(), |s| s.text_color(rgba(ink_text))))
                    .child(SharedString::from(name)),
            )
            .when(e.dirty, |d| d.child(div().flex_none().text_size(pt(TAB_FONT)).text_color(rgba(ink.text)).child("•")))
            .when(badge.visible(), move |d| {
                // 上游 `paint_tab`：先判 lost（danger），否则按 shared 画 online::mark。
                let (glyph, color) = if badge.lost {
                    (Icon::LiveDisconnected, t.danger)
                } else if badge.shared == Some(true) {
                    (Icon::LinkOn, t.success)
                } else {
                    (Icon::LinkOff, t.text_secondary)
                };
                d.child(icon::paint(glyph, rgba(color), BAR_ICON))
            });

        // Figma `.tab--button`：24×24、圆角 5px、图标 text-secondary；hover 铺 hover 色，
        // active 铺 pressed 色（映射到项目 token）。独占 24pt，不压在可拖拽的 body 上。
        let mut close = div()
            .flex_none()
            .w(pt(TAB_BUTTON))
            .h(pt(TAB_BUTTON))
            .flex()
            .items_center()
            .justify_center()
            .rounded(pt(TAB_BUTTON_RADIUS))
            .mr(pt(TAB_CLOSE_RIGHT))
            .hover(move |s| s.bg(rgba(ink_hover)));
        match key {
            TabKey::Doc { index } => {
                // 有预览标签在借用这份文档时，关设计标签 = 把数据移交给预览：不真关文档，
                // 只把设计标签藏起来（用户 2026-11-17）。否则按引擎流程关，脏了会弹 `dialog.unsaved`。
                let keeps_preview = present_tabs.iter().any(|(_, _, _, uid)| *uid == e.uid);
                let close_uid = e.uid;
                let me_left = me_close_present.clone();
                let me_mid = me_close_present;
                close = close
                    .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _window, cx: &mut App| {
                        if keeps_preview {
                            me_left.update(cx, |cv, cx| {
                                cv.close_doc_to_present(close_uid, cx);
                                cx.notify();
                            });
                        } else {
                            cmds_close.push(Cmd { id: "file.close", params: Some(json!({ "index": index, "force": e.modifiers.alt })) });
                        }
                        cx.stop_propagation();
                    })
                    .on_mouse_down(MouseButton::Middle, move |e: &MouseDownEvent, _window, cx: &mut App| {
                        if keeps_preview {
                            me_mid.update(cx, |cv, cx| {
                                cv.close_doc_to_present(close_uid, cx);
                                cx.notify();
                            });
                        } else {
                            cmds_middle.push(Cmd { id: "file.close", params: Some(json!({ "index": index, "force": e.modifiers.alt })) });
                        }
                        cx.stop_propagation();
                    });
            }
            TabKey::Present { id } => {
                close = close.on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _window, cx: &mut App| {
                    me_close_present.update(cx, |cv, cx| {
                        cv.close_present_tab(id);
                        cx.notify();
                    });
                    cx.stop_propagation();
                });
            }
            TabKey::New { id } => {
                close = close.on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _window, cx: &mut App| {
                    me_close_present.update(cx, |cv, cx| {
                        cv.close_new_tab(id);
                        cx.notify();
                    });
                    cx.stop_propagation();
                });
            }
        }
        let close = close.child(icon::paint(Icon::Close, rgba(ink.secondary), CLOSE_ICON));

        // Figma `.tab--tab` + `.tab--activeTab`（desktop_shell.css）语义 → 项目 token：
        //   普通：透明底、右侧 1px `--color-bordertranslucent` → t.border；
        //   hover：底 `--color-bghovertransparent` → t.hover（文字/图标态已在 body 上）；
        //   active：底 `--color-bg` → t.panel + 底部 1px 弱线（Figma 的
        //     color-mix(opposite 10%, bg 90%) 亮=#E6E6E6/暗=#444，正好是 t.border）+ padding-top 1。
        // active 标签 hover 不改底色（Figma `.activeTab:hover` 仍是 bg），故 hover 只挂非 active。
        let tab = div()
            .absolute()
            .left(px(tab_x - tabs_x0))
            .top(pt(0.0))
            .w(pt(w))
            .h_full()
            .flex()
            .flex_row()
            .items_center()
            .border_r_1()
            .border_color(rgba(ink.border))
            // active 只铺纯色、铺满整个标签高度：不再 border_b_1 + pt(1)（那会挤内容盒、
            // 在白底顶/底露出标题栏灰底 →「白底没到底、非 active 又到底」的 bug）。
            .when(is_active, move |d| d.bg(rgba(ink_panel)))
            .when(!is_active, move |d| d.hover(move |s| s.bg(rgba(ink_hover))))
            .child(body)
            .child(close);
        elements.push((key, tab.into_any_element()));
    }

    // 被拖的标签最后画：要压在其它标签之上，否则经过别的标签时会被盖住。
    if let Some(from) = dragged
        && let Some(pos) = elements.iter().position(|(k, _)| *k == from)
    {
        let dragged_el = elements.remove(pos);
        elements.push(dragged_el);
    }
    let mut layer = div().relative().flex_none().h_full().w(pt(tabs_w));
    for (_, el) in elements {
        layer = layer.child(el);
    }

    strip = strip.child(layer);
    x = tabs_x0 + tabs_w;

    // 溢出 chevron + 菜单（菜单的防溢出 / 400pt 内部滚动由通用 `menu_popup_in` 负责）。
    if layout.overflow {
        let chevron_x = x;
        let me_menu = me.clone();
        let cmds_menu = cmds.clone();
        let docs_menu = docs.to_vec();
        let present_menu = present_tabs.to_vec();
        let home_menu = state.home;
        strip = strip.child(
            div()
                .id("tab-overflow")
                .flex_none()
                .w(pt(OVERFLOW_WIDTH))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .hover(move |s| s.bg(rgba(t.hover)))
                .on_click(move |_e: &ClickEvent, _window, cx: &mut App| {
                    open_overflow_menu(&me_menu, &cmds_menu, &docs_menu, &present_menu, active, home_menu, chevron_x, cx);
                })
                .child(icon::paint(Icon::ChevronDown, rgba(t.icon), BAR_ICON)),
        );
    }

    // 新标签 +：开一个空白标签页（首页那套创建 / 导入入口），**不再直接 new 画板**。
    let me_new = me.clone();
    strip = strip.child(new_tab_button("tab-new").on_click(move |_e: &ClickEvent, window, cx: &mut App| {
        // 新标签属于**当前窗口**：先绑定，否则会写进最后渲染的那个窗口的 pane。
        me_new.update(cx, |cv, cx| {
            cv.bind_window_document(window, true, cx);
            cv.open_new_tab();
            cx.notify();
        });
    }));

    // 空白区 = 系统标题栏的拖拽区（上游 `drag_free_space()` 的 `StartDrag`）。
    // 壳整条已标 `WindowControlArea::Drag`；这里再标一次，保证这块 flex_1 hitbox
    // 命中时平台仍按 HTCAPTION 处理，标签 / Home / 按钮的子 hitbox 不受影响。
    strip = strip.child(div().flex_1().min_w(pt(0.0)).h_full().window_control_area(WindowControlArea::Drag));

    // 主题切换（engine `view.themeLight/Dark` 改 prefs，render 每帧据此重建 Tokens）。
    let cmds_theme = cmds.clone();
    strip = strip.child(div().flex_none().h_full().flex().items_center().pr(pt(10.0)).child(
        bar_button(if dark { Icon::Sun } else { Icon::Moon }, t, "tab-theme").on_click(move |_e: &ClickEvent, _window, _cx| {
            cmds_theme.push(Cmd { id: if dark { "view.themeLight" } else { "view.themeDark" }, params: Some(json!({})) });
        }),
    ));

    strip
}
