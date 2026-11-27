//! Two remaining engine surfaces the GPUI shell used to stub:
//!
//! - `dialog.cursorChat`: the live-session cursor chat field (`collab.cursorChat`).
//! - `panel.libraries { publish: true }`: the library publish form (`library.publish`).
//!
//! They live next to each other because both are small detached popovers owned by the canvas
//! view, not new panel surfaces.

use crate::widgets::input::TextField;
use crate::widgets::popover::{Open, popover_window, title_text};
use crate::widgets::{Checkbox, LabelButton, rgba, tokens};
use figcraft_doc::Document;
use figcraft_engine::library::{Selection, default_selection, publishable};
use figcraft_i18n::tr;
use gpui::{AnyElement, App, Entity, IntoElement, KeyDownEvent, Pixels, Point, Size, Window, div, prelude::*, px};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

/// The cursor-chat composer's text and field.
pub(crate) struct CursorChatState {
    pub text: String,
    pub field: Entity<TextField>,
    pub focus: bool,
}

/// What the publish form has picked; `all` is fixed while the form is open, `picked` toggles.
pub(crate) struct LibraryPublishState {
    pub description: String,
    pub field: Entity<TextField>,
    pub all: Selection,
    pub picked: Selection,
}

impl LibraryPublishState {
    pub(crate) fn new(doc: &Document, field: Entity<TextField>) -> Self {
        let all = publishable(doc);
        let picked = default_selection(doc, doc.published.as_ref().map(|p| &p.manifest));
        Self { description: String::new(), field, all, picked }
    }
}

/// A small live-chat field, opened at the pointer (egui's `/` cursor chat).
pub(crate) fn cursor_chat_popover(
    state: &Rc<RefCell<CursorChatState>>,
    at: Point<Pixels>,
    viewport: Size<Pixels>,
    dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
    submit: Rc<dyn Fn(&mut Window, &mut App)>,
) -> AnyElement {
    let t = tokens();
    let field = state.borrow().field.clone();
    let body = div().flex().flex_col().gap(px(6.0)).child(div().w_full().child(field)).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(div().text_size(px(10.0)).text_color(rgba(t.text_tertiary)).child(tr("Type to send")))
            .child(LabelButton::new("cursor-chat-send", tr("Send")).on_click({
                let submit = submit.clone();
                move |window, cx| submit(window, cx)
            })),
    );
    let body = body.on_key_down(move |e: &KeyDownEvent, window, cx| {
        if e.keystroke.key == "enter" && !e.keystroke.modifiers.shift {
            submit(window, cx);
            cx.stop_propagation();
        }
    });

    popover_window(Open::At(at)).width(260.0).height(110.0).title(title_text(tr("Cursor chat"))).body(body).show(viewport, dismiss).1
}

