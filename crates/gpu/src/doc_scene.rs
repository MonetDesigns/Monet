//! `Document` → [`Scene`]: the only place document semantics are read.
//!
//! This milestone accepts a strict subset and **records** everything it cannot draw, so the caller can
//! send the whole frame to the CPU. Nothing is approximated silently:
//!
//! | accepted | recorded as unsupported |
//! |---|---|
//! | page/frame/group/section/component/instance containers and their fills | text, vectors, booleans, lines |
//! | rectangles with four corner radii (no smoothing) | corner smoothing, arcs/rings on ellipses |
//! | full ellipses, regular polygons and stars (3–32 points) | strokes |
//! | solid, linear/radial/angular/diamond gradients (≤ [`MAX_STOPS`] stops) | images, patterns, gradient/pattern fills |
//! | node opacity, visibility, ancestor clipping rectangles | group opacity, other blend modes, masks |
//! | | visible effects (shadow, blur, glass, grain) |
//! | | variable-bound fills (the value is not resolved here) |

use figcraft_color::{BlendMode, GradientKind as DocGradientKind, Paint as DocPaint, PaintKind};
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::{Affine, Point};

use crate::scene::{
    is_similarity, Clip, GradientKind, Paint, Rgba, Scene, ShapeKind, ShapeNode, Stop, Viewport, MAX_STOPS,
};

/// The deepest subtree walked; a document that nests deeper is a broken file and a fallback.
const MAX_DEPTH: usize = 256;

/// Build the frame of `page` as `view` shows it.
pub fn build(doc: &Document, page: NodeId, view: &Viewport) -> Scene {
    let mut scene = Scene::new(view.width, view.height);
    let Some(page_node) = doc.node(page) else {
        scene.unsupported("page", "this page is not in the document");
        return scene;
    };
    if let NodeKind::Page { background, .. } = &page_node.kind {
        scene.clear = Rgba::from_color(*background);
    }
    let view_xf = view.affine();
    for &child in &page_node.children {
        walk(doc, child, view_xf, Clip::NONE, &mut scene, 0);
    }
    scene
}

/// Build one subtree (not the page), transparent where nothing paints — animated export frames.
pub fn build_subtree(doc: &Document, id: NodeId, view: &Viewport) -> Scene {
    let mut scene = Scene::new(view.width, view.height);
    let view_xf = view.affine();
    walk(doc, id, view_xf, Clip::NONE, &mut scene, 0);
    scene
}

/// Whether a node kind only carries children (and possibly its own box fill).
fn is_container(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Page { .. }
            | NodeKind::Frame
            | NodeKind::Group
            | NodeKind::Section { .. }
            | NodeKind::Component { .. }
            | NodeKind::ComponentSet { .. }
            | NodeKind::Instance { .. }
    )
}

