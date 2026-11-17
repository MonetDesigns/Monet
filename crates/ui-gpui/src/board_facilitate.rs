//! 画板协作条（照搬 `ui-egui/src/canvas/board_facilitate.rs`）：画布右上角的计时器 + 投票
//! 药丸、它们的弹出面板，以及页面对象上的投票徽标。所有改动都走 `board.timer` /
//! `board.vote`，与 egui 版同一套命令。
//!
//! 原文的弹层是 egui `Area` + `ui.data` 的临时状态；GPUI 里跨帧状态放在 [`State`]，
//! 由视图持有并在事件里修改。

use std::cell::RefCell;
use std::rc::Rc;

use figcraft_doc::BoardTimer;
use figcraft_engine::cmd::facilitate::{self, BADGE_PX};
use gpui::{AnyElement, Div, IntoElement, Pixels, SharedString, canvas, div, point, prelude::*, px};
use serde_json::{Value, json};

use crate::CanvasApp;
use crate::canvas::geom::*;
use crate::canvas::painter::Painter;
use crate::panels::{Cmd, Cmds};
use crate::widgets::{Icon, icon, rgba, tokens};

/// 药丸里一个格子的边长（egui `CELL`）。
const CELL: f32 = 28.0;
/// 药丸高度：格子 + 上下 4 的内边距。
const HEIGHT: f32 = CELL + 8.0;
/// 计时器预设（分钟）。
const PRESETS: [u32; 5] = [1, 3, 5, 10, 15];
/// 计时钟文字宽度：按「0:00」留够，秒数变化时药丸不跳。
const CLOCK_W: f32 = 40.0;
/// 投票范围（`board.vote {on}` 的取值）。
const SCOPES: [(&str, &str); 3] = [("all", "All objects"), ("stickies", "Sticky notes"), ("selection", "Selection")];

/// 药丸弹层的跨帧状态。
pub(crate) struct State {
    pub timer_open: bool,
    pub vote_open: bool,
    pub results_open: bool,
    /// 计时器弹层里的自定义分钟数。
    pub minutes: u32,
    /// 投票弹层里的「每人票数」。
    pub votes: u32,
    /// 投票范围（`SCOPES` 的下标）。
    pub on: usize,
    /// 结果弹层里看的是第几场投票（0 = 最新）。
    pub pick: usize,
}

impl Default for State {
    fn default() -> Self {
        Self { timer_open: false, vote_open: false, results_open: false, minutes: 5, votes: 3, on: 0, pick: 0 }
    }
}

/// 药丸上投票部分显示什么。
enum VoteView {
    Off { results: bool },
    Running { left: u32 },
}

fn vote_view(app: &CanvasApp) -> VoteView {
    let Ok(doc) = app.session.doc() else { return VoteView::Off { results: false } };
    match &doc.vote {
        Some(vote) if !vote.ended => {
            let used: u32 = facilitate::mine(doc, &facilitate::voter(&app.session, &Value::Null)).iter().map(|m| m.1).sum();
            VoteView::Running { left: vote.per_person.saturating_sub(used) }
        }
        ended => VoteView::Off { results: ended.is_some() || !doc.vote_history.is_empty() },
    }
}

/// 药丸上的小图标。`ui-egui` 的 Timer / Vote / Pause 在 GPUI 的图标表里没有对应字形，
/// 这里按同一 16 格风格自绘（圆 + 指针 / 圆 + 勾 / 双竖条）。
#[derive(Clone, Copy)]
enum Glyph {
    Clock,
    Vote,
    Pause,
}

/// 一个圆（四段三次贝塞尔；GPUI 画笔没有椭圆图元）。
fn circle_path(c: gpui::Point<Pixels>, r: f32) -> Option<gpui::Path<Pixels>> {
    let k = 0.5523 * r;
    let x = f32::from(c.x);
    let y = f32::from(c.y);
    let p = |dx: f32, dy: f32| point(px(x + dx), px(y + dy));
    let mut b = gpui::PathBuilder::stroke(px(1.4));
    b.move_to(p(0.0, -r));
    b.cubic_bezier_to(p(r, 0.0), p(k, -r), p(r, -k));
    b.cubic_bezier_to(p(0.0, r), p(r, k), p(k, r));
    b.cubic_bezier_to(p(-r, 0.0), p(-k, r), p(-r, k));
    b.cubic_bezier_to(p(0.0, -r), p(-r, -k), p(-k, -r));
    b.close();
    b.build().ok()
}

