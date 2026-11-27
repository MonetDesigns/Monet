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

use figcraft_color::{Color, Gradient, GradientKind, Paint};
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_gpu::diff::{compare, pixel, Tolerance};
use figcraft_gpu::doc_scene;
use figcraft_gpu::Viewport;
use figcraft_render::{Options, View};

const SIZE: u32 = 200;

fn view() -> View {
    View { origin: (0.0, 0.0), scale: 1.0, width: SIZE, height: SIZE }
}

fn viewport() -> Viewport {
    Viewport { origin: (0.0, 0.0), scale: 1.0, width: SIZE, height: SIZE }
}

fn cpu_pixels(doc: &Document, page: NodeId) -> Vec<u8> {
    let opts = Options { background: None, ..Default::default() };
    let pix = figcraft_render::render_view(doc, page, &view(), &opts).expect("the CPU renders this document");
    pix.data().to_vec()
}

/// The GPU's pixels for `doc`, or `None` when this machine has no adapter.
fn gpu_pixels(doc: &Document, page: NodeId) -> Option<Vec<u8>> {
    let scene = doc_scene::build(doc, page, &viewport());
    assert!(scene.is_supported(), "the test scene must be expressible: {:?}", scene.unsupported_features());
    let mut renderer = figcraft_gpu::render::Renderer::new()?;
    Some(renderer.render(&scene).expect("the GPU draws this scene"))
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
    let id = doc.create(parent, NodeKind::Rectangle, name, x, y, w, h).expect("a rectangle");
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
    let (cpu_mid, gpu_mid) = (
        pixel(&cpu, SIZE as usize, 40, 30).expect("in bounds"),
        pixel(&gpu, SIZE as usize, 40, 30).expect("in bounds"),
    );
    for k in 0..4 {
        assert!(cpu_mid[k].abs_diff(gpu_mid[k]) <= 1, "solid interior differs by more than rounding: {cpu_mid:?} vs {gpu_mid:?}");
    }
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
    let ellipse = doc.create(page, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Ellipse", 10.0, 10.0, 80.0, 60.0).expect("ellipse");
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
fn a_text_layer_is_refused_instead_of_approximated() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let _text = doc.create(page, NodeKind::Text(Default::default()), "Hello", 0.0, 0.0, 80.0, 24.0).expect("text");
    let scene = doc_scene::build(&doc, page, &viewport());
    assert!(!scene.is_supported());
    let Some(mut renderer) = figcraft_gpu::render::Renderer::new() else {
        eprintln!("SKIP: no adapter");
        return;
    };
    match renderer.render(&scene) {
        Err(figcraft_gpu::render::RenderError::Unsupported(items)) => {
            assert!(items.iter().any(|u| u.feature == "text"), "{items:?}");
        }
        other => panic!("the GPU must refuse a text frame, got {other:?}"),
    }
}
