//! The backend seam: what a JavaScript runtime has to do, and what the engine has to hand it.
//!
//! A Figma plugin is a script plus, optionally, an HTML frame. The script is **loaded once** per
//! session (so statics, module state and timers survive between callbacks), then invoked once per
//! run or per UI message. Every invocation is the plugin's whole turn: it may read and write the
//! document through the `figma` object, call the host, and return.
//!
//! ```text
//! engine: Session::grouped(label, |s| backend.invoke(&input))
//!                                 ^ one undo step, rolled back on Err
//! backend: owns the Host for the session; builds the figma object over it, runs the script,
//!          collects output and logs
//! ```
//!
//! The host is supplied when the backend is constructed, not per call: a JavaScript realm keeps
//! functions that outlive one invocation, so the object they call into must live as long as the
//! realm. The QuickJS backend takes a `Box<dyn Host>` at `load` and holds it; the engine constructs
//! it over the worker's session and commits at the callback boundary.
//!
//! `Backend` is object-safe on purpose: the engine holds a `Box<dyn Backend>` next to its
//! WebAssembly `Instance`s and does not care which engine is behind it.

use serde_json::Value;

use crate::host::Limits;

/// A plugin's sources, read from its folder (or its bundle).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    /// The JavaScript (`main`), as text.
    pub main: String,
    /// The HTML frame (`ui`), as text: `__html__` in the script is this page.
    pub ui: Option<String>,
}

impl Script {
    pub fn new(main: impl Into<String>, ui: Option<String>) -> Self {
        Self { main: main.into(), ui }
    }
}

/// Why a plugin calls back, other than the first run.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// `figma.ui.onmessage`: the frame posted a JSON message.
    Message(Value),
    /// A quick action with `manifest.parameters`: `figma.on("run")` gets these.
    Run(std::collections::BTreeMap<String, Value>),
    /// The document selection changed while the plugin's UI was open.
    SelectionChange(Vec<u64>),
    /// The document changed while the plugin's UI was open (only with `loadAllPagesAsync`).
    DocumentChange,
}

/// One invocation's input: the run's command and parameters (a UI message carries the command that
/// opened the panel), the selection at that moment, and why the plugin is being called.
#[derive(Debug, Clone, PartialEq)]
pub struct Invocation<'a> {
    pub command: &'a str,
    pub params: &'a Value,
    pub selection: &'a [u64],
    pub event: Option<&'a Event>,
}

/// What one invocation produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    /// `figma.closePlugin(message)`, or the value a parameterized run returned. `None` when the
    /// plugin said nothing.
    pub output: Option<Value>,
    pub logs: Vec<String>,
    /// Host calls made during this invocation (metering, like the WebAssembly sandbox's `calls`).
    pub calls: u64,
}

/// Why an invocation failed. Anything here rolls the callback back in the engine; the message
/// reaches the user, and `Failed` (the plugin threw) keeps the session alive for the next
/// callback, like the WebAssembly runtime's `PluginError::Failed`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    #[error("the plugin's script doesn't compile: {0}")]
    Compile(String),
    #[error("{0}")]
    Failed(String),
    #[error("the plugin ran longer than {0} ms")]
    Timeout(u64),
    #[error("the plugin used more memory than its {} MB limit", .0 / (1024 * 1024))]
    OutOfMemory(usize),
    #[error("the plugin went over a limit: {0}")]
    Limit(String),
    #[error("this plugin needs something this runtime doesn't provide: {0}")]
    Unsupported(String),
}

/// A loaded script that can be called again and again.
///
/// The first callback is the run (`event: None`); later ones carry an [`Event`]. A backend keeps
/// the JavaScript realm, its timers and its module state between calls, and must not keep a second
/// copy of the document: reads and writes go through [`crate::host::Host`].
pub trait Backend {
    /// Run one callback. The host the backend was constructed with answers the `figma` object.
    fn invoke(&mut self, input: &Invocation<'_>) -> Result<Outcome, RuntimeError>;

    /// Drop the realm (the panel closed, the plugin was removed). Hosts stay untouched.
    fn dispose(&mut self) {}
}

/// The legal backend limits, next to the script: a backend that ignores them is a bug, and the
/// engine still refuses a run whose host calls or logs go over.
pub fn check_limits(limits: &Limits) -> Result<(), RuntimeError> {
    if limits.time_ms == 0 || limits.max_calls == 0 {
        return Err(RuntimeError::Unsupported("the run's limits are zero".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Backend, Invocation, Outcome, RuntimeError};
    use serde_json::json;

    struct Echo;

    impl Backend for Echo {
        fn invoke(&mut self, input: &Invocation<'_>) -> Result<Outcome, RuntimeError> {
            Ok(Outcome { output: Some(json!({"command": input.command, "params": input.params})), logs: vec![], calls: 0 })
        }
    }

    #[test]
    fn a_backend_is_object_safe_and_gets_the_invocation() {
        let mut backend: Box<dyn Backend> = Box::new(Echo);
        let params = json!({"palette": "Ocean"});
        let input = Invocation { command: "grid", params: &params, selection: &[3, 4], event: None };
        let outcome = backend.invoke(&input).unwrap();
        assert_eq!(outcome.output, Some(json!({"command": "grid", "params": {"palette": "Ocean"}})));
        backend.dispose();
    }
}
