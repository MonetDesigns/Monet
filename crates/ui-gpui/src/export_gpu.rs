//! GPU shading for raster exports.
//!
//! The canvas draws external `.wgsl` materials through [`figcraft_render::shade`] +
//! [`crate::shade_gpu`], and its full-frame path goes through `figcraft-gpu` (see
//! [`crate::canvas::render_worker`]). Export follows the same two routes: animated-export frames
//! go through [`figcraft_gpu::doc_scene`] + [`figcraft_gpu::render`], and an area with external
//! materials renders on the CPU pass while [`crate::shade_gpu`] resolves those materials.
//!
//! The hook runs on the UI thread, but only while an export is being written. A material is one
//! layer-sized pass and readback; exports are already a synchronous command, and this keeps the
//! document/selection snapshot exactly the one being exported.

#![cfg(feature = "gpu")]

use std::sync::Arc;

use figcraft_doc::{Document, NodeId};
use figcraft_geom::Rect;
use figcraft_gpu::Viewport;
use figcraft_gpu::doc_scene;
use figcraft_render::Pixmap;
use figcraft_render::shade::ShadeBoard;

use crate::shade_gpu;
use crate::wgsl_runner::WgslRunner;

/// Install the export hooks once, from [`crate::app::CanvasApp::new`].
pub fn install() {
    figcraft_render::set_export_material_hook(Arc::new(render_export));
    figcraft_render::set_export_frame_hook(Arc::new(render_frame_gpu), Arc::new(gpu_frame_supported));
}

/// Whether the GPU scene renderer can draw this frame faithfully (structure only, no pixels).
fn gpu_frame_supported(doc: &Document, ids: &[NodeId], view: &figcraft_render::View) -> bool {
    if !figcraft_gpu::prefs::effective().is_gpu() {
        return false;
    }
    match ids {
        [id] => doc_scene::build_subtree(doc, *id, &Viewport::from_view(view)).is_supported(),
        _ => false,
    }
}

/// One animated-export frame on the GPU. `None` means "not expressible", "the user chose the CPU"
/// or "no adapter", and the caller falls back to the CPU raster.
fn render_frame_gpu(doc: &Document, ids: &[NodeId], view: &figcraft_render::View) -> Option<Pixmap> {
    let scene = doc_scene::build_subtree(doc, *ids.first()?, &Viewport::from_view(view));
    let data = figcraft_gpu::render::render_if_gpu(&scene)?;
    figcraft_render::pixmap_from_premultiplied_rgba(data, view.width, view.height)
}

