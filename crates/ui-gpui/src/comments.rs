//! Comments — the Comment tool's pins, the draft composer, the thread popover and the
//! Comments panel. 1:1 with `ui-egui/src/canvas/comments.rs`; every mutation is a real
//! `comment.*` engine command.
//!
//!   * pure helpers (`mention_query`, `mention_matches`, `insert_mention`, `cluster`, `ago`,
//!     `ago_long`, `author_color`) — the upstream rules;
//!   * `pins` / `spots` / `draw` — the avatar bubbles, unread dot, resolved tick, clusters
//!     and the draft pin;
//!   * `Overlay` — the retained-state popover (draft composer or open thread);
//!   * `panel` — the Comments panel (search / filter / sort / "Show open" / page grouping);
//!   * `keys` / `step` — Up / Down walk the panel while the Comment tool is active.

use crate::panels::{Cmd, RightFields};
use crate::widgets::{Dropdown, MenuRow, icon::Icon, pt, rgba, tokens};
use figcraft_doc::{Comment, Document, Message};
use figcraft_engine::ToolId;
use figcraft_engine::cmd::comment::{self as cc, Filter, Query, Sort, frame_of, query};
use figcraft_engine::live::initials;
use figcraft_i18n::tr;
use gpui::{App, Div, Entity, IntoElement, MouseButton, SharedString, Window, div, prelude::*, px};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

use crate::canvas::geom::{Color32, Pos2, Rect, Stroke, Vec2, pos2, vec2};

// ── metrics, from the original ────────────────────────────────────────────────────────────
const PIN: f32 = 28.0;
/// Pins whose centres come closer than this gather into a cluster.
pub const CLUSTER_R: f32 = 24.0;
/// A press on your own pin that moves further than this drags it.
const DRAG_SLOP: f32 = 3.0;
/// The reactions on offer.
pub const REACTIONS: [&str; 8] = ["👍", "❤", "😂", "🎉", "👀", "🙏", "🔥", "✅"];
/// Names the @mention picker lists at most.
const MENTION_ROWS: usize = 6;
/// A popover's width (egui `POPOVER_W`).
const POPOVER_W: f32 = 300.0;

// ── state ─────────────────────────────────────────────────────────────────────────────────
#[derive(Clone, Debug, Default)]
pub struct MentionPick {
    /// Which field: 0 = composer, 1 = edit.
    field: u8,
    range: (usize, usize),
    names: Vec<String>,
    pub sel: usize,
}

impl MentionPick {
    pub fn active(&self) -> bool {
        !self.names.is_empty()
    }
    pub fn names(&self) -> &[String] {
        &self.names
    }
}

/// Insert the picked @mention into the composer's live text and dismiss the picker. Shared by
/// the keyboard (Enter/Tab) and a click on a suggestion row.
pub(crate) fn insert_pick(ui: &CommentState, name: &str) {
    let mut u = ui.borrow_mut();
    let range = u.mention.range;
    let _ = insert_mention(&mut u.composer_text, range, name);
    u.mention = MentionPick::default();
}

#[derive(Clone, Copy, Debug)]
struct PinDrag {
    id: u64,
    from: Pos2,
    grab: Vec2,
    at: Pos2,
    mine: bool,
    moved: bool,
}

/// The comment panel's local state (egui `CommentUi`).
#[derive(Clone, Debug)]
pub(crate) struct CommentUi {
    pub filter: Filter,
    pub sort: Sort,
    pub search: String,
    pub open: Option<u64>,
    /// A new comment being written at a page point: (x, y, text).
    pub draft: Option<(f64, f64, String)>,
    /// The open thread's reply text.
    pub reply: String,
    /// A message being edited: (thread, message index, text).
    pub editing: Option<(u64, usize, String)>,
    /// The composer's live text (kept in sync by the field's `on_change`, like the palette's
    /// `palette_query`): `TextField::value()` only updates on commit, so submit must read this.
    pub composer_text: String,
    pub mention: MentionPick,
    drag: Option<PinDrag>,
    pub scroll_to_open: bool,
}

impl Default for CommentUi {
    fn default() -> Self {
        Self {
            filter: Filter::Open,
            sort: Sort::Date,
            search: String::new(),
            open: None,
            draft: None,
            reply: String::new(),
            editing: None,
            composer_text: String::new(),
            mention: MentionPick::default(),
            drag: None,
            scroll_to_open: false,
        }
    }
}

impl CommentUi {
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn show(&mut self, id: u64) {
        if self.open != Some(id) {
            self.reply.clear();
        }
        self.open = Some(id);
        self.draft = None;
        self.editing = None;
        self.composer_text.clear();
        self.mention = MentionPick::default();
    }
    pub fn close(&mut self) {
        self.open = None;
        self.draft = None;
        self.editing = None;
        self.composer_text.clear();
        self.mention = MentionPick::default();
    }
}

pub(crate) type CommentState = Rc<RefCell<CommentUi>>;

// ── pure helpers ──────────────────────────────────────────────────────────────────────────
pub fn mention_query(text: &str, cursor: usize) -> Option<(usize, usize)> {
    let end = text.char_indices().nth(cursor).map_or(text.len(), |(b, _)| b);
    let before = text.get(..end)?;
    let at = before.rfind('@')?;
    let q = before.get(at + 1..)?;
    if q.contains('\n') || q.chars().count() > 40 || q.ends_with(char::is_whitespace) {
        return None;
    }
    let starts = before.get(..at)?.chars().next_back().is_none_or(|c| !c.is_alphanumeric());
    starts.then_some((at, end))
}

pub fn mention_matches(people: &[String], query: &str, me: &str) -> Vec<String> {
    let q = query.to_lowercase();
    people
        .iter()
        .filter(|n| n.as_str() != me)
        .filter(|n| {
            let l = n.to_lowercase();
            l.starts_with(&q) || l.split_whitespace().any(|w| w.starts_with(&q))
        })
        .take(MENTION_ROWS)
        .cloned()
        .collect()
}

pub fn insert_mention(text: &mut String, range: (usize, usize), name: &str) -> usize {
    let (a, b) = range;
    if a > b || b > text.len() || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
        return text.chars().count();
    }
    let ins = format!("@{name} ");
    text.replace_range(a..b, &ins);
    text.get(..a).map_or(0, |t| t.chars().count()) + ins.chars().count()
}

