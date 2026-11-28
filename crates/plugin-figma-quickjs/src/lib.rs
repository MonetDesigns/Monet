//! The QuickJS backend for Figma-compatible plugins.
//!
//! One [`QuickJs`] is one plugin session: a QuickJS realm, the `figma` global built over the
//! [`Host`] the backend was constructed with, and the plugin's `main` evaluated once. Every
//! [`Backend::invoke`] runs in that same realm, so module state, `figma.on` handlers and `figma.ui`
//! callbacks survive between a run and the messages that follow it.
//!
//! Limits replace the WebAssembly sandbox's fuel: an interrupt handler stops the script at
//! `Limits::time_ms`, the QuickJS heap is capped at `Limits::memory_bytes`, and every host call is
//! counted. A failure rolls the callback back in the engine; the realm stays alive, like Figma
//! keeps a plugin alive after a thrown error.
//!
//! The API surface is deliberately small and honest. Node proxies carry scalar properties only;
//! anything that needs geometry, children or a command the engine doesn't have throws a catchable
//! JavaScript `Error`. The compatibility plan tracks the way to 1:1.

use std::cell::{Cell, RefCell, RefMut};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use figcraft_plugin_figma::host::{Host, Limits, UiOptions};
use figcraft_plugin_figma::ids::{figma_id, node_id};
use figcraft_plugin_figma::ops::{self, CreateKind, CreateSpec};
use figcraft_plugin_figma::props::{self, Kind};
use figcraft_plugin_figma::runtime::{Backend, Event, Invocation, Outcome, RuntimeError, Script, check_limits};
use rquickjs::{Context, Ctx, Exception, Function, Object, Runtime};
use serde_json::{Map, Value, json};

const PRELUDE: &str = include_str!("prelude.js");

type Shared = Rc<RefCell<Session>>;

fn props_bytes(props: &Map<String, Value>) -> usize {
    props.iter().map(|(key, value)| key.len() + value.to_string().len()).sum()
}

/// State shared between the bridge functions and the invocation that is running them.
struct Session {
    host: Box<dyn Host>,
    limits: Limits,
    calls: u64,
    logs: Vec<String>,
    log_bytes: usize,
    /// Property writes not yet flushed, per node: `node.x = 1; node.name = "a"` becomes one
    /// `node.set` when the callback ends or the script reads that node again.
    pending: BTreeMap<u64, Map<String, Value>>,
    /// Approximate heap held by `pending`; the QuickJS memory cap only covers the JS heap.
    pending_bytes: usize,
    output: Option<Value>,
}

impl Session {
    fn begin(&mut self) {
        self.calls = 0;
        self.logs.clear();
        self.log_bytes = 0;
        self.pending.clear();
        self.pending_bytes = 0;
        self.output = None;
    }

    fn tick(&mut self) -> Result<(), String> {
        self.calls = self.calls.saturating_add(1);
        if self.calls > u64::from(self.limits.max_calls) {
            return Err(format!("the plugin made more than {} host calls", self.limits.max_calls));
        }
        Ok(())
    }

    fn call_host(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        self.tick()?;
        self.host.call(command, params)
    }

    fn flush(&mut self, id: u64) -> Result<(), String> {
        let Some(props) = self.pending.remove(&id) else {
            return Ok(());
        };
        self.pending_bytes = self.pending_bytes.saturating_sub(props_bytes(&props));
        if props.is_empty() {
            return Ok(());
        }
        let call = ops::set(id, props).map_err(|error| error.to_string())?;
        self.call_host(call.command, &call.params).map(|_| ())
    }

    fn flush_all(&mut self) -> Result<(), String> {
        let ids: Vec<u64> = self.pending.keys().copied().collect();
        for id in ids {
            self.flush(id)?;
        }
        Ok(())
    }

    fn log_line(&mut self, line: String) {
        if self.log_bytes >= self.limits.max_log_bytes {
            return;
        }
        if self.log_bytes + line.len() > self.limits.max_log_bytes {
            self.logs.push("[log limit reached]".into());
            self.log_bytes = self.limits.max_log_bytes;
            return;
        }
        self.log_bytes += line.len();
        self.logs.push(line);
    }
}

