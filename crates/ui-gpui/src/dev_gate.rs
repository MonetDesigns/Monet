//! Developer Mode's refusal gate.
//!
//! EGUI has one gate (`ui-egui/src/menus.rs::dev_mode_refuses`) that every command entry calls
//! before running anything; GPUI used to check only the canvas keys. The engine owns the policy
//! ([`figcraft_engine::devinspect::dev_mode_allows`] — commands without history plus Dev Mode's
//! own edits), so this module only answers "does the current tab refuse this command?" and keeps
//! the wording in one place. Callers add the toast, because only a view has a toast.

use crate::app::CanvasApp;
use figcraft_i18n::tr;
use serde_json::Value;

/// Whether the Inspect tab refuses command `id`.
pub(crate) fn refuses(app: &CanvasApp, id: &str) -> bool {
    app.dev_inspect && !figcraft_engine::devinspect::dev_mode_allows(id)
}

/// [`refuses`] with command parameters. `plugin.run` isn't an undo step (the plugin decides), so
/// the engine command itself is allowed; Developer Mode must still refuse a write plugin before
/// it runs, while read plugins stay runnable (P1-5).
pub(crate) fn refuses_command(app: &CanvasApp, id: &str, params: &Value) -> bool {
    if id == "plugin.run" && app.dev_inspect {
        let Some(pid) = params.get("id").and_then(Value::as_str) else { return true };
        return app
            .session
            .prefs
            .plugins
            .iter()
            .find(|p| p.manifest.id == pid)
            .is_some_and(|p| p.manifest.permissions.access != figcraft_engine::plugins::Access::Read);
    }
    refuses(app, id)
}

/// The sentence shown when a command is refused, matching EGUI's toast.
pub(crate) fn message() -> &'static str {
    tr("Developer Mode only inspects: switch to Design (Shift+D) to edit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::Document;

    fn app_with(dev_inspect: bool) -> CanvasApp {
        let mut app = CanvasApp::new(Document::new("dev-gate"));
        app.dev_inspect = dev_inspect;
        app
    }

    #[test]
    fn dev_mode_refuses_edits_but_not_inspection() {
        let app = app_with(true);
        assert!(refuses(&app, "transform.nudge"));
        assert!(refuses(&app, "edit.delete"));
        assert!(refuses(&app, "tool.rectangle"));
        assert!(!refuses(&app, "tool.annotation"));
        assert!(!refuses(&app, "annotation.add"));
        assert!(!refuses(&app, "dev.focus"));
        assert!(!refuses(&app, "dev.compare"));
        assert!(!refuses(&app, "view.zoomFit"));
    }

    #[test]
    fn design_tab_refuses_nothing() {
        let app = app_with(false);
        assert!(!refuses(&app, "transform.nudge"));
        assert!(!refuses(&app, "tool.rectangle"));
    }

    #[test]
    fn developer_mode_runs_read_plugins_but_not_write_plugins() {
        let mut app = app_with(true);
        for (id, access) in [("org.test.read", "read"), ("org.test.write", "write")] {
            let manifest = figcraft_engine::plugins::Manifest::parse(&format!(
                r#"{{"id":"{id}","name":"{id}","main":"plugin.wasm","permissions":{{"access":"{access}"}}}}"#
            ))
            .unwrap();
            app.session.prefs.plugins.push(figcraft_engine::plugins::InstalledPlugin { manifest, path: None, source: None });
        }
        assert!(!refuses_command(&app, "plugin.run", &serde_json::json!({"id": "org.test.read"})));
        assert!(refuses_command(&app, "plugin.run", &serde_json::json!({"id": "org.test.write"})));
        assert!(!refuses_command(&app_with(false), "plugin.run", &serde_json::json!({"id": "org.test.write"})));
    }
}