pub fn cluster(points: &[Pos2], r: f32) -> Vec<Vec<usize>> {
    let mut groups: Vec<(Vec<usize>, Vec2)> = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let near = groups.iter_mut().find(|(g, sum)| {
            let centre = pos2(sum.x / g.len() as f32, sum.y / g.len() as f32);
            centre.distance(*p) < r
        });
        match near {
            Some((g, sum)) => {
                g.push(i);
                *sum = *sum + p.to_vec2();
            }
            None => groups.push((vec![i], p.to_vec2())),
        }
    }
    groups.into_iter().map(|(g, _)| g).collect()
}

fn age(t: u64) -> Option<u64> {
    let now = cc::now_secs();
    (t != 0 && now >= t).then(|| now - t)
}

pub fn ago(created: u64) -> String {
    let Some(d) = age(created) else { return String::new() };
    match d {
        0..60 => tr("now").into(),
        60..3600 => format!("{}m", d / 60),
        3600..86_400 => format!("{}h", d / 3600),
        86_400..604_800 => format!("{}d", d / 86_400),
        _ => format!("{}w", d / 604_800),
    }
}

pub fn ago_long(t: u64) -> String {
    let Some(d) = age(t) else { return String::new() };
    match d {
        0..60 => tr("Just now").into(),
        60..3600 => format!("{} minutes ago", d / 60),
        3600..86_400 => format!("{} hours ago", d / 3600),
        86_400..172_800 => tr("Yesterday").into(),
        172_800..604_800 => format!("{} days ago", d / 86_400),
        604_800..2_592_000 => format!("{} weeks ago", d / 604_800),
        2_592_000..31_536_000 => format!("{} months ago", d / 2_592_000),
        _ => format!("{} years ago", d / 31_536_000),
    }
}

pub fn author_color(name: &str) -> Color32 {
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    Color32::from_hex(figcraft_engine::live::PEOPLE[h as usize % figcraft_engine::live::PEOPLE.len()]).unwrap_or(Color32::WHITE)
}

fn text_on(fill: Color32) -> Color32 {
    let [r, g, b, _] = fill.to_array();
    let l = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
    if l > 170.0 {
        Color32::from_rgb(0x1E, 0x1E, 0x1E)
    } else {
        Color32::WHITE
    }
}

/// A stable colour per author as a GPUI colour (egui's `author_color` + `live::people`).
fn author_color_gpui(name: &str) -> gpui::Rgba {
    rgba(author_color(name))
}

// ── pins ──────────────────────────────────────────────────────────────────────────────────
fn visible(app: &crate::CanvasApp, ui: &CommentUi) -> bool {
    app.session.tool.tool == ToolId::Comment
        || app.session.ds().is_ok_and(|ds| ds.view.comments)
        || ui.open.is_some()
        || ui.draft.is_some()
}

#[derive(Clone, Debug)]
struct Pin {
    id: u64,
    author: String,
    resolved: bool,
    unread: bool,
    mine: bool,
    at: Pos2,
}

impl Pin {
    fn of(c: &Comment, me: &str, at: Pos2) -> Self {
        Self { id: c.id, author: c.first.author.clone(), resolved: c.resolved, unread: c.is_unread(me), mine: c.first.author == me, at }
    }
}

fn pins(app: &crate::CanvasApp, ui: &CommentUi) -> Vec<Pin> {
    let Ok(ds) = app.session.ds() else { return Vec::new() };
    let me = cc::user_name(&app.session);
    let q = Query { page: Some(ds.page), filter: ui.filter, ..Query::default() };
    let mut list = query(&ds.doc, &q, &me);
    list.reverse();
    if let Some(c) = ui.open.and_then(|o| ds.doc.comment(o)).filter(|c| c.page == ds.page && !list.iter().any(|x| x.id == c.id)) {
        list.push(c);
    }
    list.sort_by_key(|c| ui.open == Some(c.id));
    let drag = ui.drag.filter(|d| d.moved);
    list.into_iter()
        .map(|c| {
            let at = match drag {
                Some(d) if d.id == c.id => pos2(d.at.x, d.at.y),
                _ => {
                    let (x, y) = c.position(&ds.doc);
                    app.to_screen(x, y)
                }
            };
            Pin::of(c, &me, at)
        })
        .collect()
}

enum Spot {
    One(Pin),
    Many(Vec<u64>, Pos2),
}

fn spots(app: &crate::CanvasApp, ui: &CommentUi) -> Vec<Spot> {
    let alone = |p: &Pin| ui.open == Some(p.id) || ui.drag.is_some_and(|d| d.id == p.id);
    let (solo, rest): (Vec<Pin>, Vec<Pin>) = pins(app, ui).into_iter().partition(alone);
    let centres: Vec<Pos2> = rest.iter().map(|p| pin_rect(p.at).center()).collect();
    let mut out: Vec<Spot> = Vec::new();
    for g in cluster(&centres, CLUSTER_R) {
        let members: Vec<&Pin> = g.iter().filter_map(|i| rest.get(*i)).collect();
        match members.as_slice() {
            [one] => out.push(Spot::One((*one).clone())),
            many => {
                let sum = many.iter().fold(Vec2::ZERO, |s, p| s + p.at.to_vec2());
                let n = many.len().max(1) as f32;
                out.push(Spot::Many(many.iter().map(|p| p.id).collect(), pos2(sum.x / n, sum.y / n)));
            }
        }
    }
    out.extend(solo.into_iter().map(Spot::One));
    out
}

fn pin_rect(at: Pos2) -> Rect {
    Rect::from_min_max(pos2(at.x, at.y - PIN), pos2(at.x + PIN, at.y))
}

fn cluster_rect(at: Pos2, n: usize) -> Rect {
    let digits = n.to_string().len() as f32;
    Rect::from_min_max(pos2(at.x, at.y - PIN), pos2(at.x + PIN + 7.0 * (digits - 1.0), at.y))
}

fn bubble() -> crate::canvas::geom::CornerRadius {
    crate::canvas::geom::CornerRadius { nw: 14, ne: 14, se: 14, sw: 0 }
}

fn avatar(p: &mut crate::canvas::painter::Painter, center: Pos2, r: f32, name: &str) {
    use crate::canvas::geom::Align2;
    let fill = author_color(name);
    p.circle_filled(center, r, fill);
    p.text(center, Align2::CENTER_CENTER, initials(name), crate::canvas::geom::semibold(r * 0.8), text_on(fill));
}

