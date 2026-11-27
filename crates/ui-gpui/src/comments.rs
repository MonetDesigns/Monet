//! Comments — the Comments panel and the canvas pins.
//!
//! Structure copied from `crates/ui-egui/src/canvas/comments.rs`:
//!   `CommentUi` (filter / sort / search / open thread) · `panel()` (the list) · `draw()` (pins).
//!
//! All data and commands come from `figcraft_engine::cmd::comment` (`Filter`, `Sort`, `Query`,
//! `query`, `frame_of`, `user_name`) and the document's own `Comment` type — nothing here invents
//! a command or a widget.

use crate::panels::{Cmd, RightFields};
use crate::widgets::input::TextField;
use crate::widgets::popover::{Open, popover_window, title_text};
use crate::widgets::{Dropdown, LabelButton, MenuRow, rgba, tokens};
use figcraft_doc::{Document, Message, NodeId};
use figcraft_engine::cmd::comment::{Filter, Query, Sort, frame_of, query};
use figcraft_i18n::tr;
use gpui::{AnyElement, App, Div, Entity, IntoElement, KeyDownEvent, Pixels, Point, SharedString, Size, Window, div, prelude::*, px};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

/// The Comments panel's local state (egui `CommentUi`): which threads show, how they are ordered,
/// the search text, and the thread open in a popover.
#[derive(Clone, Debug)]
pub(crate) struct CommentUi {
    pub filter: Filter,
    pub sort: Sort,
    pub search: String,
    pub open: Option<u64>,
}

impl Default for CommentUi {
    fn default() -> Self {
        // egui starts on Open (`app.ui.comments.filter`), newest activity first.
        Self { filter: Filter::Open, sort: Sort::Date, search: String::new(), open: None }
    }
}

/// A comment being written at a page point, with the field the composer draws into.
pub(crate) struct DraftState {
    pub page: NodeId,
    pub x: f64,
    pub y: f64,
    pub text: String,
    pub field: Entity<TextField>,
    /// The composer asks for the keyboard on its first frame.
    pub focus: bool,
    /// Byte offset of the live caret; `@mention` ends here, not at the end of the draft.
    pub caret: usize,
}

/// An open thread and the reply being written into it.
pub(crate) struct ThreadState {
    pub id: u64,
    pub reply: String,
    pub field: Entity<TextField>,
    pub focus: bool,
    /// The message index (0 = first message) being edited, if any.
    pub editing: Option<usize>,
    /// The thread-level More menu is open.
    pub more: bool,
    /// The message whose reaction picker is open, if any.
    pub reacting: Option<usize>,
    /// Byte offset of the reply caret (mentions at the composer also end here).
    pub caret: usize,
}

/// Thread-level actions that need the view's command/clipboard plumbing.
#[derive(Clone)]
pub(crate) enum ThreadAction {
    MarkUnread,
    CopyLink,
    CopyForAi,
    React(usize, String),
}

/// The `@query` being typed before the live caret: its byte range from `@` through the caret.
/// Text after the caret stays outside the range, so inserting a mention preserves it.
pub(crate) fn mention_query(text: &str, caret: usize) -> Option<(usize, usize)> {
    if caret > text.len() || !text.is_char_boundary(caret) {
        return None;
    }
    let at = text.get(..caret)?.rfind('@')?;
    let q = text.get(at + 1..caret)?;
    if q.contains('\n') || q.chars().count() > 40 || q.ends_with(char::is_whitespace) {
        return None;
    }
    let starts = text.get(..at)?.chars().next_back().is_none_or(|c| !c.is_alphanumeric());
    starts.then_some((at, caret))
}

/// People (not `me`) whose name, or a word in it, starts with `query` (any case).
pub(crate) fn mention_matches(people: &[String], query: &str, me: &str) -> Vec<String> {
    let q = query.to_lowercase();
    people
        .iter()
        .filter(|n| n.as_str() != me)
        .filter(|n| {
            let l = n.to_lowercase();
            l.starts_with(&q) || l.split_whitespace().any(|w| w.starts_with(&q))
        })
        .take(6)
        .cloned()
        .collect()
}

/// Replace the `@query` at `range` with `@name `.
pub(crate) fn insert_mention(text: &mut String, range: (usize, usize), name: &str) {
    let (a, mut b) = range;
    if a > b || b > text.len() || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
        return;
    }
    // The caret often sits before the space that follows the query. Replace that one space too,
    // otherwise the inserted trailing space doubles it and the next word drifts.
    if text.get(b..b + 1) == Some(" ") {
        b += 1;
    }
    text.replace_range(a..b, &format!("@{name} "));
}

