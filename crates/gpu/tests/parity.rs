//! CPU ↔ GPU parity: the same document, two renderers, graded pixels.
//!
//! Every test builds a document, renders it with `figcraft-render` (tiny-skia) and with this crate's
//! GPU pipeline, and compares the two premultiplied buffers. A machine without an adapter skips with
//! `SKIP: no adapter` instead of failing — the plan's rule for CI.
//!
//! The tolerance is the edge band's (2% of pixels may differ by up to 24 levels): tiny-skia resolves
//! coverage analytically while this pipeline uses an SDF and `fwidth`, so one-pixel edges are allowed
//! to differ. Interiors are checked separately with the strict tolerance.

#![cfg(feature = "wgpu")]
// A parity test fails by panicking with the document it could not render.
#![allow(clippy::panic)]

use std::sync::atomic::{AtomicBool, Ordering};

use figcraft_color::{BlendMode, Color, Gradient, GradientKind, Paint};
use figcraft_doc::{Document, NodeId, NodeKind, StrokeJoin};
use figcraft_gpu::Viewport;
use figcraft_gpu::diff::{Tolerance, compare, compare_interiors, pixel};
use figcraft_gpu::scene::Scene;
use figcraft_gpu::doc_scene;
use figcraft_gpu::render::{ExportControl, RenderError};
use figcraft_render::{Options, View};

const SIZE: u32 = 200;

fn view() -> View {
    View { origin: (0.0, 0.0), scale: 1.0, width: SIZE, height: SIZE }
}

fn viewport() -> Viewport {
    Viewport { origin: (0.0, 0.0), scale: 1.0, width: SIZE, height: SIZE }
}

fn cpu_pixels(doc: &Document, page: NodeId) -> Vec<u8> {
    cpu_pixels_in(doc, page, &view())
}

fn cpu_pixels_in(doc: &Document, page: NodeId, view: &View) -> Vec<u8> {
    let opts = Options { background: None, ..Default::default() };
    let pix = figcraft_render::render_view(doc, page, view, &opts).unwrap_or_else(|| panic!("the CPU renders this document"));
    pix.data().to_vec()
}

/// The GPU's pixels for `doc`, or `None` when this machine has no adapter.
///
/// Device creation and teardown is serialised: creating a dozen adapters and devices at once from
/// parallel test threads can wedge a driver (seen with a real font installed, which stops the text
/// test from skipping early). CPU rendering still runs in parallel.
fn gpu_pixels(doc: &Document, page: NodeId) -> Option<Vec<u8>> {
    gpu_pixels_in(doc, page, &viewport())
}

fn gpu_pixels_in(doc: &Document, page: NodeId, viewport: &Viewport) -> Option<Vec<u8>> {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let scene = doc_scene::build(doc, page, viewport);
    assert!(scene.is_supported(), "the test scene must be expressible: {:?}", scene.unsupported_features());
    let mut renderer = figcraft_gpu::render::Renderer::new()?;
    Some(renderer.render(&scene).unwrap_or_else(|error| panic!("the GPU draws this scene: {error}")))
}

/// A transparent page background, so both sides leave the area outside the shapes empty.
fn transparent_page(doc: &mut Document, page: NodeId) {
    if let Ok(n) = doc.get_mut(page)
        && let NodeKind::Page { background, .. } = &mut n.kind
    {
        *background = Color::new(0.0, 0.0, 0.0, 0.0);
    }
}

fn rect(doc: &mut Document, parent: NodeId, name: &str, x: f64, y: f64, w: f64, h: f64, radius: f64, paint: Paint) -> NodeId {
    let id = doc.create(parent, NodeKind::Rectangle, name, x, y, w, h).unwrap_or_else(|error| panic!("a rectangle: {error}"));
    if let Ok(n) = doc.get_mut(id) {
        n.fills = vec![paint];
        n.corners.radii = [radius; 4];
    }
    id
}

#[test]
fn solid_shapes_round_corners_and_rotation_match_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    rect(&mut doc, page, "Plain", 10.0, 10.0, 60.0, 40.0, 0.0, Paint::solid(Color::new(0.9, 0.2, 0.1, 1.0)));
    rect(&mut doc, page, "Rounded", 90.0, 10.0, 50.0, 50.0, 12.0, Paint::solid(Color::new(0.1, 0.4, 0.9, 1.0)));
    let rotated = rect(&mut doc, page, "Rotated", 20.0, 90.0, 70.0, 30.0, 6.0, Paint::solid(Color::new(0.2, 0.7, 0.3, 0.8)));
    if let Ok(n) = doc.get_mut(rotated) {
        n.transform = figcraft_geom::Affine::rotate(std::f64::consts::FRAC_PI_6);
    }

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
    assert!(report.passes(Tolerance::EDGE), "{}", report.summary());

    // Interiors: the middle of the plain rectangle is the fill colour on both sides (within one
    // level: tiny-skia rounds its premultiplied bytes, the GPU's floats round at the end).
    let (cpu_mid, gpu_mid) = (pixel(&cpu, SIZE as usize, 40, 30).expect("in bounds"), pixel(&gpu, SIZE as usize, 40, 30).expect("in bounds"));
    for k in 0..4 {
        assert!(cpu_mid[k].abs_diff(gpu_mid[k]) <= 1, "solid interior differs by more than rounding: {cpu_mid:?} vs {gpu_mid:?}");
    }
}

#[test]
fn abutting_same_paint_rectangles_do_not_seam() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let green = Paint::solid(Color::new(0.18431373, 0.6392157, 0.41960785, 1.0));
    rect(&mut doc, page, "Seam Left", 40.0, 20.0, 240.0, 140.0, 0.0, green.clone());
    rect(&mut doc, page, "Seam Right", 280.0, 20.0, 240.0, 140.0, 0.0, green);

    // 2.94× puts the shared edge between device pixels. Two independent SDF rectangles then each
    // cover the same edge pixel half-way; source-over leaves 75% instead of 100% and a one-pixel
    // line shows. Merged, the edge is interior and fully covered.
    let view = View { origin: (0.0, 0.0), scale: 2.94, width: 1700, height: 600 };
    let vp = Viewport { origin: view.origin, scale: view.scale, width: view.width, height: view.height };
    let scene = doc_scene::build(&doc, page, &vp);
    assert_eq!(
        scene.items.iter().filter(|item| matches!(item, figcraft_gpu::scene::Item::Shape(_))).count(),
        1,
        "the two abutting rectangles merge into one shape"
    );

    let cpu = cpu_pixels_in(&doc, page, &view);
    let Some(gpu) = gpu_pixels_in(&doc, page, &vp) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    for x in 820..=826 {
        let (cpu_px, gpu_px) = (pixel(&cpu, 1700, x, 100).expect("in bounds"), pixel(&gpu, 1700, x, 100).expect("in bounds"));
        for k in 0..4 {
            assert!(cpu_px[k].abs_diff(gpu_px[k]) <= 1, "seam pixel x={x} differs: cpu={cpu_px:?} gpu={gpu_px:?}");
        }
        assert!(gpu_px[3] == 255 && gpu_px[1] > 150, "the seam must be opaque green, not a blend with the background: {gpu_px:?}");
    }

    let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
    assert!(report.passes(Tolerance::EDGE), "{}", report.summary());
}