fn paint_pin(p: &mut crate::canvas::painter::Painter, r: Rect, pin: &Pin, hot: bool, t: &crate::theme::Tokens) {
    use crate::canvas::geom::Align2;
    let b = bubble();
    p.rect_filled(r + vec2(0.0, 1.0), b, Color32::from_black_alpha(40));
    p.rect_filled(r, b, if hot { t.accent } else { Color32::WHITE });
    p.rect_stroke(r, b, Stroke::new(1.0, Color32::from_black_alpha(30)), crate::canvas::geom::StrokeKind::Inside);
    let fill = if pin.resolved { Color32::from_gray(150) } else { author_color(&pin.author) };
    p.circle_filled(r.center(), 11.0, fill);
    p.text(r.center(), Align2::CENTER_CENTER, initials(&pin.author), crate::canvas::geom::semibold(9.0), text_on(fill));
    if pin.unread {
        let c = pos2(r.max.x - 2.0, r.min.y + 2.0);
        p.circle_filled(c, 4.5, Color32::WHITE);
        p.circle_filled(c, 3.5, t.accent);
    }
    if pin.resolved {
        let c = pos2(r.max.x - 3.0, r.max.y - 3.0);
        p.line_segment([pos2(c.x - 2.0, c.y), pos2(c.x - 0.5, c.y + 2.0)], Stroke::new(1.4, t.accent));
        p.line_segment([pos2(c.x - 0.5, c.y + 2.0), pos2(c.x + 2.5, c.y - 2.0)], Stroke::new(1.4, t.accent));
    }
}

fn paint_cluster(p: &mut crate::canvas::painter::Painter, at: Pos2, n: usize, hot: bool, t: &crate::theme::Tokens) {
    use crate::canvas::geom::{Align2, CornerRadius, StrokeKind};
    let r = cluster_rect(at, n);
    let back = r + vec2(3.0, -3.0);
    let edge = Stroke::new(1.0, Color32::from_black_alpha(30));
    let b = bubble();
    p.rect_filled(back, b, Color32::WHITE);
    p.rect_stroke(back, b, edge, StrokeKind::Inside);
    p.rect_filled(r + vec2(0.0, 1.0), b, Color32::from_black_alpha(40));
    p.rect_filled(r, b, if hot { t.accent } else { Color32::WHITE });
    p.rect_stroke(r, b, edge, StrokeKind::Inside);
    let inner = r.shrink(3.0);
    p.rect_filled(inner, CornerRadius::same(11), Color32::from_gray(44));
    p.text(inner.center(), Align2::CENTER_CENTER, n.to_string(), crate::canvas::geom::semibold(11.0), Color32::WHITE);
}

/// Draw pins, clusters and the draft pin (egui `draw`).
pub(crate) fn draw(app: &crate::CanvasApp, ui: &CommentUi, painter: &mut crate::canvas::painter::Painter, t: &crate::theme::Tokens) {
    if !visible(app, ui) {
        return;
    }
    let hover = app.pointer;
    for spot in spots(app, ui) {
        match spot {
            Spot::One(pin) => {
                let r = pin_rect(pin.at);
                let hot = ui.open == Some(pin.id) || hover.is_some_and(|h| r.contains(h));
                paint_pin(painter, r, &pin, hot, t);
            }
            Spot::Many(ids, at) => {
                let r = cluster_rect(at, ids.len());
                paint_cluster(painter, at, ids.len(), hover.is_some_and(|h| r.contains(h)), t);
            }
        }
    }
    if let Some((x, y, _)) = &ui.draft {
        let at = app.to_screen(*x, *y);
        let r = pin_rect(at);
        painter.rect_filled(r, bubble(), t.accent);
        painter.circle_filled(r.center(), 3.0, t.accent_on);
    }
}

// ── canvas pointer handling ───────────────────────────────────────────────────────────────
#[derive(Debug)]
pub enum PinAction {
    Toggle(u64),
    Zoom(Vec<u64>),
    Move { id: u64, x: f64, y: f64 },
    Consumed,
    None,
}

pub fn input(app: &crate::CanvasApp, ui: &mut CommentUi, pos: Pos2, pressed: bool, down: bool) -> PinAction {
    if !visible(app, ui) {
        ui.drag = None;
        return PinAction::None;
    }
    if let Some(mut d) = ui.drag.take() {
        if down {
            d.moved |= d.mine && (pos - d.from).length() > DRAG_SLOP;
            if d.moved {
                d.at = pos - d.grab;
            }
            ui.drag = Some(d);
            return PinAction::Consumed;
        }
        if d.moved {
            let p = app.to_page(pos2(d.at.x, d.at.y));
            return PinAction::Move { id: d.id, x: p.x, y: p.y };
        }
        return PinAction::Toggle(d.id);
    }
    for spot in spots(app, ui).into_iter().rev() {
        match spot {
            Spot::One(pin) if pin_rect(pin.at).contains(pos) => {
                if pressed {
                    ui.drag = Some(PinDrag { id: pin.id, from: pos, grab: pos - pin.at, at: pin.at, mine: pin.mine, moved: false });
                }
                return PinAction::Consumed;
            }
            Spot::Many(ids, at) if cluster_rect(at, ids.len()).contains(pos) => {
                if pressed {
                    return PinAction::Zoom(ids);
                }
                return PinAction::Consumed;
            }
            _ => {}
        }
    }
    if pressed && ui.open.is_some() {
        ui.close();
        if app.session.tool.tool != ToolId::Comment {
            return PinAction::Consumed;
        }
    }
    PinAction::None
}

pub fn start_draft(ui: &mut CommentUi, x: f64, y: f64) {
    ui.close();
    ui.composer_text.clear();
    ui.draft = Some((x, y, String::new()));
}

pub fn escape(ui: &mut CommentUi) -> bool {
    if ui.open.is_none() && ui.draft.is_none() {
        return false;
    }
    if ui.editing.take().is_some() {
        return false;
    }
    ui.close();
    true
}

// ── the Comments panel ────────────────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CommentRow {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub ago: String,
    pub unread: bool,
    pub meta: String,
    pub page: String,
}

pub(crate) fn read_rows(doc: &Document, me: &str, filter: Filter, sort: Sort, search: &str) -> Vec<CommentRow> {
    let q = Query { page: None, frame: None, filter, sort, text: search };
    query(doc, &q, me)
        .into_iter()
        .map(|c| {
            let mut meta = Vec::new();
            if let Some(f) = frame_of(doc, c).and_then(|f| doc.node(f)) {
                meta.push(f.name.clone());
            }
            if !c.replies.is_empty() {
                meta.push(format!("{} replies", c.replies.len()));
            }
            if c.resolved {
                meta.push(tr("Resolved").to_string());
            }
            CommentRow {
                id: c.id,
                author: c.first.author.clone(),
                body: c.first.text.clone(),
                ago: ago(c.last_activity()),
                unread: c.is_unread(me),
                meta: meta.join(" · "),
                page: doc.node(c.page).map(|n| n.name.clone()).unwrap_or_default(),
            }
        })
        .collect()
}

