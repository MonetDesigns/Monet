//! P0 regression tests for Developer Mode parity (gate / Status / Compare).
//!
//! These stay pure: they exercise the decisions the panels make, not a live GPUI window.
//! A future `TestAppContext` pass can build on them without repeating the policy.

#[cfg(test)]
mod tests {
    use crate::app::CanvasApp;
    use crate::dev_gate;
    use crate::panels::right::inspect::{CompareWith, status_actions};
    use figcraft_doc::Document;

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
}