#[test]
fn l_shaped_same_paint_rectangles_do_not_seam() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let green = Paint::solid(Color::new(0.18431373, 0.6392157, 0.41960785, 1.0));
    rect(&mut doc, page, "Seam Left", 40.0, 20.0, 240.0, 140.0, 0.0, green.clone());
    // Half the height of its neighbour: the union is an L, not a rectangle, and still one fill.
    rect(&mut doc, page, "Seam Right", 280.0, 20.0, 240.0, 70.0, 0.0, green);

    let view = View { origin: (0.0, 0.0), scale: 2.94, width: 1700, height: 600 };
    let vp = Viewport { origin: view.origin, scale: view.scale, width: view.width, height: view.height };
    let scene = doc_scene::build(&doc, page, &vp);
    assert_eq!(scene.items.iter().filter(|item| matches!(item, figcraft_gpu::scene::Item::Mesh(_))).count(), 1, "the L merges into one mesh");
    assert_eq!(scene.items.iter().filter(|item| matches!(item, figcraft_gpu::scene::Item::Shape(_))).count(), 0, "no separate rectangles remain");

    let cpu = cpu_pixels_in(&doc, page, &view);
    let Some(gpu) = gpu_pixels_in(&doc, page, &vp) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    // The shared edge is interior in the tall half: opaque green, matching the CPU.
    for x in 820..=826 {
        let (cpu_px, gpu_px) = (pixel(&cpu, 1700, x, 100).expect("in bounds"), pixel(&gpu, 1700, x, 100).expect("in bounds"));
        for k in 0..4 {
            assert!(cpu_px[k].abs_diff(gpu_px[k]) <= 1, "seam pixel x={x} differs: cpu={cpu_px:?} gpu={gpu_px:?}");
        }
        assert!(gpu_px[3] == 255 && gpu_px[1] > 150, "the seam must be opaque green, not a blend with the background: {gpu_px:?}");
    }
    // The missing lower-right quarter stays empty.
    let (cpu_px, gpu_px) = (pixel(&cpu, 1700, 1176, 353).expect("in bounds"), pixel(&gpu, 1700, 1176, 353).expect("in bounds"));
    assert_eq!(cpu_px[3], 0, "the CPU leaves the L's notch empty: {cpu_px:?}");
    assert_eq!(gpu_px[3], 0, "the mesh must keep the L's notch empty: {gpu_px:?}");

    let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
    assert!(report.passes(Tolerance::EDGE), "{}", report.summary());
}

#[test]
fn gradients_match_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let linear = Gradient {
        kind: GradientKind::Linear,
        stops: vec![
            figcraft_color::ColorStop { position: 0.0, color: Color::new(0.1, 0.1, 0.6, 1.0) },
            figcraft_color::ColorStop { position: 1.0, color: Color::new(0.9, 0.2, 0.1, 1.0) },
        ],
        from: (0.0, 0.0),
        to: (1.0, 0.0),
        width: (0.0, 1.0),
    };
    rect(&mut doc, page, "Linear", 10.0, 10.0, 80.0, 80.0, 0.0, Paint::gradient(linear));

    let radial = Gradient {
        kind: GradientKind::Radial,
        stops: vec![
            figcraft_color::ColorStop { position: 0.0, color: Color::new(1.0, 1.0, 1.0, 1.0) },
            figcraft_color::ColorStop { position: 1.0, color: Color::new(0.0, 0.0, 0.0, 1.0) },
        ],
        from: (0.5, 0.5),
        to: (0.5, 0.0),
        width: (1.0, 0.5),
    };
    rect(&mut doc, page, "Radial", 110.0, 10.0, 80.0, 80.0, 0.0, Paint::gradient(radial));

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
    assert!(report.passes(Tolerance::EDGE), "{}", report.summary());
    let report = compare(&cpu, &gpu, Tolerance { per_channel: 12, max_bad_ratio: 0.02 }).expect("comparable");
    assert!(report.passes(Tolerance { per_channel: 12, max_bad_ratio: 0.02 }), "{}", report.summary());
}

#[test]
fn ellipses_polygons_and_stars_match_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let ellipse = doc
        .create(page, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Ellipse", 10.0, 10.0, 80.0, 60.0)
        .expect("ellipse");
    if let Ok(n) = doc.get_mut(ellipse) {
        n.fills = vec![Paint::solid(Color::new(0.1, 0.6, 0.2, 1.0))];
    }
    let polygon = doc.create(page, NodeKind::Polygon { count: 6 }, "Polygon", 100.0, 10.0, 70.0, 70.0).expect("polygon");
    if let Ok(n) = doc.get_mut(polygon) {
        n.fills = vec![Paint::solid(Color::new(0.7, 0.3, 0.1, 1.0))];
    }
    let star = doc.create(page, NodeKind::Star { count: 5, inner_ratio: 0.4 }, "Star", 20.0, 90.0, 80.0, 80.0).expect("star");
    if let Ok(n) = doc.get_mut(star) {
        n.fills = vec![Paint::solid(Color::new(0.2, 0.2, 0.8, 1.0))];
    }

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
    assert!(report.passes(Tolerance::EDGE), "{}", report.summary());
}

#[test]
fn a_clipping_frame_stops_its_child_on_both_sides() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let frame = doc.create(page, NodeKind::Frame, "Clip", 20.0, 20.0, 60.0, 60.0).expect("frame");
    if let Ok(n) = doc.get_mut(frame) {
        n.clip = true;
    }
    let child = rect(&mut doc, frame, "Wide", 0.0, 25.0, 120.0, 10.0, 0.0, Paint::solid(Color::new(0.9, 0.8, 0.1, 1.0)));
    let _ = child;

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
    assert!(report.passes(Tolerance::EDGE), "{}", report.summary());
    // Right of the clip (x = 90) both sides are transparent.
    assert_eq!(pixel(&cpu, SIZE as usize, 90, 50).expect("in bounds")[3], 0, "cpu clipped");
    assert_eq!(pixel(&gpu, SIZE as usize, 90, 50).expect("in bounds")[3], 0, "gpu clipped");
    // Inside (x = 40) both sides paint.
    assert!(pixel(&cpu, SIZE as usize, 40, 50).expect("in bounds")[3] > 0);
    assert!(pixel(&gpu, SIZE as usize, 40, 50).expect("in bounds")[3] > 0);
}

#[test]
fn a_rounded_clipping_frame_trims_its_child_on_both_sides() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let frame = doc.create(page, NodeKind::Frame, "Rounded clip", 20.0, 20.0, 60.0, 60.0).expect("frame");
    if let Ok(n) = doc.get_mut(frame) {
        n.clip = true;
        n.corners.radii = [20.0; 4];
        n.fills.clear();
    }
    let child = rect(&mut doc, frame, "Wide", 0.0, 0.0, 120.0, 120.0, 0.0, Paint::solid(Color::new(0.9, 0.8, 0.1, 1.0)));
    let _ = child;

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
    let shape = scene.shapes().next().expect("the child shape");
    assert!(shape.clip.has_radii(), "the rounded clip reaches the instance");
    assert!((shape.clip.1[0] - 20.0).abs() < 1e-4, "the radius scales with the view: {}", shape.clip.1[0]);

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
    assert!(report.passes(Tolerance::EDGE), "{}", report.summary());
    // The frame's top-left corner (22, 22) is outside the rounded clip; the center paints.
    assert_eq!(pixel(&cpu, SIZE as usize, 22, 22).expect("in bounds")[3], 0, "cpu clipped the corner");
    assert_eq!(pixel(&gpu, SIZE as usize, 22, 22).expect("in bounds")[3], 0, "gpu clipped the corner");
    assert!(pixel(&cpu, SIZE as usize, 50, 50).expect("in bounds")[3] > 0);
    assert!(pixel(&gpu, SIZE as usize, 50, 50).expect("in bounds")[3] > 0);
}

#[test]
fn a_rounded_clip_trims_a_stroke_mesh() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let frame = doc.create(page, NodeKind::Frame, "Rounded clip", 20.0, 20.0, 60.0, 60.0).expect("frame");
    if let Ok(n) = doc.get_mut(frame) {
        n.clip = true;
        n.corners.radii = [20.0; 4];
        n.fills.clear();
    }
    let ring = rect(&mut doc, frame, "Ring", 0.0, 0.0, 60.0, 60.0, 0.0, Paint::solid(Color::new(0.1, 0.3, 0.7, 0.0)));
    if let Ok(n) = doc.get_mut(ring) {
        n.fills.clear();
        n.strokes = vec![Paint::solid(Color::new(0.1, 0.3, 0.7, 1.0))];
        n.stroke.weight = 20.0;
        n.stroke.join = StrokeJoin::Miter;
    }
    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
    assert_eq!(scene.mesh_count(), 1);

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
    assert!(report.passes(Tolerance::EDGE), "{}", report.summary());
    assert_eq!(pixel(&cpu, SIZE as usize, 22, 22).expect("in bounds")[3], 0, "cpu clipped the stroke corner");
    assert_eq!(pixel(&gpu, SIZE as usize, 22, 22).expect("in bounds")[3], 0, "gpu clipped the stroke corner");
    assert!(pixel(&cpu, SIZE as usize, 30, 50).expect("in bounds")[3] > 0, "the stroke paints along the frame edge");
    assert!(pixel(&gpu, SIZE as usize, 30, 50).expect("in bounds")[3] > 0, "the stroke paints along the frame edge");
}