/// A composer body submits only on Enter. Keeping this on the popover subtree matters: the
/// editor consumes Enter (`EditBuffer::Submit`) and blurs, but its parent still sees the bubble.
fn with_enter_submit(body: Div, submit: Rc<dyn Fn(&mut Window, &mut App)>) -> Div {
    body.on_key_down(move |e: &KeyDownEvent, window, cx| {
        if e.keystroke.key == "enter" && !e.keystroke.modifiers.shift {
            submit(window, cx);
            cx.stop_propagation();
        }
    })
}

/// A comment composer next to its pin (egui `draft_composer`). Posting hands the text back to
/// the view, which owns the `comment.add` command and closes the popover only on success.
pub(crate) fn draft_popover(
    app: &crate::CanvasApp,
    state: &Rc<RefCell<DraftState>>,
    canvas_origin: (f32, f32),
    viewport: Size<Pixels>,
    dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
    submit: Rc<dyn Fn(&mut Window, &mut App)>,
    pick: Rc<dyn Fn(&str, &mut Window, &mut App)>,
) -> AnyElement {
    let t = tokens();
    let (text, field, x, y, page, caret) = {
        let s = state.borrow();
        (s.text.clone(), s.field.clone(), s.x, s.y, s.page, s.caret)
    };
    // A draft only draws on its own page (the engine pins the page when the click is recorded).
    if app.session.page().is_ok_and(|p| p != page) {
        return div().into_any_element();
    }
    let at = app.to_screen(x, y);
    let pos = Point { x: px(f32::from(at.x) + canvas_origin.0 + 36.0), y: px(f32::from(at.y) + canvas_origin.1 - 28.0) };
    let people = app.session.ds().map(|ds| ds.doc.comment_people()).unwrap_or_default();
    let me = figcraft_engine::cmd::comment::user_name(&app.session);
    let names = mention_query(&text, caret).map(|(a, b)| mention_matches(&people, text.get(a + 1..b).unwrap_or(""), &me)).unwrap_or_default();
    // The estimate only drives viewport clamping; include the mention list so a picker near the
    // bottom of the screen lifts instead of drawing past it.
    let estimated_h = 112.0 + names.len() as f32 * 26.0;

    let mut body = div().flex().flex_col().gap(px(6.0)).child(div().w_full().child(field));
    if !names.is_empty() {
        let mut list = div().flex().flex_col().rounded(px(6.0)).bg(rgba(t.field)).overflow_hidden();
        for name in names {
            let picked = name.clone();
            let pick = pick.clone();
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(26.0))
                    .px(px(8.0))
                    .cursor_pointer()
                    .hover(|d| d.bg(rgba(t.hover)))
                    .id(gpui::ElementId::from(SharedString::from(format!("comment-mention-{name}"))))
                    .on_click(move |_, window, cx| pick(&picked, window, cx))
                    .child(avatar(&name))
                    .child(div().text_size(px(12.0)).text_color(rgba(t.text)).child(name)),
            );
        }
        body = body.child(list);
    }
    body = body.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(div().text_size(px(10.0)).text_color(rgba(t.text_tertiary)).child(tr("Click Post · @ to mention")))
            .child(LabelButton::new("comment-draft-post", tr("Post")).on_click({
                let submit = submit.clone();
                move |window, cx| submit(window, cx)
            })),
    );
    let body = with_enter_submit(body, submit);

    popover_window(Open::At(pos)).width(300.0).height(estimated_h).title(title_text(tr("Comment"))).body(body).show(viewport, dismiss).1
}

