//! `LabelButton` — a small text action (Post / Reply / Send / Publish).
//!
//! The egui shell draws these with `widgets::button`; the GPUI shell only had icon buttons and
//! ad-hoc clickable `div()`s. Keep this new control separate rather than changing the frozen
//! `IconButton` / `dialogs::Button` APIs.

use crate::canvas::geom::Color32;
use crate::widgets::{rgba, tokens};
use gpui::{App, Div, ElementId, IntoElement, SharedString, Stateful, Window, div, prelude::*, px};

type ClickHandler = Box<dyn Fn(&mut Window, &mut App)>;

pub struct LabelButton {
    id: ElementId,
    label: SharedString,
    primary: bool,
    disabled: bool,
    on_click: Option<ClickHandler>,
}

impl LabelButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self { id: id.into(), label: label.into(), primary: true, disabled: false, on_click: None }
    }

    /// Secondary actions use the field surface instead of the accent fill.
    pub fn secondary(mut self) -> Self {
        self.primary = false;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl IntoElement for LabelButton {
    type Element = Stateful<Div>;

    fn into_element(self) -> Self::Element {
        let t = tokens();
        let primary = self.primary;
        let disabled = self.disabled;
        let label = self.label;
        let click = self.on_click;
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(24.0))
            .px(px(10.0))
            .rounded(px(5.0))
            .bg(rgba(if disabled {
                t.field
            } else if primary {
                t.accent
            } else {
                t.field
            }))
            .when(disabled, |d| d.opacity(0.45))
            .when(!disabled, |d| d.cursor_pointer())
            .when(!disabled && !primary, |d| d.hover(|d| d.bg(rgba(t.hover))))
            .when_some(click.filter(|_| !disabled), |d, f| d.on_click(move |_, window, cx| f(window, cx)))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgba(if disabled {
                        t.text_tertiary
                    } else if primary {
                        Color32::WHITE
                    } else {
                        t.text
                    }))
                    .child(label),
            )
    }
}