#[test]
fn strokes_are_tessellated_from_the_cpu_outline_and_match_inside() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let stroked = rect(&mut doc, page, "Stroked", 30.0, 30.0, 110.0, 70.0, 14.0, Paint::solid(Color::new(0.95, 0.95, 0.95, 1.0)));
    if let Ok(n) = doc.get_mut(stroked) {
        n.strokes = vec![Paint::solid(Color::new(0.1, 0.3, 0.7, 1.0))];
        n.stroke.weight = 6.0;
        n.stroke.join = StrokeJoin::Round;
    }
    let dashed = rect(&mut doc, page, "Dashed", 30.0, 120.0, 140.0, 30.0, 6.0, Paint::solid(Color::new(0.98, 0.9, 0.4, 1.0)));
    if let Ok(n) = doc.get_mut(dashed) {
        n.strokes = vec![Paint::solid(Color::new(0.15, 0.15, 0.15, 1.0))];
        n.stroke.weight = 10.0;
        n.stroke.dashes = vec![16.0, 10.0];
    }

    // Both frames must stay on the GPU path: a stroke is a tessellated mesh, not a fallback.
    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
    assert!(scene.mesh_count() >= 2, "each stroke is a mesh");

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    // Four multisamples versus tiny-skia's analytic coverage: the one-pixel edge band is skipped,
    // the rest of the stroke band and its interior must agree exactly.
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, Tolerance::SOLID).expect("comparable");
    assert!(report.passes(Tolerance::SOLID), "{}", report.summary());
}

#[test]
fn a_translucent_stroke_leaf_matches_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let ring = rect(&mut doc, page, "Ring", 30.0, 30.0, 110.0, 70.0, 14.0, Paint::solid(Color::new(0.95, 0.95, 0.95, 0.0)));
    if let Ok(n) = doc.get_mut(ring) {
        n.fills.clear();
        n.strokes = vec![Paint::solid(Color::new(0.1, 0.3, 0.7, 1.0))];
        n.stroke.weight = 10.0;
        n.stroke.join = StrokeJoin::Round;
        n.opacity = 0.5;
    }
    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
    assert_eq!(scene.mesh_count(), 1);

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 2, max_bad_ratio: 0.02 };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());
}

#[test]
fn a_dialogue_free_ring_and_a_star_stroke_stay_gpu_drawable() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let ring =
        doc.create(page, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.45 }, "Ring", 20.0, 20.0, 90.0, 90.0).expect("ring");
    if let Ok(n) = doc.get_mut(ring) {
        n.fills = vec![Paint::solid(Color::new(0.2, 0.5, 0.3, 1.0))];
    }
    let star = doc.create(page, NodeKind::Star { count: 5, inner_ratio: 0.5 }, "Star", 120.0, 20.0, 90.0, 90.0).expect("star");
    if let Ok(n) = doc.get_mut(star) {
        n.strokes = vec![Paint::solid(Color::new(0.8, 0.2, 0.2, 1.0))];
        n.stroke.weight = 5.0;
    }
    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "{:?}", scene.unsupported_features());

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, Tolerance::SOLID).expect("comparable");
    assert!(report.passes(Tolerance::SOLID), "{}", report.summary());
}

#[test]
fn a_drop_shadow_matches_the_cpu_blur() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let card = rect(&mut doc, page, "Card", 40.0, 40.0, 90.0, 60.0, 16.0, Paint::solid(Color::new(0.97, 0.97, 0.97, 1.0)));
    if let Ok(n) = doc.get_mut(card) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::DropShadow(figcraft_doc::Shadow {
                color: Color::new(0.0, 0.0, 0.0, 0.55),
                offset: (6.0, 10.0),
                radius: 12.0,
                spread: 0.0,
                blend: figcraft_color::BlendMode::Normal,
                show_behind: false,
            }),
            visible: true,
        }];
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "the shadow must stay on the GPU path: {:?}", scene.unsupported_features());
    assert_eq!(scene.shadow_count(), 1);

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    // The shadow is a blur, so a one-level difference at the caster's antialiased edge spreads a few
    // pixels; grade the interiors and allow that band to carry a couple of levels.
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, Tolerance { per_channel: 2, max_bad_ratio: 0.02 }).expect("comparable");
    assert!(report.passes(Tolerance { per_channel: 2, max_bad_ratio: 0.02 }), "{}", report.summary());
    // Below the card's bottom edge (inside the offset, blurred shadow) both sides paint a shadow.
    let (cpu_shadow, gpu_shadow) = (pixel(&cpu, SIZE as usize, 90, 112).expect("in bounds"), pixel(&gpu, SIZE as usize, 90, 112).expect("in bounds"));
    assert!(cpu_shadow[3] > 0, "the CPU drew no shadow: {cpu_shadow:?}");
    assert!(gpu_shadow[3] > 0, "the GPU drew no shadow: {gpu_shadow:?}");
}

#[test]
fn an_inner_shadow_matches_the_cpu_blur() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let card = rect(&mut doc, page, "Card", 40.0, 40.0, 100.0, 70.0, 14.0, Paint::solid(Color::new(0.92, 0.92, 0.92, 1.0)));
    if let Ok(n) = doc.get_mut(card) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::InnerShadow(figcraft_doc::Shadow {
                color: Color::new(0.0, 0.0, 0.0, 0.6),
                offset: (4.0, 6.0),
                radius: 10.0,
                spread: 0.0,
                blend: figcraft_color::BlendMode::Normal,
                show_behind: false,
            }),
            visible: true,
        }];
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "the inner shadow must stay on the GPU path: {:?}", scene.unsupported_features());
    assert_eq!(scene.shadow_count(), 1);
    // An inner shadow paints over the fills: the fill item comes first in painter's order.
    assert!(matches!(scene.items.first(), Some(figcraft_gpu::Item::Shape(_))), "{:?}", scene.items.first());
    assert!(matches!(scene.items.get(1), Some(figcraft_gpu::Item::Shadow(_))), "{:?}", scene.items.get(1));

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 2, max_bad_ratio: 0.02 };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());
    // A point just inside the box near its top edge is darkened by the inner shadow; the middle is not.
    let edge = pixel(&cpu, SIZE as usize, 90, 44).expect("in bounds");
    let middle = pixel(&cpu, SIZE as usize, 90, 75).expect("in bounds");
    assert!(
        u32::from(edge[0]) < u32::from(middle[0]) && u32::from(edge[1]) < u32::from(middle[1]),
        "the CPU drew no inner shadow (an opaque fill without one would also pass an alpha check): {edge:?} vs {middle:?}"
    );
    let gpu_edge = pixel(&gpu, SIZE as usize, 90, 44).expect("in bounds");
    assert!(gpu_edge[0].abs_diff(edge[0]) <= 3, "the GPU's inner shadow differs at the edge: {gpu_edge:?} vs {edge:?}");
}

#[test]
fn two_drop_shadows_on_a_rotated_card_match_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let card = rect(&mut doc, page, "Card", 100.0, 50.0, 60.0, 40.0, 10.0, Paint::solid(Color::new(0.95, 0.95, 0.95, 1.0)));
    if let Ok(n) = doc.get_mut(card) {
        n.transform = figcraft_geom::Affine::translate((80.0, 50.0)) * figcraft_geom::Affine::rotate(0.25);
        n.effects = vec![
            figcraft_doc::Effect {
                kind: figcraft_doc::EffectKind::DropShadow(figcraft_doc::Shadow {
                    color: Color::new(0.0, 0.0, 0.0, 0.45),
                    offset: (8.0, 10.0),
                    radius: 9.0,
                    spread: 0.0,
                    blend: figcraft_color::BlendMode::Normal,
                    show_behind: false,
                }),
                visible: true,
            },
            figcraft_doc::Effect {
                kind: figcraft_doc::EffectKind::DropShadow(figcraft_doc::Shadow {
                    color: Color::new(0.8, 0.1, 0.1, 0.3),
                    offset: (-6.0, 4.0),
                    radius: 4.0,
                    spread: 0.0,
                    blend: figcraft_color::BlendMode::Normal,
                    show_behind: false,
                }),
                visible: true,
            },
        ];
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
    assert_eq!(scene.shadow_count(), 2);

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 2, max_bad_ratio: 0.02 };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());
}