fn walk(doc: &Document, id: NodeId, view_xf: Affine, clip: Clip, scene: &mut Scene, depth: usize) {
    if depth > MAX_DEPTH {
        scene.unsupported("depth", format!("a subtree nests deeper than {MAX_DEPTH}"));
        return;
    }
    let Some(n) = doc.node(id) else {
        scene.unsupported("node", "a child points at a node that is not in the document");
        return;
    };
    if !n.visible {
        return;
    }
    let container = is_container(&n.kind);
    if n.effects.iter().any(|e| e.visible) {
        scene.unsupported("effect", format!("`{}` has a visible effect", n.name));
    }
    if !matches!(n.blend, BlendMode::Normal | BlendMode::PassThrough) {
        scene.unsupported("blend", format!("`{}` blends with {:?}", n.name, n.blend));
    }
    if n.is_mask {
        scene.unsupported("mask", format!("`{}` is a mask", n.name));
    }
    if container && n.opacity < 0.999 {
        scene.unsupported("group-opacity", format!("`{}` is a group at {} opacity", n.name, n.opacity));
    }
    if n.strokes.iter().any(|s| s.visible) {
        scene.unsupported("stroke", format!("`{}` has a visible stroke", n.name));
    }
    if n.bound.keys().any(|k| k.starts_with("fills")) {
        scene.unsupported("variable-fill", format!("`{}` has a fill bound to a variable", n.name));
    }

    let (w, h) = (n.width, n.height);
    let m = view_xf * doc.world_transform(id);
    let sized = w > 0.0 && h > 0.0;

    // A clipping node narrows what it and everything under it may paint.
    let mut inner = clip;
    if n.clip && sized {
        let (b, c) = (m.as_coeffs()[1], m.as_coeffs()[2]);
        if b.abs() > 1e-6 || c.abs() > 1e-6 {
            scene.unsupported("rotated-clip", format!("`{}` clips with a rotated frame", n.name));
        } else {
            if n.corners.radii.iter().any(|r| *r > 0.0) {
                scene.unsupported("rounded-clip", format!("`{}` clips with rounded corners", n.name));
            }
            let (x0, y0) = map_point(m, 0.0, 0.0);
            let (x1, y1) = map_point(m, w, h);
            inner = inner.intersect([
                x0.min(x1) as f32,
                y0.min(y1) as f32,
                (x1 - x0).abs() as f32,
                (y1 - y0).abs() as f32,
            ]);
        }
    }

    if sized {
        let shape = shape_of(n, m, w, h, scene);
        if let Some(shape) = shape {
            for (index, fill) in n.fills.iter().enumerate() {
                if !fill.visible {
                    continue;
                }
                if !matches!(fill.blend, BlendMode::Normal) {
                    scene.unsupported("paint-blend", format!("`{}` fill {} blends with {:?}", n.name, index, fill.blend));
                    continue;
                }
                let Some(paint) = paint_of(fill, scene, &n.name, index) else {
                    continue;
                };
                if let Some(paint) = paint.normalised() {
                    scene.push(ShapeNode {
                        mat: crate::scene::matrix(m),
                        size: [w as f32, h as f32],
                        shape,
                        paint,
                        clip: inner,
                        opacity: n.opacity.clamp(0.0, 1.0),
                    });
                }
            }
        }
    }

    for &child in &n.children {
        walk(doc, child, view_xf, inner, scene, depth + 1);
    }
}

/// The shape a node's box is filled as, or `None` (recorded) when this milestone cannot draw it.
fn shape_of(n: &figcraft_doc::Node, m: Affine, w: f64, h: f64, scene: &mut Scene) -> Option<ShapeKind> {
    if !n.fills.iter().any(|f| f.visible) {
        return None;
    }
    let radii = corner_radii(n, m, w, h, scene)?;
    match &n.kind {
        NodeKind::Rectangle | NodeKind::Frame | NodeKind::Group | NodeKind::Section { .. } | NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. } | NodeKind::Page { .. } => {
            Some(ShapeKind::RoundedRect { radii })
        }
        NodeKind::Ellipse { start, sweep, inner } => {
            let full = (*start).abs() < 1e-6 && (*sweep - std::f64::consts::TAU).abs() < 1e-6 && inner.abs() < 1e-6;
            if !full {
                scene.unsupported("ellipse-arc", format!("`{}` is an arc or a ring", n.name));
                return None;
            }
            Some(ShapeKind::Ellipse)
        }
        NodeKind::Polygon { count } => {
            if !(3..=32).contains(count) {
                scene.unsupported("polygon-count", format!("`{}` has {count} sides", n.name));
                return None;
            }
            Some(ShapeKind::Polygon { count: *count })
        }
        NodeKind::Star { count, inner_ratio } => {
            if !(3..=32).contains(count) || !(0.0..1.0).contains(inner_ratio) {
                scene.unsupported("star-count", format!("`{}` is not a drawable star", n.name));
                return None;
            }
            Some(ShapeKind::Star { count: *count, inner_ratio: *inner_ratio as f32 })
        }
        NodeKind::Line => {
            scene.unsupported("line", format!("`{}` is a line", n.name));
            None
        }
        NodeKind::Vector { .. } => {
            scene.unsupported("vector", format!("`{}` is a vector path", n.name));
            None
        }
        NodeKind::Text(_) => {
            scene.unsupported("text", format!("`{}` is text", n.name));
            None
        }
        NodeKind::BooleanOp { .. } => {
            scene.unsupported("boolean", format!("`{}` is a boolean operation", n.name));
            None
        }
        NodeKind::Icon { .. } | NodeKind::Slice => {
            scene.unsupported("icon", format!("`{}` is an icon or slice", n.name));
            None
        }
    }
}