/// A loaded Figma plugin.
pub struct QuickJs {
    context: Context,
    _runtime: Runtime,
    state: Shared,
    source: String,
    evaluated: bool,
    deadline: Rc<Cell<Instant>>,
    timed_out: Rc<Cell<bool>>,
    limits: Limits,
}

impl QuickJs {
    /// Create the realm and install the `figma` object; the plugin's `main` is evaluated on the
    /// first [`Backend::invoke`] so a script error is reported with the run that caused it.
    ///
    /// `host` answers every `figma` call for the life of this session; the engine constructs it
    /// over the worker's document session and commits at the callback boundary.
    pub fn load(script: &Script, limits: &Limits, host: Box<dyn Host>) -> Result<Self, RuntimeError> {
        check_limits(limits)?;
        let runtime = Runtime::new().map_err(|error| RuntimeError::Compile(error.to_string()))?;
        runtime.set_memory_limit(limits.memory_bytes);
        runtime.set_max_stack_size(4 * 1024 * 1024);

        let deadline = Rc::new(Cell::new(Instant::now() + Duration::from_millis(limits.time_ms)));
        let timed_out = Rc::new(Cell::new(false));
        runtime.set_interrupt_handler(Some(Box::new({
            let deadline = Rc::clone(&deadline);
            let timed_out = Rc::clone(&timed_out);
            move || {
                if timed_out.get() {
                    return true;
                }
                if Instant::now() >= deadline.get() {
                    timed_out.set(true);
                    return true;
                }
                false
            }
        })));

        let context = Context::full(&runtime).map_err(|error| RuntimeError::Compile(error.to_string()))?;
        let state = Rc::new(RefCell::new(Session {
            host,
            limits: *limits,
            calls: 0,
            logs: Vec::new(),
            log_bytes: 0,
            pending: BTreeMap::new(),
            pending_bytes: 0,
            output: None,
        }));
        context.with(|ctx| install(&ctx, &state, script.ui.as_deref()))?;

        Ok(Self { context, _runtime: runtime, state, source: script.main.clone(), evaluated: false, deadline, timed_out, limits: *limits })
    }

    /// The realm's logs so far, for a caller that wants them before the next invocation.
    pub fn logs(&self) -> Vec<String> {
        self.state.try_borrow().map(|state| state.logs.clone()).unwrap_or_default()
    }
}

impl Backend for QuickJs {
    fn invoke(&mut self, input: &Invocation<'_>) -> Result<Outcome, RuntimeError> {
        {
            let mut state = self.state.try_borrow_mut().map_err(|_| RuntimeError::Failed("the plugin's session is already in use".into()))?;
            state.begin();
        }
        self.deadline.set(Instant::now() + Duration::from_millis(self.limits.time_ms));
        self.timed_out.set(false);

        let first = !self.evaluated;
        let event = input.event.map(event_json);
        let context = &self.context;
        let source = self.source.clone();
        let limits = self.limits;
        let result = context.with(|ctx| -> Result<(), RuntimeError> {
            if first {
                ctx.eval::<(), _>(source.clone()).map_err(|error| load_error(&ctx, error, &limits))?;
            }
            if let Some(event) = event.as_deref() {
                let dispatch: Function<'_> = ctx.globals().get("__figma_dispatch").map_err(|error| runtime_error(&ctx, error, &limits))?;
                dispatch.call::<_, ()>((event,)).map_err(|error| runtime_error(&ctx, error, &limits))?;
            }
            Ok(())
        });
        if result.is_ok() && first {
            self.evaluated = true;
        }
        if let Err(error) = result {
            return if self.timed_out.get() { Err(RuntimeError::Timeout(self.limits.time_ms)) } else { Err(error) };
        }

        let mut state = self.state.try_borrow_mut().map_err(|_| RuntimeError::Failed("the plugin's session is already in use".into()))?;
        state.flush_all().map_err(RuntimeError::Failed)?;
        Ok(Outcome { output: state.output.take(), logs: state.logs.clone(), calls: state.calls })
    }
}