#[test]
fn a_single_child_group_opacity_matches_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let group = doc.create(page, figcraft_doc::NodeKind::Group, "Faded", 50.0, 50.0, 100.0, 100.0).expect("group");
    let card = doc.create(group, figcraft_doc::NodeKind::Rectangle, "Card", 0.0, 0.0, 100.0, 100.0).expect("card");
    if let Ok(n) = doc.get_mut(card) {
        n.corners.radii = [12.0; 4];
        n.fills = vec![Paint::solid(Color::new(0.9, 0.35, 0.15, 1.0))];
    }
    if let Ok(n) = doc.get_mut(group) {
        n.opacity = 0.6;
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "the single-leaf group must stay on the GPU path: {:?}", scene.unsupported_features());
    assert_eq!(scene.shapes().count(), 1);
    assert!((scene.shapes().next().expect("shape").opacity - 0.6).abs() < 1e-4);

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 2, max_bad_ratio: 0.02 };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());
}

#[test]
fn an_overlapping_multi_child_group_opacity_matches_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let group = doc.create(page, figcraft_doc::NodeKind::Group, "Stack", 50.0, 50.0, 120.0, 100.0).expect("group");
    let back = doc.create(group, figcraft_doc::NodeKind::Rectangle, "Back", 0.0, 0.0, 80.0, 80.0).expect("back");
    let front = doc.create(group, figcraft_doc::NodeKind::Rectangle, "Front", 40.0, 20.0, 80.0, 80.0).expect("front");
    if let Ok(n) = doc.get_mut(back) {
        n.corners.radii = [8.0; 4];
        n.fills = vec![Paint::solid(Color::new(0.9, 0.35, 0.15, 1.0))];
    }
    if let Ok(n) = doc.get_mut(front) {
        n.corners.radii = [8.0; 4];
        n.fills = vec![Paint::solid(Color::new(0.1, 0.45, 0.85, 0.8))];
    }
    if let Ok(n) = doc.get_mut(group) {
        n.opacity = 0.6;
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "the overlapping group must use the isolated layer: {:?}", scene.unsupported_features());
    assert_eq!(scene.group_count(), 1);
    assert_eq!(scene.shapes().count(), 2);
    // The leaves keep their own alpha; only the finished layer is multiplied by the group's.
    assert!(scene.shapes().all(|s| (s.opacity - 1.0).abs() < 1e-4));

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 2, max_bad_ratio: 0.02 };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());
}

#[test]
fn nested_group_opacity_layers_match_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let outer = doc.create(page, figcraft_doc::NodeKind::Group, "Outer", 40.0, 30.0, 120.0, 120.0).expect("outer");
    let inner = doc.create(outer, figcraft_doc::NodeKind::Group, "Inner", 10.0, 20.0, 90.0, 70.0).expect("inner");
    let back = doc.create(inner, figcraft_doc::NodeKind::Rectangle, "Back", 0.0, 0.0, 70.0, 70.0).expect("back");
    let front = doc.create(inner, figcraft_doc::NodeKind::Rectangle, "Front", 30.0, 15.0, 60.0, 55.0).expect("front");
    let loose = doc.create(outer, figcraft_doc::NodeKind::Rectangle, "Loose", 0.0, 90.0, 120.0, 30.0).expect("loose");
    if let Ok(n) = doc.get_mut(back) {
        n.corners.radii = [10.0; 4];
        n.fills = vec![Paint::solid(Color::new(0.85, 0.4, 0.2, 1.0))];
    }
    if let Ok(n) = doc.get_mut(front) {
        n.corners.radii = [10.0; 4];
        n.fills = vec![Paint::solid(Color::new(0.15, 0.4, 0.8, 0.75))];
    }
    if let Ok(n) = doc.get_mut(loose) {
        n.fills = vec![Paint::solid(Color::new(0.2, 0.7, 0.3, 0.9))];
    }
    if let Ok(n) = doc.get_mut(inner) {
        n.opacity = 0.7;
    }
    if let Ok(n) = doc.get_mut(outer) {
        n.opacity = 0.5;
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
    assert_eq!(scene.group_count(), 2);
    assert_eq!(scene.shapes().count(), 3);

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 2, max_bad_ratio: 0.02 };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());
}

#[test]
fn a_tiled_document_matches_the_untiled_pixels() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let group = doc.create(page, figcraft_doc::NodeKind::Group, "Band", 20.0, 20.0, 160.0, 160.0).expect("group");
    let a = doc.create(group, figcraft_doc::NodeKind::Rectangle, "A", 0.0, 0.0, 120.0, 120.0).expect("a");
    let b = doc.create(group, figcraft_doc::NodeKind::Rectangle, "B", 40.0, 50.0, 120.0, 100.0).expect("b");
    if let Ok(n) = doc.get_mut(a) {
        n.corners.radii = [14.0; 4];
        n.fills = vec![Paint::solid(Color::new(0.85, 0.3, 0.2, 1.0))];
    }
    if let Ok(n) = doc.get_mut(b) {
        n.corners.radii = [14.0; 4];
        n.fills = vec![Paint::solid(Color::new(0.15, 0.5, 0.8, 0.8))];
    }
    if let Ok(n) = doc.get_mut(group) {
        n.opacity = 0.6;
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
    assert!(!scene.has_tile_unsafe_effects());
    let Some(mut renderer) = figcraft_gpu::render::Renderer::new() else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let whole = renderer.render(&scene).expect("the untiled frame");
    let tiled = renderer.render_document_tiled(&doc, page, &viewport(), 64).expect("the tiled frame");
    let tol = Tolerance { per_channel: 2, max_bad_ratio: 0.0 };
    let report = compare(&whole, &tiled, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());
}

#[test]
fn a_large_blurred_frame_is_not_tiled() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let card = rect(&mut doc, page, "Card", 40.0, 40.0, 120.0, 80.0, 10.0, Paint::solid(Color::new(0.3, 0.6, 0.3, 1.0)));
    if let Ok(n) = doc.get_mut(card) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::DropShadow(figcraft_doc::Shadow {
                color: Color::new(0.0, 0.0, 0.0, 0.35),
                offset: (6.0, 8.0),
                radius: 12.0,
                spread: 0.0,
                blend: figcraft_color::BlendMode::Normal,
                show_behind: false,
            }),
            visible: true,
        }];
    }
    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
    assert!(scene.has_tile_unsafe_effects(), "a shadow reads outside its tile");
    let Some(mut renderer) = figcraft_gpu::render::Renderer::new() else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let err = renderer.render_document_tiled(&doc, page, &viewport(), 64).expect_err("tiling must be refused");
    assert!(matches!(err, figcraft_gpu::render::RenderError::Unsupported(ref list) if list.iter().any(|u| u.feature == "tiled-effect")), "{err:?}");
}