/// A thread popover next to its pin (egui `thread`): messages, resolve state and a reply field.
pub(crate) fn thread_popover(
    app: &crate::CanvasApp,
    state: &Rc<RefCell<crate::comments::ThreadState>>,
    canvas_origin: (f32, f32),
    viewport: Size<Pixels>,
    dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
    submit: Rc<dyn Fn(&mut Window, &mut App)>,
    resolve: Rc<dyn Fn(bool, &mut Window, &mut App)>,
    delete: Rc<dyn Fn(Option<usize>, &mut Window, &mut App)>,
    action: Rc<dyn Fn(ThreadAction, &mut Window, &mut App)>,
    pick_mention: Rc<dyn Fn(&str, &mut Window, &mut App)>,
) -> AnyElement {
    let t = tokens();
    let id = state.borrow().id;
    let reply_field = state.borrow().field.clone();
    let Ok(ds) = app.session.ds() else { return div().into_any_element() };
    let Some(c) = ds.doc.comment(id).cloned() else { return div().into_any_element() };
    if c.page != ds.page {
        return div().into_any_element();
    }
    let (x, y) = c.position(&ds.doc);
    let at = app.to_screen(x, y);
    let pos = Point { x: px(f32::from(at.x) + canvas_origin.0 + 36.0), y: px(f32::from(at.y) + canvas_origin.1 - 28.0) };
    let me = figcraft_engine::cmd::comment::user_name(&app.session);
    let (reply, caret) = {
        let s = state.borrow();
        (s.reply.clone(), s.caret)
    };
    let people = app.session.ds().map(|ds| ds.doc.comment_people()).unwrap_or_default();
    let names = mention_query(&reply, caret).map(|(a, b)| mention_matches(&people, reply.get(a + 1..b).unwrap_or(""), &me)).unwrap_or_default();

    let mut body = div().flex().flex_col().gap(px(8.0));
    body = body.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(div().text_size(px(11.0)).text_color(rgba(if c.resolved { t.text_secondary } else { t.accent })).child(if c.resolved {
                tr("Resolved")
            } else {
                tr("Open")
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .child(LabelButton::new("comment-thread-resolve", if c.resolved { tr("Reopen") } else { tr("Resolve") }).secondary().on_click({
                        let resolved = c.resolved;
                        move |window, cx| resolve(!resolved, window, cx)
                    }))
                    .child(LabelButton::new("comment-thread-more", tr("More")).secondary().on_click({
                        let state = state.clone();
                        move |window, cx| {
                            let next = {
                                let mut s = state.borrow_mut();
                                s.more = !s.more;
                                s.more
                            };
                            if next {
                                state.borrow_mut().reacting = None;
                            }
                            window.refresh();
                        }
                    })),
            ),
    );
    if state.borrow().more {
        let menu = |id: &'static str, label: &'static str, act: ThreadAction| {
            let action = action.clone();
            LabelButton::new(id, tr(label)).secondary().on_click(move |window, cx| action(act.clone(), window, cx))
        };
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .rounded(px(6.0))
                .bg(rgba(t.field))
                .border_1()
                .border_color(rgba(t.border))
                .p(px(4.0))
                .child(menu("comment-thread-unread", "Mark as unread", ThreadAction::MarkUnread))
                .child(menu("comment-thread-link", "Copy link", ThreadAction::CopyLink))
                .child(menu("comment-thread-ai", "Copy for AI", ThreadAction::CopyForAi)),
        );
    }

    let editing = state.borrow().editing;
    let mut messages = div().id("comment-thread-messages").flex().flex_col().gap(px(10.0)).max_h(px(320.0)).overflow_y_scroll();
    for (index, m) in c.messages().enumerate() {
        let mut entry = message_row(m, &me);
        if m.author == me {
            let edit = {
                let state = state.clone();
                let field = reply_field.clone();
                let text = m.text.clone();
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    let field = field.clone();
                    let text = text.clone();
                    {
                        let mut s = state.borrow_mut();
                        s.editing = Some(index);
                        s.reply = text.clone();
                    }
                    field.update(cx, |tf, cx| {
                        tf.edit().update(cx, |edit, _| edit.set_draft_text(text));
                        tf.edit().update(cx, |edit, _| edit.focus_editor(window));
                    });
                    window.refresh();
                }) as Rc<dyn Fn(&mut Window, &mut App)>
            };
            let del = {
                let delete = delete.clone();
                Rc::new(move |window: &mut Window, cx: &mut App| delete(index.checked_sub(1), window, cx)) as Rc<dyn Fn(&mut Window, &mut App)>
            };
            entry = entry.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .pl(px(30.0))
                    .child(
                        LabelButton::new(gpui::ElementId::from(("comment-edit", index as u64)), tr("Edit"))
                            .secondary()
                            .on_click(move |window, cx| edit(window, cx)),
                    )
                    .child(
                        LabelButton::new(
                            gpui::ElementId::from(("comment-delete", index as u64)),
                            if index == 0 { tr("Delete thread") } else { tr("Delete") },
                        )
                        .secondary()
                        .on_click(move |window, cx| del(window, cx)),
                    ),
            );
        }
        let mut reactions = div().flex().flex_wrap().gap(px(4.0)).pl(px(30.0));
        for (ri, r) in m.reactions.iter().enumerate() {
            let on = r.users.iter().any(|u| u == &me);
            let emoji = r.emoji.clone();
            let act = ThreadAction::React(index, emoji.clone());
            let action = action.clone();
            let label = format!("{emoji} {}", r.users.len());
            let id = gpui::ElementId::from(SharedString::from(format!("comment-reaction-{index}-{ri}")));
            let button = LabelButton::new(id, label).secondary().selected(on);
            reactions = reactions.child(button.on_click(move |window, cx| action(act.clone(), window, cx)));
        }
        let toggle = state.clone();
        let add_id = gpui::ElementId::from(SharedString::from(format!("comment-react-add-{index}")));
        reactions = reactions.child(LabelButton::new(add_id, "+").secondary().on_click(move |window, _| {
            let mut s = toggle.borrow_mut();
            s.reacting = if s.reacting == Some(index) { None } else { Some(index) };
            s.more = false;
            window.refresh();
        }));
        entry = entry.child(reactions);
        if state.borrow().reacting == Some(index) {
            let mut picker = div().flex().flex_wrap().gap(px(2.0)).pl(px(30.0));
            for emoji in REACTIONS {
                let action = action.clone();
                let act = ThreadAction::React(index, emoji.to_string());
                let pick_id = gpui::ElementId::from(SharedString::from(format!("comment-react-pick-{index}-{emoji}")));
                picker = picker.child(LabelButton::new(pick_id, emoji).secondary().on_click(move |window, cx| {
                    action(act.clone(), window, cx);
                    // The picker closes after a pick; the command's result repaints the thread.
                }));
            }
            entry = entry.child(picker);
        }
        messages = messages.child(entry);
    }
    body = body.child(messages);
    let cancel = {
        let state = state.clone();
        let field = reply_field.clone();
        Rc::new(move |window: &mut Window, cx: &mut App| {
            let field = field.clone();
            {
                let mut s = state.borrow_mut();
                s.editing = None;
                s.reply.clear();
            }
            field.update(cx, |tf, cx| {
                tf.edit().update(cx, |edit, _| edit.set_draft_text(""));
                tf.edit().update(cx, |edit, _| edit.focus_editor(window));
            });
            window.refresh();
        }) as Rc<dyn Fn(&mut Window, &mut App)>
    };
    let mut reply_actions = div().flex().flex_row().justify_end().gap(px(6.0)).child(
        LabelButton::new("comment-thread-reply", if editing.is_some() { tr("Save") } else { tr("Reply") }).on_click({
            let submit = submit.clone();
            move |window, cx| submit(window, cx)
        }),
    );
    if editing.is_some() {
        reply_actions =
            reply_actions.child(LabelButton::new("comment-thread-cancel", tr("Cancel")).secondary().on_click(move |window, cx| cancel(window, cx)));
    }
    let mut composer = div().flex().flex_col().gap(px(4.0));
    if !names.is_empty() {
        let mut list = div().flex().flex_col().rounded(px(6.0)).bg(rgba(t.field)).overflow_hidden();
        for name in names {
            let picked = name.clone();
            let pick = pick_mention.clone();
            let id = gpui::ElementId::from(SharedString::from(format!("comment-reply-mention-{name}")));
            list = list.child(LabelButton::new(id, name).secondary().on_click(move |window, cx| pick(&picked, window, cx)));
        }
        composer = composer.child(list);
    }
    composer = composer.child(div().w_full().child(reply_field));
    body =
        body.child(div().flex().flex_col().gap(px(6.0)).border_t_1().border_color(rgba(t.border)).pt(px(8.0)).child(composer).child(reply_actions));
    let body = with_enter_submit(body, submit);

    popover_window(Open::At(pos)).width(320.0).height(560.0).title(title_text(tr("Comment"))).body(body).show(viewport, dismiss).1
}