pub(crate) fn panel(f: &RightFields, rows: &[CommentRow], total: usize, search: Entity<crate::widgets::TextField>) -> Div {
    let t = tokens();
    let cu = f.comment_ui().borrow().clone();
    let by_page = cu.sort == Sort::Page;
    let mut body = div().flex().flex_col().w_full();
    body = body.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(40.0))
            .px(px(16.0))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgba(t.text))
                    .child(tr("Comments")),
            )
            .child(filter_button(f, cu.filter, cu.sort)),
    );
    body = body.child(div().h(px(32.0)).px(px(16.0)).pb(px(8.0)).child(search));
    if cu.filter != Filter::Open {
        body = body.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .px(px(16.0))
                .pb(px(8.0))
                .text_size(px(11.0))
                .text_color(rgba(t.text_secondary))
                .child(tr(cu.filter.label()))
                .child(show_open(f)),
        );
    }
    if rows.is_empty() {
        let (title, sub) = if total == 0 {
            (tr("No comments yet"), tr("Click anywhere on the canvas to leave one."))
        } else {
            (tr("No matching comments"), tr("Try another search or filter."))
        };
        return body.child(
            div()
                .px(px(16.0))
                .py(px(24.0))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .child(div().text_size(px(12.0)).text_color(rgba(t.text_secondary)).child(title))
                .child(div().text_size(px(11.0)).text_color(rgba(t.text_tertiary)).child(sub)),
        );
    }
    let mut last_page = String::new();
    for row in rows {
        if by_page && last_page != row.page {
            last_page = row.page.clone();
            body = body.child(
                div()
                    .flex()
                    .flex_row()
                    .px(px(16.0))
                    .pt(px(6.0))
                    .pb(px(2.0))
                    .text_size(px(11.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgba(t.text_secondary))
                    .child(row.page.clone()),
            );
        }
        body = body.child(comment_row(f, row, cu.open == Some(row.id)));
    }
    body
}

fn show_open(f: &RightFields) -> Div {
    let ui = f.comment_ui();
    let t = tokens();
    div()
        .cursor_pointer()
        .text_size(px(11.0))
        .text_color(rgba(t.accent))
        .on_mouse_down(MouseButton::Left, move |_, _, _cx: &mut App| {
            ui.borrow_mut().filter = Filter::Open;
        })
        .child(tr("Show open"))
}

fn comment_row(f: &RightFields, row: &CommentRow, open: bool) -> impl IntoElement {
    let t = tokens();
    let cmds = f.queue();
    let ui = f.comment_ui();
    let id = row.id;
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(8.0))
        .px(px(16.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(rgba(t.border))
        .when(open, |d| d.bg(rgba(t.selected_row)))
        .cursor_pointer()
        .id(gpui::ElementId::from(SharedString::from(format!("comment-row-{id}"))))
        .on_mouse_down(MouseButton::Left, move |_, _, cx: &mut App| {
            cmds.push(Cmd { id: "comment.open", params: Some(json!({ "id": id, "view": "center" })) });
            ui.borrow_mut().show(id);
        })
        .child(
            div()
                .w(px(8.0))
                .flex_none()
                .flex()
                .justify_center()
                .pt(px(6.0))
                .child(div().w(px(7.0)).h(px(7.0)).rounded_full().when(row.unread, |d| d.bg(rgba(t.accent)))),
        )
        .child(avatar_div(&row.author, 12.0))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.0))
                                .font_weight(if row.unread { gpui::FontWeight::SEMIBOLD } else { gpui::FontWeight::NORMAL })
                                .text_color(rgba(t.text))
                                .child(row.author.clone()),
                        )
                        .child(div().text_size(px(11.0)).text_color(rgba(t.text_tertiary)).child(row.ago.clone())),
                )
                .child(div().text_size(px(12.0)).text_color(rgba(t.text)).child(row.body.clone()))
                .when(!row.meta.is_empty(), |d| d.child(div().text_size(px(11.0)).text_color(rgba(t.text_secondary)).child(row.meta.clone()))),
        )
}

pub(crate) fn avatar_div(name: &str, r: f32) -> Div {
    let color = author_color(name);
    let fill = rgba(color);
    let dark = {
        let [r, g, b, _] = color.to_array();
        0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 170.0
    };
    div()
        .w(px(r * 2.0))
        .h(px(r * 2.0))
        .flex_none()
        .rounded_full()
        .bg(gpui::Hsla::from(fill))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(r * 0.8))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgba(if dark {
            Color32::from_rgb(0x1E, 0x1E, 0x1E)
        } else {
            Color32::WHITE
        }))
        .child(initials(name))
}

fn filter_button(f: &RightFields, filter: Filter, sort: Sort) -> Dropdown {
    let rows: Vec<MenuRow> = {
        let mut v: Vec<MenuRow> = Filter::ALL.iter().map(|x| MenuRow::new(tr(x.label())).checked(Some(*x == filter))).collect();
        v.extend(Sort::ALL.iter().map(|x| MenuRow::new(tr(x.label())).separator().checked(Some(*x == sort))));
        v.push(MenuRow::new(tr("Mark all as read")).separator());
        v
    };
    let mut d = Dropdown::new(tr(filter.label()), 0.0, None).fill();
    if let Some(root) = f.root() {
        let ui = f.comment_ui();
        let cmds = f.queue();
        d = d.on_click(move |anchor, _window, cx: &mut App| {
            let ui = ui.clone();
            let cmds = cmds.clone();
            let menu = rows.clone();
            let on_pick = Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some(x) = Filter::ALL.iter().find(|x| tr(x.label()) == label) {
                    ui.borrow_mut().filter = *x;
                } else if let Some(s) = Sort::ALL.iter().find(|x| tr(x.label()) == label) {
                    ui.borrow_mut().sort = *s;
                } else if label == tr("Mark all as read") {
                    cmds.push(Cmd { id: "comment.markRead", params: Some(json!({})) });
                }
                cmds.wake();
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), menu, on_pick, cx));
        });
    }
    d
}

// ── the draft / thread popover ────────────────────────────────────────────────────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingAct {
    Submit,
    Cancel,
    MentionUp,
    MentionDown,
    MentionPick,
}