#[test]
fn the_feature_zoo_matches_the_cpu_at_three_scales() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);

    let linear = Gradient {
        kind: GradientKind::Linear,
        stops: vec![
            figcraft_color::ColorStop { position: 0.0, color: Color::new(0.08, 0.1, 0.35, 1.0) },
            figcraft_color::ColorStop { position: 1.0, color: Color::new(0.85, 0.35, 0.12, 1.0) },
        ],
        from: (0.0, 0.0),
        to: (1.0, 1.0),
        width: (0.0, 1.0),
    };
    rect(&mut doc, page, "Base", 0.0, 0.0, 400.0, 300.0, 0.0, Paint::gradient(linear));

    // A rounded clip over a child that overflows its frame.
    let clip = doc.create(page, NodeKind::Frame, "Clip", 20.0, 20.0, 90.0, 70.0).expect("clip");
    if let Ok(n) = doc.get_mut(clip) {
        n.clip = true;
        n.corners.radii = [16.0; 4];
        n.fills.clear();
    }
    rect(&mut doc, clip, "Overflow", 0.0, 0.0, 120.0, 100.0, 0.0, Paint::solid(Color::new(0.95, 0.85, 0.2, 1.0)));

    // Two overlapping leaves under one group alpha: the isolated layer path.
    let group = doc.create(page, NodeKind::Group, "Fade", 140.0, 20.0, 95.0, 70.0).expect("group");
    rect(&mut doc, group, "G1", 0.0, 0.0, 70.0, 60.0, 10.0, Paint::solid(Color::new(0.9, 0.3, 0.25, 1.0)));
    rect(&mut doc, group, "G2", 25.0, 15.0, 70.0, 50.0, 10.0, Paint::solid(Color::new(0.15, 0.45, 0.85, 0.8)));
    if let Ok(n) = doc.get_mut(group) {
        n.opacity = 0.65;
    }

    // Drop shadow and inner shadow on solid cards.
    let shadow = rect(&mut doc, page, "Shadow", 255.0, 20.0, 100.0, 60.0, 14.0, Paint::solid(Color::new(0.95, 0.95, 0.92, 1.0)));
    if let Ok(n) = doc.get_mut(shadow) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::DropShadow(figcraft_doc::Shadow {
                color: Color::new(0.0, 0.0, 0.0, 0.4),
                offset: (8.0, 10.0),
                radius: 10.0,
                spread: 0.0,
                blend: figcraft_color::BlendMode::Normal,
                show_behind: false,
            }),
            visible: true,
        }];
    }
    let inner = rect(&mut doc, page, "Inner", 255.0, 95.0, 100.0, 60.0, 14.0, Paint::solid(Color::new(0.2, 0.55, 0.35, 1.0)));
    if let Ok(n) = doc.get_mut(inner) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::InnerShadow(figcraft_doc::Shadow {
                color: Color::new(0.0, 0.0, 0.0, 0.55),
                offset: (4.0, 6.0),
                radius: 10.0,
                spread: 0.0,
                blend: figcraft_color::BlendMode::Normal,
                show_behind: false,
            }),
            visible: true,
        }];
    }

    // A stroked star: path meshes.
    let star = doc.create(page, NodeKind::Star { count: 5, inner_ratio: 0.45 }, "Star", 30.0, 105.0, 80.0, 80.0).expect("star");
    if let Ok(n) = doc.get_mut(star) {
        n.fills = vec![Paint::solid(Color::new(0.2, 0.25, 0.8, 1.0))];
        n.strokes = vec![Paint::solid(Color::new(0.95, 0.4, 0.1, 1.0))];
        n.stroke.weight = 5.0;
    }

    // Background blur over the gradient, then the three grain/texture/blur cards.
    let frosted = rect(&mut doc, page, "Frosted", 140.0, 110.0, 100.0, 70.0, 14.0, Paint::solid(Color::new(1.0, 1.0, 1.0, 0.3)));
    if let Ok(n) = doc.get_mut(frosted) {
        n.effects = vec![figcraft_doc::Effect { kind: figcraft_doc::EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(10.0)), visible: true }];
    }

    let noisy = rect(&mut doc, page, "Noise", 20.0, 215.0, 80.0, 60.0, 10.0, Paint::solid(Color::new(0.25, 0.65, 0.35, 1.0)));
    if let Ok(n) = doc.get_mut(noisy) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::Noise(figcraft_doc::Noise {
                size: 1.2,
                density: 0.55,
                color: Color::new(0.02, 0.05, 0.02, 0.35),
                ..Default::default()
            }),
            visible: true,
        }];
    }
    let textured = rect(&mut doc, page, "Texture", 115.0, 215.0, 75.0, 60.0, 10.0, Paint::solid(Color::new(0.8, 0.35, 0.2, 0.75)));
    if let Ok(n) = doc.get_mut(textured) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::Texture(figcraft_doc::Texture { size: 1.0, radius: 5.0, clip_to_shape: true, ..Default::default() }),
            visible: true,
        }];
    }
    let blurred = rect(&mut doc, page, "Layer blur", 200.0, 215.0, 75.0, 60.0, 10.0, Paint::solid(Color::new(0.95, 0.55, 0.2, 1.0)));
    if let Ok(n) = doc.get_mut(blurred) {
        n.effects = vec![figcraft_doc::Effect { kind: figcraft_doc::EffectKind::LayerBlur(figcraft_doc::Blur::uniform(7.0)), visible: true }];
    }
    let ramped = rect(&mut doc, page, "Ramp", 290.0, 215.0, 75.0, 60.0, 10.0, Paint::solid(Color::new(0.25, 0.35, 0.9, 1.0)));
    if let Ok(n) = doc.get_mut(ramped) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::LayerBlur(figcraft_doc::Blur {
                blur_type: figcraft_doc::BlurType::Progressive,
                radius: 10.0,
                start_radius: 0.0,
                start: (0.5, 0.0),
                end: (0.5, 1.0),
            }),
            visible: true,
        }];
    }

    // A multiply group: its layer blends with everything already painted. It must sit before the
    // mask so the mask run does not include a blended layer.
    let blend_group = doc.create(page, NodeKind::Group, "Multiply", 10.0, 10.0, 100.0, 60.0).expect("blend group");
    rect(&mut doc, blend_group, "Warm", 0.0, 0.0, 60.0, 60.0, 8.0, Paint::solid(Color::new(0.9, 0.35, 0.2, 0.8)));
    rect(&mut doc, blend_group, "Cool", 35.0, 10.0, 60.0, 50.0, 8.0, Paint::solid(Color::new(0.2, 0.5, 0.9, 0.75)));
    if let Ok(n) = doc.get_mut(blend_group) {
        n.blend = BlendMode::Multiply;
        n.opacity = 0.9;
    }

    // A rounded alpha mask, last in z order so its run is only the card right after it.
    let mask = rect(&mut doc, page, "Mask", 275.0, 150.0, 90.0, 45.0, 12.0, Paint::solid(Color::new(1.0, 1.0, 1.0, 0.7)));
    if let Ok(n) = doc.get_mut(mask) {
        n.is_mask = true;
    }
    rect(&mut doc, page, "Masked", 260.0, 142.0, 120.0, 62.0, 0.0, Paint::solid(Color::new(0.2, 0.8, 0.9, 0.8)));

    let scenes: Vec<(f64, Scene)> = [0.5_f64, 1.0, 2.0]
        .into_iter()
        .map(|scale| {
            let view = figcraft_render::View { origin: (0.0, 0.0), scale, width: (400.0 * scale) as u32, height: (300.0 * scale) as u32 };
            let scene = doc_scene::build(&doc, page, &Viewport::from_view(&view));
            assert!(scene.is_supported(), "scale {scale}: {:?}", scene.unsupported_features());
            (scale, scene)
        })
        .collect();
    let Some(mut renderer) = figcraft_gpu::render::Renderer::new() else {
        eprintln!("SKIP: no adapter");
        return;
    };
    for (scale, scene) in scenes {
        let view = figcraft_render::View { origin: (0.0, 0.0), scale, width: (400.0 * scale) as u32, height: (300.0 * scale) as u32 };
        let cpu = figcraft_render::render_view(&doc, page, &view, &Options { background: None, ..Default::default() })
            .expect("the CPU renders the zoo")
            .data()
            .to_vec();
        let gpu = renderer.render(&scene).expect("the GPU renders the zoo");
        // The zoo stacks every tolerance's worst case in one frame; the edge band's 2% covers
        // each feature alone, while three scales plus grain and texture together need 3%.
        let tol = Tolerance { per_channel: 24, max_bad_ratio: 0.03 };
        let report = compare(&cpu, &gpu, tol).expect("comparable buffers");
        assert!(report.passes(tol), "scale {scale}: {}", report.summary());
    }
}

#[test]
fn a_layer_blur_matches_the_cpu_blur() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let card = rect(&mut doc, page, "Blurred", 60.0, 60.0, 90.0, 60.0, 12.0, Paint::solid(Color::new(0.95, 0.5, 0.2, 1.0)));
    if let Ok(n) = doc.get_mut(card) {
        n.effects = vec![figcraft_doc::Effect { kind: figcraft_doc::EffectKind::LayerBlur(figcraft_doc::Blur::uniform(8.0)), visible: true }];
    }
    // A translucent fill under a partially transparent layer: the tint folds both alphas.
    let faded = rect(&mut doc, page, "Faded", 20.0, 140.0, 70.0, 40.0, 8.0, Paint::solid(Color::new(0.2, 0.4, 0.8, 0.8)));
    if let Ok(n) = doc.get_mut(faded) {
        n.opacity = 0.65;
        n.effects = vec![figcraft_doc::Effect { kind: figcraft_doc::EffectKind::LayerBlur(figcraft_doc::Blur::uniform(6.0)), visible: true }];
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "the layer blur must stay on the GPU path: {:?}", scene.unsupported_features());
    assert_eq!(scene.blur_count(), 2);
    assert_eq!(scene.shapes().count(), 0, "the fills are replaced by the blurred layers");

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 3, max_bad_ratio: 0.03 };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());
    // The blur spills above the card: both sides paint there.
    let (cpu_edge, gpu_edge) = (pixel(&cpu, SIZE as usize, 105, 56).expect("in bounds"), pixel(&gpu, SIZE as usize, 105, 56).expect("in bounds"));
    assert!(cpu_edge[3] > 0, "the CPU drew no blurred spill: {cpu_edge:?}");
    assert!(gpu_edge[3] > 0, "the GPU drew no blurred spill: {gpu_edge:?}");
}

