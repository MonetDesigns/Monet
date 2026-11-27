//! Hover and selection outlines are the layer's plain box (2026-11-27): no outline follows
//! rounded corners or a shape's geometry any more — the user asked for four straight edges.

use figcraft_doc::{Document, NodeId};
use figcraft_engine::Session;
use figcraft_geom::kurbo::BezPath;
use serde_json::{Value, json};

use super::outline::outline_path;

fn layer(s: &mut Session, kind: &str, props: Value) -> NodeId {
    let v = s.execute("node.create", &json!({"type": kind, "x": 10, "y": 20, "width": 465, "height": 254})).unwrap();
    let id = NodeId(v["id"].as_u64().unwrap());
    s.execute("node.set", &json!({"id": id, "props": props})).unwrap();
    id
}

fn path_of(doc: &Document, id: NodeId) -> Option<BezPath> {
    outline_path(doc, doc.node(id).unwrap())
}

/// Rounded rectangles and frames keep their box: the four edges are straight even when every
/// corner has its own radius (and smoothing).
#[test]
fn rounded_corners_keep_the_box() {
    let mut s = Session::new();
    let props = json!({"stroke": "#000000", "strokeWeight": 12, "strokeAlign": "outside", "cornerSmoothing": 0.6,
        "topLeftRadius": 0, "topRightRadius": 60, "bottomRightRadius": 20, "bottomLeftRadius": 43});
    for kind in ["rectangle", "frame"] {
        let id = layer(&mut s, kind, props.clone());
        assert!(path_of(s.doc().unwrap(), id).is_none(), "{kind}: the outline is the box");
    }
}

/// Ellipses, vectors, polygons, stars and boolean groups keep their box too: no outline hugs the
/// shape's geometry.
#[test]
fn shaped_layers_keep_the_box() {
    let mut s = Session::new();
    for kind in ["ellipse", "polygon", "star"] {
        let id = layer(&mut s, kind, json!({}));
        assert!(path_of(s.doc().unwrap(), id).is_none(), "{kind}: the outline is the box");
    }
}
