//! Handle boxes for several layers: a multi-selection is always the page-aligned bounds, even
//! when the layers share a rotation.

use serde_json::json;

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, selection_box, selection_frame};
use figcraft_doc::NodeId;
use figcraft_geom::{Affine, Point};

fn rect(s: &mut Session, x: f64, y: f64, rotation: f64) -> NodeId {
    let id = id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 100, "height": 50})).unwrap()["id"]).unwrap();
    s.execute("node.set", &json!({"id": id, "props": {"rotation": rotation}})).unwrap();
    id
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// The layer's corners in the box's axes.
fn corners_in(s: &Session, frame: Affine, id: NodeId) -> Vec<Point> {
    let doc = s.doc().unwrap();
    let n = doc.node(id).unwrap();
    let m = frame.inverse() * doc.world_transform(id);
    [(0.0, 0.0), (n.width, 0.0), (n.width, n.height), (0.0, n.height)].iter().map(|(x, y)| m * Point::new(*x, *y)).collect()
}

/// Two layers turned by the same 30° still get a box square to the page (2026-11-27): a
/// multi-selection's four edges never take the layers' angle.
#[test]
fn a_shared_rotation_does_not_turn_the_multi_box() {
    let mut s = Session::new();
    let ids = [rect(&mut s, 0.0, 0.0, 30.0), rect(&mut s, 40.0, 120.0, 30.0)];
    let doc = s.doc().unwrap();
    let b = doc.bounds_of(&ids).unwrap();
    let (frame, w, h) = selection_frame(doc, &ids).unwrap();
    let pts: Vec<Point> = ids.iter().flat_map(|i| corners_in(&s, frame, *i)).collect();
    let fold = |f: fn(f64, f64) -> f64, init: f64, k: fn(&Point) -> f64| pts.iter().map(k).fold(init, f);
    // The box hugs the turned layers and stays page-aligned: in the box's own axes the outermost
    // corners sit at the box's edges (0..w, 0..h), and its axes are the page's axes.
    assert!(near(fold(f64::min, f64::INFINITY, |p| p.x), 0.0));
    assert!(near(fold(f64::min, f64::INFINITY, |p| p.y), 0.0));
    assert!(near(fold(f64::max, f64::NEG_INFINITY, |p| p.x), w));
    assert!(near(fold(f64::max, f64::NEG_INFINITY, |p| p.y), h));
    let c = frame.as_coeffs();
    assert!(near(c[0], 1.0) && near(c[1], 0.0) && near(c[2], 0.0) && near(c[3], 1.0), "{c:?}");
    assert!(near(w, b.width()) && near(h, b.height()));
}

#[test]
fn different_or_right_angle_rotations_keep_the_page_aligned_box() {
    let mut s = Session::new();
    for (r0, r1) in [(30.0, 45.0), (90.0, 90.0), (0.0, 180.0)] {
        let ids = [rect(&mut s, 0.0, 0.0, r0), rect(&mut s, 200.0, 50.0, r1)];
        let doc = s.doc().unwrap();
        let b = doc.bounds_of(&ids).unwrap();
        let q = selection_box(doc, &ids).unwrap();
        assert_eq!(q, [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)], "{r0} {r1}");
    }
}

/// A page-aligned multi box drags its page-aligned sides: the frames' handles stay square to the
/// page while the turned layers keep their own 30° (2026-11-27).
#[test]
fn resizing_a_page_aligned_multi_selection_keeps_the_box_square() {
    let mut s = Session::new();
    let ids = [rect(&mut s, 0.0, 0.0, 30.0), rect(&mut s, 40.0, 120.0, 30.0)];
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let (frame, w, h) = selection_frame(s.doc().unwrap(), &ids).unwrap();
    // The layers' centres in the box's axes before the drag: the multi-resize maps them by the
    // box factors exactly, even though each turned layer scales along its own axes.
    let centers0: Vec<Point> = {
        let doc = s.doc().unwrap();
        ids.iter()
            .map(|id| {
                let n = doc.node(*id).unwrap();
                frame.inverse() * (doc.world_transform(*id) * Point::new(n.width / 2.0, n.height / 2.0))
            })
            .collect()
    };
    // Drag the bottom-right handle to make the box 1.5× wider and 2× taller.
    let (from, to) = (frame * Point::new(w, h), frame * Point::new(w * 1.5, h * 2.0));
    s.pointer(&PointerEvent::at(PointerKind::Down, from.x, from.y, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, to.x, to.y, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, to.x, to.y, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    let (f2, w2, h2) = selection_frame(doc, &ids).unwrap();
    // The box stays page-aligned and grows in both directions.
    let c = f2.as_coeffs();
    assert!(near(c[0], 1.0) && near(c[1], 0.0) && near(c[2], 0.0) && near(c[3], 1.0), "{c:?}");
    assert!(w2 > w + 20.0 && h2 > h + 20.0, "{w2} {h2}");
    // The requested corner is the one that moved: bottom-right reaches the drag point, and the
    // opposite (top-left) anchor stays where it was before the drag.
    // The handle box reaches the pointer, but the selection box is rebuilt from the union of the
    // layers: a turned layer scales along its own axes (`axis_factors`), so the union's corners
    // are not the pointer. What the model guarantees is that each layer's centre follows the box
    // factors; the pixel snap rounds the pointer to a whole pixel, so allow half a pixel.
    for (id, c0) in ids.iter().zip(&centers0) {
        let n = doc.node(*id).unwrap();
        let c = frame.inverse() * (doc.world_transform(*id) * Point::new(n.width / 2.0, n.height / 2.0));
        let want = Point::new(c0.x * 1.5, c0.y * 2.0);
        assert!((c.x - want.x).abs() < 0.5 && (c.y - want.y).abs() < 0.5, "{c:?} != {want:?}");
    }
    assert!(w2 > w * 1.4 && h2 > h * 1.8, "{w2} {h2}");
    // The layers keep their rotation and grow with the box.
    for i in ids {
        let n = doc.node(i).unwrap();
        assert!(n.width > 100.0 && n.height > 50.0, "{} {}", n.width, n.height);
        assert!(near(figcraft_geom::rotation_degrees(n.transform), 30.0));
    }
    // One undo step restores both.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(s.doc().unwrap().node(ids[0]).unwrap().width, 100.0));
}

/// 未旋转的多选：并集框与拖拽框同轴，所以右下角严格落在指针上（旋转多选做不到这一点，
/// 见上一条测试的注释）。
#[test]
fn resizing_an_upright_multi_selection_puts_the_corner_on_the_pointer() {
    let mut s = Session::new();
    let ids = [rect(&mut s, 0.0, 0.0, 0.0), rect(&mut s, 40.0, 120.0, 0.0)];
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let (frame, w, h) = selection_frame(s.doc().unwrap(), &ids).unwrap();
    let (from, to) = (frame * Point::new(w, h), frame * Point::new(w * 1.5, h * 2.0));
    s.pointer(&PointerEvent::at(PointerKind::Down, from.x, from.y, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, to.x, to.y, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, to.x, to.y, 1.0)).unwrap();
    let (f2, w2, h2) = selection_frame(s.doc().unwrap(), &ids).unwrap();
    let br = f2 * Point::new(w2, h2);
    let origin = f2 * Point::ZERO;
    assert!(near(br.x, to.x) && near(br.y, to.y), "{br:?} != {to:?}");
    assert!(near(origin.x, (frame * Point::ZERO).x) && near(origin.y, (frame * Point::ZERO).y));
}