#[test]
fn a_progressive_layer_blur_matches_the_cpu_blur() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let card = rect(&mut doc, page, "Ramp", 50.0, 50.0, 100.0, 100.0, 12.0, Paint::solid(Color::new(0.9, 0.4, 0.2, 1.0)));
    if let Ok(n) = doc.get_mut(card) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::LayerBlur(figcraft_doc::Blur {
                radius: 12.0,
                blur_type: figcraft_doc::BlurType::Progressive,
                start_radius: 0.0,
                start: (0.5, 0.0),
                end: (0.5, 1.0),
            }),
            visible: true,
        }];
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "the progressive layer blur must stay on the GPU path: {:?}", scene.unsupported_features());
    assert_eq!(scene.progressive_blur_count(), 1);
    assert_eq!(scene.shapes().count(), 0, "the fill is replaced by the blurred layer");

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 3, max_bad_ratio: 0.03 };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());

    // The ramp runs top (sigma 0) to bottom (sigma 12): the top edge stays sharp, the bottom spills.
    let (cpu_top, gpu_top) = (pixel(&cpu, SIZE as usize, 100, 47).expect("in bounds"), pixel(&gpu, SIZE as usize, 100, 47).expect("in bounds"));
    assert!(cpu_top[3] < 8, "the CPU blurred the sharp end: {cpu_top:?}");
    assert!(gpu_top[3] < 8, "the GPU blurred the sharp end: {gpu_top:?}");
    let (cpu_bottom, gpu_bottom) =
        (pixel(&cpu, SIZE as usize, 100, 153).expect("in bounds"), pixel(&gpu, SIZE as usize, 100, 153).expect("in bounds"));
    assert!(cpu_bottom[3] > 8, "the CPU drew no spill at the blurred end: {cpu_bottom:?}");
    assert!(gpu_bottom[3] > 8, "the GPU drew no spill at the blurred end: {gpu_bottom:?}");
}

#[test]
fn a_background_blur_matches_the_cpu_blur() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    // Two hard colour fields under the card: the blur should melt their seam.
    let _red = rect(&mut doc, page, "Red", 0.0, 0.0, 100.0, 200.0, 0.0, Paint::solid(Color::new(0.9, 0.2, 0.15, 1.0)));
    let _blue = rect(&mut doc, page, "Blue", 100.0, 0.0, 100.0, 200.0, 0.0, Paint::solid(Color::new(0.15, 0.3, 0.9, 1.0)));
    let card = rect(&mut doc, page, "Frosted", 50.0, 50.0, 110.0, 100.0, 16.0, Paint::solid(Color::new(1.0, 1.0, 1.0, 0.35)));
    if let Ok(n) = doc.get_mut(card) {
        n.effects = vec![figcraft_doc::Effect { kind: figcraft_doc::EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(10.0)), visible: true }];
    }
    // A second blurred layer lower down: the frame is resolved, blurred and resumed twice.
    let lower = rect(&mut doc, page, "Lower", 30.0, 150.0, 60.0, 40.0, 8.0, Paint::solid(Color::new(1.0, 1.0, 1.0, 0.3)));
    if let Ok(n) = doc.get_mut(lower) {
        n.effects = vec![figcraft_doc::Effect { kind: figcraft_doc::EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(6.0)), visible: true }];
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "the background blur must stay on the GPU path: {:?}", scene.unsupported_features());
    assert_eq!(scene.backdrop_count(), 2);

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 3, max_bad_ratio: 0.03 };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());
    // The seam is gone under the card, on both sides. Compare with the same card without the blur:
    // a blurless render can also produce a muted pixel, so require the red to fall and the blue
    // to rise across the seam.
    let mut plain = doc.clone();
    if let Ok(n) = plain.get_mut(card) {
        n.effects.clear();
    }
    let baseline = cpu_pixels(&plain, page);
    let base = pixel(&baseline, SIZE as usize, 105, 100).expect("in bounds");
    let (cpu_seam, gpu_seam) = (pixel(&cpu, SIZE as usize, 105, 100).expect("in bounds"), pixel(&gpu, SIZE as usize, 105, 100).expect("in bounds"));
    for (name, px) in [("CPU", cpu_seam), ("GPU", gpu_seam)] {
        assert!(px[0] > base[0], "{name} did not blend red into the seam: {px:?} vs blurless {base:?}");
        assert!(px[2] < base[2], "{name} did not pull blue in: {px:?} vs blurless {base:?}");
    }
}

#[test]
fn mono_noise_matches_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let mut paint = Paint::solid(Color::new(0.15, 0.65, 0.35, 1.0));
    // A fill opacity below one: `solid_color()` folds it into the layer alpha the grain rides on.
    paint.opacity = 0.6;
    let card = rect(&mut doc, page, "Grain", 30.0, 30.0, 140.0, 120.0, 10.0, paint);
    if let Ok(n) = doc.get_mut(card) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::Noise(figcraft_doc::Noise {
                size: 1.2,
                density: 0.55,
                color: Color::new(0.05, 0.02, 0.0, 0.35),
                ..Default::default()
            }),
            visible: true,
        }];
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "mono noise must stay on the GPU path: {:?}", scene.unsupported_features());
    assert_eq!(scene.noise_count(), 1);
    assert_eq!(scene.shapes().count(), 1, "the grain rides on the node's own fill");

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 3, max_bad_ratio: 0.02 };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());

    // Both sides actually drew grains: count interior pixels that moved away from the same card
    // rendered without the noise effect. A straight RGB fill is not the baseline — the pixels are
    // premultiplied and the fill opacity is folded into them.
    let mut plain = doc.clone();
    if let Ok(n) = plain.get_mut(card) {
        n.effects.clear();
    }
    let baseline = cpu_pixels(&plain, page);
    let grain_pixels = |bytes: &[u8]| {
        let mut count = 0usize;
        for y in 40..140 {
            for x in 40..160 {
                let p = pixel(bytes, SIZE as usize, x, y).expect("in bounds");
                let b = pixel(&baseline, SIZE as usize, x, y).expect("in bounds");
                if p.iter().zip(b).any(|(a, b)| (*a).abs_diff(b) > 6) {
                    count += 1;
                }
            }
        }
        count
    };
    let (cpu_grain, gpu_grain) = (grain_pixels(&cpu), grain_pixels(&gpu));
    assert!(cpu_grain > 400, "the CPU drew too few grains: {cpu_grain}");
    assert!(gpu_grain > 400, "the GPU drew too few grains: {gpu_grain}");
}