fn message_params(thread: u64, index: usize) -> Value {
    match index.checked_sub(1) {
        Some(r) => json!({ "id": thread, "reply": r }),
        None => json!({ "id": thread }),
    }
}

/// The command a composer submit runs, from the live text and the composer's mode. Extracted so
/// the "typed text actually gets posted" rule is testable without a live view. `None` when there
/// is nothing to post (the text is empty).
fn submit_params(thread: Option<u64>, page: (f64, f64), text: &str, editing: Option<&(u64, usize, String)>) -> Option<(&'static str, Value)> {
    if text.trim().is_empty() {
        return None;
    }
    let text = text.trim();
    match (thread, editing) {
        (_, Some((tid, idx, _))) => {
            let mut p = message_params(*tid, *idx);
            p["text"] = json!(text);
            Some(("comment.edit", p))
        }
        (Some(id), None) => Some(("comment.reply", json!({ "id": id, "text": text }))),
        (None, None) => Some((
            "comment.add",
            json!({ "x": page.0, "y": page.1, "text": text, "anchor": true }),
        )),
    }
}

/// The retained state of one open comment popover (a draft or a thread).
pub(crate) struct Overlay {
    ui: CommentState,
    cmds: crate::panels::Cmds,
    root: Entity<crate::CanvasView>,
    thread: Option<u64>,
    page: (f64, f64),
    people: Vec<String>,
    me: String,
    draft_field: Entity<crate::widgets::TextField>,
    edit_field: Entity<crate::widgets::TextField>,
    /// Focus the composer on the first frame (palette focuses its search box on open).
    focus_pending: bool,
}

impl Overlay {
    pub(crate) fn draft(app: &crate::CanvasApp, ui: CommentState, cmds: crate::panels::Cmds, root: Entity<crate::CanvasView>, cx: &mut App) -> Self {
        let (x, y) = ui.borrow().draft.as_ref().map(|d| (d.0, d.1)).unwrap_or((0.0, 0.0));
        Self::build(app, ui, cmds, root, None, (x, y), cx)
    }

    pub(crate) fn thread(app: &crate::CanvasApp, ui: CommentState, cmds: crate::panels::Cmds, root: Entity<crate::CanvasView>, id: u64, cx: &mut App) -> Self {
        let page = app
            .session
            .ds()
            .ok()
            .and_then(|ds| ds.doc.comment(id).map(|c| c.position(&ds.doc)))
            .unwrap_or((0.0, 0.0));
        let mut o = Self::build(app, ui, cmds, root, Some(id), page, cx);
        if app.session.ds().ok().and_then(|ds| ds.doc.comment(id).cloned()).is_some_and(|c| c.is_unread(&o.me)) && app.session.ds().is_ok_and(|ds| ds.preview.is_none()) {
            o.cmds.push(Cmd { id: "comment.markRead", params: Some(json!({ "id": id })) });
        }
        o.ui.borrow_mut().open = Some(id);
        o
    }

    fn build(
        app: &crate::CanvasApp,
        ui: CommentState,
        cmds: crate::panels::Cmds,
        root: Entity<crate::CanvasView>,
        thread: Option<u64>,
        page: (f64, f64),
        cx: &mut App,
    ) -> Self {
        let me = cc::user_name(&app.session);
        let people = app.session.doc().map(|d| d.comment_people()).unwrap_or_default();
        let draft_field = crate::widgets::text_field(cx, POPOVER_W - 24.0, "", "Add a comment", crate::widgets::Slot::None, crate::widgets::Slot::None);
        let edit_field = crate::widgets::text_field(cx, POPOVER_W - 24.0, "", "Edit comment", crate::widgets::Slot::None, crate::widgets::Slot::None);
        Self { ui, cmds, root, thread, page, people, me, draft_field, edit_field, focus_pending: true }
    }

    pub(crate) fn anchor(&self) -> (f64, f64) {
        self.page
    }
    pub(crate) fn cancel(&mut self) -> bool {
        if self.ui.borrow().editing.is_some() {
            self.ui.borrow_mut().editing = None;
            return false;
        }
        self.ui.borrow_mut().close();
        true
    }

    pub(crate) fn key(&mut self, key: PendingAct, cx: &mut App) {
        let pick_active = matches!(key, PendingAct::MentionUp | PendingAct::MentionDown | PendingAct::MentionPick) && self.ui.borrow().mention.active();
        match key {
            PendingAct::MentionUp | PendingAct::MentionDown if pick_active => {
                let mut u = self.ui.borrow_mut();
                let n = u.mention.names.len();
                u.mention.sel = if key == PendingAct::MentionDown { (u.mention.sel + 1) % n } else { (u.mention.sel + n - 1) % n };
                let _ = cx;
            }
            PendingAct::MentionPick if pick_active => {
                let name = self.ui.borrow().mention.names.get(self.ui.borrow().mention.sel).cloned().unwrap_or_default();
                insert_pick(&self.ui, &name);
                let _ = cx;
            }
            PendingAct::Submit => self.submit(cx),
            PendingAct::Cancel => {
                self.cancel();
                let _ = cx;
            }
            _ => {}
        }
    }

    pub(crate) fn submit(&mut self, cx: &mut App) {
        let (text, editing) = {
            let u = self.ui.borrow();
            // Read the live text from state (`on_change` keeps it in sync). `TextField::value()`
            // only updates on commit, so reading it here posts an empty comment.
            (u.composer_text.trim().to_string(), u.editing.clone())
        };
        let Some((id, params)) = submit_params(self.thread, self.page, &text, editing.as_ref()) else { return };
        self.cmds.push(Cmd { id, params: Some(params) });
        {
            let mut u = self.ui.borrow_mut();
            if u.draft.is_some() {
                u.draft = None;
            } else {
                u.reply.clear();
            }
            u.composer_text.clear();
            if editing.is_some() {
                u.editing = None;
            }
        }
        self.cmds.wake();
        let _ = cx;
    }

