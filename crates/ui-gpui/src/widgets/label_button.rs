//! `LabelButton` — a small text action (Post / Reply / Send / Publish).
//!
//! The egui shell draws these with `widgets::button`; the GPUI shell only had icon buttons and
//! ad-hoc clickable `div()`s. Keep this new control separate rather than changing the frozen
//! `IconButton` / `dialogs::Button` APIs.

use crate::a11y::{A11yExt, Meta, focus_for};
use crate::canvas::geom::Color32;
use crate::widgets::{rgba, tokens};
use gpui::{AnyElement, App, Div, ElementId, FocusHandle, IntoElement, SharedString, Window, div, prelude::*, px};
use std::rc::Rc;

type ClickHandler = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct LabelButton {
    id: ElementId,
    label: SharedString,
    primary: bool,
    disabled: bool,
    selected: bool,
    focus: Option<FocusHandle>,
    on_click: Option<ClickHandler>,
}

impl LabelButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self { id: id.into(), label: label.into(), primary: true, disabled: false, selected: false, focus: None, on_click: None }
    }

    /// Register this button in the keyboard focus order. The handle is keyed by the element id,
    /// so it survives re-renders instead of creating a new focus target every frame.
    pub fn focusable(mut self, cx: &mut App) -> Self {
        let key = format!("label-button:{}", self.id);
        // `focus_for` returns a stable handle; mark it as a tab stop as well, since an explicit
        // `track_focus` handle does not inherit the div's `tab_stop` flag.
        self.focus = Some(focus_for(cx, &key).tab_stop(true));
        self
    }

    /// Secondary actions use the field surface instead of the accent fill.
    pub fn secondary(mut self) -> Self {
        self.primary = false;
        self
    }

    /// A persistent on/off state (reaction chips, toggle-like actions).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl IntoElement for LabelButton {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let t = tokens();
        let primary = self.primary;
        let disabled = self.disabled;
        let selected = self.selected;
        let a11y_key = format!("label-button:{}", self.id);
        let a11y_label = self.label.clone();
        let focus = self.focus;
        let click = self.on_click;
        let mut button = div()
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
            } else if selected {
                t.selected_row
            } else {
                t.field
            }))
            .when(disabled, |d| d.opacity(0.45))
            .when(!disabled, |d| d.cursor_pointer())
            .when(!disabled && !primary && !selected, |d| d.hover(|d| d.bg(rgba(t.hover))))
            .when(!disabled, |d| {
                d.focus(move |s| {
                    s.bg(rgba(if primary {
                        t.accent_hover
                    } else if selected {
                        t.hover
                    } else {
                        t.selected_row
                    }))
                })
            })
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgba(if disabled {
                        t.text_tertiary
                    } else if primary {
                        Color32::WHITE
                    } else if selected {
                        t.accent
                    } else {
                        t.text
                    }))
                    .child(self.label),
            );
        if !disabled {
            if let Some(f) = click.as_ref().cloned() {
                button = button.on_click(move |_, window, cx| f(window, cx));
            }
            if let Some(handle) = focus.as_ref() {
                let keyboard = click.clone();
                button = button.track_focus(handle).tab_stop(true).on_key_down(move |e, window, cx| {
                    let key = e.keystroke.key.as_str();
                    if matches!(key, "enter" | "space" | " ") {
                        if let Some(f) = keyboard.as_ref() {
                            f(window, cx);
                        }
                        cx.stop_propagation();
                    }
                });
            }
        }
        let meta = Meta::button(a11y_key, a11y_label);
        let meta = if !disabled { if let Some(handle) = focus.as_ref() { meta.focus(handle) } else { meta } } else { meta };
        button.a11y(meta.disabled(disabled)).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Render, TestAppContext};
    use std::cell::Cell;

    struct Harness {
        clicks: Rc<Cell<usize>>,
    }

    impl Render for Harness {
        fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();
            div()
                .size_full()
                .child(LabelButton::new("label-button-test", "Activate").focusable(cx).on_click(move |_, _| clicks.set(clicks.get() + 1)))
        }
    }

    /// 共享按钮要真的能拿到焦点：点一下聚焦，随后 Enter / Space 和指针一样触发同一个动作。
    #[gpui::test]
    async fn pointer_focus_then_enter_and_space_activate(cx: &mut TestAppContext) {
        let clicks = Rc::new(Cell::new(0));
        let (view, window) = cx.add_window_view(|_window, _cx| Harness { clicks: clicks.clone() });

        let p = gpui::point(px(12.0), px(12.0));
        window.simulate_mouse_down(p, gpui::MouseButton::Left, gpui::Modifiers::default());
        window.simulate_mouse_up(p, gpui::MouseButton::Left, gpui::Modifiers::default());
        assert_eq!(clicks.get(), 1, "the pointer must still activate the button");

        window.simulate_keystrokes("enter");
        assert_eq!(clicks.get(), 2, "Enter on the focused LabelButton must run its action");

        window.simulate_keystrokes("space");
        assert_eq!(clicks.get(), 3, "Space on the focused LabelButton must run its action");

        // Keep the view alive so the test window is not dropped before the assertions.
        let _ = view;
    }
}