/// Reactions offered on a message (the reference's set).
pub(crate) const REACTIONS: [&str; 8] = ["👍", "❤", "😂", "🎉", "👀", "🙏", "🔥", "✅"];

/// One message: author + age, then its text (mentions stay readable as `@name`).
fn message_row(m: &Message, me: &str) -> Div {
    let t = tokens();
    let mut row = div().flex().flex_col().gap(px(2.0));
    let mut head = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(avatar(&m.author))
        .child(div().text_size(px(11.0)).text_color(rgba(t.text)).child(m.author.clone()))
        .when(m.author == me, |d| d.child(div().text_size(px(9.0)).text_color(rgba(t.text_tertiary)).child(tr("you"))))
        .child(div().text_size(px(10.0)).text_color(rgba(t.text_tertiary)).child(ago(m.created)))
        .when(m.edited != 0, |d| d.child(div().text_size(px(9.0)).text_color(rgba(t.text_tertiary)).child(tr("edited"))));
    if m.agent {
        head = head.child(div().text_size(px(9.0)).text_color(rgba(t.text_tertiary)).child(tr("AI")));
    }
    row = row.child(head);
    row.child(div().pl(px(30.0)).text_size(px(12.0)).text_color(rgba(t.text)).child(m.text.clone()))
}

/// One row of the panel's list, formatted once per frame in `CanvasView::sync_panel`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CommentRow {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub ago: String,
    pub unread: bool,
    /// `frame · N replies · Resolved` (egui's `meta.join(" · ")`).
    pub meta: String,
}