    pub(crate) fn render(&mut self, app: &crate::CanvasApp, viewport: gpui::Size<gpui::Pixels>, canvas_left: f32, canvas_top: f32, window: &mut Window, cx: &mut App) -> Vec<gpui::AnyElement> {
        // Put the caret in the composer on the first frame, like the Actions palette does, so
        // typing goes into the field instead of the canvas shortcuts.
        if self.focus_pending {
            self.focus_pending = false;
            let field = if self.ui.borrow().editing.is_some() { self.edit_field.clone() } else { self.draft_field.clone() };
            let edit = field.read(cx).edit();
            edit.update(cx, |te, _| te.focus_editor(window));
        }
        let t = tokens();
        // `app` is passed in by the caller: reading `self.root` here would borrow CanvasView
        // while it is being rendered → "cannot read CanvasView while it is already being updated".
        let at = app.to_screen(self.page.0, self.page.1);
        let anchor = pos2(canvas_left + at.x, canvas_top + at.y);
        let w = POPOVER_W;
        let mut x = anchor.x + PIN + 8.0;
        if x + w > f32::from(viewport.width) - 8.0 {
            x = anchor.x - w - 8.0;
        }
        let x = x.max(8.0);
        let y = (anchor.y - PIN).max(8.0).min((f32::from(viewport.height) - 220.0).max(8.0));

        let mut body = div().flex().flex_col().gap(pt(8.0)).w(px(w - 24.0));
        if self.thread.is_none() {
            body = body.child(div().text_size(pt(12.0)).text_color(rgba(t.text)).child(tr("Add a comment")));
            body = body.child(self.composer(SharedString::from(tr("Add a comment")), cx));
        } else {
            let id = self.thread.unwrap();
            let doc = app.session.doc().ok().cloned();
            let Some(doc) = doc else { return Vec::new() };
            let Some(c) = doc.comment(id).cloned() else {
                self.ui.borrow_mut().close();
                return Vec::new();
            };
            let mut header = div().flex().flex_row().items_center().gap(pt(4.0));
            header = header.child(div().flex_1().text_size(pt(12.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(tr("Comment")));
            let resolve = {
                let cmds = self.cmds.clone();
                let resolved = c.resolved;
                icon_button(Icon::Check, c.resolved.then_some(t.accent), SharedString::from(tr(if c.resolved { "Unresolve" } else { "Resolve" })), move |_w, cx: &mut App| {
                    cmds.push(Cmd { id: "comment.resolve", params: Some(json!({ "id": id, "resolved": !resolved })) });
                })
            };
            let close = {
                let root = self.root.clone();
                icon_button(Icon::Close, None, SharedString::from(tr("Close")), move |_, cx: &mut App| root.update(cx, |cv, cx| cv.close_comment(cx)))
            };
            header = header.child(resolve).child(close);
            body = body.child(header);

            let more_rows = {
                let mut rows = vec![
                    MenuRow::new(tr("Mark as unread")),
                    MenuRow::new(tr("Copy link")),
                    MenuRow::new(tr("Copy for AI")),
                    MenuRow::new(tr(if c.resolved { "Mark as unresolved" } else { "Mark as resolved" })),
                ];
                if c.first.author == self.me {
                    rows.push(MenuRow::new(tr("Delete thread")).separator());
                }
                rows
            };
            let more_ui = self.ui.clone();
            let more_cmds = self.cmds.clone();
            let more_root = self.root.clone();
            let resolved = c.resolved;
            let more = Dropdown::new(SharedString::from(tr("More")), 0.0, None).on_click(move |anchor, _w, cx: &mut App| {
                let rows = more_rows.clone();
                let ui = more_ui.clone();
                let cmds = more_cmds.clone();
                let root = more_root.clone();
                let on_pick_root = root.clone();
                let on_pick = Rc::new(move |label: &str, _w: &mut Window, cx: &mut App| {
                    if label == tr("Mark as unread") {
                        cmds.push(Cmd { id: "comment.markRead", params: Some(json!({ "id": id, "read": false })) });
                        ui.borrow_mut().close();
                    } else if label == tr("Copy link") {
                        let link = on_pick_root.update(cx, |cv, _| cv.app.session.execute("comment.link", &json!({ "id": id })).ok()).and_then(|v| v.get("link").and_then(Value::as_str).map(str::to_string));
                        if let Some(link) = link {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(link));
                            on_pick_root.update(cx, |cv, cx| cv.toast(tr("Link copied"), cx));
                        }
                    } else if label == tr("Copy for AI") {
                        let r = on_pick_root.update(cx, |cv, _| cv.app.session.execute("comment.prompt", &json!({ "ids": [id] })).ok());
                        if let Some(text) = r.as_ref().and_then(|v| v.get("text").and_then(Value::as_str)) {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_string()));
                            on_pick_root.update(cx, |cv, cx| cv.toast(tr("Copied comments for AI"), cx));
                        }
                    } else if label == tr("Mark as resolved") || label == tr("Mark as unresolved") {
                        cmds.push(Cmd { id: "comment.resolve", params: Some(json!({ "id": id, "resolved": !resolved })) });
                    } else if label == tr("Delete thread") {
                        cmds.push(Cmd { id: "comment.delete", params: Some(json!({ "id": id })) });
                        ui.borrow_mut().close();
                    }
                    on_pick_root.update(cx, |cv, cx| cv.close_popup(cx));
                });
                root.clone().update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), rows, on_pick, cx));
            });
            body = body.child(more);

            let people: Vec<String> = self.people.iter().cloned().chain(std::iter::once(self.me.clone())).collect();
            let people_refs: Vec<&str> = people.iter().map(String::as_str).collect();
            let editing = self.ui.borrow().editing.clone();
            for (i, m) in c.messages().enumerate() {
                if editing.as_ref().is_some_and(|e| e.0 == id && e.1 == i) {
                    body = body.child(
                        div().pl(pt(36.0)).text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(tr("Enter to save · Esc to cancel")),
                    );
                    body = body.child(self.composer(SharedString::from(tr("Edit comment")), cx));
                } else {
                    body = body.child(message_div(i, m, &people_refs));
                }
                if !m.reactions.is_empty() {
                    let mut chips = div().flex().flex_row().flex_wrap().gap(pt(4.0)).pl(pt(36.0));
                    for r in &m.reactions {
                        let on = r.users.iter().any(|u| u == &self.me);
                        let mut chip = div()
                            .flex()
                            .flex_row()
                            .gap(pt(4.0))
                            .px(pt(7.0))
                            .py(pt(3.0))
                            .rounded(pt(10.0))
                            .bg(rgba(if on { t.selected_row } else { t.field }))
                            .text_size(pt(12.0))
                            .text_color(rgba(t.text))
                            .child(r.emoji.clone())
                            .child(r.users.len().to_string());
                        let cmds = self.cmds.clone();
                        let mut p = message_params(id, i);
                        p["emoji"] = json!(r.emoji);
                        chip = chip.on_mouse_down(MouseButton::Left, move |_, _, _cx: &mut App| {
                            cmds.push(Cmd { id: "comment.react", params: Some(p.clone()) });
                        });
                        chips = chips.child(chip);
                    }
                    body = body.child(chips);
                }
            }
            // Reply composer. Do NOT re-seed the field each frame: `set_value` would reset what
            // the user is typing (and no-ops anyway while editing). Its live text lives in
            // `CommentUi::composer_text`, synced by `on_change`.
            body = body.child(self.composer(SharedString::from(tr("Reply")), cx));
        }

        let card = div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .p(pt(12.0))
            .flex()
            .flex_col()
            .gap(pt(8.0))
            .rounded(pt(12.0))
            .bg(rgba(t.panel))
            .border_1()
            .border_color(rgba(t.border))
            .shadow(vec![gpui::BoxShadow {
                color: rgba(t.shadow).into(),
                offset: gpui::point(px(0.0), px(4.0)),
                blur_radius: px(16.0),
                spread_radius: px(0.0),
            }])
            .on_mouse_down(MouseButton::Left, |_, _, cx: &mut App| cx.stop_propagation())
            .child(body);
        vec![card.into_any_element()]
    }

    /// The shared composer. Its text lives in `draft_field` (reply / draft) or `edit_field`.
    /// The @mention picker is rendered below it (egui `composer`), showing the names Enter/Tab
    /// will insert — never a bare input with a hidden picker.
    fn composer(&mut self, hint: SharedString, cx: &mut App) -> Div {
        let t = tokens();
        let editing = self.ui.borrow().editing.is_some();
        let field = if editing { self.edit_field.clone() } else { self.draft_field.clone() };
        let slot = if editing { 1u8 } else { 0u8 };
        let root = self.root.clone();
        setup_field(&field, slot, &self.ui, &self.people, &self.me, hint, &root, cx);
        let send = div()
            .flex_none()
            .size(pt(24.0))
            .rounded_full()
            .bg(rgba(t.accent))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, _, cx: &mut App| root.update(cx, |cv, cx| cv.submit_comment(cx)))
            .child(div().text_size(pt(14.0)).text_color(rgba(t.accent_on)).child("↑"));
        let mut wrap = div().flex().flex_col().gap(pt(4.0)).child(div().flex().flex_row().items_center().gap(pt(8.0)).child(field).child(send));

        // The @mention candidates (egui `composer`: avatar + name, 26 pt rows, selected/hover
        // fill, click inserts). `setup_field` already collected the matches into `mention`.
        let (active, names, sel) = {
            let u = self.ui.borrow();
            let m = &u.mention;
            (m.field == slot && m.active(), m.names.clone(), m.sel)
        };
        if active {
            let mut list = div().flex().flex_col().rounded(pt(5.0)).overflow_hidden();
            for (i, name) in names.iter().enumerate() {
                let ui = self.ui.clone();
                let chosen = name.clone();
                let root = self.root.clone();
                list = list.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(pt(8.0))
                        .h(pt(26.0))
                        .px(pt(6.0))
                        .rounded(pt(5.0))
                        .bg(rgba(if i == sel { t.selected_row } else { t.hover }))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx: &mut App| {
                            let ui = ui.clone();
                            let chosen = chosen.clone();
                            // Route through the view so this click follows the exact same
                            // mention-insert path as Enter/Tab, with a proper Context to notify.
                            root.update(cx, move |cv, cx| {
                                cv.insert_comment_mention(ui, &chosen, cx);
                            });
                        })
                        .child(avatar_div(name, 9.0))
                        .child(div().text_size(pt(12.0)).text_color(rgba(t.text)).child(name.clone())),
                );
            }
            wrap = wrap.child(list);
        }
        wrap
    }
}

