//! A21: the control registry that GPUI 0.2 does not provide.
//!
//! Copied/adapted from gpui-pre 0.3.12 (`src/window/a11y.rs`) and the gpui-kit
//! accessibility guide: an AccessKit node needs a stable id, a role, a name,
//! states and actions. GPUI 0.2 has none of the element-side hooks, so instead
//! of waiting for the framework, [`super::element::A11yElement`] records every
//! wrapped control during `prepaint` (where GPUI already hands us the final
//! bounds) and the rest of the module turns the records into a tree.
//!
//! Two buffers matter: `current` is built this frame, `previous` is what the
//! user actually saw (and what Tab / focus rings must use).

use crate::a11y::Region;
use accesskit::{Action, NodeId, Role};
use gpui::{App, Bounds, FocusHandle, Global, Pixels};
use std::hash::{Hash, Hasher};

/// AccessKit's fixed window root id (copied from gpui-pre 0.3.12).
pub(crate) const ROOT_NODE_ID: NodeId = NodeId(0);

/// Stable, per-control states reported to assistive technology.
#[derive(Clone, Debug, Default)]
pub(crate) struct States {
    pub label: String,
    pub description: String,
    pub toggled: Option<bool>,
    pub selected: bool,
    pub expanded: Option<bool>,
    pub value: Option<String>,
    pub numeric: Option<f64>,
    pub range: Option<(f64, f64)>,
    pub disabled: bool,
}

/// One control registered by a wrapped element this frame.
#[derive(Clone)]
pub(crate) struct Control {
    pub node_id: NodeId,
    /// Stable domain key, used to derive `node_id` and to find it again.
    pub key: String,
    pub region: Region,
    pub role: Role,
    pub bounds: Bounds<Pixels>,
    pub states: States,
    /// Set when the control is a keyboard target; drives Tab and tree focus.
    pub focus: Option<FocusHandle>,
    /// Engine command invoked by an AccessKit `Click` action.
    pub command: Option<String>,
    pub actions: Vec<Action>,
}

impl Control {
    /// Whether this control is reachable by Tab in `region`.
    pub(crate) fn is_tab_stop(&self) -> bool {
        self.focus.is_some() && !self.states.disabled
    }
}

/// Per-frame control records. Kept in an App global so wrapper elements can
/// write into it without every view carrying another field.
#[derive(Default)]
pub(crate) struct Registry {
    pub(crate) current: Vec<Control>,
    pub(crate) previous: Vec<Control>,
}

impl Registry {
    /// Called once at the top of the frame that builds the element tree.
    pub(crate) fn begin_frame(&mut self) {
        self.previous = std::mem::take(&mut self.current);
    }

    pub(crate) fn push(&mut self, control: Control) {
        self.current.push(control);
    }

    /// Last frame's controls in one region, in paint order (egui's
    /// `focusables(ctx, nav, region)` equivalent).
    pub(crate) fn focusables(&self, region: Region) -> Vec<&Control> {
        self.previous.iter().filter(|c| c.region == region && c.is_tab_stop()).collect()
    }

    pub(crate) fn control(&self, node_id: NodeId) -> Option<&Control> {
        self.previous.iter().find(|c| c.node_id == node_id)
    }

    pub(crate) fn clear(&mut self) {
        self.current.clear();
        self.previous.clear();
    }
}

/// App-global holder for the registry and keyed focus handles.
///
/// gpui-kit's `Button` uses `window.use_keyed_state` to keep a stable
/// `FocusHandle` for stateless components; GPUI 0.2 has no such helper, so the
/// handles live here, keyed by the same domain key as the AccessKit node.
#[derive(Default)]
pub(crate) struct A11yGlobal {
    pub(crate) registry: Registry,
    pub(crate) focus: std::collections::HashMap<String, FocusHandle>,
}

impl Global for A11yGlobal {}

/// Get (or create) the stable focus handle for a control key.
pub(crate) fn focus_for(cx: &mut App, key: &str) -> FocusHandle {
    ensure(cx);
    if let Some(handle) = cx.global::<A11yGlobal>().focus.get(key).cloned() {
        return handle;
    }
    let handle = cx.focus_handle();
    cx.global_mut::<A11yGlobal>().focus.insert(key.to_string(), handle.clone());
    handle
}

/// Initialise once in `run()` before opening the window.
pub(crate) fn init(cx: &mut App) {
    cx.set_global(A11yGlobal::default());
}

/// Tests construct views without going through `run()`, so install the
/// global on first use instead of requiring manual setup.
pub(crate) fn ensure(cx: &mut App) {
    if cx.try_global::<A11yGlobal>().is_none() {
        cx.set_global(A11yGlobal::default());
    }
}

pub(crate) fn with<F, R>(cx: &mut App, f: F) -> R
where
    F: FnOnce(&mut Registry) -> R,
{
    ensure(cx);
    f(&mut cx.global_mut::<A11yGlobal>().registry)
}

/// Derive a stable AccessKit id from a domain key. Duplicated keys are a
/// caller bug; the tree builder drops duplicates defensively.
pub(crate) fn node_id(key: &str) -> NodeId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    let id = hasher.finish();
    NodeId(if id == 0 { 1 } else { id })
}
