//! Copy/paste of layers that carry Motion animation: the copy keeps its timing, stays on the
//! same page spot, and the destination timeline grows enough to show it.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::{AnimProperty, AnimValue, TimelineSettings};
use figcraft_geom::Vec2;

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, parent: Option<NodeId>, x: f64, y: f64) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": 40, "height": 20});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(&s.execute("shape.rectangle", &p).unwrap())
}

fn frame(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id(&s.execute("frame.create", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn key(s: &mut Session, node: NodeId, time: u32, x: f64, y: f64) {
    let doc = s.doc_mut().unwrap();
    let n = doc.node_mut(node).unwrap();
    n.motion.ensure_track(AnimProperty::Position).set_key(time, AnimValue::Vec2(Vec2::new(x, y)), None);
}

fn settings(duration_ms: u32, fps: u32, looping: bool) -> TimelineSettings {
    TimelineSettings { duration_ms, fps, looping }
}

fn copy(s: &mut Session, node: NodeId) {
    s.set_selection(vec![node]);
    s.execute("edit.copy", &json!({})).unwrap();
}

fn paste(s: &mut Session, params: Value) -> Vec<NodeId> {
    s.execute("edit.paste", &params).unwrap()["ids"].as_array().unwrap().iter().filter_map(id_of).collect()
}

fn motion_span(s: &Session, node: NodeId, duration: u32) -> Option<(u32, u32)> {
    s.doc().unwrap().node(node).unwrap().motion.span_ms(duration)
}

#[test]
fn pasted_frame_keeps_its_timeline_settings() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 200.0, 200.0);
    let dot = rect(&mut s, Some(f), 10.0, 10.0);
    key(&mut s, dot, 0, 10.0, 10.0);
    key(&mut s, dot, 500, 150.0, 10.0);
    s.doc_mut().unwrap().timelines.insert(f, settings(700, 24, true));

    copy(&mut s, f);
    // Clear the selection: the frame is copied as a page-level sibling instead of inside itself.
    s.set_selection(vec![]);
    let pasted = paste(&mut s, json!({}));
    assert_eq!(pasted.len(), 1);
    let page = s.doc().unwrap().page_of(f).unwrap();
    assert_eq!(s.doc().unwrap().node(pasted[0]).unwrap().parent, Some(page));
    assert_eq!(
        s.doc().unwrap().timeline_settings_for(pasted[0]),
        settings(700, 24, true),
        "the pasted frame plays on its own clock, not the page's default 2000 ms"
    );
}

#[test]
fn pasted_layer_into_a_frame_keeps_its_page_position() {
    let mut s = Session::new();
    let f = frame(&mut s, 50.0, 50.0, 200.0, 200.0);
    let r = rect(&mut s, None, 100.0, 100.0);
    key(&mut s, r, 0, 100.0, 100.0);
    key(&mut s, r, 500, 300.0, 100.0);

    copy(&mut s, r);
    s.set_selection(vec![f]);
    let pasted = paste(&mut s, json!({}));
    assert_eq!(pasted.len(), 1);
    let n = s.doc().unwrap().node(pasted[0]).unwrap();
    assert_eq!(n.parent, Some(f));
    let b = s.doc().unwrap().world_bounds(pasted[0]).unwrap();
    assert_eq!((b.x0, b.y0), (100.0, 100.0), "the copy lands where it was");
    let pos = n.motion.track(AnimProperty::Position).unwrap();
    assert_eq!(pos.keys[0].value, AnimValue::Vec2(Vec2::new(50.0, 50.0)), "keys are frame-local now");
    assert_eq!(pos.keys[1].value, AnimValue::Vec2(Vec2::new(250.0, 50.0)), "and the whole path moved with it");
}

#[test]
fn pasted_layer_with_an_offset_moves_its_animation_too() {
    let mut s = Session::new();
    let r = rect(&mut s, None, 0.0, 0.0);
    key(&mut s, r, 0, 0.0, 0.0);
    key(&mut s, r, 500, 100.0, 0.0);

    copy(&mut s, r);
    s.set_selection(vec![]);
    let pasted = paste(&mut s, json!({"x": 200, "y": 100}));
    let n = s.doc().unwrap().node(pasted[0]).unwrap();
    let b = s.doc().unwrap().world_bounds(pasted[0]).unwrap();
    assert_eq!((b.x0, b.y0), (200.0, 100.0));
    let pos = n.motion.track(AnimProperty::Position).unwrap();
    assert_eq!(pos.keys[0].value, AnimValue::Vec2(Vec2::new(200.0, 100.0)));
    assert_eq!(pos.keys[1].value, AnimValue::Vec2(Vec2::new(300.0, 100.0)));
}

#[test]
fn paste_grows_the_destination_timeline_to_fit_the_pasted_animation() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 200.0, 200.0);
    s.doc_mut().unwrap().timelines.insert(f, settings(400, 30, false));
    let r = rect(&mut s, None, 10.0, 10.0);
    key(&mut s, r, 0, 10.0, 10.0);
    key(&mut s, r, 3000, 300.0, 10.0);

    copy(&mut s, r);
    s.set_selection(vec![f]);
    let pasted = paste(&mut s, json!({}));
    let settings = s.doc().unwrap().timeline_settings_for(f);
    assert_eq!(settings.duration_ms, 3000, "the host frame grows so the pasted bar is on screen");
    assert_eq!((settings.fps, settings.looping), (30, false), "its other controls stay as the user set them");
    assert_eq!(motion_span(&s, pasted[0], settings.duration_ms), Some((0, 3000)));
}