fn glyph(kind: Glyph, size: f32, ink: gpui::Rgba) -> impl IntoElement {
    canvas(
        move |_, _, _| (),
        move |bounds, _, window, _| {
            let w = f32::from(bounds.size.width);
            let h = f32::from(bounds.size.height);
            let c = point(bounds.origin.x + px(w / 2.0), bounds.origin.y + px(h / 2.0));
            let r = (w.min(h) / 2.0 - 1.5).max(1.0);
            let x = f32::from(c.x);
            let y = f32::from(c.y);
            let p = |dx: f32, dy: f32| point(px(x + dx), px(y + dy));
            match kind {
                Glyph::Clock => {
                    if let Some(circle) = circle_path(c, r) {
                        window.paint_path(circle, ink);
                    }
                    let mut b = gpui::PathBuilder::stroke(px(1.4));
                    b.move_to(p(0.0, 0.0));
                    b.line_to(p(0.0, -r * 0.55));
                    b.move_to(p(0.0, 0.0));
                    b.line_to(p(r * 0.45, r * 0.2));
                    if let Ok(path) = b.build() {
                        window.paint_path(path, ink);
                    }
                }
                Glyph::Vote => {
                    if let Some(circle) = circle_path(c, r) {
                        window.paint_path(circle, ink);
                    }
                    let mut b = gpui::PathBuilder::stroke(px(1.4));
                    b.move_to(p(-r * 0.5, r * 0.05));
                    b.line_to(p(-r * 0.12, r * 0.42));
                    b.line_to(p(r * 0.55, -r * 0.38));
                    if let Ok(path) = b.build() {
                        window.paint_path(path, ink);
                    }
                }
                Glyph::Pause => {
                    for dx in [-3.2, 3.2] {
                        let quad = gpui::fill(
                            gpui::Bounds { origin: point(px(x + dx - 2.0), px(y - r * 0.7)), size: gpui::size(px(3.2), px(r * 1.4)) },
                            ink,
                        )
                        .corner_radii(gpui::Corners { top_left: px(1.0), top_right: px(1.0), bottom_right: px(1.0), bottom_left: px(1.0) });
                        window.paint_quad(quad);
                    }
                }
            }
        },
    )
    .size(px(size))
}

fn icon_cell(id: impl Into<gpui::ElementId>, el: impl IntoElement) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .size(px(CELL))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(tokens().hover)))
        .child(el)
}

fn divider() -> impl IntoElement {
    div().flex_none().w(px(1.0)).h(px(20.0)).mx(px(2.0)).bg(rgba(tokens().border))
}

fn label(text: impl Into<SharedString>, secondary: bool) -> impl IntoElement {
    let t = tokens();
    div()
        .text_size(px(12.0))
        .text_color(rgba(if secondary { t.text_secondary } else { t.text }))
        .child(text.into())
}

/// 弹层外壳（与菜单同一套观感）。
fn shell(width: f32) -> Div {
    let t = tokens();
    div()
        .w(px(width))
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .rounded(px(10.0))
        .bg(rgba(t.menu))
        .border_1()
        .border_color(rgba(t.border))
        .shadow(vec![gpui::BoxShadow {
            color: rgba(t.shadow).into(),
            offset: gpui::point(px(0.0), px(3.0)),
            blur_radius: px(14.0),
            spread_radius: px(0.0),
        }])
        .occlude()
}

fn title(text: impl Into<SharedString>) -> impl IntoElement {
    let t = tokens();
    div()
        .h(px(18.0))
        .flex()
        .items_center()
        .text_size(px(12.0))
        .font_weight(crate::panels::UI_FONT_REGULAR)
        .text_color(rgba(t.menu_text))
        .child(text.into())
}