/// The library publish form: publishable components/styles/collections, a description and
/// `library.publish`. Mirrors `ui-egui/src/panels/assets_panel.rs::publish_form`.
pub(crate) fn library_publish_popover(
    app: &crate::CanvasApp,
    state: &Rc<RefCell<LibraryPublishState>>,
    viewport: Size<Pixels>,
    dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
    publish: Rc<dyn Fn(&mut Window, &mut App)>,
) -> AnyElement {
    let t = tokens();
    let Ok(ds) = app.session.ds() else { return div().into_any_element() };
    let doc = &ds.doc;
    let (field, all, picked, any, empty) = {
        let s = state.borrow();
        let any = !s.picked.is_empty();
        let empty = s.all.is_empty();
        (s.field.clone(), s.all.clone(), s.picked.clone(), any, empty)
    };
    let mut body = div().flex().flex_col().gap(px(6.0));
    if empty {
        body = body.child(
            div()
                .text_size(px(11.0))
                .text_color(rgba(t.text_secondary))
                .child(tr("Create components, styles or variables to publish them as a library.")),
        );
    } else {
        let mut choices = div().id("library-publish-choices").flex().flex_col().gap(px(2.0)).max_h(px(360.0)).overflow_y_scroll();
        if !all.components.is_empty() {
            choices = choices.child(section_title(tr("Components")));
            for id in &all.components {
                let name = doc.node(*id).map(|n| n.name.clone()).unwrap_or_default();
                choices = choices.child(toggle_row(state, "c", &id.0.to_string(), name, picked.components.contains(id)));
            }
        }
        if !all.styles.is_empty() {
            choices = choices.child(section_title(tr("Styles")));
            for id in &all.styles {
                let name = doc.style(id).map(|s| s.name.clone()).unwrap_or_default();
                choices = choices.child(toggle_row(state, "s", id, name, picked.styles.contains(id)));
            }
        }
        if !all.collections.is_empty() {
            choices = choices.child(section_title(tr("Variable collections")));
            for id in &all.collections {
                let name = doc.variables.collection_by_id(id).map(|c| c.name.clone()).unwrap_or_default();
                choices = choices.child(toggle_row(state, "v", id, name, picked.collections.contains(id)));
            }
        }
        body = body.child(choices).child(
            div()
                .mt(px(4.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(div().text_size(px(10.0)).text_color(rgba(t.text_secondary)).child(tr("Description (optional)")))
                .child(div().w_full().child(field)),
        );
    }
    body = body.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(8.0))
            .mt(px(4.0))
            .child(LabelButton::new("library-publish", tr("Publish")).disabled(!any).on_click(move |window, cx| publish(window, cx))),
    );

    popover_window(Open::At(Point { x: px(crate::panels::RAIL_W + 24.0), y: px(116.0) }))
        .width(320.0)
        .height(520.0)
        .title(title_text(tr("Publish library")))
        .body(body)
        .show(viewport, dismiss)
        .1
}

fn section_title(text: impl Into<gpui::SharedString>) -> impl IntoElement {
    let t = tokens();
    div().pt(px(2.0)).text_size(px(10.0)).text_color(rgba(t.text_secondary)).child(text.into())
}

/// One publishable asset; the shared `Checkbox` owns selection and accessibility.
fn toggle_row(state: &Rc<RefCell<LibraryPublishState>>, kind: &'static str, key: &str, label: String, on: bool) -> impl IntoElement {
    let state = state.clone();
    let key = key.to_string();
    let id = gpui::ElementId::from(gpui::SharedString::from(format!("library-publish-{kind}-{key}")));
    Checkbox::new(id).label(label).checked(on).text_xs().on_change(move |next, window, _| {
        let wanted = *next;
        let mut s = state.borrow_mut();
        match kind {
            "c" => {
                let Some(id) = key.parse::<u64>().ok().map(figcraft_doc::NodeId) else { return };
                if wanted {
                    if !s.picked.components.contains(&id) {
                        s.picked.components.push(id);
                    }
                } else {
                    s.picked.components.retain(|x| *x != id);
                }
            }
            "s" => {
                if wanted {
                    if !s.picked.styles.iter().any(|x| x == &key) {
                        s.picked.styles.push(key.clone());
                    }
                } else {
                    s.picked.styles.retain(|x| x != &key);
                }
            }
            _ => {
                if wanted {
                    if !s.picked.collections.iter().any(|x| x == &key) {
                        s.picked.collections.push(key.clone());
                    }
                } else {
                    s.picked.collections.retain(|x| x != &key);
                }
            }
        }
        drop(s);
        window.refresh();
    })
}

/// The include list `library.publish` takes from the form.
pub(crate) fn publish_params(state: &LibraryPublishState) -> Value {
    let mut include: Vec<Value> = Vec::new();
    include.extend(state.picked.components.iter().map(|id| json!(id.0)));
    include.extend(state.picked.styles.iter().map(|id| json!(id)));
    include.extend(state.picked.collections.iter().map(|id| json!(id)));
    json!({ "description": state.description.trim(), "include": include })
}