/// Put the bridge and the prelude into a fresh realm. The `ui` page becomes `__html__`.
fn install(ctx: &Ctx<'_>, state: &Shared, ui: Option<&str>) -> Result<(), RuntimeError> {
    let globals = ctx.globals();
    let limits = state.try_borrow().map(|state| state.limits).unwrap_or_default();
    if let Some(ui) = ui {
        globals.set("__html__", ui).map_err(|error| runtime_error(ctx, error, &limits))?;
    }

    let bridge = Object::new(ctx.clone()).map_err(|error| runtime_error(ctx, error, &limits))?;

    macro_rules! bind {
        ($name:literal, $body:expr) => {{
            let function = Function::new(ctx.clone(), $body).map_err(|error| runtime_error(ctx, error, &limits))?;
            bridge.set($name, function).map_err(|error| runtime_error(ctx, error, &limits))?;
        }};
    }

    bind!("get", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, id: String, prop: String| bridge_get(&ctx, Rc::clone(&state), &id, &prop)
    });
    bind!("set", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, id: String, prop: String, value: String| bridge_set(&ctx, Rc::clone(&state), &id, &prop, &value)
    });
    bind!("call", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, command: String, params: String| bridge_call(&ctx, Rc::clone(&state), &command, &params)
    });
    bind!("create", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, kind: String| bridge_create(&ctx, Rc::clone(&state), &kind)
    });
    bind!("append", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, child: String, parent: String| bridge_append(&ctx, Rc::clone(&state), &child, &parent)
    });
    bind!("remove", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, id: String| bridge_remove(&ctx, Rc::clone(&state), &id)
    });
    bind!("selection", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>| bridge_selection(&ctx, Rc::clone(&state))
    });
    bind!("setSelection", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, ids: String| bridge_set_selection(&ctx, Rc::clone(&state), &ids)
    });
    bind!("page", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>| bridge_page(&ctx, Rc::clone(&state))
    });
    bind!("exists", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, id: String| bridge_exists(&ctx, Rc::clone(&state), &id)
    });
    bind!("notify", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, message: String, options: String| bridge_notify(&ctx, Rc::clone(&state), &message, &options)
    });
    bind!("close", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, message: String| bridge_close(&ctx, Rc::clone(&state), &message)
    });
    bind!("commitUndo", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>| bridge_commit_undo(&ctx, Rc::clone(&state))
    });
    bind!("showUi", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, html: String, options: String| bridge_show_ui(&ctx, Rc::clone(&state), &html, &options)
    });
    bind!("postMessage", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, message: String| bridge_post_message(&ctx, Rc::clone(&state), &message)
    });
    bind!("closeUi", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>| bridge_close_ui(&ctx, Rc::clone(&state))
    });
    bind!("log", {
        let state = Rc::clone(state);
        move |ctx: Ctx<'_>, line: String| bridge_log(&ctx, Rc::clone(&state), &line)
    });

    globals.set("__figcraft", bridge).map_err(|error| runtime_error(ctx, error, &limits))?;
    ctx.eval::<(), _>(PRELUDE.to_string()).map_err(|error| runtime_error(ctx, error, &limits))?;
    Ok(())
}

fn session<'a>(ctx: &Ctx<'_>, state: &'a Shared) -> rquickjs::Result<RefMut<'a, Session>> {
    state.try_borrow_mut().map_err(|_| throw(ctx, "the plugin's session is already in use"))
}

fn throw(ctx: &Ctx<'_>, message: impl Into<String>) -> rquickjs::Error {
    let message = message.into();
    Exception::throw_message(ctx, &message)
}

fn check_size(ctx: &Ctx<'_>, limits: &Limits, text: &str) -> rquickjs::Result<()> {
    if text.len() > limits.max_message {
        return Err(throw(ctx, format!("the value is larger than the {} byte limit", limits.max_message)));
    }
    Ok(())
}

fn bridge_get(ctx: &Ctx<'_>, state: Shared, id_text: &str, prop: &str) -> rquickjs::Result<String> {
    let id = node_id(id_text).map_err(|error| throw(ctx, error.to_string()))?;
    // `engine_key` rejects reads of read-only fields by design; `read_key` knows the few
    // read-only fields that still live under an engine key (`type`), while writes stay guarded.
    let key = props::read_key(prop).ok_or_else(|| throw(ctx, format!("`{prop}` isn't supported yet")))?;
    let def = props::by_figma(prop);
    let mut session = session(ctx, &state)?;
    session.flush(id).map_err(|error| throw(ctx, error))?;
    let call = ops::get(id, Some(&[key])).map_err(|error| throw(ctx, error.to_string()))?;
    let value = session.call_host(call.command, &call.params).map_err(|error| throw(ctx, error))?;
    let mut value = value.get(key).cloned().unwrap_or(Value::Null);
    if def.is_some_and(|def| def.kind == Kind::Enum)
        && let Some(figma) = props::enum_to_figma(prop, &value)
    {
        value = figma;
    }
    let text = serde_json::to_string(&value).unwrap_or_else(|_| "null".into());
    check_size(ctx, &session.limits, &text)?;
    Ok(text)
}

