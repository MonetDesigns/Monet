//! P0 regression tests for Developer Mode parity (gate / Status / Compare).
//!
//! These stay pure: they exercise the decisions the panels make, not a live GPUI window.
//! A future `TestAppContext` pass can build on them without repeating the policy.

#[cfg(test)]
mod tests {
    use crate::app::CanvasApp;
    use crate::dev_gate;
    use crate::panels::right::data::read_playground;
    use crate::panels::right::inspect::{CompareWith, status_actions};
    use figcraft_doc::Document;
    use serde_json::{Value, json};

    fn app() -> CanvasApp {
        CanvasApp::new(Document::new("devmode-tests"))
    }

    #[test]
    fn focused_target_still_compares_with_the_newest_version() {
        let (focus, compare) = status_actions(true, false, Some("v3"));
        assert!(!focus, "a focused frame/section must hide Focus view");
        assert_eq!(compare, Some(CompareWith::Version("v3")), "Compare must carry the newest version id");
    }

    #[test]
    fn changed_target_compares_without_a_version() {
        let (focus, compare) = status_actions(false, true, Some("v3"));
        assert!(focus, "an unfocused target offers Focus view");
        assert_eq!(compare, Some(CompareWith::Changed), "a changed target lets dev.compare find the baseline");
    }

    #[test]
    fn unchanged_target_without_a_version_has_no_compare() {
        let (_, compare) = status_actions(false, false, None);
        assert_eq!(compare, None, "dev.compare would fail without a version; don't show the button");
    }

    #[test]
    fn the_gate_covers_every_edit_entry_point() {
        let mut app = app();
        app.dev_inspect = true;
        for id in [
            "edit.delete",
            "transform.nudge",
            "object.opacityKeys",
            "object.toggleVisible",
            "object.toggleLock",
            "node.set",
            "node.reparent",
            "page.delete",
            "page.add",
            "instance.create",
            "library.acceptUpdates",
            "style.apply",
            "file.revert",
        ] {
            assert!(dev_gate::refuses(&app, id), "{id} must be refused in Developer Mode");
        }
    }

    #[test]
    fn the_gate_allows_dev_modes_own_edits() {
        let mut app = app();
        app.dev_inspect = true;
        for id in [
            "tool.move",
            "tool.hand",
            "tool.comment",
            "tool.annotation",
            "annotation.add",
            "dev.focus",
            "dev.compare",
            "devMode.addMeasurement",
            "comment.add",
            "view.zoomFit",
            "select.set",
        ] {
            assert!(!dev_gate::refuses(&app, id), "{id} must stay available in Developer Mode");
        }
    }

    /// A component with one text property, and an instance of it: the smallest real Playground.
    fn playground_doc() -> (Document, figcraft_doc::NodeId) {
        let mut s = figcraft_engine::Session::new();
        let id = |v: Value| figcraft_engine::params::id_of(&v["id"]).unwrap();
        let frame = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 120, "height": 40})).unwrap());
        let label = id(s.execute("text.create", &json!({"text": "Buy", "x": 8, "y": 8, "parent": frame})).unwrap());
        let comp = id(s.execute("component.create", &json!({"ids": [frame]})).unwrap());
        s.execute("component.addProperty", &json!({"id": comp, "name": "Label", "type": "text", "default": "Buy"})).unwrap();
        s.execute("component.bindProperty", &json!({"id": label, "property": "Label", "field": "characters"})).unwrap();
        let inst = id(s.execute("instance.create", &json!({"component": comp, "x": 0, "y": 300})).unwrap());
        (s.doc().unwrap().clone(), inst)
    }

    #[test]
    fn the_playground_tries_values_on_a_copy_and_renders_both() {
        let (doc, inst) = playground_doc();
        // The cache key's revision is supplied by the caller (`DocState::revision`); 0 is fine here.
        let revision = 0;
        let p = read_playground(&doc, inst, &serde_json::Map::new(), figcraft_engine::codegen::CodeLang::React, revision)
            .expect("a component instance must open the Playground");
        assert!(!p.tried);
        assert!(p.preview.as_ref().is_some_and(|(_, _, px)| !px.is_empty()), "the preview must render pixels");
        assert!(p.code.contains("Buy"), "{}", p.code);
        assert!(!p.properties.is_empty(), "the property list drives the controls");
        let mut values = serde_json::Map::new();
        values.insert("Label".into(), Value::String("Pay now".into()));
        let changed = read_playground(&doc, inst, &values, figcraft_engine::codegen::CodeLang::React, revision)
            .expect("valid values must build");
        assert!(changed.tried);
        assert!(changed.code.contains("Pay now") && changed.code != p.code, "code must follow the tried value");
    }
}