fn setup_field(
    field: &Entity<crate::widgets::TextField>,
    slot: u8,
    ui: &CommentState,
    people: &[String],
    me: &str,
    hint: SharedString,
    root: &Entity<crate::CanvasView>,
    cx: &mut App,
) {
    let people = people.to_vec();
    let me = me.to_string();
    let u = ui.clone();
    // Enter is consumed by the focused TextEdit and never reaches the canvas' key handler, so
    // post from the field's own commit callback — exactly like the Actions palette's search box.
    let root = root.clone();
    field.update(cx, |tf, cx| {
        tf.set_placeholder(hint);
        tf.set_on_commit(move |_text, _w, cx| {
            root.update(cx, |cv, cx| cv.submit_comment(cx));
        });
        tf.set_on_change(move |text, _w, cx: &mut App| {
            let cursor = text.chars().count();
            let found = mention_query(text, cursor);
            let names = found
                .and_then(|r| text.get(r.0 + 1..r.1))
                .map(|q| mention_matches(&people, q, &me))
                .unwrap_or_default();
            let mut u = u.borrow_mut();
            // The live text is the single source of truth for submit (palette's `palette_query`
            // pattern) — `TextField::value()` lags until commit.
            u.composer_text = text.to_string();
            if let Some(r) = found
                && !names.is_empty()
            {
                if u.mention.field != slot || u.mention.range.0 != r.0 {
                    u.mention.sel = 0;
                }
                u.mention.field = slot;
                u.mention.range = r;
                u.mention.names = names;
            } else if u.mention.field == slot {
                u.mention.names.clear();
            }
        });
    });
}