fn bridge_set(ctx: &Ctx<'_>, state: Shared, id_text: &str, prop: &str, json: &str) -> rquickjs::Result<()> {
    let id = node_id(id_text).map_err(|error| throw(ctx, error.to_string()))?;
    let def = props::by_figma(prop).ok_or_else(|| throw(ctx, format!("`{prop}` isn't supported yet")))?;
    if def.kind == Kind::ReadOnly {
        return Err(throw(ctx, format!("`{prop}` is read-only")));
    }
    if def.kind == Kind::Derived {
        return Err(throw(ctx, format!("`{prop}` is composed from more than one field; not writable yet")));
    }
    let key = def.engine.ok_or_else(|| throw(ctx, format!("`{prop}` has no engine key")))?;
    let mut value: Value = serde_json::from_str(json).map_err(|error| throw(ctx, format!("`{prop}` needs a JSON value: {error}")))?;
    if def.kind == Kind::Enum
        && let Some(text) = value.as_str()
    {
        value = props::convert_enum(prop, text).ok_or_else(|| throw(ctx, format!("`{text}` is not a valid `{prop}`")))?;
    }
    let mut session = session(ctx, &state)?;
    check_size(ctx, &session.limits, json)?;
    session.tick().map_err(|error| throw(ctx, error))?;
    let (before, after) = {
        let entry = session.pending.entry(id).or_default();
        // Account only the replaced property, not the whole node: a large unchanged `name`
        // must not be reserialized on every `x = ...`.
        let before = entry.get(key).map(|value| key.len() + value.to_string().len()).unwrap_or(0);
        let after = key.len() + value.to_string().len();
        entry.insert(key.to_string(), value);
        (before, after)
    };
    session.pending_bytes = session.pending_bytes.saturating_sub(before).saturating_add(after);
    // `pending` values live on the Rust heap, outside `set_memory_limit`. Flush them into
    // engine commands once their aggregate exceeds the per-message cap.
    if session.pending_bytes > session.limits.max_message {
        session.flush_all().map_err(|error| throw(ctx, error))?;
    }
    Ok(())
}

fn bridge_call(ctx: &Ctx<'_>, state: Shared, command: &str, params: &str) -> rquickjs::Result<String> {
    let params: Value = serde_json::from_str(params).map_err(|error| throw(ctx, format!("params must be JSON: {error}")))?;
    let mut session = session(ctx, &state)?;
    let result = session.call_host(command, &params).map_err(|error| throw(ctx, error))?;
    let text = serde_json::to_string(&result).unwrap_or_else(|_| "null".into());
    check_size(ctx, &session.limits, &text)?;
    Ok(text)
}

fn bridge_create(ctx: &Ctx<'_>, state: Shared, kind: &str) -> rquickjs::Result<String> {
    let kind = match kind {
        "frame" => CreateKind::Frame,
        "rectangle" => CreateKind::Rectangle,
        "ellipse" => CreateKind::Ellipse,
        "polygon" => CreateKind::Polygon,
        "star" => CreateKind::Star,
        "line" => CreateKind::Line,
        "vector" => CreateKind::Vector,
        "text" => CreateKind::Text,
        "section" => CreateKind::Section,
        "component" => CreateKind::Component,
        "slice" => CreateKind::Slice,
        other => return Err(throw(ctx, format!("`{other}` isn't a node kind Figma creates"))),
    };
    let call = ops::create(kind, &CreateSpec::default()).map_err(|error| throw(ctx, error.to_string()))?;
    let mut session = session(ctx, &state)?;
    let result = session.call_host(call.command, &call.params).map_err(|error| throw(ctx, error))?;
    let id = result.get("id").and_then(Value::as_u64).ok_or_else(|| throw(ctx, "node.create returned no id"))?;
    figma_id(id).map_err(|error| throw(ctx, error.to_string()))
}