#[test]
fn a_rough_texture_matches_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let clipped = rect(&mut doc, page, "Clipped texture", 20.0, 20.0, 60.0, 50.0, 6.0, Paint::solid(Color::new(0.85, 0.35, 0.15, 0.7)));
    if let Ok(n) = doc.get_mut(clipped) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::Texture(figcraft_doc::Texture { size: 1.0, radius: 5.0, clip_to_shape: true, ..Default::default() }),
            visible: true,
        }];
    }
    let spilling = rect(&mut doc, page, "Free texture", 110.0, 100.0, 60.0, 50.0, 6.0, Paint::solid(Color::new(0.2, 0.45, 0.8, 1.0)));
    if let Ok(n) = doc.get_mut(spilling) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::Texture(figcraft_doc::Texture { size: 1.0, radius: 6.0, clip_to_shape: false, ..Default::default() }),
            visible: true,
        }];
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "texture must stay on the GPU path: {:?}", scene.unsupported_features());
    assert_eq!(scene.texture_count(), 2);
    assert_eq!(scene.shapes().count(), 0, "the texture replaces each node's fill");

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 24, max_bad_ratio: 0.03 };
    let report = compare(&cpu, &gpu, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());

    // `clip_to_shape` keeps the first card inside its own pixels; without it the second card spills
    // above its top edge. Check both sides agree.
    let outside_clipped = (pixel(&cpu, SIZE as usize, 50, 16).expect("in bounds"), pixel(&gpu, SIZE as usize, 50, 16).expect("in bounds"));
    assert_eq!(outside_clipped.0[3], 0, "the CPU clipped card 1");
    assert_eq!(outside_clipped.1[3], 0, "the GPU clipped card 1");
    // Without `clip_to_shape` the second card spills above its top edge; count those pixels rather
    // than guessing a coordinate a grain may not reach.
    let spill = |bytes: &[u8]| {
        let mut count = 0usize;
        for y in 86..100 {
            for x in 110..170 {
                if pixel(bytes, SIZE as usize, x, y).is_some_and(|p| p[3] > 0) {
                    count += 1;
                }
            }
        }
        count
    };
    assert!(spill(&cpu) > 0, "the CPU did not spill card 2");
    assert!(spill(&gpu) > 0, "the GPU did not spill card 2");
}

#[test]
fn duo_and_multi_noise_match_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);

    let duo = rect(&mut doc, page, "Duo", 15.0, 25.0, 80.0, 120.0, 8.0, Paint::solid(Color::new(0.2, 0.55, 0.3, 1.0)));
    if let Ok(n) = doc.get_mut(duo) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::Noise(figcraft_doc::Noise {
                mode: figcraft_doc::NoiseMode::Duo,
                size: 1.3,
                density: 0.6,
                color: Color::new(0.9, 0.1, 0.05, 0.35),
                secondary: Color::new(0.05, 0.15, 0.9, 0.65),
                ..Default::default()
            }),
            visible: true,
        }];
    }

    let multi = rect(&mut doc, page, "Multi", 105.0, 25.0, 80.0, 120.0, 8.0, Paint::solid(Color::new(0.5, 0.5, 0.5, 1.0)));
    if let Ok(n) = doc.get_mut(multi) {
        n.effects = vec![figcraft_doc::Effect {
            kind: figcraft_doc::EffectKind::Noise(figcraft_doc::Noise {
                mode: figcraft_doc::NoiseMode::Multi,
                size: 1.3,
                density: 0.6,
                opacity: 0.45,
                ..Default::default()
            }),
            visible: true,
        }];
    }

    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(scene.is_supported(), "duo/multi noise must stay on the GPU path: {:?}", scene.unsupported_features());
    assert_eq!(scene.noise_count(), 2);
    assert_eq!(scene.shapes().count(), 2, "each grain rides on its own fill");

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let tol = Tolerance { per_channel: 4, max_bad_ratio: 0.02 };
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, tol).expect("comparable");
    assert!(report.passes(tol), "{}", report.summary());

    // Both cards really show grains, on both sides.
    let changed = |bytes: &[u8], x0: usize| {
        let mut count = 0usize;
        for y in 35..135 {
            for x in x0..x0 + 60 {
                let p = pixel(bytes, SIZE as usize, x, y).expect("in bounds");
                // The fills are known; any large deviation is a grain.
                if p[0].abs_diff(51) > 8 || p[1].abs_diff(140) > 8 || p[2].abs_diff(76) > 8 {
                    if x0 > 100 {
                        // The multi card's fill is grey: count colour, not the fill.
                        if p[0].abs_diff(p[1]) > 6 || p[1].abs_diff(p[2]) > 6 {
                            count += 1;
                        }
                    } else {
                        count += 1;
                    }
                }
            }
        }
        count
    };
    assert!(changed(&cpu, 25) > 100, "the CPU drew too few duo grains");
    assert!(changed(&gpu, 25) > 100, "the GPU drew too few duo grains");
    assert!(changed(&cpu, 115) > 100, "the CPU drew too few multi grains");
    assert!(changed(&gpu, 115) > 100, "the GPU drew too few multi grains");
}

#[test]
fn an_alpha_mask_matches_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let mask = rect(&mut doc, page, "Mask", 30.0, 30.0, 100.0, 100.0, 18.0, Paint::solid(Color::new(1.0, 1.0, 1.0, 0.7)));
    rect(&mut doc, page, "Content", 0.0, 0.0, 200.0, 200.0, 0.0, Paint::solid(Color::new(0.1, 0.5, 0.9, 0.8)));
    if let Ok(n) = doc.get_mut(mask) {
        n.is_mask = true;
    }

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
    assert!(report.passes(Tolerance::EDGE), "{}", report.summary());

    // Inside the mask both sides show the content through 0.7 of the mask layer's alpha.
    let (cpu_mid, gpu_mid) = (pixel(&cpu, SIZE as usize, 80, 80).expect("in bounds"), pixel(&gpu, SIZE as usize, 80, 80).expect("in bounds"));
    for k in 0..4 {
        assert!(cpu_mid[k].abs_diff(gpu_mid[k]) <= 2, "masked interior differs: {cpu_mid:?} vs {gpu_mid:?}");
    }
    // The mask is translucent: content alpha 0.8 times mask alpha 0.7, not a binary clip.
    let expected = (255.0 * 0.8 * 0.7) as u8;
    for (name, px) in [("CPU", cpu_mid), ("GPU", gpu_mid)] {
        assert!(px[3].abs_diff(expected) <= 3, "{name} binarised the mask: {px:?}, expected alpha near {expected}");
    }
    // Outside the mask the content is clipped away on both sides.
    let (cpu_out, gpu_out) = (pixel(&cpu, SIZE as usize, 10, 10).expect("in bounds"), pixel(&gpu, SIZE as usize, 10, 10).expect("in bounds"));
    assert_eq!(cpu_out[3], 0, "the CPU clipped the content outside the mask");
    assert_eq!(gpu_out[3], 0, "the GPU clipped the content outside the mask");
}

#[test]
fn a_luminance_mask_matches_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let mask = rect(&mut doc, page, "Mask", 20.0, 20.0, 160.0, 160.0, 0.0, Paint::solid(Color::new(0.5, 0.5, 0.5, 1.0)));
    rect(&mut doc, page, "Content", 0.0, 0.0, 200.0, 200.0, 0.0, Paint::solid(Color::new(0.9, 0.2, 0.1, 1.0)));
    if let Ok(n) = doc.get_mut(mask) {
        n.is_mask = true;
        n.mask_type = figcraft_doc::MaskType::Luminance;
    }

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
    assert!(report.passes(Tolerance::EDGE), "{}", report.summary());

    // A 0.5 grey mask keeps half the premultiplied content (tiny-skia ceils its mask byte).
    let (cpu_mid, gpu_mid) = (pixel(&cpu, SIZE as usize, 100, 100).expect("in bounds"), pixel(&gpu, SIZE as usize, 100, 100).expect("in bounds"));
    for k in 0..4 {
        assert!(cpu_mid[k].abs_diff(gpu_mid[k]) <= 2, "luminance-masked interior differs: {cpu_mid:?} vs {gpu_mid:?}");
    }
    assert!(cpu_mid[3] < 140 && cpu_mid[3] > 120, "the CPU halved the content alpha: {cpu_mid:?}");
}

#[test]
fn a_tiled_export_reports_progress_and_stops_between_tiles() {
    let Some(mut renderer) = figcraft_gpu::render::Renderer::new() else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    rect(&mut doc, page, "Card", 20.0, 20.0, 160.0, 160.0, 12.0, Paint::solid(Color::new(0.2, 0.5, 0.8, 1.0)));

    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let mut progress = |p: figcraft_gpu::render::ExportProgress| {
        seen.push((p.done, p.total));
        if p.done == 1 {
            cancel.store(true, Ordering::Relaxed);
        }
    };
    let control = ExportControl { cancel: Some(&cancel), progress: Some(&mut progress) };
    let err = renderer.render_document_tiled_with(&doc, page, &viewport(), 64, control).expect_err("the second tile must not start after the cancel");
    assert_eq!(err, RenderError::Cancelled);
    assert_eq!(seen, [(0, 16), (1, 16)], "exactly the zero and the finished tile, with no duplicate initial report");

    // A token already set stops before the first tile, with only the zero report.
    let cancel = AtomicBool::new(true);
    let mut seen = Vec::new();
    let mut progress = |p: figcraft_gpu::render::ExportProgress| seen.push((p.done, p.total));
    let err = renderer
        .render_document_tiled_with(&doc, page, &viewport(), 64, ExportControl { cancel: Some(&cancel), progress: Some(&mut progress) })
        .expect_err("a set token stops at once");
    assert_eq!(err, RenderError::Cancelled);
    assert_eq!(seen, [(0, 16)]);
}

