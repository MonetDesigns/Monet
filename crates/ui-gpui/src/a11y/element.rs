//! Element-side hook: wraps any GPUI element and records its semantics.
//!
//! gpui-pre 0.3.12 does this inside `Element::a11y_role` / `write_a11y_info`;
//! GPUI 0.2.2 has neither, so the same information is carried by a thin
//! wrapper element. `prepaint` receives the final bounds from GPUI, which is
//! exactly what AccessKit needs for hit testing and screen-reader focus.

use super::registry::{self, Control, States};
use super::tree::CLICKABLE;
use crate::a11y::Region;
use accesskit::{Action, Role};
use gpui::{
    App, Bounds, Element, ElementId, FocusHandle, GlobalElementId, IntoElement, LayoutId,
    Pixels, Window,
};
use std::panic::Location;

/// What a screen reader should see for one control.
#[derive(Clone)]
pub(crate) struct Meta {
    pub(crate) key: String,
    pub(crate) region: Region,
    pub(crate) role: Role,
    pub(crate) states: States,
    pub(crate) focus: Option<FocusHandle>,
    /// Whether a stable keyed focus handle should be attached automatically.
    pub(crate) focusable: bool,
    pub(crate) command: Option<String>,
    pub(crate) actions: Vec<Action>,
}

impl Meta {
    fn new(key: impl Into<String>, role: Role, label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            key: key.into(),
            region: Region::Canvas,
            role,
            states: States { label, ..States::default() },
            focus: None,
            focusable: false,
            command: None,
            actions: Vec::new(),
        }
    }

    pub(crate) fn button(key: impl Into<String>, label: impl Into<String>) -> Self {
        let mut m = Self::new(key, Role::Button, label);
        m.focusable = true;
        m.actions.extend_from_slice(&CLICKABLE);
        m
    }

    pub(crate) fn toggle(key: impl Into<String>, label: impl Into<String>, on: bool) -> Self {
        let mut m = Self::button(key, label);
        m.states.toggled = Some(on);
        m
    }

    pub(crate) fn tab(key: impl Into<String>, label: impl Into<String>, selected: bool) -> Self {
        let mut m = Self::new(key, Role::Tab, label);
        m.focusable = true;
        m.states.selected = selected;
        m.actions.extend_from_slice(&CLICKABLE);
        m
    }

    pub(crate) fn row(key: impl Into<String>, label: impl Into<String>, selected: bool) -> Self {
        let mut m = Self::new(key, Role::TreeItem, label);
        m.focusable = true;
        m.states.selected = selected;
        m.actions.extend_from_slice(&CLICKABLE);
        m
    }

    pub(crate) fn region(mut self, region: Region) -> Self {
        self.region = region;
        self
    }

    pub(crate) fn focus(mut self, handle: &FocusHandle) -> Self {
        self.actions.push(Action::Focus);
        self.focus = Some(handle.clone());
        self
    }

    pub(crate) fn description(mut self, text: impl Into<String>) -> Self {
        self.states.description = text.into();
        self
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.states.disabled = disabled;
        if disabled {
            self.actions.clear();
        }
        self
    }

    pub(crate) fn value(mut self, value: impl Into<String>) -> Self {
        self.states.value = Some(value.into());
        self
    }

    /// Engine command to run when assistive technology activates the control.
    pub(crate) fn command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }
}

/// Wraps an element, registering one AccessKit node during `prepaint`.
pub(crate) struct A11yElement<E> {
    inner: E,
    meta: Meta,
}

impl<E: Element> Element for A11yElement<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.inner.id()
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        self.inner.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.inner.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let state = self.inner.prepaint(id, inspector_id, bounds, request_layout, window, cx);
        let meta = self.meta.clone();
        let focus = match meta.focus.clone() {
            Some(handle) => Some(handle),
            None if meta.focusable => Some(registry::focus_for(cx, &meta.key)),
            None => None,
        };
        registry::with(cx, |reg| {
            reg.push(Control {
                node_id: registry::node_id(&meta.key),
                key: meta.key,
                region: meta.region,
                role: meta.role,
                bounds,
                states: meta.states,
                focus,
                command: meta.command,
                actions: meta.actions,
            });
        });
        state
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.paint(id, inspector_id, bounds, request_layout, prepaint, window, cx);
    }
}

impl<E: Element> IntoElement for A11yElement<E> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Attach a11y semantics to any element. Call sites outside `widgets/` (which
/// are frozen) use this; the wrapper is inert visually.
pub(crate) trait A11yExt: IntoElement + Sized {
    fn a11y(self, meta: Meta) -> A11yElement<Self::Element> {
        A11yElement { inner: self.into_element(), meta }
    }
}

impl<T: IntoElement> A11yExt for T {}