/// 一个小按钮（药丸弹层里的预设 / Start / Results 这类）。
fn small_button(id: impl Into<gpui::ElementId>, text: impl Into<SharedString>, primary: bool) -> gpui::Stateful<Div> {
    let t = tokens();
    div()
        .id(id)
        .h(px(24.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .cursor_pointer()
        .text_size(px(12.0))
        .text_color(rgba(if primary { t.accent_on } else { t.text }))
        .bg(rgba(if primary { t.accent } else { t.hover }))
        .hover(|s| s.opacity(0.9))
        .child(text.into())
}

/// 药丸本体：计时器 + 分隔线 + 投票，右上角浮在画布上。
pub(crate) fn pill(app: &CanvasApp, cmds: Cmds, st: &Rc<RefCell<State>>) -> Option<AnyElement> {
    let Ok(doc) = app.session.doc() else { return None };
    if !figcraft_engine::board::is_board(doc) {
        return None;
    }
    let timer = doc.timer;
    let vote = vote_view(app);
    let editable = app.session.active_read_only().is_none();
    let now = facilitate::now_ms();
    let t = tokens();

    let mut row = div().flex().flex_row().items_center().gap(px(2.0));

    // ── 计时器 ────────────────────────────────────────────────────────────────
    match timer {
        None => {
            row = row.child(timer_button(st, editable));
        }
        Some(timer) => {
            let state = facilitate::timer_state(Some(timer), now);
            let text = facilitate::clock(timer.left(now));
            let color = match state {
                "done" => t.danger,
                "paused" => t.text_secondary,
                _ => t.text,
            };
            row = row
                .child(div().w(px(18.0)).flex().items_center().justify_center().child(glyph(Glyph::Clock, 16.0, rgba(color))))
                .child(
                    div()
                        .w(px(CLOCK_W))
                        .h(px(CELL))
                        .flex()
                        .items_center()
                        .text_size(px(13.0))
                        .font_weight(crate::panels::UI_FONT_REGULAR)
                        .text_color(rgba(color))
                        .child(SharedString::from(text)),
                );
            if editable {
                if state != "done" {
                    let (kind, action) = match timer {
                        BoardTimer::Paused { .. } => ("play", "resume"),
                        BoardTimer::Running { .. } => ("pause", "pause"),
                    };
                    let cmds_toggle = cmds.clone();
                    let el: AnyElement = if kind == "play" {
                        icon::paint(Icon::Play, rgba(t.text), 14.0).into_any_element()
                    } else {
                        glyph(Glyph::Pause, 14.0, rgba(t.text)).into_any_element()
                    };
                    row = row.child(
                        icon_cell("bf-timer-toggle", el).on_click(move |_, _, _| {
                            cmds_toggle.push(Cmd { id: "board.timer", params: Some(json!({ "action": action })) });
                        }),
                    );
                }
                let cmds_add = cmds.clone();
                row = row.child(
                    icon_cell("bf-timer-add", icon::paint(Icon::Plus, rgba(t.text), 14.0)).on_click(move |_, _, _| {
                        cmds_add.push(Cmd { id: "board.timer", params: Some(json!({ "action": "add", "minutes": 1 })) });
                    }),
                );
                let cmds_stop = cmds.clone();
                row = row.child(
                    icon_cell("bf-timer-stop", icon::paint(Icon::Close, rgba(t.text), 14.0)).on_click(move |_, _, _| {
                        cmds_stop.push(Cmd { id: "board.timer", params: Some(json!({ "action": "stop" })) });
                    }),
                );
            }
        }
    }

    row = row.child(divider());

    // ── 投票 ──────────────────────────────────────────────────────────────────
    match vote {
        VoteView::Off { results } => {
            row = row.child(vote_button(st, editable));
            if results {
                let st2 = st.clone();
                row = row.child(
                    small_button("bf-results", figcraft_i18n::tr("Results"), false).on_click(move |_, _, _| {
                        let mut s = st2.borrow_mut();
                        // CodeRabbit PR#17: toggle so a read-only user can close the results popup
                        // (the clear/vote close paths aren't available to them).
                        s.results_open = !s.results_open;
                        s.pick = 0;
                    }),
                );
            }
        }
        VoteView::Running { left } => {
            row = row
                .child(div().w(px(18.0)).flex().items_center().justify_center().child(glyph(Glyph::Vote, 16.0, rgba(t.accent))))
                .child(label(figcraft_i18n::trn("{n} votes left", left as usize, &[]), false));
            if editable {
                let cmds = cmds.clone();
                row = row.child(
                    small_button("bf-end-vote", figcraft_i18n::tr("End vote"), true).on_click(move |_, _, _| {
                        cmds.push(Cmd { id: "board.vote", params: Some(json!({ "action": "end" })) });
                    }),
                );
            }
        }
    }

    let r = app.canvas.rect;
    let mut root = div()
        .id("board-facilitate")
        .relative()
        .h(px(HEIGHT))
        .px(px(4.0))
        .flex()
        .items_center()
        .rounded(px(10.0))
        .bg(rgba(t.panel))
        .border_1()
        .border_color(rgba(t.border))
        .shadow(vec![gpui::BoxShadow {
            color: rgba(t.shadow).into(),
            offset: gpui::point(px(0.0), px(2.0)),
            blur_radius: px(12.0),
            spread_radius: px(0.0),
        }])
        .on_mouse_down(gpui::MouseButton::Left, |_e, _w, cx| cx.stop_propagation())
        .child(row);

    // 弹层挂在药丸下方，右对齐。
    let s = st.borrow();
    let mut popup: Option<AnyElement> = None;
    if s.timer_open && editable {
        popup = Some(timer_popup(cmds.clone(), st.clone()));
    } else if s.vote_open && editable {
        popup = Some(vote_popup(app, cmds.clone(), st.clone()));
    } else if s.results_open {
        popup = Some(results_popup(app, cmds.clone(), st.clone(), editable));
    }
    drop(s);
    if let Some(pop) = popup {
        root = root.child(div().absolute().top(px(HEIGHT + 6.0)).right(px(0.0)).child(pop));
    }

    // 药丸贴画布右上角（层次容器是整窗的，所以按画布矩形摆）。
    Some(
        div()
            .absolute()
            .left(px(f32::from(r.min.x)))
            .top(px(f32::from(r.min.y)))
            .w(px(r.width()))
            .h(px(r.height()))
            .flex()
            .justify_end()
            .items_start()
            .p(px(12.0))
            .child(root)
            .into_any_element(),
    )
}

fn timer_button(st: &Rc<RefCell<State>>, editable: bool) -> AnyElement {
    let st = st.clone();
    let mut b = icon_cell("bf-timer", glyph(Glyph::Clock, 16.0, rgba(tokens().text)));
    if editable {
        b = b.on_click(move |_, _, _| {
            let mut s = st.borrow_mut();
            s.timer_open = !s.timer_open;
            s.vote_open = false;
            s.results_open = false;
        });
    }
    b.into_any_element()
}

fn vote_button(st: &Rc<RefCell<State>>, editable: bool) -> AnyElement {
    let st = st.clone();
    let mut b = icon_cell("bf-vote", glyph(Glyph::Vote, 16.0, rgba(tokens().text)));
    if editable {
        b = b.on_click(move |_, _, _| {
            let mut s = st.borrow_mut();
            s.vote_open = !s.vote_open;
            s.timer_open = false;
            s.results_open = false;
        });
    }
    b.into_any_element()
}

/// 计时器弹层：预设 + 自定义分钟数。
fn timer_popup(cmds: Cmds, st: Rc<RefCell<State>>) -> AnyElement {
    let mut root = shell(240.0).child(title(figcraft_i18n::tr("Timer")));
    let mut presets = div().flex().flex_row().gap(px(4.0));
    for m in PRESETS {
        let cmds = cmds.clone();
        let st2 = st.clone();
        presets = presets.child(
            small_button(("bf-preset", m as usize), figcraft_i18n::trf("{n} min", &[("n", &m)]), false).on_click(move |_, _, _| {
                cmds.push(Cmd { id: "board.timer", params: Some(json!({ "action": "start", "minutes": m })) });
                st2.borrow_mut().timer_open = false;
            }),
        );
    }
    let minutes = st.borrow().minutes.clamp(1, 1440);
    let st1 = st.clone();
    let st2 = st.clone();
    root = root.child(presets).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(label(figcraft_i18n::tr("Minutes"), true))
            .child(
                icon_cell(
                    "bf-minutes-minus",
                    icon::paint(Icon::Minus, rgba(tokens().text), 12.0),
                )
                .size(px(22.0))
                .on_click(move |_, _, _| {
                    let mut s = st1.borrow_mut();
                    s.minutes = s.minutes.clamp(1, 1440).saturating_sub(1).max(1);
                }),
            )
            .child(div().w(px(28.0)).flex().justify_center().child(label(minutes.to_string(), false)))
            .child(icon_cell("bf-minutes-plus", icon::paint(Icon::Plus, rgba(tokens().text), 12.0)).size(px(22.0)).on_click(move |_, _, _| {
                let mut s = st2.borrow_mut();
                s.minutes = (s.minutes.clamp(1, 1440) + 1).min(1440);
            })),
    );
    let st3 = st.clone();
    root = root.child(
        div().flex().flex_row().justify_end().child(
            small_button("bf-minutes-start", figcraft_i18n::tr("Start"), true).on_click(move |_, _, _| {
                let minutes = st3.borrow().minutes.clamp(1, 1440);
                st3.borrow_mut().timer_open = false;
                cmds.push(Cmd { id: "board.timer", params: Some(json!({ "action": "start", "minutes": minutes })) });
            }),
        ),
    );
    root.into_any_element()
}

/// 投票弹层：每人票数、投哪些对象、Start。
fn vote_popup(app: &CanvasApp, cmds: Cmds, st: Rc<RefCell<State>>) -> AnyElement {
    let selected = !app.session.selection().is_empty();
    let mut on = st.borrow().on;
    if on == 2 && !selected {
        on = 0;
    }
    let votes = st.borrow().votes.clamp(1, 99);
    let mut root = shell(236.0).child(title(figcraft_i18n::tr("Voting session")));

    let st1 = st.clone();
    let st2 = st.clone();
    root = root.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(label(figcraft_i18n::tr("Votes per person"), true))
            .child(icon_cell("bf-votes-minus", icon::paint(Icon::Minus, rgba(tokens().text), 12.0)).size(px(22.0)).on_click(move |_, _, _| {
                let mut s = st1.borrow_mut();
                s.votes = s.votes.clamp(1, 99).saturating_sub(1).max(1);
            }))
            .child(div().w(px(24.0)).flex().justify_center().child(label(votes.to_string(), false)))
            .child(icon_cell("bf-votes-plus", icon::paint(Icon::Plus, rgba(tokens().text), 12.0)).size(px(22.0)).on_click(move |_, _, _| {
                let mut s = st2.borrow_mut();
                s.votes = (s.votes.clamp(1, 99) + 1).min(99);
            })),
    );

    for (i, (_, text)) in SCOPES.iter().enumerate() {
        // 有选中对象时才提供 Selection。
        if i == 2 && !selected {
            continue;
        }
        let st3 = st.clone();
        let active = on == i;
        root = root.child(radio_row(("bf-scope", i as u64), figcraft_i18n::tr(text), active).on_click(move |_, _, _| {
            st3.borrow_mut().on = i;
        }));
    }

    let st4 = st.clone();
    let ids: Vec<figcraft_doc::NodeId> = app.session.selection().to_vec();
    root = root.child(div().flex().flex_row().justify_end().child(
        small_button("bf-vote-start", figcraft_i18n::tr("Start vote"), true).on_click(move |_, _, _| {
            let s = st4.borrow();
            // CodeRabbit PR#17/#23: apply the same "selection needs a selection" normalisation the
            // popup renders, otherwise Start submits `on: selection` with empty ids and is rejected.
            let on = if s.on == 2 && ids.is_empty() { 0 } else { s.on };
            let scope = SCOPES.get(on).map_or("all", |x| x.0);
            let votes = s.votes.clamp(1, 99);
            drop(s);
            let mut params = json!({ "action": "start", "votes": votes, "on": scope });
            if scope == "selection" {
                params["ids"] = json!(ids);
            }
            cmds.push(Cmd { id: "board.vote", params: Some(params) });
            st4.borrow_mut().vote_open = false;
        }),
    ));
    root.into_any_element()
}

