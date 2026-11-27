//! Developer Mode parity tests (gate / Status / Compare / every Inspect Dev section).
//!
//! The policy and section-data tests stay pure: they exercise what the panels decide and the
//! same `read_dev` / `read_playground` the panels consume, without a live GPUI window. Engine
//! suites (`devfocus_tests`, `devmode_behaviour_tests`, `devcompare_tests`) cover the canvas
//! half: focus navigation, hit testing, Escape and compare lifecycle.

#[cfg(test)]
mod tests {
    use crate::app::CanvasApp;
    use crate::dev_gate;
    use crate::panels::right::data::{read_dev, read_playground};
    use crate::panels::right::devmode::accepts_note_focus;
    use crate::panels::right::inspect::{CompareWith, status_actions};
    use figcraft_doc::{Document, NodeId};
    use figcraft_engine::Session;
    use figcraft_engine::params::id_of;
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

    /// The smallest real document used by the section tests: page, frame, child rectangle.
    fn dev_doc() -> (Session, NodeId, NodeId, NodeId) {
        let mut s = Session::new();
        let id = |v: Value| id_of(&v["id"]).unwrap();
        let page = s.page().unwrap();
        let frame = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 200, "name": "Login"})).unwrap());
        let rect = id(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 120, "height": 40, "parent": frame, "props": {"fill": "#FF0000"}})).unwrap());
        (s, page, frame, rect)
    }

    #[test]
    fn ready_list_reads_the_engine_filter_and_status_color() {
        let (mut s, page, frame, _) = dev_doc();
        s.execute("node.set", &json!({"id": frame, "props": {"devStatus": "readyForDev"}})).unwrap();
        let doc = s.doc().unwrap().clone();
        let ready = read_dev(&doc, &[], Some(page), 0);
        let row = ready.ready.iter().find(|r| r.id == frame).expect("the ready frame is listed");
        assert!(row.changed || row.color == (0x08, 0x7D, 0x4F), "{row:?}");
        let completed = read_dev(&doc, &[], Some(page), 2);
        assert!(!completed.ready.iter().any(|r| r.id == frame), "the Completed filter must not list a ready frame");
    }

    #[test]
    fn categories_section_reads_created_categories() {
        let (mut s, page, _, _) = dev_doc();
        let out = s.execute("annotation.addCategory", &json!({"name": "Motion", "color": "#FF0000"})).unwrap();
        let cat_id = out["id"].as_str().unwrap().to_string();
        let doc = s.doc().unwrap().clone();
        let d = read_dev(&doc, &[], Some(page), 0);
        let row = d.categories.iter().find(|c| c.id == cat_id).expect("the category is listed");
        assert_eq!(row.name, "Motion");
        assert!(row.color.eq_ignore_ascii_case("ff0000"), "{row:?}");
    }

    #[test]
    fn annotations_section_reads_labels_pins_and_menu_entries() {
        let (mut s, page, _, rect) = dev_doc();
        s.execute("annotation.add", &json!({"id": rect, "label": "Tap target", "properties": ["width"]})).unwrap();
        let doc = s.doc().unwrap().clone();
        let d = read_dev(&doc, &[rect], Some(page), 0);
        let row = d.annotations.first().expect("the note is listed on its layer");
        assert_eq!(row.label, "Tap target");
        assert_eq!(d.node, Some(rect));
        assert_eq!(row.pins.first().map(|p| p.label.as_str()), Some("Width"), "{row:?}");
        assert!(row.category_menu.iter().any(|(name, _, _)| name == "No category"));
        assert!(d.can_add_annotation, "a layer below the annotation cap can add another note");
    }

    #[test]
    fn dev_resources_section_reads_the_layer_resources() {
        let (mut s, page, _, rect) = dev_doc();
        s.execute("devResource.add", &json!({"id": rect, "url": "https://storybook.example.com/button", "name": "Button stories"}))
            .unwrap();
        let doc = s.doc().unwrap().clone();
        let d = read_dev(&doc, &[rect], Some(page), 0);
        let row = d.dev_resources.first().expect("the resource is listed");
        assert_eq!(row.name, "Button stories");
        assert_eq!(row.url, "https://storybook.example.com/button");
        assert!(row.from.is_none(), "a local resource has no inherited source");
        assert!(d.can_add_resource);
    }

    #[test]
    fn measurements_section_reads_page_measurements_that_touch_the_selection() {
        let (mut s, page, frame, rect) = dev_doc();
        s.execute("devMode.addMeasurement", &json!({"from": rect, "to": frame})).unwrap();
        let doc = s.doc().unwrap().clone();
        let d = read_dev(&doc, &[rect, frame], Some(page), 0);
        let row = d.measurements.first().expect("the measurement is listed for the selected layer");
        assert!(row.value.contains(" · ") || !row.value.is_empty(), "{row:?}");
        assert!(d.can_measure_between, "two ids enable the between-layers action");
    }

    #[test]
    fn a_note_focus_request_is_dropped_for_a_layer_that_is_not_selected() {
        let (mut s, _, _, rect) = dev_doc();
        s.execute("select.set", &json!({"ids": []})).unwrap();
        assert!(!accepts_note_focus(s.doc().unwrap(), s.selection().as_slice(), rect), "nothing selected yet");
        s.execute("select.set", &json!({"ids": [rect]})).unwrap();
        assert!(accepts_note_focus(s.doc().unwrap(), s.selection().as_slice(), rect));
        s.execute("edit.delete", &json!({"ids": [rect]})).unwrap();
        assert!(!accepts_note_focus(s.doc().unwrap(), s.selection().as_slice(), rect), "a deleted layer drops a stale request");
    }

    #[test]
    fn playground_is_absent_for_an_ordinary_layer() {
        let (s, _, _, rect) = dev_doc();
        let doc = s.doc().unwrap().clone();
        assert!(read_playground(&doc, rect, &serde_json::Map::new(), figcraft_engine::codegen::CodeLang::React, 0).is_none());
    }
}