/// Render `ids` over `area` at `scale`, with every external material resolved at that scale.
/// `None` means "leave it to the CPU renderer": no material in this subtree, no GPU adapter, or a
/// shader that could not run. Falling back is important — the CPU pass still draws the node's fill,
/// which is the previous export result rather than a transparent hole.
fn render_export(doc: &Document, ids: &[NodeId], area: Rect, scale: f64) -> Option<Pixmap> {
    let page = ids.iter().find_map(|id| doc.page_of(*id).or_else(|| doc.pages.contains(id).then_some(*id)))?;
    let specs = shade_gpu::specs(doc, page);
    if specs.is_empty() {
        return None;
    }

    let board = ShadeBoard::new(specs);
    let mut runner = WgslRunner::new()?;
    let mut last = None;
    // First pass misses (records requests), the pump shades them, the second pass hits. Three
    // rounds leave one retry for a layer whose backdrop changed between passes.
    for _ in 0..3 {
        let pix = figcraft_render::render_area_shaded(doc, ids, area, scale, &board)?;
        let pending = board.pending();
        last = Some(pix);
        if pending == 0 {
            return last;
        }
        let shaded = shade_gpu::shade_pending(&board, &mut runner, doc);
        if shaded == 0 {
            return None;
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::Document;
    use serde_json::json;

    /// The animated-export hook: a solid-shape frame is accepted and rendered through the GPU
    /// (premultiplied pixels wrapped as a `Pixmap`), with the CPU path staying the fallback.
    #[test]
    fn an_export_frame_goes_through_the_gpu_hook() {
        if WgslRunner::new().is_none() {
            eprintln!("skip: no GPU adapter");
            return;
        }
        install();
        let mut doc = Document::default();
        let page = doc.pages[0];
        let frame = doc.create(page, figcraft_doc::NodeKind::Frame, "Frame", 10.0, 10.0, 100.0, 100.0).expect("a frame");
        let rect = doc.create(frame, figcraft_doc::NodeKind::Rectangle, "Rect", 0.0, 0.0, 40.0, 40.0).expect("a rectangle");
        if let Ok(n) = doc.get_mut(rect) {
            n.fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)];
        }
        let view = figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 200 };
        assert!(figcraft_render::export_frame_gpu_supported(&doc, &[frame], &view), "the scene subset accepts this frame");
        let pix = figcraft_render::render_frame_for_export(&doc, &[frame], &view).expect("a rendered frame");
        assert_eq!((pix.width(), pix.height()), (200, 200));
        let alpha = |x: usize, y: usize| pix.data()[(y * 200 + x) * 4 + 3];
        assert!(alpha(50, 50) > 0, "inside the frame paints");
        assert_eq!(alpha(190, 190), 0, "outside the frame stays transparent");
    }

    /// The animated-export hook now uses `figcraft-gpu`, so a frame the old solid-rect scene
    /// rejected (an ellipse) still renders on the GPU and only the CPU fallback is left for frames
    /// the new scene marks unsupported.
    #[test]
    fn an_ellipse_export_frame_goes_through_figcraft_gpu() {
        if !figcraft_gpu::prefs::effective().is_gpu() {
            eprintln!("skip: no GPU adapter");
            return;
        }
        install();
        let mut doc = Document::default();
        let page = doc.pages[0];
        let frame = doc.create(page, figcraft_doc::NodeKind::Frame, "Frame", 10.0, 10.0, 100.0, 100.0).expect("a frame");
        if let Ok(n) = doc.get_mut(frame) {
            n.fills.clear();
        }
        let ellipse = doc
            .create(
                frame,
                figcraft_doc::NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 },
                "Ellipse",
                20.0,
                20.0,
                60.0,
                40.0,
            )
            .expect("an ellipse");
        if let Ok(n) = doc.get_mut(ellipse) {
            n.fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::new(0.2, 0.4, 0.8, 1.0))];
        }
        let view = figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 200 };
        let scene = doc_scene::build_subtree(&doc, frame, &Viewport::from_view(&view));
        assert_eq!(scene.shapes().count(), 1, "the ellipse is one SDF instance");
        assert!(figcraft_gpu::render::render_if_gpu(&scene).is_some(), "the new pipeline renders it");
        assert!(figcraft_render::export_frame_gpu_supported(&doc, &[frame], &view), "the new scene accepts an ellipse");
        let pix = figcraft_render::render_frame_for_export(&doc, &[frame], &view).expect("a rendered frame");
        let alpha = |x: usize, y: usize| pix.data()[(y * 200 + x) * 4 + 3];
        assert!(alpha(50, 50) > 0, "inside the ellipse paints");
        assert_eq!(alpha(20, 20), 0, "outside the ellipse stays transparent");

        let mut cpu = figcraft_render::Pixmap::new(view.width, view.height).expect("cpu pixmap");
        figcraft_render::render_nodes_into(&doc, &[frame], &view, &mut cpu);
        let tol = figcraft_gpu::diff::Tolerance { per_channel: 2, max_bad_ratio: 0.02 };
        let report = figcraft_gpu::diff::compare_interiors(cpu.data(), pix.data(), 200, 200, tol).expect("same size");
        assert!(report.passes(tol), "{}", report.summary());
    }

    #[test]
    fn external_wgsl_material_is_in_a_raster_export() {
        // A real adapter is required; the same test binary also runs in CI containers without one.
        if WgslRunner::new().is_none() {
            eprintln!("skip: no GPU adapter");
            return;
        }
        install();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let bytes = std::fs::read(root.join("examples/glass-and-vectors.figcraft")).expect("example document");
        let mut loaded = figcraft_format::load_bytes_with_versions(&bytes).expect("decode");
        let doc: &mut Document = &mut loaded.doc;
        let page = doc.pages.first().copied().expect("page");
        let frame = doc.find_by_name("Glass weather", Some(page)).into_iter().next().expect("glass frame");
        let card = doc.find_by_name("Glass card", Some(page)).into_iter().next().expect("glass card");
        let shader = root.join("assets/wgsl/glass.wgsl").to_string_lossy().to_string();
        let binding = json!({
            "path": shader,
            "params": {
                "u.depth": 100.0, "u.dispersion": 1.0, "u.frost": 0.0,
                "u.light_angle": 145.0, "u.light_int": 1.0, "u.refraction": 1.0, "u.splay": 1.0
            }
        })
        .to_string();
        doc.node_mut(card).expect("card node").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding);

        let area = figcraft_render::export_bounds(doc, frame).expect("frame bounds");
        let cpu = figcraft_render::render_area(doc, &[frame], area, 1.0).expect("cpu render");
        let gpu = figcraft_render::render_area_for_export(doc, &[frame], area, 1.0).expect("gpu render");
        let b = doc.node(card).and_then(|n| doc.world_bounds(n.id)).expect("card bounds");
        let x = ((b.x0 + b.x1) / 2.0 - area.x0) as usize;
        let y = ((b.y0 + b.y1) / 2.0 - area.y0) as usize;
        let at = |p: &Pixmap| {
            let i = (y * p.width() as usize + x) * 4;
            [p.data()[i], p.data()[i + 1], p.data()[i + 2], p.data()[i + 3]]
        };
        let (c, g) = (at(&cpu), at(&gpu));
        assert_ne!(c, g, "the export must not fall back to the flat fill: cpu={c:?} gpu={g:?}");
        let diff: u32 = (0..3).map(|i| c[i].abs_diff(g[i]) as u32).sum();
        assert!(diff > 24, "the material must change the exported pixel: cpu={c:?} gpu={g:?}");
    }
}