/// The comments `filter` / `sort` / `search` keep, formatted for the panel.
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
            }
        })
        .collect()
}

/// The Comments panel body (egui `comments::panel`): title + filter/sort menu, then the list, or
/// the empty state.
pub(crate) fn panel(f: &RightFields, rows: &[CommentRow], total: usize) -> Div {
    let t = tokens();
    let cu = f.comment_ui().borrow().clone();
    let mut body = div().flex().flex_col().w_full();
    // Header: "Comments" on the left, the filter/sort menu on the right (egui `Filter` + `Sort`).
    body = body.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(40.0))
            .px(px(16.0))
            .child(div().flex_1().text_size(px(12.0)).font_weight(crate::panels::UI_FONT_REGULAR).text_color(rgba(t.text)).child(tr("Comments")))
            .child(filter_button(f, cu.filter, cu.sort)),
    );
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
    for row in rows {
        body = body.child(comment_row(f, row, cu.open == Some(row.id)));
    }
    body
}

/// One thread row: unread dot, avatar, author + age, body, meta (egui's 52 + 16·lines row).
fn comment_row(f: &RightFields, row: &CommentRow, open: bool) -> impl IntoElement {
    let t = tokens();
    let cmds = f.queue();
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
        .on_click(move |_, _, _| cmds.push(Cmd { id: "comment.open", params: Some(json!({ "id": id })) }))
        // Unread dot (egui draws it at x+8; the column keeps the avatar aligned).
        .child(
            div()
                .w(px(8.0))
                .flex_none()
                .flex()
                .justify_center()
                .pt(px(6.0))
                .child(div().w(px(7.0)).h(px(7.0)).rounded_full().when(row.unread, |d| d.bg(rgba(t.accent)))),
        )
        .child(avatar(&row.author))
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
                        .child(div().flex_1().text_size(px(12.0)).text_color(rgba(t.text)).child(row.author.clone()))
                        .child(div().text_size(px(11.0)).text_color(rgba(t.text_tertiary)).child(row.ago.clone())),
                )
                .child(div().text_size(px(12.0)).text_color(rgba(t.text)).child(row.body.clone()))
                .when(!row.meta.is_empty(), |d| d.child(div().text_size(px(11.0)).text_color(rgba(t.text_secondary)).child(row.meta.clone()))),
        )
}

/// A round avatar with the author's initial (egui `avatar` + `author_color`).
fn avatar(name: &str) -> Div {
    let color = author_color(name);
    let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    div()
        .w(px(24.0))
        .h(px(24.0))
        .flex_none()
        .rounded_full()
        .bg(rgba(color))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.0))
        .text_color(rgba(crate::canvas::geom::Color32::WHITE))
        .child(initial)
}

/// The filter/sort button: a dropdown whose menu sets the local filter/sort or marks all read
/// (egui's `Adjust` icon + `menu_popup` with `Filter::ALL`, `Sort::ALL`, "Mark all as read").
fn filter_button(f: &RightFields, filter: Filter, sort: Sort) -> Dropdown {
    let rows: Vec<MenuRow> = {
        let mut v: Vec<MenuRow> = Filter::ALL.iter().map(|x| MenuRow::row(tr(x.label())).checked(Some(*x == filter))).collect();
        v.extend(Sort::ALL.iter().map(|x| MenuRow::row(tr(x.label())).separator().checked(Some(*x == sort))));
        v.push(MenuRow::row(tr("Mark all as read")));
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
                if let Some(f) = Filter::ALL.iter().find(|x| tr(x.label()) == label) {
                    ui.borrow_mut().filter = *f;
                } else if let Some(s) = Sort::ALL.iter().find(|x| tr(x.label()) == label) {
                    ui.borrow_mut().sort = *s;
                } else if label == tr("Mark all as read") {
                    cmds.push(Cmd { id: "comment.markRead", params: Some(json!({})) });
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), menu, on_pick, cx));
        });
    }
    d
}