fn bridge_append(ctx: &Ctx<'_>, state: Shared, child: &str, parent: &str) -> rquickjs::Result<()> {
    let child = node_id(child).map_err(|error| throw(ctx, error.to_string()))?;
    let parent = node_id(parent).map_err(|error| throw(ctx, error.to_string()))?;
    let call = ops::append_child(parent, child, None).map_err(|error| throw(ctx, error.to_string()))?;
    let mut session = session(ctx, &state)?;
    session.call_host(call.command, &call.params).map(|_| ()).map_err(|error| throw(ctx, error))
}

fn bridge_remove(ctx: &Ctx<'_>, state: Shared, id_text: &str) -> rquickjs::Result<()> {
    let id = node_id(id_text).map_err(|error| throw(ctx, error.to_string()))?;
    let call = ops::remove(id).map_err(|error| throw(ctx, error.to_string()))?;
    let mut session = session(ctx, &state)?;
    if let Some(props) = session.pending.remove(&id) {
        session.pending_bytes = session.pending_bytes.saturating_sub(props_bytes(&props));
    }
    session.call_host(call.command, &call.params).map(|_| ()).map_err(|error| throw(ctx, error))
}

fn bridge_selection(ctx: &Ctx<'_>, state: Shared) -> rquickjs::Result<String> {
    let mut session = session(ctx, &state)?;
    session.tick().map_err(|error| throw(ctx, error))?;
    let selection = session.host.selection();
    let ids: Vec<String> = selection.into_iter().filter_map(|id| figma_id(id).ok()).collect();
    let text = serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into());
    check_size(ctx, &session.limits, &text)?;
    Ok(text)
}

fn bridge_set_selection(ctx: &Ctx<'_>, state: Shared, json: &str) -> rquickjs::Result<()> {
    let ids: Vec<String> = serde_json::from_str(json).map_err(|error| throw(ctx, format!("selection must be an array of ids: {error}")))?;
    let ids: Vec<u64> = ids.iter().map(|id| node_id(id)).collect::<Result<_, _>>().map_err(|error| throw(ctx, error.to_string()))?;
    let mut session = session(ctx, &state)?;
    session.tick().map_err(|error| throw(ctx, error))?;
    session.host.set_selection(&ids).map_err(|error| throw(ctx, error))
}

fn bridge_page(ctx: &Ctx<'_>, state: Shared) -> rquickjs::Result<String> {
    let mut session = session(ctx, &state)?;
    let pages = session.call_host("page.list", &json!({})).map_err(|error| throw(ctx, error))?;
    let page = pages
        .as_array()
        .and_then(|pages| pages.iter().find(|page| page.get("current").and_then(Value::as_bool) == Some(true)))
        .and_then(|page| page.get("id"))
        .and_then(Value::as_u64)
        .ok_or_else(|| throw(ctx, "no current page"))?;
    figma_id(page).map_err(|error| throw(ctx, error.to_string()))
}