#[test]
fn an_instance_reads_the_main_timeline_duration() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 100.0, 100.0);
    let dot = rect(&mut s, Some(f), 10.0, 10.0);
    key(&mut s, dot, 0, 10.0, 10.0);
    key(&mut s, dot, 600, 60.0, 10.0);
    s.doc_mut().unwrap().timelines.insert(f, settings(600, 24, true));
    let comp = id(&s.execute("component.create", &json!({"ids": [f]})).unwrap());
    let inst = id(&s.execute("instance.create", &json!({"component": comp, "x": 200, "y": 0})).unwrap());
    assert_eq!(s.doc().unwrap().timeline_settings_for(inst), settings(600, 24, true));
}

#[test]
fn pasting_an_animated_frame_into_another_grows_the_host_timeline() {
    let mut s = Session::new();
    let inner = frame(&mut s, 0.0, 0.0, 100.0, 100.0);
    let dot = rect(&mut s, Some(inner), 10.0, 10.0);
    key(&mut s, dot, 0, 10.0, 10.0);
    key(&mut s, dot, 700, 60.0, 10.0);
    s.doc_mut().unwrap().timelines.insert(inner, settings(700, 24, true));
    let host = frame(&mut s, 300.0, 0.0, 400.0, 400.0);
    s.doc_mut().unwrap().timelines.insert(host, settings(400, 30, false));

    copy(&mut s, inner);
    s.set_selection(vec![host]);
    let pasted = paste(&mut s, json!({}));
    assert_eq!(s.doc().unwrap().node(pasted[0]).unwrap().parent, Some(host));
    let settings = s.doc().unwrap().timeline_settings_for(host);
    assert_eq!((settings.duration_ms, settings.fps, settings.looping), (700, 30, false), "the host grows, keeping its own fps/loop");
}

#[test]
fn pasted_instance_keeps_the_main_timeline_duration() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 100.0, 100.0);
    let dot = rect(&mut s, Some(f), 10.0, 10.0);
    key(&mut s, dot, 0, 10.0, 10.0);
    key(&mut s, dot, 600, 60.0, 10.0);
    s.doc_mut().unwrap().timelines.insert(f, settings(600, 24, true));
    let comp = id(&s.execute("component.create", &json!({"ids": [f]})).unwrap());

    copy(&mut s, comp);
    s.set_selection(vec![]);
    let pasted = paste(&mut s, json!({}));
    assert_eq!(pasted.len(), 1);
    assert!(matches!(s.doc().unwrap().node(pasted[0]).unwrap().kind, figcraft_doc::NodeKind::Instance { .. }));
    assert!(!s.doc().unwrap().timelines.contains_key(&pasted[0]), "instances have no clock of their own");
    assert_eq!(s.doc().unwrap().timeline_settings_for(pasted[0]), settings(600, 24, true), "an animated component's clock travels with its instance");
}