/// Pins on the canvas: the current page's threads, the open one on top (egui `pins` + `paint_pin`).
pub(crate) fn draw(app: &crate::CanvasApp, painter: &mut crate::canvas::painter::Painter, t: &crate::theme::Tokens) {
    let Ok(ds) = app.session.ds() else { return };
    let me = figcraft_engine::cmd::comment::user_name(&app.session);
    let q = Query { page: Some(ds.page), ..Query::default() };
    let mut list = query(&ds.doc, &q, &me);
    list.reverse();
    for c in list {
        let (x, y) = c.position(&ds.doc);
        let at = app.to_screen(x, y);
        let size = 28.0;
        let rect =
            crate::canvas::geom::Rect::from_min_max(crate::canvas::geom::pos2(at.x, at.y - size), crate::canvas::geom::pos2(at.x + size, at.y));
        let fill = if c.resolved { t.text_tertiary } else { t.accent };
        painter.rect_filled(rect, crate::canvas::geom::CornerRadius::same(8), fill);
        painter.text(
            crate::canvas::geom::pos2(rect.min.x + size / 2.0, rect.min.y + size / 2.0),
            crate::canvas::geom::Align2::CENTER_CENTER,
            c.id.to_string(),
            crate::canvas::geom::regular(12.0),
            crate::canvas::geom::Color32::WHITE,
        );
    }
}

/// A stable colour per author (egui `author_color`): the name's hash into a small palette.
pub(crate) fn author_color(name: &str) -> crate::canvas::geom::Color32 {
    const PALETTE: [&str; 6] = ["#4C8DFF", "#E5484D", "#30A46C", "#F76B15", "#8E4EC6", "#E93D82"];
    let mut h: u64 = 0;
    for b in name.bytes() {
        h = h.wrapping_mul(31).wrapping_add(u64::from(b));
    }
    let hex = PALETTE[(h % PALETTE.len() as u64) as usize];
    crate::canvas::geom::Color32::from_hex(hex).unwrap_or(crate::canvas::geom::Color32::WHITE)
}

/// `now` / `3m` / `2h` / `5d` for a Unix timestamp (egui `ago`).
pub(crate) fn ago(created: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let d = now.saturating_sub(created);
    if created == 0 {
        String::new()
    } else if d < 60 {
        tr("now").to_string()
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86_400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86_400)
    }
}

/// A local UI-state cell for the comment panel (used by `RightFields`).
pub(crate) type CommentState = Rc<RefCell<CommentUi>>;

#[cfg(test)]
mod tests {
    use super::{insert_mention, mention_matches, mention_query};

    #[test]
    fn mention_query_only_starts_a_token() {
        assert_eq!(mention_query("hi @Ad", 6), Some((3, 6)));
        assert_eq!(mention_query("@Ad", 3), Some((0, 3)));
        assert_eq!(mention_query("mail@Ad", 7), None);
        assert_eq!(mention_query("@Ad ", 4), None);
        assert_eq!(mention_query("no mention", 10), None);
    }

    #[test]
    fn mention_query_ends_at_the_caret_and_keeps_the_tail() {
        assert_eq!(mention_query("Hi @Ad tomorrow", 6), Some((3, 6)));
        assert_eq!(mention_query("Hi @Ad tomorrow", 15), Some((3, 15)));
    }

    #[test]
    fn mention_matches_skips_me_and_matches_words() {
        let people = vec!["Ada Lovelace".to_string(), "Alan Turing".to_string(), "Me".to_string()];
        assert_eq!(mention_matches(&people, "ad", "Me"), vec!["Ada Lovelace".to_string()]);
        assert_eq!(mention_matches(&people, "tur", "Me"), vec!["Alan Turing".to_string()]);
        assert!(mention_matches(&people, "me", "Me").is_empty());
    }

    #[test]
    fn insert_mention_replaces_the_query() {
        let mut text = "hi @Ad".to_string();
        insert_mention(&mut text, (3, 6), "Ada Lovelace");
        assert_eq!(text, "hi @Ada Lovelace ");
        let mut text = "Hi @Ad tomorrow".to_string();
        insert_mention(&mut text, (3, 6), "Ada Lovelace");
        assert_eq!(text, "Hi @Ada Lovelace tomorrow");
    }
}