fn bridge_exists(ctx: &Ctx<'_>, state: Shared, id_text: &str) -> rquickjs::Result<bool> {
    let id = node_id(id_text).map_err(|error| throw(ctx, error.to_string()))?;
    let mut session = session(ctx, &state)?;
    session.tick().map_err(|error| throw(ctx, error))?;
    match session.host.get(id, Some(&["type"])) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

fn bridge_notify(ctx: &Ctx<'_>, state: Shared, message: &str, _options: &str) -> rquickjs::Result<()> {
    let mut session = session(ctx, &state)?;
    check_size(ctx, &session.limits, message)?;
    session.host.notify(message);
    Ok(())
}

fn bridge_close(ctx: &Ctx<'_>, state: Shared, message: &str) -> rquickjs::Result<()> {
    let mut session = session(ctx, &state)?;
    check_size(ctx, &session.limits, message)?;
    session.flush_all().map_err(|error| throw(ctx, error))?;
    if !message.is_empty() {
        session.output = Some(Value::String(message.to_string()));
    }
    session.host.close_ui();
    Ok(())
}

fn bridge_commit_undo(ctx: &Ctx<'_>, state: Shared) -> rquickjs::Result<()> {
    let mut session = session(ctx, &state)?;
    session.flush_all().map_err(|error| throw(ctx, error))?;
    session.host.commit_undo();
    Ok(())
}

fn bridge_show_ui(ctx: &Ctx<'_>, state: Shared, html: &str, options: &str) -> rquickjs::Result<()> {
    let options = if options.is_empty() {
        UiOptions::default()
    } else {
        let value: Value = serde_json::from_str(options).map_err(|error| throw(ctx, format!("UI options must be JSON: {error}")))?;
        UiOptions::from_json(&value)
    };
    let mut session = session(ctx, &state)?;
    check_size(ctx, &session.limits, html)?;
    session.host.show_ui(html, &options).map_err(|error| throw(ctx, error))
}

fn bridge_post_message(ctx: &Ctx<'_>, state: Shared, json: &str) -> rquickjs::Result<()> {
    let message: Value = serde_json::from_str(json).map_err(|error| throw(ctx, format!("the message must be JSON: {error}")))?;
    let mut session = session(ctx, &state)?;
    check_size(ctx, &session.limits, json)?;
    session.host.post_message(&message).map_err(|error| throw(ctx, error))
}

fn bridge_close_ui(ctx: &Ctx<'_>, state: Shared) -> rquickjs::Result<()> {
    let mut session = session(ctx, &state)?;
    session.flush_all().map_err(|error| throw(ctx, error))?;
    session.host.close_ui();
    Ok(())
}

fn bridge_log(ctx: &Ctx<'_>, state: Shared, line: &str) -> rquickjs::Result<()> {
    let mut session = session(ctx, &state)?;
    check_size(ctx, &session.limits, line)?;
    session.log_line(line.to_string());
    Ok(())
}

fn event_json(event: &Event) -> String {
    match event {
        Event::Message(message) => json!({"type": "message", "message": message}).to_string(),
        Event::Run(parameters) => json!({"type": "run", "parameters": parameters}).to_string(),
        Event::SelectionChange(selection) => {
            let ids: Vec<String> = selection.iter().filter_map(|id| figma_id(*id).ok()).collect();
            json!({"type": "selectionchange", "selection": ids}).to_string()
        }
        Event::DocumentChange => json!({"type": "documentchange"}).to_string(),
    }
}

fn load_error(ctx: &Ctx<'_>, error: rquickjs::Error, limits: &Limits) -> RuntimeError {
    match runtime_error(ctx, error, limits) {
        RuntimeError::Failed(message) => RuntimeError::Compile(message),
        other => other,
    }
}

fn runtime_error(ctx: &Ctx<'_>, error: rquickjs::Error, limits: &Limits) -> RuntimeError {
    match error {
        rquickjs::Error::Allocation => RuntimeError::OutOfMemory(limits.memory_bytes),
        rquickjs::Error::Exception => {
            let caught = ctx.catch();
            let message = caught
                .clone()
                .into_exception()
                .and_then(|exception| exception.message())
                .or_else(|| caught.as_string().and_then(|text| text.to_string().ok()))
                .unwrap_or_else(|| "the plugin threw a value".into());
            RuntimeError::Failed(message)
        }
        other => RuntimeError::Failed(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Backend, Invocation, Limits, QuickJs, Script};
    use figcraft_plugin_figma::host::{Host, UiOptions};
    use figcraft_plugin_figma::runtime::{Outcome, RuntimeError};
    use serde_json::{Map, Value, json};
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    type MapState = Rc<RefCell<SessionState>>;

    struct MapHost {
        state: MapState,
    }

    impl MapHost {
        fn new() -> Self {
            let mut nodes = BTreeMap::new();
            nodes.insert(
                5,
                json!({"type": "FRAME", "name": "Card", "x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0, "visible": true, "locked": false})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            );
            Self {
                state: Rc::new(RefCell::new(SessionState {
                    nodes,
                    next: 100,
                    selection: vec![5],
                    calls: Vec::new(),
                    notified: Vec::new(),
                    committed: 0,
                    closed: false,
                    ui: None,
                    posted: Vec::new(),
                })),
            }
        }

        fn state(&self) -> std::cell::RefMut<'_, SessionState> {
            self.state.borrow_mut()
        }
    }

    struct SessionState {
        nodes: BTreeMap<u64, Map<String, Value>>,
        next: u64,
        selection: Vec<u64>,
        calls: Vec<String>,
        notified: Vec<String>,
        committed: usize,
        closed: bool,
        ui: Option<String>,
        posted: Vec<Value>,
    }

    impl Host for MapHost {
        fn call(&mut self, command: &str, params: &Value) -> Result<Value, String> {
            let mut state = self.state();
            state.calls.push(format!("{command} {params}"));
            match command {
                "node.get" => {
                    let id = params["id"].as_u64().ok_or("id")?;
                    let node = state.nodes.get(&id).ok_or_else(|| format!("no node {id}"))?;
                    let mut out = Map::new();
                    out.insert("id".into(), json!(id));
                    match params.get("fields").and_then(Value::as_array) {
                        Some(fields) => {
                            for field in fields {
                                if let Some(key) = field.as_str() {
                                    out.insert(key.into(), node.get(key).cloned().unwrap_or(Value::Null));
                                }
                            }
                        }
                        None => {
                            out = node.clone();
                            out.insert("id".into(), json!(id));
                        }
                    }
                    Ok(Value::Object(out))
                }
                "node.set" => {
                    let id = params["id"].as_u64().ok_or("id")?;
                    let node = state.nodes.get_mut(&id).ok_or_else(|| format!("no node {id}"))?;
                    if let Some(props) = params.get("props").and_then(Value::as_object) {
                        for (key, value) in props {
                            node.insert(key.clone(), value.clone());
                        }
                    }
                    Ok(json!({"ok": true}))
                }
                "node.create" => {
                    let id = state.next;
                    state.next += 1;
                    let mut node = Map::new();
                    node.insert("type".into(), params.get("type").cloned().unwrap_or(json!("RECTANGLE")));
                    node.insert("name".into(), params.get("name").cloned().unwrap_or(json!("")));
                    for key in ["x", "y", "width", "height"] {
                        node.insert(key.into(), params.get(key).cloned().unwrap_or(json!(0.0)));
                    }
                    state.nodes.insert(id, node);
                    Ok(json!({"id": id}))
                }
                "edit.delete" => {
                    if let Some(id) = params.get("id").and_then(Value::as_u64) {
                        state.nodes.remove(&id);
                    }
                    Ok(json!({"ok": true}))
                }
                "select.set" => {
                    state.selection =
                        params.get("ids").and_then(Value::as_array).map(|ids| ids.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
                    Ok(json!({"ok": true}))
                }
                "node.reparent" => Ok(json!({"ok": true})),
                "page.list" => Ok(json!([{"id": 1, "name": "Page 1", "current": true}])),
                other => Err(format!("unknown command `{other}`")),
            }
        }

        fn selection(&mut self) -> Vec<u64> {
            self.state().selection.clone()
        }

        fn notify(&mut self, message: &str) {
            self.state().notified.push(message.to_string());
        }

        fn show_ui(&mut self, html: &str, _options: &UiOptions) -> Result<(), String> {
            self.state().ui = Some(html.to_string());
            Ok(())
        }

        fn post_message(&mut self, message: &Value) -> Result<(), String> {
            self.state().posted.push(message.clone());
            Ok(())
        }

        fn close_ui(&mut self) {
            self.state().closed = true;
        }

        fn commit_undo(&mut self) {
            self.state().committed += 1;
        }
    }

    fn run(source: &str, host: MapHost) -> (QuickJs, MapState, Result<Outcome, RuntimeError>) {
        let state = Rc::clone(&host.state);
        let limits = Limits::default();
        let mut plugin = QuickJs::load(&Script::new(source, None), &limits, Box::new(host)).expect("load");
        let params = json!({});
        let selection = [5_u64];
        let result = plugin.invoke(&Invocation { command: "run", params: &params, selection: &selection, event: None });
        (plugin, state, result)
    }

    #[test]
    fn reads_and_writes_go_through_one_batched_command() {
        let source = r#"
            const node = figma.currentPage.selection[0];
            figma.notify('hello ' + node.name);
            node.x = 40;
            node.y = 20;
            console.log('name=' + node.name, 'x=' + node.x);
            figma.closePlugin('done');
        "#;
        let (_plugin, state, result) = run(source, MapHost::new());
        let outcome = result.unwrap();
        assert_eq!(outcome.output, Some(json!("done")));
        assert_eq!(outcome.logs[0], "name=Card x=40");
        let state = state.borrow();
        assert_eq!(state.notified, vec!["hello Card".to_string()]);
        assert_eq!(state.calls.iter().filter(|call| call.starts_with("node.set")).count(), 1);
        assert_eq!(state.nodes[&5]["x"], json!(40));
    }

    #[test]
    fn read_only_type_and_enum_reads_come_back_in_figma_spelling() {
        let source = r#"
            const node = figma.currentPage.selection[0];
            const type = node.type;
            node.layoutMode = 'HORIZONTAL';
            console.log(type + ' ' + node.layoutMode);
            figma.closePlugin('ok');
        "#;
        let (_plugin, _state, result) = run(source, MapHost::new());
        assert_eq!(result.unwrap().logs[0], "FRAME HORIZONTAL");
    }

    #[test]
    fn creates_nodes_and_sets_the_selection() {
        let source = r#"
            const r = figma.createRectangle();
            r.x = 10;
            r.name = 'R';
            figma.currentPage.selection = [r];
            figma.closePlugin('ok');
        "#;
        let (_plugin, state, result) = run(source, MapHost::new());
        assert_eq!(result.unwrap().output, Some(json!("ok")));
        let state = state.borrow();
        assert_eq!(state.selection, vec![100]);
        assert_eq!(state.nodes[&100]["x"], json!(10));
        assert_eq!(state.nodes[&100]["name"], json!("R"));
    }

    #[test]
    fn plugin_scope_has_no_host_handles() {
        let source = r#"
            if (typeof process !== 'undefined') throw new Error('process leaked');
            if (typeof require !== 'undefined') throw new Error('require leaked');
            if (typeof __figcraft !== 'undefined') throw new Error('bridge leaked');
            const global = Function('return this')();
            if (typeof global.__figcraft !== 'undefined') throw new Error('bridge leaked through global');
            figma.notify('safe');
        "#;
        let (_plugin, _state, result) = run(source, MapHost::new());
        assert!(result.is_ok());
    }

    #[test]
    fn unsupported_properties_throw_a_catchable_error() {
        let source = r#"
            const node = figma.getNodeById('0:5');
            try {
                node.topLeftRadius = 4;
                figma.closePlugin('not thrown');
            } catch (error) {
                figma.closePlugin('caught: ' + error.message);
            }
        "#;
        let (_plugin, _state, result) = run(source, MapHost::new());
        assert!(result.unwrap().output.as_ref().and_then(Value::as_str).unwrap_or_default().starts_with("caught:"));
    }

    #[test]
    fn ui_messages_and_undo_go_to_the_host() {
        let source = r#"
            figma.showUI('<html>hi</html>', { width: 400, height: 300 });
            figma.ui.postMessage({ hello: 'frame' });
            figma.commitUndo();
            figma.closePlugin('');
        "#;
        let (_plugin, state, result) = run(source, MapHost::new());
        assert_eq!(result.unwrap().output, None);
        let state = state.borrow();
        assert_eq!(state.ui.as_deref(), Some("<html>hi</html>"));
        assert_eq!(state.posted[0], json!({"hello": "frame"}));
        assert_eq!(state.committed, 1);
        assert!(state.closed);
    }

    #[test]
    fn one_hundred_thousand_property_reads_stay_under_a_second() {
        let source = r#"
            const node = figma.getNodeById('0:5');
            let sum = 0;
            for (let i = 0; i < 100000; i += 1) {
                sum += node.width;
            }
            figma.closePlugin('sum=' + sum);
        "#;
        let started = std::time::Instant::now();
        let (_plugin, _state, result) = run(source, MapHost::new());
        let elapsed = started.elapsed();
        eprintln!("100k reads: {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_secs(1), "100k reads took {elapsed:?}");
        assert_eq!(result.unwrap().output, Some(json!("sum=10000000")));
    }

    #[test]
    fn an_endless_script_times_out() {
        let limits = Limits { time_ms: 50, ..Limits::default() };
        let mut plugin = QuickJs::load(&Script::new("while (true) {}", None), &limits, Box::new(MapHost::new())).expect("load");
        let params = json!({});
        let result = plugin.invoke(&Invocation { command: "run", params: &params, selection: &[], event: None });
        assert!(matches!(result, Err(RuntimeError::Timeout(50))));
    }
}