fn icon_button(icon: Icon, fill: Option<Color32>, _tip: SharedString, on_click: impl Fn(&mut Window, &mut App) + 'static) -> Div {
    let t = tokens();
    div()
        .flex_none()
        .size(pt(24.0))
        .rounded(pt(4.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when_some(fill, |d, c| d.bg(rgba(c)))
        .hover(|s| s.bg(rgba(t.hover)))
        .on_mouse_down(MouseButton::Left, move |_, w, cx| on_click(w, cx))
        .child(crate::widgets::icon::paint(icon, rgba(t.icon), 14.0))
}

fn message_div(i: usize, m: &Message, people: &[&str]) -> Div {
    let t = tokens();
    let when = if m.edited > 0 { format!("{} · edited", ago(m.created)) } else { ago(m.created) };
    let _ = i;
    div()
        .flex()
        .flex_col()
        .gap(pt(2.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(4.0))
                .child(avatar_div(&m.author, 12.0))
                .child(div().text_size(pt(12.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(m.author.clone()))
                .when(m.agent, |d| {
                    d.child(
                        div()
                            .px(pt(4.0))
                            .rounded(pt(4.0))
                            .bg(rgba(t.hover))
                            .text_size(pt(10.0))
                            .text_color(rgba(t.text_secondary))
                            .child(tr("Agent")),
                    )
                })
                .child(div().text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(when)),
        )
        .child(div().pl(pt(36.0)).text_size(pt(12.0)).text_color(rgba(t.text)).child(highlight_mentions(&m.text, people, t.accent)))
}

fn highlight_mentions(text: &str, people: &[&str], accent: Color32) -> Div {
    let mut parts: Vec<(usize, usize, bool)> = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let mut best: Option<(usize, usize)> = None;
        for name in people {
            let needle = format!("@{name}");
            if let Some(pos) = text[cursor..].find(&needle) {
                let start = cursor + pos;
                if best.is_none_or(|(b, _)| start < b) {
                    best = Some((start, start + needle.len()));
                }
            }
        }
        match best {
            Some((a, b)) => {
                if a > cursor {
                    parts.push((cursor, a, false));
                }
                parts.push((a, b, true));
                cursor = b;
            }
            None => {
                parts.push((cursor, text.len(), false));
                break;
            }
        }
    }
    let mut row = div().flex().flex_wrap();
    for (a, b, mention) in parts {
        let s: SharedString = text[a..b].to_string().into();
        row = row.child(div().text_color(if mention { rgba(accent) } else { rgba(tokens().text) }).child(s));
    }
    row
}

// ── keyboard navigation ───────────────────────────────────────────────────────────────────
fn panel_params(ui: &CommentUi, dir: i32) -> Value {
    json!({ "id": ui.open, "dir": dir, "page": "all", "filter": ui.filter.id(), "sort": ui.sort.id(), "query": ui.search })
}

pub fn step(session: &mut figcraft_engine::Session, ui: &mut CommentUi, dir: i32) -> Option<Value> {
    let p = panel_params(ui, dir);
    let r = session.execute("comment.next", &p).ok();
    ui.scroll_to_open = r.is_some();
    r
}

pub fn keys(session: &mut figcraft_engine::Session, ui: &mut CommentUi, tool: ToolId, dir: i32) -> bool {
    if tool != ToolId::Comment || ui.draft.is_some() || ui.editing.is_some() || ui.mention.active() {
        return false;
    }
    step(session, ui, dir).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The @mention helpers: byte ranges from a char cursor, name matching and insertion.
    #[test]
    fn mention_helpers() {
        assert_eq!(mention_query("Hi @Sa", 6), Some((3, 6)));
        assert_eq!(mention_query("@", 1), Some((0, 1)));
        assert_eq!(mention_query("Hi @Sam Lee", 11), Some((3, 11)));
        assert_eq!(mention_query("Hi @Sam ", 8), None, "a space ends it");
        assert_eq!(mention_query("mail@x", 6), None, "inside a word");
        assert_eq!(mention_query("@Sa\nx", 5), None, "on another line");
        assert_eq!(mention_query("é @Sa", 4), Some((3, 5)), "byte range, char cursor");
        assert_eq!(mention_query("Hi @Sa", 99), Some((3, 6)), "a cursor past the end");
        let people: Vec<String> = ["Ada Lovelace", "Sam", "Sam Lee", "You"].iter().map(|s| s.to_string()).collect();
        assert_eq!(mention_matches(&people, "sa", "You"), vec!["Sam", "Sam Lee"]);
        assert_eq!(mention_matches(&people, "love", "You"), vec!["Ada Lovelace"], "a later word");
        assert_eq!(mention_matches(&people, "", "You").len(), 3, "everyone but you");
        let mut t = String::from("Hi @Sa, ok");
        let at = insert_mention(&mut t, (3, 6), "Sam Lee");
        assert_eq!(t, "Hi @Sam Lee , ok");
        assert_eq!(at, 12);
        let mut t = String::from("é");
        assert_eq!(insert_mention(&mut t, (1, 2), "X"), 1, "not on a char boundary: unchanged");
        assert_eq!(insert_mention(&mut t, (5, 9), "X"), 1, "out of range: unchanged");
        assert_eq!(t, "é");
    }

    #[test]
    fn composer_submits_the_live_typed_text_not_an_empty_field() {
        // Regression: TextField::value() only updates on commit, so the composer keeps its live
        // text in CommentUi::composer_text. A click-draft with typed text must post comment.add
        // carrying that text and the page point — this is the whole "can't create a comment" bug.
        let (id, params) = submit_params(None, (120.5, -30.0), "  hello  ", None).expect("non-empty text posts");
        assert_eq!(id, "comment.add");
        assert_eq!(params["text"], "hello");
        assert_eq!(params["x"], 120.5);
        assert_eq!(params["y"], -30.0);
        assert_eq!(params["anchor"], true);
        assert!(submit_params(None, (0.0, 0.0), "   ", None).is_none(), "empty text posts nothing");

        // A thread's reply and an edit go to their own commands, also carrying the live text.
        let (id, params) = submit_params(Some(7), (0.0, 0.0), "a reply", None).unwrap();
        assert_eq!(id, "comment.reply");
        assert_eq!(params["id"], 7);
        assert_eq!(params["text"], "a reply");
        let editing = (9u64, 2usize, "old".to_string());
        let (id, params) = submit_params(Some(9), (0.0, 0.0), "edited", Some(&editing)).unwrap();
        assert_eq!(id, "comment.edit");
        assert_eq!(params["id"], 9);
        assert_eq!(params["reply"], 1, "message index 2 is reply 1");
        assert_eq!(params["text"], "edited");
    }

    #[test]
    fn clusters_group_nearby_points() {
        let pts = [pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(100.0, 0.0), pos2(5.0, 12.0), pos2(300.0, 300.0)];
        assert_eq!(cluster(&pts, CLUSTER_R), vec![vec![0, 1, 3], vec![2], vec![4]]);
        assert!(cluster(&[], CLUSTER_R).is_empty());
        assert_eq!(cluster(&pts, 0.0).len(), 5);
    }

    #[test]
    fn show_closes_the_draft_and_reply_reset_rules() {
        let mut ui = CommentUi::default();
        ui.draft = Some((1.0, 2.0, "draft".into()));
        ui.reply = "typed".into();
        ui.show(7);
        assert_eq!(ui.open, Some(7));
        assert!(ui.draft.is_none(), "showing a thread closes the draft");
        assert!(ui.reply.is_empty(), "a different thread clears the reply");
        ui.reply = "again".into();
        ui.show(7);
        assert_eq!(ui.reply, "again", "re-showing the same thread keeps the reply");
        ui.close();
        assert!(ui.open.is_none() && ui.draft.is_none(), "close ends the thread and the draft (reply text is kept, as upstream)");
    }
}
