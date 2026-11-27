//! What the engine answers while a plugin runs.
//!
//! The bridge calls only through this trait, so the crate stays independent of
//! `figcraft-engine`: the engine implements it over its `Session` (commands, the selection, the
//! plugin's settings, a file the plugin offers, the UI frame and `figma.commitUndo`), and tests
//! implement it over a map.
//!
//! Everything here happens **inside** the transaction the engine already opens for a plugin
//! callback. A refusal comes back as `Err(message)` and becomes a JavaScript `Error` in the
//! backend; the engine's rollback still owns the document.

use serde_json::Value;

/// What one script run may use.
///
/// Figma has no instruction budget, so a JavaScript runtime can't be metered in fuel like the
/// WebAssembly sandbox. These limits are the replacement: wall-clock time (checked by the engine's
/// interrupt handler), a memory cap on the JavaScript heap, a call count to the host and the same
/// message and log budgets the WebAssembly plugins get.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Wall-clock milliseconds one `figma_run` (or one call from an open panel) may take.
    pub time_ms: u64,
    /// The JavaScript heap's cap.
    pub memory_bytes: usize,
    /// Host calls per run.
    pub max_calls: u32,
    /// Largest string across the boundary.
    pub max_message: usize,
    /// Total log bytes per run.
    pub max_log_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { time_ms: 30_000, memory_bytes: 256 * 1024 * 1024, max_calls: 1_000_000, max_message: 32 * 1024 * 1024, max_log_bytes: 1024 * 1024 }
    }
}

impl Limits {
    /// The JSON an install confirmation can show, so a user sees the budget before running code.
    pub fn describe(&self) -> Value {
        serde_json::json!({
            "time_ms": self.time_ms,
            "memory_mb": self.memory_bytes / (1024 * 1024),
            "max_calls": self.max_calls,
        })
    }
}

/// How `figma.showUI` opens the HTML frame.
#[derive(Debug, Clone, PartialEq)]
pub struct UiOptions {
    pub width: f32,
    pub height: f32,
    pub title: Option<String>,
    /// `themeColors: true`: Figma injects `--figma-color-*` CSS variables into the frame.
    pub theme_colors: bool,
}

impl Default for UiOptions {
    fn default() -> Self {
        Self { width: 300.0, height: 200.0, title: None, theme_colors: false }
    }
}

impl UiOptions {
    /// The options a `showUI` call's second argument carries.
    pub fn from_json(value: &Value) -> Self {
        let mut options = Self::default();
        if let Some(width) = value.get("width").and_then(Value::as_f64).filter(|w| w.is_finite() && *w > 0.0) {
            options.width = width as f32;
        }
        if let Some(height) = value.get("height").and_then(Value::as_f64).filter(|h| h.is_finite() && *h > 0.0) {
            options.height = height as f32;
        }
        if let Some(title) = value.get("title").and_then(Value::as_str) {
            options.title = Some(title.to_string());
        }
        options.theme_colors = value.get("themeColors").and_then(Value::as_bool).unwrap_or(false);
        options
    }
}

/// The engine's side of a running plugin.
pub trait Host {
    /// Run a command with JSON params, as the menus and MCP server do. `Err` carries the reason
    /// the command refused.
    fn call(&mut self, command: &str, params: &Value) -> Result<Value, String>;

    /// The layer ids selected now, in the document's order.
    fn selection(&mut self) -> Vec<u64>;

    /// `figma.getNodeById(id)`'s data (`node.get`), narrowed to `fields` when the proxy only needs
    /// some of them.
    fn get(&mut self, id: u64, fields: Option<&[&str]>) -> Result<Value, String> {
        let call = crate::ops::get(id, fields).map_err(|e| e.to_string())?;
        self.call(call.command, &call.params)
    }

    /// `figma.currentPage.selection = nodes`.
    fn set_selection(&mut self, ids: &[u64]) -> Result<(), String> {
        let call = crate::ops::select(ids).map_err(|e| e.to_string())?;
        self.call(call.command, &call.params).map(|_| ())
    }

    /// `figma.notify(message)`: a toast in the app.
    fn notify(&mut self, _message: &str) {}

    /// `console.log`: a line in the run's log.
    fn log(&mut self, _line: &str) {}

    /// `figma.clientStorage.getAsync(key)`.
    fn setting(&mut self, _key: &str) -> Option<Value> {
        None
    }

    /// `figma.clientStorage.setAsync(key, value)`, or `deleteAsync` with `None`.
    fn set_setting(&mut self, _key: &str, _value: Option<&Value>) -> Result<(), String> {
        Err("settings aren't available here".into())
    }

    /// `figma.exportAsync` / a plugin's file offer: the engine asks the user where it goes and the
    /// plugin never learns a path.
    fn save_file(&mut self, _name: &str, _mime: &str, _data: &[u8]) -> Result<(), String> {
        Err("saving files isn't available here".into())
    }

    /// `figma.showUI(__html__, options)`: open (or replace) the plugin's HTML frame. `html` is the
    /// page with `__html__` already substituted from the manifest's `ui`.
    fn show_ui(&mut self, _html: &str, _options: &UiOptions) -> Result<(), String> {
        Err("this host has no UI frame".into())
    }

    /// `figma.ui.postMessage(message)`: plugin → frame, JSON only.
    fn post_message(&mut self, _message: &Value) -> Result<(), String> {
        Err("no UI frame is open".into())
    }

    /// `figma.ui.close()` / `figma.closePlugin()`.
    fn close_ui(&mut self) {}

    /// `figma.commitUndo()`: close the current undo step so the next change starts a new one.
    /// Without it, Figma folds a whole plugin session into one step.
    fn commit_undo(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::{Host, Limits, UiOptions};
    use serde_json::{Value, json};

    struct MapHost {
        nodes: Value,
        calls: Vec<String>,
    }

    impl Host for MapHost {
        fn call(&mut self, command: &str, params: &Value) -> Result<Value, String> {
            self.calls.push(format!("{command} {}", params));
            match command {
                "node.get" => Ok(self.nodes.clone()),
                "select.set" => Ok(json!({"ok": true})),
                _ => Err(format!("unknown command {command}")),
            }
        }

        fn selection(&mut self) -> Vec<u64> {
            vec![3, 4]
        }
    }

    #[test]
    fn default_get_and_selection_run_the_engine_commands() {
        let mut host = MapHost { nodes: json!({"id": 7, "name": "Card"}), calls: Vec::new() };
        let node = host.get(7, Some(&["name"])).unwrap();
        assert_eq!(node["name"], "Card");
        assert_eq!(host.calls[0], "node.get {\"fields\":[\"name\"],\"id\":7}");
        host.set_selection(&[3, 4]).unwrap();
        assert_eq!(host.calls[1], "select.set {\"ids\":[3,4]}");
    }

    #[test]
    fn options_default_to_figma_s_frame_size() {
        let options = UiOptions::default();
        assert_eq!((options.width, options.height), (300.0, 200.0));
        assert!(!options.theme_colors);
        let options = UiOptions::from_json(&json!({"width": 640, "height": 480, "title": "Tidy", "themeColors": true}));
        assert_eq!((options.width, options.height, options.theme_colors), (640.0, 480.0, true));
        assert_eq!(options.title.as_deref(), Some("Tidy"));
    }

    #[test]
    fn limits_are_described_for_the_install_confirmation() {
        let described = Limits::default().describe();
        assert_eq!(described["memory_mb"], 256);
        assert_eq!(described["time_ms"], 30_000);
    }
}