#[test]
fn a_tiled_export_reports_every_tile() {
    let Some(mut renderer) = figcraft_gpu::render::Renderer::new() else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    rect(&mut doc, page, "Card", 20.0, 20.0, 160.0, 160.0, 12.0, Paint::solid(Color::new(0.9, 0.4, 0.2, 1.0)));

    let mut seen = Vec::new();
    let mut progress = |p: figcraft_gpu::render::ExportProgress| seen.push((p.done, p.total));
    let pixels = renderer
        .render_document_tiled_with(&doc, page, &viewport(), 64, ExportControl { cancel: None, progress: Some(&mut progress) })
        .expect("a plain tiled export");
    assert_eq!(pixels.len(), (SIZE as usize) * (SIZE as usize) * 4);
    let done: Vec<u32> = seen.iter().map(|&(done, _)| done).collect();
    assert_eq!(done, (0..=16).collect::<Vec<_>>(), "one report per tile, after the zero");
    assert!(seen.iter().all(|&(_, total)| total == 16));
}

#[test]
fn a_multiply_group_blends_over_the_frame() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    rect(&mut doc, page, "Back", 0.0, 0.0, 200.0, 200.0, 0.0, Paint::solid(Color::new(0.85, 0.9, 0.95, 1.0)));
    let group = doc.create(page, NodeKind::Group, "Blend", 0.0, 0.0, 200.0, 200.0).expect("group");
    rect(&mut doc, group, "Warm", 20.0, 20.0, 110.0, 110.0, 16.0, Paint::solid(Color::new(0.9, 0.35, 0.2, 0.75)));
    rect(&mut doc, group, "Cool", 70.0, 70.0, 110.0, 110.0, 16.0, Paint::solid(Color::new(0.2, 0.45, 0.9, 0.7)));
    if let Ok(n) = doc.get_mut(group) {
        n.blend = BlendMode::Multiply;
        n.opacity = 0.8;
    }

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
    assert!(report.passes(Tolerance::EDGE), "{}", report.summary());
    for (x, y) in [(60, 60), (125, 125), (140, 40), (8, 8)] {
        let (c, g) = (pixel(&cpu, SIZE as usize, x, y).expect("in bounds"), pixel(&gpu, SIZE as usize, x, y).expect("in bounds"));
        for k in 0..4 {
            assert!(c[k].abs_diff(g[k]) <= 5, "blend interior differs at {x},{y}: {c:?} vs {g:?}");
        }
    }
}

#[test]
fn a_multiply_leaf_falls_back() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let card = rect(&mut doc, page, "Card", 10.0, 10.0, 80.0, 80.0, 8.0, Paint::solid(Color::new(0.2, 0.5, 0.8, 1.0)));
    if let Ok(n) = doc.get_mut(card) {
        n.blend = BlendMode::Multiply;
    }
    let scene = doc_scene::build_subtree(&doc, page, &viewport());
    assert!(!scene.is_supported(), "a leaf with its own paint cannot be isolated by this layer path");
    assert!(scene.unsupported_features().iter().any(|u| u.feature == "blend"));
}

#[test]
fn every_non_normal_blend_mode_matches_the_cpu() {
    use BlendMode as B;
    const MODES: [BlendMode; 17] = [
        B::Darken,
        B::Multiply,
        B::PlusDarker,
        B::ColorBurn,
        B::Lighten,
        B::Screen,
        B::PlusLighter,
        B::ColorDodge,
        B::Overlay,
        B::SoftLight,
        B::HardLight,
        B::Difference,
        B::Exclusion,
        B::Hue,
        B::Saturation,
        B::Color,
        B::Luminosity,
    ];
    let Some(mut renderer) = figcraft_gpu::render::Renderer::new() else {
        eprintln!("SKIP: no adapter");
        return;
    };
    let mut failures = Vec::new();
    for mode in MODES {
        let mut doc = Document::default();
        let page = doc.pages[0];
        transparent_page(&mut doc, page);
        rect(&mut doc, page, "Back", 0.0, 0.0, 200.0, 200.0, 0.0, Paint::solid(Color::new(0.85, 0.9, 0.95, 1.0)));
        let group = doc.create(page, NodeKind::Group, "Blend", 0.0, 0.0, 200.0, 200.0).expect("group");
        rect(&mut doc, group, "Warm", 20.0, 20.0, 110.0, 110.0, 16.0, Paint::solid(Color::new(0.9, 0.35, 0.2, 0.75)));
        rect(&mut doc, group, "Cool", 70.0, 70.0, 110.0, 110.0, 16.0, Paint::solid(Color::new(0.2, 0.45, 0.9, 0.7)));
        if let Ok(n) = doc.get_mut(group) {
            n.blend = mode;
            n.opacity = 0.85;
        }
        let cpu = cpu_pixels(&doc, page);
        let scene = doc_scene::build(&doc, page, &viewport());
        assert!(scene.is_supported(), "{mode:?} should build: {:?}", scene.unsupported_features());
        let gpu = renderer.render(&scene).expect("the GPU draws the blend");
        let report = compare(&cpu, &gpu, Tolerance::EDGE).expect("comparable buffers");
        if !report.passes(Tolerance::EDGE) {
            failures.push(format!("{mode:?}: {}", report.summary()));
            continue;
        }
        for (x, y) in [(60, 60), (125, 125), (140, 40), (8, 8)] {
            let (c, g) = (pixel(&cpu, SIZE as usize, x, y).expect("in bounds"), pixel(&gpu, SIZE as usize, x, y).expect("in bounds"));
            if (0..4).any(|k| c[k].abs_diff(g[k]) > 6) {
                failures.push(format!("{mode:?} at {x},{y}: {c:?} vs {g:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "blend modes drifted:\n{}", failures.join("\n"));
}

#[test]
fn text_glyphs_match_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let Some(family) = ["Inter", "Segoe UI", "Arial"].into_iter().find(|f| figcraft_text::find_face(f, "Regular").is_some()) else {
        eprintln!("SKIP: no text font is installed here");
        return;
    };
    let mut data = figcraft_doc::TextData::new("Hi Monet");
    data.style.font_size = 32.0;
    data.style.font_family = family.into();
    let text = doc.create(page, NodeKind::Text(data), "Hello", 10.0, 10.0, 180.0, 50.0).expect("text");
    if let Ok(n) = doc.get_mut(text) {
        n.fills = vec![Paint::solid(Color::new(0.05, 0.05, 0.05, 1.0))];
    }
    let scene = doc_scene::build(&doc, page, &viewport());
    if scene.mesh_count() == 0 {
        eprintln!("SKIP: no Inter font here laid the text out");
        return;
    }
    assert!(scene.is_supported(), "{:?}", scene.unsupported_features());

    let cpu = cpu_pixels(&doc, page);
    let Some(gpu) = gpu_pixels(&doc, page) else {
        eprintln!("SKIP: no adapter");
        return;
    };
    // Glyph stems are a couple of pixels wide, so the edge band covers most of the ink: compare the
    // total coverage (summed alpha) to catch a mis-sized or mis-placed layout, and grade the
    // interiors that survive the edge mask.
    let mass = |bytes: &[u8]| bytes.chunks_exact(4).map(|p| u64::from(p[3])).sum::<u64>();
    let (cpu_mass, gpu_mass) = (mass(&cpu), mass(&gpu));
    assert!(cpu_mass > 0, "the CPU did not draw the text either");
    let relative = (cpu_mass as f64 - gpu_mass as f64).abs() / cpu_mass as f64;
    assert!(relative < 0.03, "text coverage {cpu_mass} vs {gpu_mass} ({:.2}%)", relative * 100.0);
    let report = compare_interiors(&cpu, &gpu, SIZE as usize, SIZE as usize, Tolerance::SOLID).expect("comparable");
    assert!(report.passes(Tolerance::SOLID), "{}", report.summary());
}
