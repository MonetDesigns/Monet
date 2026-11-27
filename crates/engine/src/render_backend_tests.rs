//! `pref.renderBackend` — the canvas renderer preference the GPU crate reads.

use serde_json::json;

use crate::Session;

#[test]
fn the_renderer_preference_defaults_to_auto_and_round_trips() {
    let mut s = Session::new();
    assert_eq!(s.execute("pref.renderBackend", &json!({})).unwrap()["backend"], "auto");
    assert_eq!(s.prefs.render_backend, "auto");
    for value in ["gpu", "cpu", "auto"] {
        assert_eq!(s.execute("pref.renderBackend", &json!({ "backend": value })).unwrap()["backend"], value);
        assert_eq!(s.prefs.render_backend, value);
    }
}

#[test]
fn a_hand_written_value_reads_as_itself_and_junk_is_an_error() {
    let mut s = Session::new();
    // Case and padding are forgiven: a preferences file edited by hand still works.
    s.execute("pref.renderBackend", &json!({ "backend": " GPU " })).unwrap();
    assert_eq!(s.prefs.render_backend, "gpu");
    // Anything else is refused, and the preference stays where it was.
    assert!(s.execute("pref.renderBackend", &json!({ "backend": "metal" })).is_err());
    assert!(s.execute("pref.renderBackend", &json!({ "backend": 3 })).is_err(), "not even a number");
    assert_eq!(s.prefs.render_backend, "gpu");
}