/// One fill as a paintable fill. `None` means it was recorded as unsupported.
fn paint_of(fill: &DocPaint, scene: &mut Scene, name: &str, index: usize) -> Option<Paint> {
    let opacity = fill.opacity.clamp(0.0, 1.0);
    match &fill.kind {
        PaintKind::Solid { color } => Some(Paint::Solid(Rgba::new(color.r, color.g, color.b, color.a * opacity))),
        PaintKind::Gradient(g) => {
            if g.stops.len() > MAX_STOPS {
                scene.unsupported("gradient-stops", format!("`{name}` fill {index} has {} stops (at most {MAX_STOPS})", g.stops.len()));
                return None;
            }
            let mut stops: Vec<Stop> = g
                .stops
                .iter()
                .map(|s| Stop {
                    at: s.position.clamp(0.0, 1.0),
                    color: Rgba::new(s.color.r, s.color.g, s.color.b, s.color.a * opacity),
                })
                .collect();
            // The CPU sorts its stops before sampling; the shader's piecewise lookup needs them in
            // order, and a file may hold them unsorted.
            stops.sort_by(|a, b| a.at.total_cmp(&b.at));
            let kind = match g.kind {
                DocGradientKind::Linear => GradientKind::Linear,
                DocGradientKind::Radial => GradientKind::Radial,
                DocGradientKind::Angular => GradientKind::Angular,
                DocGradientKind::Diamond => GradientKind::Diamond,
            };
            Some(Paint::Gradient {
                kind,
                from: [g.from.0 as f32, g.from.1 as f32],
                to: [g.to.0 as f32, g.to.1 as f32],
                width: [g.width.0 as f32, g.width.1 as f32],
                stops,
            })
        }
        PaintKind::Image(_) => {
            scene.unsupported("image-fill", format!("`{name}` fill {index} is an image"));
            None
        }
        PaintKind::Pattern(_) => {
            scene.unsupported("pattern-fill", format!("`{name}` fill {index} is a pattern"));
            None
        }
    }
}

fn map_point(m: Affine, x: f64, y: f64) -> (f64, f64) {
    let p = m * Point::new(x, y);
    (p.x, p.y)
}

/// The four corner radii in document units, or `None` (recorded) when this milestone cannot draw them:
/// corner smoothing, or a circular corner under a scale that is not a similarity.
fn corner_radii(n: &figcraft_doc::Node, m: Affine, w: f64, h: f64, scene: &mut Scene) -> Option<[f32; 4]> {
    if n.corners.smoothing > 0.01 {
        scene.unsupported("corner-smoothing", format!("`{}` uses corner smoothing", n.name));
        return None;
    }
    let limit = (w.min(h) * 0.5) as f32;
    let radii = n.corners.radii.map(|v| (v as f32).clamp(0.0, limit));
    if radii.iter().any(|v| *v > 0.0) && !is_similarity(crate::scene::matrix(m)) {
        // A circle under a non-uniform scale is an ellipse; the SDF evaluates in local space.
        scene.unsupported("elliptical-corner", format!("`{}` has round corners under a non-uniform scale", n.name));
        return None;
    }
    Some(radii)
}

#[cfg(test)]
mod tests {
    use figcraft_color::Color;
    use figcraft_doc::{Document, NodeKind};
    use figcraft_geom::Affine;

