//! Path geometry: the CPU's own outlines, tessellated into triangles.
//!
//! Strokes are not offset here. `figcraft-render` already turns a node's stroke (`align`, joins,
//! caps, dashes, per-side weights and arrowheads included) into an ordinary fillable `BezPath`
//! through `figcraft-pathops`, and the CPU fills exactly that path; this module fills the same one,
//! so a GPU stroke cannot drift from the CPU's joins or dash phase.
//!
//! `lyon` only does the triangulation: it splits the path into non-overlapping triangles, which the
//! multisampled target then covers per sample without double-blending shared edges.

use std::sync::Arc;

use figcraft_doc::{Document, Node};
use figcraft_geom::BezPath;
use lyon_path::Path as LyonPath;
use lyon_tessellation::geometry_builder::BuffersBuilder;
use lyon_tessellation::{FillOptions, FillTessellator, FillVertex, VertexBuffers};
use tiny_skia::FillRule;

/// Curves are flattened at this tolerance, in document units. Below a tenth of a pixel at 100% zoom,
/// so the triangles' outline is the CPU's outline as far as any pixel can tell.
const TOLERANCE: f32 = 0.05;

/// Triangles covering a filled path, in the node's local space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Triangles {
    pub verts: Vec<[f32; 2]>,
    pub tris: Vec<u32>,
}

impl Triangles {
    pub fn is_empty(&self) -> bool {
        self.tris.is_empty() || self.verts.is_empty()
    }

    /// The area the triangles cover (shoelace, summed). Tests use it to check that tessellation
    /// covers the shape the CPU covers, without a GPU.
    pub fn area(&self) -> f64 {
        let mut sum = 0.0;
        for tri in self.tris.chunks_exact(3) {
            let (a, b, c) = (self.verts[tri[0] as usize], self.verts[tri[1] as usize], self.verts[tri[2] as usize]);
            sum += (f64::from(b[0] - a[0]) * f64::from(c[1] - a[1]) - f64::from(c[0] - a[0]) * f64::from(b[1] - a[1])) * 0.5;
        }
        sum.abs()
    }
}

/// The path a node's fill covers, with its fill rule — the CPU's [`figcraft_render::node_outline`].
pub fn fill_outline(doc: &Document, node: &Node) -> Option<(BezPath, FillRule)> {
    figcraft_render::node_outline(doc, node)
}

/// The path a node's stroke covers, with its fill rule — the CPU's [`figcraft_render::stroke_fill`].
pub fn stroke_outline(doc: &Document, node: &Node) -> Option<(Arc<BezPath>, FillRule)> {
    figcraft_render::stroke_fill(doc, node)
}

/// Triangulate `path` for the given fill rule.
pub fn tessellate(path: &BezPath, rule: FillRule) -> Triangles {
    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut tessellator = FillTessellator::new();
    let options = FillOptions::tolerance(TOLERANCE).with_fill_rule(match rule {
        FillRule::EvenOdd => lyon_tessellation::FillRule::EvenOdd,
        FillRule::Winding => lyon_tessellation::FillRule::NonZero,
    });
    let lyon_path = to_lyon(path);
    let built = tessellator.tessellate_path(
        &lyon_path,
        &options,
        &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| vertex.position().to_array()),
    );
    if built.is_err() {
        return Triangles::default();
    }
    Triangles { verts: buffers.vertices, tris: buffers.indices }
}

/// A kurbo path as a lyon path (move/line/quad/cubic/close, no loss).
fn to_lyon(path: &BezPath) -> LyonPath {
    let mut builder = LyonPath::builder();
    for element in path.elements() {
        match *element {
            figcraft_geom::kurbo::PathEl::MoveTo(p) => {
                builder.begin(lyon_path::math::point(p.x as f32, p.y as f32));
            }
            figcraft_geom::kurbo::PathEl::LineTo(p) => {
                builder.line_to(lyon_path::math::point(p.x as f32, p.y as f32));
            }
            figcraft_geom::kurbo::PathEl::QuadTo(c, p) => {
                builder.quadratic_bezier_to(
                    lyon_path::math::point(c.x as f32, c.y as f32),
                    lyon_path::math::point(p.x as f32, p.y as f32),
                );
            }
            figcraft_geom::kurbo::PathEl::CurveTo(c1, c2, p) => {
                builder.cubic_bezier_to(
                    lyon_path::math::point(c1.x as f32, c1.y as f32),
                    lyon_path::math::point(c2.x as f32, c2.y as f32),
                    lyon_path::math::point(p.x as f32, p.y as f32),
                );
            }
            figcraft_geom::kurbo::PathEl::ClosePath => builder.end(true),
        }
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use figcraft_color::{Color, Paint};
    use figcraft_doc::{Document, NodeKind};
    use figcraft_geom::{Affine, Shape};

    use super::*;

    #[test]
    fn a_rectangle_is_tessellated_to_its_own_area() {
        let rect = figcraft_geom::kurbo::Rect::new(0.0, 0.0, 100.0, 50.0).to_path(0.05);
        let tris = tessellate(&rect, FillRule::Winding);
        assert!(!tris.is_empty());
        assert!((tris.area() - 5000.0).abs() < 50.0, "area {} vs 5000", tris.area());
    }

    #[test]
    fn an_even_odd_ring_leaves_its_hole_empty() {
        let mut path = figcraft_geom::kurbo::Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.05);
        path.extend(figcraft_geom::kurbo::Rect::new(25.0, 25.0, 75.0, 75.0).to_path(0.05));
        let ring = tessellate(&path, FillRule::EvenOdd);
        assert!((ring.area() - 7500.0).abs() < 100.0, "even-odd area {} vs 7500", ring.area());
        let solid = tessellate(&path, FillRule::Winding);
        assert!((solid.area() - 10000.0).abs() < 100.0, "non-zero area {} vs 10000", solid.area());
    }

    #[test]
    fn a_stroke_node_gives_the_cpu_its_own_outline_and_the_gpu_triangles() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let line = doc.create(page, NodeKind::Line, "Line", 0.0, 0.0, 100.0, 0.0).expect("a line");
        if let Ok(n) = doc.get_mut(line) {
            n.strokes = vec![Paint::solid(Color::new(0.0, 0.0, 0.0, 1.0))];
            n.stroke.weight = 8.0;
            n.transform = Affine::translate((10.0, 40.0));
        }
        let node = doc.node(line).expect("the line");
        let (path, rule) = stroke_outline(&doc, node).expect("the stroke's outline");
        let tris = tessellate(&path, rule);
        assert!(!tris.is_empty(), "an 8px stroke over 100 units has triangles");
        let area = tris.area();
        assert!((700.0..=1100.0).contains(&area), "stroke area {area}");
    }

    #[test]
    fn a_rotated_quad_keeps_its_area() {
        let mut path = figcraft_geom::kurbo::Rect::new(0.0, 0.0, 60.0, 60.0).to_path(0.05);
        path.apply_affine(Affine::rotate(0.4));
        let tris = tessellate(&path, FillRule::Winding);
        assert!((tris.area() - 3600.0).abs() < 40.0, "rotated area {}", tris.area());
    }
}