#[test]
fn pasting_two_animated_frames_grows_each_scope_by_its_own_end() {
    let mut s = Session::new();
    let slow = frame(&mut s, 0.0, 0.0, 200.0, 200.0);
    let slow_dot = rect(&mut s, Some(slow), 10.0, 10.0);
    key(&mut s, slow_dot, 0, 10.0, 10.0);
    key(&mut s, slow_dot, 3000, 150.0, 10.0);
    s.doc_mut().unwrap().timelines.insert(slow, settings(400, 30, false));

    let quick = frame(&mut s, 300.0, 0.0, 200.0, 200.0);
    let quick_dot = rect(&mut s, Some(quick), 10.0, 10.0);
    key(&mut s, quick_dot, 0, 10.0, 10.0);
    key(&mut s, quick_dot, 900, 150.0, 10.0);
    s.doc_mut().unwrap().timelines.insert(quick, settings(500, 24, true));

    s.set_selection(vec![slow, quick]);
    s.execute("edit.copy", &json!({})).unwrap();
    s.set_selection(vec![]);
    let pasted = paste(&mut s, json!({}));
    assert_eq!(pasted.len(), 2);
    let mut durations: Vec<(u32, u32, bool)> = pasted
        .iter()
        .map(|root| {
            let st = s.doc().unwrap().timeline_settings_for(*root);
            (st.duration_ms, st.fps, st.looping)
        })
        .collect();
    durations.sort();
    assert_eq!(
        durations,
        vec![(900, 24, true), (3000, 30, false)],
        "each pasted frame grows only for the animation it contains"
    );
}

#[test]
fn pasting_a_top_level_instance_does_not_create_an_instance_timeline() {
    let mut s = Session::new();
    let page = s.doc().unwrap().first_page().unwrap();
    s.doc_mut().unwrap().timelines.insert(page, settings(300, 30, false));

    let main = frame(&mut s, 0.0, 0.0, 100.0, 100.0);
    let dot = rect(&mut s, Some(main), 10.0, 10.0);
    key(&mut s, dot, 0, 10.0, 10.0);
    key(&mut s, dot, 600, 60.0, 10.0);
    s.doc_mut().unwrap().timelines.insert(main, settings(600, 24, true));
    let comp = id(&s.execute("component.create", &json!({"ids": [main]})).unwrap());

    copy(&mut s, comp);
    s.set_selection(vec![]);
    let pasted = paste(&mut s, json!({}));
    let doc = s.doc().unwrap();
    assert!(matches!(doc.node(pasted[0]).unwrap().kind, figcraft_doc::NodeKind::Instance { .. }));
    assert!(!doc.timelines.contains_key(&pasted[0]), "the instance still follows its main, not a private clock");
    assert_eq!(doc.timelines.get(&page), Some(&settings(300, 30, false)), "the page clock is left alone");
    assert_eq!(doc.timeline_settings_for(pasted[0]), settings(600, 24, true));
}

#[test]
fn pasting_an_instance_keeps_a_foreign_mains_timeline() {
    use figcraft_doc::ImportedComponent;

    let mut s = Session::new();
    let (main, dot) = {
        let doc = s.doc_mut().unwrap();
        let store = doc.ensure_library_store();
        let main = doc
            .create(
                store,
                figcraft_doc::NodeKind::Component {
                    description: String::new(),
                    props: Default::default(),
                    prop_order: Vec::new(),
                },
                "Main",
                0.0,
                0.0,
                100.0,
                100.0,
            )
            .unwrap();
        let dot = doc.create(main, figcraft_doc::NodeKind::Rectangle, "Dot", 10.0, 10.0, 20.0, 20.0).unwrap();
        (main, dot)
    };
    key(&mut s, dot, 0, 10.0, 10.0);
    key(&mut s, dot, 700, 60.0, 10.0);
    let main = main;
    {
        let doc = s.doc_mut().unwrap();
        doc.timelines.insert(main, settings(700, 24, true));
        doc.libraries_mut().components.push(ImportedComponent {
            library: "Other".into(),
            key: main,
            root: main,
            version: 1,
            ids: [(main, main)].into(),
        });
    }
    let inst = id(&s.execute("instance.create", &json!({"component": main, "x": 200, "y": 0})).unwrap());

    copy(&mut s, inst);
    s.execute("file.new", &json!({"name": "Other"})).unwrap();
    let pasted = paste(&mut s, json!({}));
    assert_eq!(pasted.len(), 1);
    let doc = s.doc().unwrap();
    let main = match doc.node(pasted[0]).unwrap().kind {
        figcraft_doc::NodeKind::Instance { component, .. } => component,
        ref other => panic!("pasted an instance, got {other:?}"),
    };
    assert!(doc.is_remote(main), "the pasted instance uses the adopted library main");
    assert_eq!(doc.timelines.get(&main), Some(&settings(700, 24, true)), "the adopted main brings its own clock");
    assert_eq!(doc.timeline_settings_for(pasted[0]), settings(700, 24, true));
}