    use super::*;

    fn doc_with_rect() -> (Document, NodeId, NodeId) {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, NodeKind::Rectangle, "Rect", 10.0, 20.0, 100.0, 50.0).expect("a rectangle");
        if let Ok(n) = doc.get_mut(rect) {
            n.fills = vec![DocPaint::solid(Color::new(0.0, 0.0, 0.0, 1.0))];
        }
        (doc, page, rect)
    }

    fn view() -> Viewport {
        Viewport { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 120 }
    }

    #[test]
    fn a_solid_rectangle_becomes_one_instance_at_its_device_box() {
        let (doc, page, _) = doc_with_rect();
        let scene = build(&doc, page, &view());
        assert!(scene.is_supported());
        assert_eq!(scene.nodes.len(), 1, "one fill, one instance");
        let n = &scene.nodes[0];
        assert_eq!(n.size, [100.0, 50.0]);
        assert!(matches!(n.shape, ShapeKind::RoundedRect { .. }));
        assert!(matches!(n.paint, Paint::Solid(c) if c.a == 1.0));
        // Local (10, 20) lands on device pixel (10, 20).
        assert!((n.mat[4] - 10.0).abs() < 0.01 && (n.mat[5] - 20.0).abs() < 0.01, "{:?}", n.mat);
    }

    #[test]
    fn text_is_recorded_and_the_frame_falls_back() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let _text = doc.create(page, NodeKind::Text(Default::default()), "Hi", 0.0, 0.0, 10.0, 10.0).expect("text");
        let scene = build(&doc, page, &view());
        assert!(!scene.is_supported());
        assert!(scene.unsupported_features().iter().any(|u| u.feature == "text"));
    }

    #[test]
    fn a_clipping_frame_narrows_its_children() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let frame = doc.create(page, NodeKind::Frame, "Frame", 10.0, 10.0, 50.0, 50.0).expect("frame");
        if let Ok(n) = doc.get_mut(frame) {
            n.clip = true;
        }
        let child = doc.create(frame, NodeKind::Rectangle, "Child", 0.0, 0.0, 100.0, 10.0).expect("child");
        if let Ok(n) = doc.get_mut(child) {
            n.fills = vec![DocPaint::solid(Color::new(1.0, 0.0, 0.0, 1.0))];
        }
        let scene = build(&doc, page, &view());
        let node = scene.nodes.iter().find(|n| n.size == [100.0, 10.0]).expect("the child's instance");
        assert_eq!(node.clip.0, [10.0, 10.0, 50.0, 50.0], "clipped to the frame's own box");
    }

    #[test]
    fn a_gradient_with_too_many_stops_falls_back() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, NodeKind::Rectangle, "Rect", 0.0, 0.0, 10.0, 10.0).expect("rect");
        let mut g = figcraft_color::Gradient::default_for(figcraft_color::GradientKind::Linear, Color::new(0.0, 0.0, 0.0, 1.0));
        g.stops = (0..8)
            .map(|i| figcraft_color::ColorStop { position: i as f32 / 8.0, color: Color::new(1.0, 0.0, 0.0, 1.0) })
            .collect();
        if let Ok(n) = doc.get_mut(rect) {
            n.fills = vec![DocPaint::gradient(g)];
        }
        let scene = build(&doc, page, &view());
        assert!(!scene.is_supported());
        assert!(scene.unsupported_features().iter().any(|u| u.feature == "gradient-stops"));
    }

    #[test]
    fn a_rotated_clipping_frame_falls_back() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let frame = doc.create(page, NodeKind::Frame, "Frame", 10.0, 10.0, 50.0, 50.0).expect("frame");
        if let Ok(n) = doc.get_mut(frame) {
            n.clip = true;
            n.transform = Affine::rotate(0.3);
        }
        let scene = build(&doc, page, &view());
        assert!(scene.unsupported_features().iter().any(|u| u.feature == "rotated-clip"));
    }
}