fn radio_row(id: impl Into<gpui::ElementId>, text: impl Into<SharedString>, active: bool) -> gpui::Stateful<Div> {
    let t = tokens();
    div()
        .id(id)
        .h(px(24.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .child(
            div()
                .size(px(12.0))
                .rounded_full()
                .border_1()
                .border_color(rgba(if active { t.accent } else { t.border_strong }))
                .flex()
                .items_center()
                .justify_center()
                .children(active.then(|| div().size(px(6.0)).rounded_full().bg(rgba(t.accent)))),
        )
        .child(label(text, false))
}

/// 已结束投票的结果：最新一场在前，可翻看更早的；每行对象 + 谁投了它。
fn results_popup(app: &CanvasApp, cmds: Cmds, st: Rc<RefCell<State>>, editable: bool) -> AnyElement {
    let sessions = facilitate::sessions(&app.session).unwrap_or_default();
    let total = sessions.len();
    let pick = st.borrow().pick.min(total.saturating_sub(1));
    let t = tokens();
    let mut root = shell(260.0);
    // 标题 + 「‹ n / m ›」。
    let mut head = div().flex().flex_row().items_center().gap(px(6.0)).child(title(figcraft_i18n::tr("Results")));
    if sessions.len() > 1 {
        let prev = st.clone();
        let next = st.clone();
        head = head
            .child(
                icon_cell("bf-results-prev", icon::paint(Icon::ChevronLeft, rgba(t.menu_text), 12.0))
                    .size(px(20.0))
                    .when(pick == 0, |d| d.opacity(0.35))
                    .on_click(move |_, _, _| {
                        let mut s = prev.borrow_mut();
                        s.pick = s.pick.saturating_sub(1);
                    }),
            )
            .child(label(
                figcraft_i18n::trf("{n} of {total}", &[("n", &(pick + 1)), ("total", &total)]),
                true,
            ))
            .child(
                icon_cell("bf-results-next", icon::paint(Icon::ChevronRight, rgba(t.menu_text), 12.0))
                    .size(px(20.0))
                    .when(pick + 1 >= total, |d| d.opacity(0.35))
                    .on_click(move |_, _, _| {
                        let mut s = next.borrow_mut();
                        s.pick = (s.pick + 1).min(total.saturating_sub(1));
                    }),
            );
    }
    root = root.child(head);
    if sessions.get(pick).is_none_or(|s| s.rows.is_empty()) {
        root = root.child(label(figcraft_i18n::tr("No votes yet"), true));
    }
    let Some(session) = sessions.get(pick) else { return root.into_any_element() };
    // CodeRabbit PR#17/#23: bound the result rows to a scroll area so a long session can't push
    // the Clear/Delete footer out of the (clamped) popup. header/footer stay outside the scroll.
    let mut rows = div().id("bf-results-rows").max_h(px(280.0)).overflow_y_scroll().flex().flex_col().gap(px(2.0));
    for row in &session.rows {
        let cmds = cmds.clone();
        let id = row.id;
        rows = rows.child(
            div()
                .id(gpui::ElementId::from(SharedString::from(format!("bf-row-{}", id.0))))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .px(px(4.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .hover(|s| s.bg(rgba(t.hover)))
                .cursor_pointer()
                .on_click(move |_, _, _| cmds.push(Cmd { id: "select.set", params: Some(json!({ "ids": [id] })) }))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(label(format!("{}. {}", row.rank, elide(&row.name, 34)), false))
                        .child(label(figcraft_i18n::trn("{n} votes", row.votes as usize, &[]), true)),
                )
                .child(div().pl(px(14.0)).child(label(elide(&voters_line(row), 44), true))),
        );
    }
    root = root.child(rows);
    if editable {
        let mut footer = div().flex().flex_row().gap(px(6.0)).pt(px(2.0));
        if session.on_board {
            let cmds = cmds.clone();
            let close = st.clone();
            footer = footer.child(small_button("bf-results-clear", figcraft_i18n::tr("Clear results"), false).on_click(move |_, _, _| {
                cmds.push(Cmd { id: "board.vote", params: Some(json!({ "action": "clear" })) });
                close.borrow_mut().results_open = false;
            }));
        }
        if let Some(index) = session.index {
            let cmds = cmds.clone();
            footer = footer.child(small_button("bf-results-delete", figcraft_i18n::tr("Delete results"), false).on_click(move |_, _, _| {
                cmds.push(Cmd { id: "board.vote", params: Some(json!({ "action": "delete", "index": index })) });
            }));
        }
        root = root.child(footer);
    }
    root.into_any_element()
}

/// 「Ada, Grace ×2」：同一个人在一行对象上多票时注明次数。
fn voters_line(row: &facilitate::ResultRow) -> String {
    let who: Vec<String> = row
        .voters
        .iter()
        .map(|(_, name, n)| {
            let name = name.clone().unwrap_or_else(|| figcraft_i18n::tr("Someone who left").to_string());
            if *n > 1 {
                figcraft_i18n::trf("{name} ×{n}", &[("name", &name), ("n", &n.to_string())])
            } else {
                name
            }
        })
        .collect();
    who.join(", ")
}

fn elide(s: &str, max: usize) -> String {
    let short: String = s.chars().take(max).collect();
    if short.len() < s.len() { format!("{short}…") } else { short }
}

/// 页面上的投票徽标：投票进行中显示你自己的票，结束后显示所有人的票。
pub(crate) fn badges(app: &CanvasApp, painter: &mut Painter) {
    let Ok(ds) = app.session.ds() else { return };
    let doc = &ds.doc;
    let Some(vote) = &doc.vote else { return };
    let counts = if vote.ended { vote.tally(doc, &doc.votes) } else { facilitate::mine(doc, &facilitate::voter(&app.session, &Value::Null)) };
    let t = tokens();
    let r = BADGE_PX as f32;
    for (id, n) in counts {
        let Some(b) = doc.world_bounds(id).filter(|_| doc.page_of(id) == Some(ds.page)) else { continue };
        let (c, _) = facilitate::badge(b, ds.view.zoom);
        let c = app.to_screen(c.x, c.y);
        painter.circle_filled(c + vec2(0.0, 1.0), r + 1.0, Color32::from_black_alpha(40));
        painter.circle_filled(c, r, t.accent);
        painter.circle_stroke(c, r, Stroke::new(1.5, Color32::WHITE));
        painter.text(c, Align2::CENTER_CENTER, n.to_string(), semibold(11.0), t.accent_on);
    }
}

/// 计时器在走：画布需要每秒醒一次（egui `request_repaint_after`）。
pub(crate) fn ticking(app: &CanvasApp) -> bool {
    let Ok(doc) = app.session.doc() else { return false };
    matches!(doc.timer, Some(BoardTimer::Running { .. })) && doc.timer.is_some_and(|t| t.left(facilitate::now_ms()) > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn board_app() -> CanvasApp {
        let mut app = crate::CanvasApp::new(crate::sample_doc());
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
        app.session.execute("file.newBoard", &json!({})).unwrap();
        app
    }

    /// 药丸只在画板上出现（示例文档不是画板）。
    #[test]
    fn the_pill_shows_only_on_a_board() {
        let mut app = crate::CanvasApp::new(crate::sample_doc());
        let st = Rc::new(RefCell::new(State::default()));
        let cmds = Cmds::new(&(Rc::new(|| {}) as Rc<dyn Fn()>));
        assert!(pill(&app, cmds.clone(), &st).is_none());
        app.session.execute("file.newBoard", &json!({})).unwrap();
        assert!(pill(&app, cmds, &st).is_some());
    }

    /// 计时器一走，画布每秒续帧；暂停/停止后不再续。
    #[test]
    fn ticking_follows_the_timer() {
        let app = board_app();
        assert!(!ticking(&app), "no timer");
        let mut app = app;
        app.session.execute("board.timer", &json!({ "action": "start", "minutes": 1 })).unwrap();
        assert!(ticking(&app), "a running timer");
        app.session.execute("board.timer", &json!({ "action": "pause" })).unwrap();
        assert!(!ticking(&app), "paused");
        app.session.execute("board.timer", &json!({ "action": "stop" })).unwrap();
        assert!(!ticking(&app), "stopped");
    }

    /// 投票开着时药丸报剩余票数，结束后回到「结果」状态。
    #[test]
    fn the_vote_part_reports_votes_left() {
        let mut app = board_app();
        assert!(matches!(vote_view(&app), VoteView::Off { results: false }));
        app.session.execute("board.vote", &json!({ "action": "start", "votes": 3, "on": "all" })).unwrap();
        match vote_view(&app) {
            VoteView::Running { left } => assert_eq!(left, 3, "three votes, none used"),
            _ => panic!("a vote is running"),
        }
        app.session.execute("board.vote", &json!({ "action": "end" })).unwrap();
        assert!(matches!(vote_view(&app), VoteView::Off { results: true }), "the results are available");
    }
}
