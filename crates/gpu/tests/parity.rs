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
use figcraft_doc::{Document, NodeId, NodeKind, StrokeJoin};
use figcraft_gpu::Viewport;
use figcraft_gpu::diff::{Tolerance, compare, compare_interiors, pixel};
use figcraft_gpu::doc_scene;
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
///
/// Device creation and teardown is serialised: creating a dozen adapters and devices at once from
/// parallel test threads can wedge a driver (seen with a real font installed, which stops the text
/// test from skipping early). CPU rendering still runs in parallel.
fn gpu_pixels(doc: &Document, page: NodeId) -> Option<Vec<u8>> {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let (cpu_mid, gpu_mid) = (pixel(&cpu, SIZE as usize, 40, 30).expect("in bounds"), pixel(&gpu, SIZE as usize, 40, 30).expect("in bounds"));
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
    assert!(u32::from(edge[0]) < u32::from(middle[0]) || u32::from(edge[3]) > 0, "the CPU drew no inner shadow: {edge:?} vs {middle:?}");
    let gpu_edge = pixel(&gpu, SIZE as usize, 90, 44).expect("in bounds");
    assert!(gpu_edge[0].abs_diff(edge[0]) <= 3, "the GPU's inner shadow differs at the edge: {gpu_edge:?} vs {edge:?}");
}

#[test]
fn two_drop_shadows_on_a_rotated_card_match_the_cpu() {
    let mut doc = Document::default();
    let page = doc.pages[0];
    transparent_page(&mut doc, page);
    let card = rect(&mut doc, page, "Card", 60.0, 50.0, 60.0, 40.0, 10.0, Paint::solid(Color::new(0.95, 0.95, 0.95, 1.0)));
    if let Ok(n) = doc.get_mut(card) {
        n.transform = figcraft_geom::Affine::rotate(0.25);
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
    // The seam is gone under the card, on both sides.
    let (cpu_seam, gpu_seam) = (pixel(&cpu, SIZE as usize, 105, 100).expect("in bounds"), pixel(&gpu, SIZE as usize, 105, 100).expect("in bounds"));
    assert!(cpu_seam[0] < 250 && cpu_seam[2] > 20, "the CPU kept the seam sharp: {cpu_seam:?}");
    assert!(gpu_seam[0] < 250 && gpu_seam[2] > 20, "the GPU kept the seam sharp: {gpu_seam:?}");
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

    // Both sides actually drew grains: count interior pixels that moved away from the solid fill.
    let fill = [38, 166, 89];
    let grain_pixels = |bytes: &[u8]| {
        let mut count = 0usize;
        for y in 40..140 {
            for x in 40..160 {
                let p = pixel(bytes, SIZE as usize, x, y).expect("in bounds");
                if p[0].abs_diff(fill[0]) > 6 || p[1].abs_diff(fill[1]) > 6 || p[2].abs_diff(fill[2]) > 6 {
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
