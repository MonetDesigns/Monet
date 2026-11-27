//! `Document` → [`Scene`]: the only place document semantics are read.
//!
//! The front end accepts a strict subset and **records** everything it cannot draw, so the caller can
//! send the whole frame to the CPU. Nothing is approximated silently:
//!
//! | accepted | recorded as unsupported |
//! |---|---|
//! | page/frame/group/section/component/instance containers and their fills | icons and slices |
//! | rectangles (four corner radii, smoothing), full ellipses, arcs and rings, polygons, stars | gradient or image paint on a path/stroke |
//! | vector networks, booleans, lines — filled and stroked through the CPU's own outlines | images, patterns |
//! | strokes: weight, align, joins, caps, dashes, per-side weights (via `figcraft-pathops`) | group opacity, other blend modes, masks |
//! | solid, linear/radial/angular/diamond gradients (≤ [`MAX_STOPS`] stops) on SDF shapes | round or noisy effects this build cannot draw |
//! | drop shadows from an opaque SDF box: offset, blur (box-approximated exactly as the CPU), tint | drop shadows that spread, blend differently, or cast from groups, text or translucent/outlined layers |
//! | inner shadows from an SDF box: offset, blur, tint, masked to the box | inner shadows that spread or blend differently, and group inner shadows |
//! | uniform or progressive layer blur over one opaque SDF box fill | layers with children, strokes, clips or other effects |
//! | uniform background blur over one SDF box with one fill (the frame is resolved and blurred in place) | background blur over children, strokes, clips, group opacity, and glass (out of scope) |
//! | text: glyph and decoration outlines from `figcraft_text`, solid fills and strokes per run | colour glyphs (emoji, COLR) |
//! | mono/duo/multi grains over one SDF box fill, hashed exactly as the CPU | grains over children, strokes, clips, groups or other effects |
//! | texture over one SDF box fill: whole-pixel scatter hashed exactly as the CPU | textures combined with children, strokes, clips, or other effects |
//! | node opacity, visibility, ancestor clipping rectangles | rounded or rotated clips, variable-bound fills |

use std::collections::BTreeMap;
use std::sync::Arc;

use figcraft_color::{BlendMode, GradientKind as DocGradientKind, Paint as DocPaint, PaintKind};
use figcraft_doc::{Document, EffectKind, Node, NodeId, NodeKind, NoiseMode, StrokeAlign};
use figcraft_geom::{Affine, BezPath, Point};

use crate::geometry;
use crate::scene::{
    BackdropNode, Clip, GradientKind, GrainMode, LayerBlurNode, MAX_STOPS, MeshNode, NoiseNode, Paint, ProgressiveBlurNode, Rgba, Scene, ShadowNode,
    ShapeKind, ShapeNode, Stop, TextureNode, Viewport, is_similarity,
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
    let scale = {
        let c = view_xf.as_coeffs();
        (c[0] * c[0] + c[1] * c[1]).sqrt()
    };
    for &child in &page_node.children {
        walk(doc, child, view_xf, scale, Clip::NONE, &mut scene, 0);
    }
    scene
}

/// Build one subtree (not the page), transparent where nothing paints — animated export frames.
pub fn build_subtree(doc: &Document, id: NodeId, view: &Viewport) -> Scene {
    let mut scene = Scene::new(view.width, view.height);
    let view_xf = view.affine();
    let scale = {
        let c = view_xf.as_coeffs();
        (c[0] * c[0] + c[1] * c[1]).sqrt()
    };
    walk(doc, id, view_xf, scale, Clip::NONE, &mut scene, 0);
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

fn walk(doc: &Document, id: NodeId, view_xf: Affine, scale: f64, clip: Clip, scene: &mut Scene, depth: usize) {
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
    let drop_shadows: Vec<&figcraft_doc::Shadow> =
        n.effects.iter().filter(|e| e.visible).filter_map(|e| if let EffectKind::DropShadow(s) = &e.kind { Some(s) } else { None }).collect();
    let inner_shadows: Vec<&figcraft_doc::Shadow> =
        n.effects.iter().filter(|e| e.visible).filter_map(|e| if let EffectKind::InnerShadow(s) = &e.kind { Some(s) } else { None }).collect();
    let layer_blurs: Vec<&figcraft_doc::Blur> =
        n.effects.iter().filter(|e| e.visible).filter_map(|e| if let EffectKind::LayerBlur(b) = &e.kind { Some(b) } else { None }).collect();
    let backdrops: Vec<&figcraft_doc::Blur> = n
        .effects
        .iter()
        .filter(|e| e.visible)
        .filter_map(|e| if let EffectKind::BackgroundBlur(b) = &e.kind { Some(b) } else { None })
        .filter(|b| b.max_radius() > 0.0)
        .collect();
    let noises: Vec<&figcraft_doc::Noise> =
        n.effects.iter().filter(|e| e.visible).filter_map(|e| if let EffectKind::Noise(nz) = &e.kind { Some(nz) } else { None }).collect();
    let textures: Vec<&figcraft_doc::Texture> =
        n.effects.iter().filter(|e| e.visible).filter_map(|e| if let EffectKind::Texture(t) = &e.kind { Some(t) } else { None }).collect();
    if n.effects.iter().any(|e| {
        e.visible
            && !matches!(
                e.kind,
                EffectKind::DropShadow(_)
                    | EffectKind::InnerShadow(_)
                    | EffectKind::LayerBlur(_)
                    | EffectKind::BackgroundBlur(_)
                    | EffectKind::Noise(_)
                    | EffectKind::Texture(_)
            )
    }) {
        scene.unsupported("effect", format!("`{}` has a visible effect this build cannot draw", n.name));
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
    if n.bound.keys().any(|k| k.starts_with("fills")) {
        scene.unsupported("variable-fill", format!("`{}` has a fill bound to a variable", n.name));
    }
    match &n.kind {
        NodeKind::Icon { .. } | NodeKind::Slice => scene.unsupported("icon", format!("`{}` is an icon or slice", n.name)),
        _ => {}
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
            inner = inner.intersect([x0.min(x1) as f32, y0.min(y1) as f32, (x1 - x0).abs() as f32, (y1 - y0).abs() as f32]);
        }
    }

    // Drop shadows sit under the node's own content, so they are emitted before its fills and
    // children. Only a node whose silhouette is exactly its SDF box can cast one in GPU.
    let shape = if sized { sdf_shape(n, &m, w, h) } else { None };
    backdrop(n, &backdrops, &layer_blurs, shape, clip, m, w, h, scale, scene);
    let blurred = layer_blur(n, &layer_blurs, shape, clip, m, w, h, scale, scene.width, scene.height, scene);
    if !drop_shadows.is_empty() {
        match shape {
            Some(shape) if can_cast_shadow(n, &drop_shadows) => {
                for shadow in &drop_shadows {
                    emit_shadow(shape, m, w, h, scale, clip, shadow, false, scene);
                }
            }
            _ => scene.unsupported("effect", format!("`{}` casts a drop shadow this build cannot draw", n.name)),
        }
    }

    // A texture replaces the layer's pixels with whole-pixel-scattered copies of the fill. Only a
    // one-fill SDF box is accepted, so its layer content is exactly that fill; the normal fill is
    // skipped when the texture is active, exactly like a blurred layer.
    let texture_ok = !textures.is_empty() && shape.is_some() && can_texture(n, &textures, clip);
    let texture_node = match (texture_ok, shape) {
        (true, Some(shape)) => make_texture(id, n, &textures[0], shape, m, w, h, clip, scale),
        _ => None,
    };
    let textured = texture_node.is_some();

    // Fills: the SDF when the box is one of its primitives, the CPU's own outline tessellated
    // otherwise (vectors, booleans, lines, and boxes the SDF cannot express exactly). Text paints
    // glyph outlines instead of its box. A blurred layer already paints the fill, so it replaces it.
    if sized && !blurred && !textured {
        if let NodeKind::Text(_) = n.kind {
            emit_text(n, m, clip, scene);
        } else if let Some(shape) = shape {
            emit_sdf_fills(n, shape, m, w, h, clip, scene);
        } else if n.fills.iter().any(|f| f.visible) {
            emit_path_fills(doc, n, m, clip, scene);
        }
    }

    // A mono grain layer rides on the node's own fill, after it and before shadows/children/strokes
    // (the CPU's layer noise sees the layer's content). It is only accepted on a one-fill SDF box.
    if !noises.is_empty() {
        match shape {
            Some(shape) if can_noise(n, &noises, clip) => {
                emit_noise(id, n, &noises[0], shape, m, w, h, clip, scene);
            }
            _ => scene.unsupported("effect", format!("`{}` carries a noise effect this build cannot draw", n.name)),
        }
    }

    if let Some(node) = texture_node {
        scene.push_texture(node);
    } else if !textures.is_empty() && !texture_ok {
        scene.unsupported("effect", format!("`{}` carries a texture effect this build cannot draw", n.name));
    }

    // Inner shadows paint over the fills, under the children and the stroke, like the CPU.
    if !inner_shadows.is_empty() {
        match shape {
            Some(shape) if can_cast_inner_shadow(n, &inner_shadows) => {
                for shadow in &inner_shadows {
                    emit_shadow(shape, m, w, h, scale, clip, shadow, true, scene);
                }
            }
            _ => scene.unsupported("effect", format!("`{}` casts an inner shadow this build cannot draw", n.name)),
        }
    }

    // Boolean children are operands, not layers: the CPU draws the folded result and skips them.
    if !matches!(n.kind, NodeKind::BooleanOp { .. }) {
        for &child in &n.children {
            walk(doc, child, view_xf, scale, inner, scene, depth + 1);
        }
    }

    // Strokes paint over the node's children, like the CPU. Text strokes follow its glyphs and are
    // part of `emit_text`.
    if n.has_visible_stroke() && !matches!(n.kind, NodeKind::Text(_)) {
        emit_stroke(doc, n, m, clip, scene);
    }
}

/// The SDF primitive for a node's box, or `None` when the box needs the tessellated path instead.
///
/// Purely a capability question: nothing is recorded here, so a box the SDF declines is simply
/// drawn through the (exact) path fallback.
fn sdf_shape(n: &Node, m: &Affine, w: f64, h: f64) -> Option<ShapeKind> {
    if n.corners.smoothing > 0.01 {
        return None;
    }
    if n.corners.radii.iter().any(|r| *r > 0.0) && !is_similarity(crate::scene::matrix(*m)) {
        // A round corner under a non-uniform scale is an ellipse; the path fallback has the true
        // outline, the SDF would draw a circle in local space.
        return None;
    }
    match &n.kind {
        NodeKind::Rectangle
        | NodeKind::Frame
        | NodeKind::Group
        | NodeKind::Section { .. }
        | NodeKind::Component { .. }
        | NodeKind::ComponentSet { .. }
        | NodeKind::Instance { .. }
        | NodeKind::Page { .. } => {
            let limit = (w.min(h) * 0.5) as f32;
            let radii = n.corners.radii.map(|v| (v as f32).clamp(0.0, limit));
            Some(ShapeKind::RoundedRect { radii })
        }
        NodeKind::Ellipse { start, sweep, inner } => {
            let full = (*start).abs() < 1e-6 && (*sweep - std::f64::consts::TAU).abs() < 1e-6 && inner.abs() < 1e-6;
            full.then_some(ShapeKind::Ellipse)
        }
        NodeKind::Polygon { count } => (3..=32).contains(count).then_some(ShapeKind::Polygon { count: *count }),
        NodeKind::Star { count, inner_ratio } => {
            (3..=32).contains(count).then(|| ShapeKind::Star { count: *count, inner_ratio: *inner_ratio as f32 })
        }
        NodeKind::Line | NodeKind::Vector { .. } | NodeKind::Text(_) | NodeKind::BooleanOp { .. } | NodeKind::Icon { .. } | NodeKind::Slice => None,
    }
}

/// The SDF path: one instance per visible fill, gradients included.
fn emit_sdf_fills(n: &Node, shape: ShapeKind, m: Affine, w: f64, h: f64, inner: Clip, scene: &mut Scene) {
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

/// The shadow properties both kinds can express here: normal blend, no spread, finite radius and
/// offset.
fn shadow_is_plain(s: &figcraft_doc::Shadow) -> bool {
    s.blend == BlendMode::Normal && s.spread == 0.0 && s.radius.is_finite() && s.radius >= 0.0 && s.offset.0.is_finite() && s.offset.1.is_finite()
}

/// Whether this node casts its drop shadows from exactly its SDF box: an opaque solid fill or fills,
/// no visible stroke, no children (or a clipping frame whose own fill is the whole layer), full node
/// opacity, and plain shadows.
fn can_cast_shadow(n: &Node, shadows: &[&figcraft_doc::Shadow]) -> bool {
    if n.opacity < 0.999 || n.has_visible_stroke() {
        return false;
    }
    if !n.children.is_empty() && !(n.clip && n.fills.iter().any(|f| f.visible)) {
        return false;
    }
    let mut painted = false;
    for fill in n.fills.iter().filter(|f| f.visible) {
        painted = true;
        if !matches!(fill.blend, BlendMode::Normal) || fill.opacity < 0.999 || fill.solid_color().is_none_or(|c| c.a < 0.999) {
            return false;
        }
    }
    painted && shadows.iter().all(|s| shadow_is_plain(s))
}

/// Whether this node casts its inner shadows from its own SDF box. Unlike a drop shadow the CPU casts
/// an inner shadow from the node's outline, not its pixels, so fills, children and strokes do not get
/// in the way; only the node's own opacity would need a layer, and the shape must be the SDF box.
fn can_cast_inner_shadow(n: &Node, shadows: &[&figcraft_doc::Shadow]) -> bool {
    n.opacity >= 0.999 && shadows.iter().all(|s| shadow_is_plain(s))
}

/// One shadow: the node's box offset by the shadow's device-pixel offset, a sigma the CPU's blur
/// uses (`radius / 2 * scale`) and the tint. An inner shadow is masked to the box's own coverage.
#[allow(clippy::too_many_arguments)]
fn emit_shadow(shape: ShapeKind, m: Affine, w: f64, h: f64, scale: f64, clip: Clip, shadow: &figcraft_doc::Shadow, inner: bool, scene: &mut Scene) {
    let offset = [(shadow.offset.0 * scale).round() as f32, (shadow.offset.1 * scale).round() as f32];
    scene.push_shadow(ShadowNode {
        mat: crate::scene::matrix(m),
        size: [w as f32, h as f32],
        shape,
        offset,
        sigma: (shadow.radius / 2.0 * scale) as f32,
        color: Rgba::from_color(shadow.color),
        clip,
        inner,
    });
}

/// Emits this node's layer blur when it is one this build can draw; returns whether the node's fill
/// has been replaced by the blurred layer. Records `effect` for everything else.
#[allow(clippy::too_many_arguments)]
fn layer_blur(
    n: &Node,
    blurs: &[&figcraft_doc::Blur],
    shape: Option<ShapeKind>,
    clip: Clip,
    m: Affine,
    w: f64,
    h: f64,
    scale: f64,
    frame_width: u32,
    frame_height: u32,
    scene: &mut Scene,
) -> bool {
    if blurs.is_empty() {
        return false;
    }
    let Some(shape) = shape.filter(|_| can_blur_layer(n, blurs, clip, m, w, h, scale, frame_width, frame_height)) else {
        scene.unsupported("effect", format!("`{}` carries a layer blur this build cannot draw", n.name));
        return false;
    };
    emit_layer_blur(shape, m, w, h, scale, blurs[0], n, clip, scene);
    true
}

/// Whether this node's layer is exactly one opaque SDF box fill, so blurring the box's coverage and
/// repainting it is what the CPU's layer blur does. Children, strokes, its own or an ancestor's clip,
/// other effects and boxes close enough to the frame edge that the blur would read past it are all
/// declined (the CPU's layer can carry content outside the frame; this build cannot).
#[allow(clippy::too_many_arguments)]
fn can_blur_layer(
    n: &Node,
    blurs: &[&figcraft_doc::Blur],
    clip: Clip,
    m: Affine,
    w: f64,
    h: f64,
    scale: f64,
    frame_width: u32,
    frame_height: u32,
) -> bool {
    let [_blur] = blurs else {
        return false;
    };
    if !n.children.is_empty() || n.has_visible_stroke() || n.clip || !clip.is_none() {
        return false;
    }
    if n.effects.iter().any(|e| e.visible && matches!(e.kind, EffectKind::DropShadow(_) | EffectKind::InnerShadow(_))) {
        return false;
    }
    let mut fills = 0;
    for fill in n.fills.iter().filter(|f| f.visible) {
        fills += 1;
        if !matches!(fill.blend, BlendMode::Normal) || fill.solid_color().is_none() {
            return false;
        }
    }
    if fills != 1 {
        return false;
    }
    // The CPU blurs a layer that may extend past the frame; this pipeline only has the frame, so a
    // box whose blur reaches outside it would read different pixels at the border.
    let reach = blurs[0].max_radius() * 1.5 * scale;
    if !reach.is_finite() {
        return false;
    }
    let (x0, y0) = map_point(m, 0.0, 0.0);
    let (x1, y1) = map_point(m, w, h);
    let (min_x, max_x) = (x0.min(x1), x0.max(x1));
    let (min_y, max_y) = (y0.min(y1), y0.max(y1));
    min_x - reach >= 0.0 && min_y - reach >= 0.0 && max_x + reach <= f64::from(frame_width) && max_y + reach <= f64::from(frame_height)
}

/// A layer blur: the box's coverage blurred by the same chain and repainted with its single fill.
#[allow(clippy::too_many_arguments)]
fn emit_layer_blur(shape: ShapeKind, m: Affine, w: f64, h: f64, scale: f64, blur: &figcraft_doc::Blur, n: &Node, clip: Clip, scene: &mut Scene) {
    let Some(fill) = n.fills.iter().filter(|f| f.visible).find_map(|f| f.solid_color()) else {
        return;
    };
    let mut color = Rgba::from_color(fill);
    color.a *= n.opacity.clamp(0.0, 1.0);
    let b = blur.sanitized();
    if b.is_progressive() {
        let (s0, s1) = (b.start_radius / 2.0 * scale, b.radius / 2.0 * scale);
        let start = map_point(m, b.start.0 * w, b.start.1 * h);
        let end = map_point(m, b.end.0 * w, b.end.1 * h);
        scene.push_progressive_blur(ProgressiveBlurNode {
            mat: crate::scene::matrix(m),
            size: [w as f32, h as f32],
            shape,
            s0: s0 as f32,
            s1: s1 as f32,
            start: [start.0 as f32, start.1 as f32],
            end: [end.0 as f32, end.1 as f32],
            levels: progressive_levels(s0, s1, start, end),
            color,
            clip,
        });
    } else {
        scene.push_blur(LayerBlurNode {
            mat: crate::scene::matrix(m),
            size: [w as f32, h as f32],
            shape,
            sigma: (b.radius / 2.0 * scale) as f32,
            color,
            clip,
        });
    }
}

/// The CPU's `blur::levels`: `lo`, then geometric steps of 1.5 up to `hi`, at most eight.
fn progressive_levels(s0: f64, s1: f64, start: (f64, f64), end: (f64, f64)) -> Vec<f32> {
    let cap = |s: f64| if s.is_finite() { s.clamp(0.0, 1_000_000.0) } else { 0.0 };
    let (s0, s1) = (cap(s0), cap(s1));
    let d = (end.0 - start.0, end.1 - start.1);
    if (s1 - s0).abs() < 0.05 || d.0 * d.0 + d.1 * d.1 <= 1e-12 {
        return vec![s1 as f32];
    }
    let (lo, hi) = (s0.min(s1), s0.max(s1));
    let mut out = Vec::with_capacity(8);
    let base = lo.max(1.0);
    if lo < base {
        out.push(lo);
    }
    if hi <= base {
        if hi > lo || out.is_empty() {
            out.push(hi);
        }
        return out.into_iter().map(|v| v as f32).collect();
    }
    let steps = ((hi / base).ln() / 1.5_f64.ln()).ceil().clamp(1.0, 6.0) as usize;
    for k in 0..steps {
        out.push(base * (hi / base).powf(k as f64 / steps as f64));
    }
    out.push(hi);
    out.into_iter().map(|v| v as f32).collect()
}

/// Emits this node's background blur when it is one this build can draw; it paints before the node's
/// own fills and children. Records `effect` for everything else.
#[allow(clippy::too_many_arguments)]
fn backdrop(
    n: &Node,
    backdrops: &[&figcraft_doc::Blur],
    layer_blurs: &[&figcraft_doc::Blur],
    shape: Option<ShapeKind>,
    clip: Clip,
    m: Affine,
    w: f64,
    h: f64,
    scale: f64,
    scene: &mut Scene,
) {
    if backdrops.is_empty() {
        return;
    }
    let Some(shape) = shape.filter(|_| can_backdrop(n, backdrops, layer_blurs, clip)) else {
        scene.unsupported("effect", format!("`{}` carries a background blur this build cannot draw", n.name));
        return;
    };
    let blur = backdrops[0];
    scene.push_backdrop(BackdropNode {
        mat: crate::scene::matrix(m),
        size: [w as f32, h as f32],
        shape,
        sigma: (blur.max_radius() / 2.0 * scale) as f32,
        clip,
    });
}

/// Whether this node's noise is the CPU's grain over exactly its own SDF box fill: one noise,
/// no children, no stroke, no clip, full node opacity, one plain solid fill, and no other effect
/// (the grains ride on the layer's content alpha, which is just that fill).
fn can_noise(n: &Node, noises: &[&figcraft_doc::Noise], clip: Clip) -> bool {
    let [noise] = noises else {
        return false;
    };
    if !noise.density.is_finite() || !matches!(noise.blend, BlendMode::Normal) {
        return false;
    }
    if !n.children.is_empty() || n.clip || !clip.is_none() || n.has_visible_stroke() || n.opacity < 0.999 {
        return false;
    }
    if !matches!(n.blend, BlendMode::Normal) {
        return false;
    }
    if n.effects.iter().filter(|e| e.visible).count() != 1 {
        return false;
    }
    let mut fills = n.fills.iter().filter(|f| f.visible);
    let Some(fill) = fills.next() else {
        return false;
    };
    if fills.next().is_some() || !matches!(fill.blend, BlendMode::Normal) {
        return false;
    }
    fill.solid_color().is_some_and(|c| c.a.is_finite())
}

/// The alpha of the one visible solid fill, including the fill's own opacity and the node's.
fn noise_fill_alpha(n: &Node) -> f32 {
    let Some(fill) = n.fills.iter().find(|f| f.visible) else {
        return 0.0;
    };
    // `solid_color()` already folds the paint's opacity; the node's opacity is the only other factor
    // (the grain is only accepted at `n.opacity = 1`, but keep the expression honest).
    let Some(color) = fill.solid_color() else {
        return 0.0;
    };
    (color.a * n.opacity.clamp(0.0, 1.0)).clamp(0.0, 1.0) as f32
}

/// Emit the CPU's grains: one full-screen quad masked by the node's box, hashed per document
/// cell. The layer alpha under each grain is the fill's own alpha; the renderer reproduces the
/// CPU's 64-bit cell hash in WGSL, so the pattern lands on the same pixels. Mono, Duo and Multi
/// differ only in where the grain colour comes from.
fn emit_noise(id: NodeId, n: &Node, noise: &figcraft_doc::Noise, shape: ShapeKind, m: Affine, w: f64, h: f64, clip: Clip, scene: &mut Scene) {
    let density = (noise.density as f32).clamp(0.0, 1.0);
    if density <= 0.0 {
        return;
    }
    let (gx, gy) = noise.grain();
    let opacity = if noise.opacity.is_finite() { noise.opacity.clamp(0.0, 1.0) } else { 0.0 };
    scene.push_noise(NoiseNode {
        mat: crate::scene::matrix(m),
        size: [w as f32, h as f32],
        shape,
        clip,
        seed: id.0 as u32,
        density,
        cell: [(gx * 2.0) as f32, (gy * 2.0) as f32],
        color: Rgba::from_color(noise.color.sanitized()),
        secondary: Rgba::from_color(noise.secondary.sanitized()),
        opacity,
        mode: match noise.mode {
            NoiseMode::Mono => GrainMode::Mono,
            NoiseMode::Duo => GrainMode::Duo,
            NoiseMode::Multi => GrainMode::Multi,
        },
        fill_alpha: noise_fill_alpha(n),
    });
}

/// Whether this node's texture is the CPU's scatter over exactly its own SDF box fill: one texture,
/// no children, no stroke, no clip, full node opacity, one plain solid fill, and no other effect
/// (the layer being scattered is just that fill).
fn can_texture(n: &Node, textures: &[&figcraft_doc::Texture], clip: Clip) -> bool {
    let [texture] = textures else {
        return false;
    };
    if !texture.radius.is_finite() {
        return false;
    }
    if !n.children.is_empty() || n.clip || !clip.is_none() || n.has_visible_stroke() || n.opacity < 0.999 {
        return false;
    }
    if !matches!(n.blend, BlendMode::Normal) {
        return false;
    }
    if n.effects.iter().filter(|e| e.visible).count() != 1 {
        return false;
    }
    let mut fills = n.fills.iter().filter(|f| f.visible);
    let Some(fill) = fills.next() else {
        return false;
    };
    if fills.next().is_some() || !matches!(fill.blend, BlendMode::Normal) {
        return false;
    }
    fill.solid_color().is_some_and(|c| c.a.is_finite())
}

/// The CPU's whole-pixel scatter as a scene node: one full-screen quad whose destination pixels
/// hash to a cell, then read the fill at a source pixel offset by up to `radius` device pixels.
/// `clip_to_shape` multiplies the result by the fill's alpha at the destination, exactly like the
/// CPU's `keep`. `None` when the radius is too small to move anything (the CPU leaves the layer
/// untouched).
fn make_texture(
    id: NodeId,
    n: &Node,
    texture: &figcraft_doc::Texture,
    shape: ShapeKind,
    m: Affine,
    w: f64,
    h: f64,
    clip: Clip,
    scale: f64,
) -> Option<TextureNode> {
    let radius = texture.radius * scale;
    if !(radius.is_finite() && radius >= 0.5) {
        return None;
    }
    let (gx, gy) = texture.grain();
    let Some(fill) = n.fills.iter().find(|f| f.visible).and_then(|f| f.solid_color()) else {
        return None;
    };
    Some(TextureNode {
        mat: crate::scene::matrix(m),
        size: [w as f32, h as f32],
        shape,
        clip,
        seed: id.0 as u32,
        radius: (radius.min(2048.0)) as f32,
        clip_to_shape: texture.clip_to_shape,
        cell: [(gx * 2.0) as f32, (gy * 2.0) as f32],
        fill: Rgba::from_color(fill),
    })
}

/// Whether this node's backdrop effect is one uniform blur over an SDF box whose layer is just its
/// own fill (no children, no layer blur on top, no clip, no group opacity). The CPU paints the
/// blurred backdrop into the layer first, so the node's fills, shadows and strokes go over it.
fn can_backdrop(n: &Node, backdrops: &[&figcraft_doc::Blur], layer_blurs: &[&figcraft_doc::Blur], clip: Clip) -> bool {
    let [blur] = backdrops else {
        return false;
    };
    if blur.is_progressive() || !blur.max_radius().is_finite() {
        return false;
    }
    if !n.children.is_empty() || n.clip || !clip.is_none() || !layer_blurs.is_empty() {
        return false;
    }
    if n.has_visible_stroke() {
        // The CPU masks the backdrop by the node's fill outline; a stroke widens what it covers.
        return false;
    }
    if n.opacity < 0.999 || !matches!(n.blend, BlendMode::Normal) {
        return false;
    }
    // The blurred backdrop is one layer image: several visible fills would each be painted over it,
    // which is still what the CPU does, but this build takes the simple backdrop card (one fill).
    n.fills.iter().filter(|f| f.visible).count() == 1
}

/// The path fallback: tessellate the CPU's own fill outline and paint each visible solid fill.
fn emit_path_fills(doc: &Document, n: &Node, m: Affine, inner: Clip, scene: &mut Scene) {
    // Text fills are glyph outlines: expensive to build and already recorded as unsupported.
    if matches!(n.kind, NodeKind::Text(_) | NodeKind::Icon { .. } | NodeKind::Slice) {
        return;
    }
    let Some((path, rule)) = geometry::fill_outline(doc, n) else {
        // No fillable geometry (a line's box, say): the CPU paints nothing either.
        return;
    };
    let tris = geometry::tessellate(&path, rule);
    if tris.is_empty() {
        return;
    }
    let (verts, indices) = device_geometry(&tris, m);
    for (index, fill) in n.fills.iter().enumerate() {
        if !fill.visible {
            continue;
        }
        if !matches!(fill.blend, BlendMode::Normal) {
            scene.unsupported("paint-blend", format!("`{}` fill {} blends with {:?}", n.name, index, fill.blend));
            continue;
        }
        match fill.solid_color() {
            Some(color) => scene.push_mesh(MeshNode {
                verts: verts.clone(),
                indices: indices.clone(),
                color: Rgba::from_color(color),
                opacity: n.opacity.clamp(0.0, 1.0),
                clip: inner,
            }),
            None => scene.unsupported("gradient-on-path", format!("`{}` fill {index} is a gradient on a path", n.name)),
        }
    }
}

/// The stroke: the CPU's `stroke_fill` outline (align, joins, caps, dashes all baked in), painted
/// with each visible solid stroke paint.
fn emit_stroke(doc: &Document, n: &Node, m: Affine, inner: Clip, scene: &mut Scene) {
    let Some((path, rule)) = geometry::stroke_outline(doc, n) else {
        return;
    };
    let tris = geometry::tessellate(&path, rule);
    if tris.is_empty() {
        return;
    }
    let (verts, indices) = device_geometry(&tris, m);
    for (index, paint) in n.strokes.iter().enumerate() {
        if !paint.visible {
            continue;
        }
        if !matches!(paint.blend, BlendMode::Normal) {
            scene.unsupported("paint-blend", format!("`{}` stroke {} blends with {:?}", n.name, index, paint.blend));
            continue;
        }
        match paint.solid_color() {
            Some(color) => scene.push_mesh(MeshNode {
                verts: verts.clone(),
                indices: indices.clone(),
                color: Rgba::from_color(color),
                opacity: n.opacity.clamp(0.0, 1.0),
                clip: inner,
            }),
            None => scene.unsupported("stroke-paint", format!("`{}` stroke {index} is a gradient or an image", n.name)),
        }
    }
}

/// Text: the CPU's glyph and decoration outlines (`figcraft_text::layout_node`), tessellated like any
/// other path. Runs keep their own fills; colour glyphs (emoji, COLR layers) are not expressible and
/// send the frame to the CPU.
fn emit_text(n: &Node, base: Affine, inner: Clip, scene: &mut Scene) {
    let Some((layout, dy)) = figcraft_text::layout_node(n) else { return };
    let gxf = base * Affine::translate((0.0, dy));
    let scale = gxf.determinant().abs().sqrt();
    let mut by_style: BTreeMap<usize, BezPath> = BTreeMap::new();
    for glyph in &layout.glyphs {
        if glyph.color(scale).is_some() {
            scene.unsupported("text-color", format!("`{}` has colour glyphs (emoji or COLR)", n.name));
            return;
        }
        by_style.entry(glyph.style).or_default().extend(glyph.path().elements().iter().copied());
    }
    for decoration in &layout.decorations {
        by_style.entry(decoration.style).or_default().extend(decoration.path().elements().iter().copied());
    }
    for (style, path) in &by_style {
        let fills = layout.styles.get(*style).and_then(|s| s.fills.as_ref()).unwrap_or(&n.fills);
        push_path(n, path, figcraft_render::FillRule::Winding, fills, gxf, inner, scene, "text fill", "text-gradient");
    }
    if n.has_visible_stroke() {
        let rule = text_stroke_rule(n);
        for path in by_style.values() {
            let area = figcraft_pathops::stroke_outline_of(path, &n.stroke, true, None);
            push_path(n, &area, rule, &n.strokes, gxf, inner, scene, "text stroke", "text-stroke-paint");
        }
    }
}

/// The rule CPU text strokes fill their glyph band by (`stroke_rule` with `closed = true`).
fn text_stroke_rule(n: &Node) -> figcraft_render::FillRule {
    if n.stroke.sides.is_some() || (n.stroke.dashes.is_empty() && n.stroke.align != StrokeAlign::Center) {
        figcraft_render::FillRule::EvenOdd
    } else {
        figcraft_render::FillRule::Winding
    }
}

/// Tessellate `path` and push one mesh per visible solid paint.
fn push_path(
    n: &Node,
    path: &BezPath,
    rule: figcraft_render::FillRule,
    paints: &[DocPaint],
    m: Affine,
    clip: Clip,
    scene: &mut Scene,
    what: &str,
    gradient_feature: &'static str,
) {
    let tris = geometry::tessellate(path, rule);
    if tris.is_empty() {
        return;
    }
    let (verts, indices) = device_geometry(&tris, m);
    for (index, paint) in paints.iter().enumerate() {
        if !paint.visible {
            continue;
        }
        if !matches!(paint.blend, BlendMode::Normal) {
            scene.unsupported("paint-blend", format!("`{}` {what} {index} blends with {:?}", n.name, paint.blend));
            continue;
        }
        match paint.solid_color() {
            Some(color) => scene.push_mesh(MeshNode {
                verts: verts.clone(),
                indices: indices.clone(),
                color: Rgba::from_color(color),
                opacity: n.opacity.clamp(0.0, 1.0),
                clip,
            }),
            None => scene.unsupported(gradient_feature, format!("`{}` {what} {index} is a gradient or an image", n.name)),
        }
    }
}

/// The tessellation in device pixels: the same triangles, with the node transform baked into the
/// vertices so the mesh shader needs no matrix.
fn device_geometry(tris: &geometry::Triangles, m: Affine) -> (Arc<[[f32; 2]]>, Arc<[u32]>) {
    let verts: Vec<[f32; 2]> = tris
        .verts
        .iter()
        .map(|v| {
            let p = m * Point::new(f64::from(v[0]), f64::from(v[1]));
            [p.x as f32, p.y as f32]
        })
        .collect();
    (Arc::from(verts), Arc::from(tris.tris.clone()))
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
                .map(|s| Stop { at: s.position.clamp(0.0, 1.0), color: Rgba::new(s.color.r, s.color.g, s.color.b, s.color.a * opacity) })
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

/// The mesh items of a scene, for tests that do not run a GPU.
#[cfg(test)]
fn meshes(scene: &Scene) -> impl Iterator<Item = &MeshNode> {
    scene.items.iter().filter_map(|item| match item {
        crate::scene::Item::Mesh(mesh) => Some(mesh),
        crate::scene::Item::Shape(_)
        | crate::scene::Item::Shadow(_)
        | crate::scene::Item::Blur(_)
        | crate::scene::Item::ProgressiveBlur(_)
        | crate::scene::Item::Backdrop(_)
        | crate::scene::Item::Noise(_)
        | crate::scene::Item::Texture(_) => None,
    })
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
        assert_eq!(scene.shapes().count(), 1, "one fill, one instance");
        assert_eq!(scene.mesh_count(), 0, "a plain rectangle stays on the SDF");
        let n = scene.shapes().next().expect("one shape");
        assert_eq!(n.size, [100.0, 50.0]);
        assert!(matches!(n.shape, ShapeKind::RoundedRect { .. }));
        assert!(matches!(n.paint, Paint::Solid(c) if c.a == 1.0));
        // Local (10, 20) lands on device pixel (10, 20).
        assert!((n.mat[4] - 10.0).abs() < 0.01 && (n.mat[5] - 20.0).abs() < 0.01, "{:?}", n.mat);
    }

    #[test]
    fn a_stroke_becomes_a_tessellated_mesh_after_its_children() {
        let (mut doc, page, rect) = doc_with_rect();
        if let Ok(n) = doc.get_mut(rect) {
            n.strokes = vec![DocPaint::solid(Color::new(1.0, 0.0, 0.0, 1.0))];
            n.stroke.weight = 4.0;
        }
        let scene = build(&doc, page, &view());
        assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
        assert_eq!(scene.shapes().count(), 1, "the fill is still an SDF shape");
        assert_eq!(scene.mesh_count(), 1, "the stroke is a mesh");
        let mesh = meshes(&scene).next().expect("a mesh");
        assert!(!mesh.indices.is_empty());
        // Four device pixels wide, drawn inside the 100×50 box at (10, 20) (the default alignment).
        let xs: Vec<f32> = mesh.verts.iter().map(|v| v[0]).collect();
        let min = xs.iter().copied().fold(f32::INFINITY, f32::min);
        let max = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!((min - 10.0).abs() < 0.6, "stroke starts at x = 10, got {min}");
        assert!((max - 110.0).abs() < 0.6, "stroke ends at x = 110, got {max}");
    }

    #[test]
    fn a_line_is_drawn_by_its_stroke_mesh() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let line = doc.create(page, NodeKind::Line, "Line", 10.0, 40.0, 100.0, 0.0).expect("line");
        if let Ok(n) = doc.get_mut(line) {
            n.strokes = vec![DocPaint::solid(Color::new(0.0, 0.0, 0.0, 1.0))];
            n.stroke.weight = 6.0;
        }
        let scene = build(&doc, page, &view());
        assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
        assert_eq!(scene.shapes().count(), 0);
        assert_eq!(scene.mesh_count(), 1);
    }

    #[test]
    fn text_glyphs_become_meshes() {
        let Some(family) = ["Inter", "Segoe UI", "Arial"].into_iter().find(|f| figcraft_text::find_face(f, "Regular").is_some()) else {
            eprintln!("SKIP: no text font is installed here");
            return;
        };
        let mut doc = Document::default();
        let page = doc.pages[0];
        let mut data = figcraft_doc::TextData::new("Hi");
        data.style.font_size = 32.0;
        data.style.font_family = family.into();
        let text = doc.create(page, NodeKind::Text(data), "Hi", 10.0, 10.0, 80.0, 40.0).expect("text");
        if let Ok(n) = doc.get_mut(text) {
            n.fills = vec![DocPaint::solid(Color::new(0.0, 0.0, 0.0, 1.0))];
        }
        let scene = build(&doc, page, &view());
        assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
        assert!(scene.mesh_count() >= 1, "each run's glyph outlines are a mesh");
        assert_eq!(scene.shapes().count(), 0, "text is never an SDF box");
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
        let node = scene.shapes().find(|n| n.size == [100.0, 10.0]).expect("the child's instance");
        assert_eq!(node.clip.0, [10.0, 10.0, 50.0, 50.0], "clipped to the frame's own box");
    }

    #[test]
    fn a_gradient_with_too_many_stops_falls_back() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, NodeKind::Rectangle, "Rect", 0.0, 0.0, 10.0, 10.0).expect("rect");
        let mut g = figcraft_color::Gradient::default_for(figcraft_color::GradientKind::Linear, Color::new(0.0, 0.0, 0.0, 1.0));
        g.stops = (0..8).map(|i| figcraft_color::ColorStop { position: i as f32 / 8.0, color: Color::new(1.0, 0.0, 0.0, 1.0) }).collect();
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

    #[test]
    fn a_ring_ellipse_is_drawn_by_the_path_fallback() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let ring =
            doc.create(page, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.6 }, "Ring", 0.0, 0.0, 80.0, 80.0).expect("ring");
        if let Ok(n) = doc.get_mut(ring) {
            n.fills = vec![DocPaint::solid(Color::new(0.2, 0.4, 0.8, 1.0))];
        }
        let scene = build(&doc, page, &view());
        assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
        assert_eq!(scene.shapes().count(), 0, "the SDF cannot draw a ring");
        assert_eq!(scene.mesh_count(), 1, "the path fallback can");
    }

    #[test]
    fn an_inner_shadow_paints_after_the_fill() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, NodeKind::Rectangle, "Card", 10.0, 10.0, 60.0, 40.0).expect("rect");
        if let Ok(n) = doc.get_mut(rect) {
            n.fills = vec![DocPaint::solid(Color::new(0.9, 0.9, 0.9, 1.0))];
            n.effects = vec![figcraft_doc::Effect {
                kind: EffectKind::InnerShadow(figcraft_doc::Shadow {
                    color: Color::new(0.0, 0.0, 0.0, 0.5),
                    offset: (2.0, 3.0),
                    radius: 6.0,
                    spread: 0.0,
                    blend: BlendMode::Normal,
                    show_behind: false,
                }),
                visible: true,
            }];
        }
        let scene = build(&doc, page, &view());
        assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
        assert_eq!(scene.shadow_count(), 1);
        assert!(matches!(scene.items.first(), Some(crate::scene::Item::Shape(_))), "the fill comes first");
        assert!(matches!(scene.items.get(1), Some(crate::scene::Item::Shadow(s)) if s.inner), "the inner shadow paints over it");
    }

    #[test]
    fn a_spread_inner_shadow_falls_back() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, NodeKind::Rectangle, "Card", 10.0, 10.0, 60.0, 40.0).expect("rect");
        if let Ok(n) = doc.get_mut(rect) {
            n.fills = vec![DocPaint::solid(Color::new(0.9, 0.9, 0.9, 1.0))];
            n.effects = vec![figcraft_doc::Effect {
                kind: EffectKind::InnerShadow(figcraft_doc::Shadow { spread: 4.0, ..Default::default() }),
                visible: true,
            }];
        }
        let scene = build(&doc, page, &view());
        assert!(!scene.is_supported(), "the spread reaches outside the box");
        assert!(scene.unsupported_features().iter().any(|u| u.feature == "effect"));
    }

    #[test]
    fn a_layer_blur_replaces_the_fill() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, NodeKind::Rectangle, "Blurred", 10.0, 10.0, 60.0, 40.0).expect("rect");
        if let Ok(n) = doc.get_mut(rect) {
            n.fills = vec![DocPaint::solid(Color::new(0.2, 0.5, 0.8, 1.0))];
            n.effects = vec![figcraft_doc::Effect { kind: EffectKind::LayerBlur(figcraft_doc::Blur::uniform(6.0)), visible: true }];
        }
        let scene = build(&doc, page, &view());
        assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
        assert_eq!(scene.blur_count(), 1);
        assert_eq!(scene.shapes().count(), 0, "the blurred layer paints the fill, nothing sharp");
        assert!(matches!(scene.items.first(), Some(crate::scene::Item::Blur(_))));
    }

    #[test]
    fn a_layer_blur_over_children_falls_back() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let frame = doc.create(page, NodeKind::Frame, "Blurred", 10.0, 10.0, 60.0, 40.0).expect("frame");
        let _child = doc.create(frame, NodeKind::Rectangle, "Child", 5.0, 5.0, 20.0, 20.0).expect("child");
        if let Ok(n) = doc.get_mut(frame) {
            n.effects = vec![figcraft_doc::Effect { kind: EffectKind::LayerBlur(figcraft_doc::Blur::uniform(6.0)), visible: true }];
        }
        let scene = build(&doc, page, &view());
        assert!(!scene.is_supported(), "the layer's children are part of what the CPU blurs");
        assert!(scene.unsupported_features().iter().any(|u| u.feature == "effect"));
    }

    #[test]
    fn a_progressive_layer_blur_becomes_a_progressive_node() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, NodeKind::Rectangle, "Ramp", 40.0, 40.0, 60.0, 40.0).expect("rect");
        if let Ok(n) = doc.get_mut(rect) {
            n.fills = vec![DocPaint::solid(Color::new(0.2, 0.5, 0.8, 1.0))];
            n.effects = vec![figcraft_doc::Effect {
                kind: EffectKind::LayerBlur(figcraft_doc::Blur {
                    radius: 12.0,
                    blur_type: figcraft_doc::BlurType::Progressive,
                    start_radius: 0.0,
                    start: (0.5, 0.0),
                    end: (0.5, 1.0),
                }),
                visible: true,
            }];
        }
        let scene = build(&doc, page, &view());
        assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
        assert_eq!(scene.progressive_blur_count(), 1);
        assert_eq!(scene.blur_count(), 1);
        assert_eq!(scene.shapes().count(), 0, "the progressive layer paints the fill");
        let crate::scene::Item::ProgressiveBlur(blur) = scene.items.first().expect("one item") else {
            panic!("the progressive blur replaces the fill");
        };
        assert!(blur.levels.len() > 1 && blur.levels.len() <= 8);
        assert!((blur.start[0] as f64 - 70.0).abs() < 1e-3, "device start x");
        assert!((blur.start[1] as f64 - 40.0).abs() < 1e-3, "device start y");
    }

    #[test]
    fn a_background_blur_paints_before_the_fill() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, NodeKind::Rectangle, "Frosted", 10.0, 10.0, 60.0, 40.0).expect("rect");
        if let Ok(n) = doc.get_mut(rect) {
            n.fills = vec![DocPaint::solid(Color::new(1.0, 1.0, 1.0, 0.4))];
            n.effects = vec![figcraft_doc::Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(8.0)), visible: true }];
        }
        let scene = build(&doc, page, &view());
        assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
        assert_eq!(scene.backdrop_count(), 1);
        assert!(matches!(scene.items.first(), Some(crate::scene::Item::Backdrop(_))), "the backdrop paints first");
        assert!(matches!(scene.items.get(1), Some(crate::scene::Item::Shape(_))), "the fill paints over it");
    }

    #[test]
    fn a_background_blur_over_children_falls_back() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let frame = doc.create(page, NodeKind::Frame, "Frosted", 10.0, 10.0, 60.0, 40.0).expect("frame");
        let _child = doc.create(frame, NodeKind::Rectangle, "Child", 5.0, 5.0, 20.0, 20.0).expect("child");
        if let Ok(n) = doc.get_mut(frame) {
            n.fills = vec![DocPaint::solid(Color::new(1.0, 1.0, 1.0, 0.4))];
            n.effects = vec![figcraft_doc::Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(8.0)), visible: true }];
        }
        let scene = build(&doc, page, &view());
        assert!(!scene.is_supported(), "the layer's children are part of what the CPU shows over the backdrop");
        assert!(scene.unsupported_features().iter().any(|u| u.feature == "effect"));
    }

    #[test]
    fn a_mono_noise_rides_on_one_fill() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, NodeKind::Rectangle, "Grain", 10.0, 10.0, 60.0, 40.0).expect("rect");
        if let Ok(n) = doc.get_mut(rect) {
            n.fills = vec![DocPaint::solid(Color::new(0.2, 0.6, 0.3, 1.0))];
            n.effects = vec![figcraft_doc::Effect { kind: EffectKind::Noise(figcraft_doc::Noise::default()), visible: true }];
        }
        let scene = build(&doc, page, &view());
        assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
        assert_eq!(scene.noise_count(), 1);
        assert!(matches!(scene.items.first(), Some(crate::scene::Item::Shape(_))), "the fill paints first");
        assert!(matches!(scene.items.get(1), Some(crate::scene::Item::Noise(_))), "the grain rides on it");
    }

    #[test]
    fn duo_and_multi_noise_ride_on_one_fill() {
        for (mode, expected) in [(NoiseMode::Duo, crate::scene::GrainMode::Duo), (NoiseMode::Multi, crate::scene::GrainMode::Multi)] {
            let mut doc = Document::default();
            let page = doc.pages[0];
            let rect = doc.create(page, NodeKind::Rectangle, "Grain", 10.0, 10.0, 60.0, 40.0).expect("rect");
            if let Ok(n) = doc.get_mut(rect) {
                n.fills = vec![DocPaint::solid(Color::new(0.2, 0.6, 0.3, 1.0))];
                let mut noise = figcraft_doc::Noise::default();
                noise.mode = mode;
                n.effects = vec![figcraft_doc::Effect { kind: EffectKind::Noise(noise), visible: true }];
            }
            let scene = build(&doc, page, &view());
            assert!(scene.is_supported(), "{mode:?}: {:?}", scene.unsupported_features());
            assert_eq!(scene.noise_count(), 1);
            let crate::scene::Item::Noise(noise) = &scene.items[1] else {
                panic!("the grain rides on the fill");
            };
            assert_eq!(noise.mode, expected);
        }
    }

    #[test]
    fn a_texture_replaces_one_fill() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, NodeKind::Rectangle, "Rough", 10.0, 10.0, 60.0, 40.0).expect("rect");
        if let Ok(n) = doc.get_mut(rect) {
            n.fills = vec![DocPaint::solid(Color::new(0.2, 0.6, 0.3, 0.7))];
            n.effects = vec![figcraft_doc::Effect { kind: EffectKind::Texture(figcraft_doc::Texture::default()), visible: true }];
        }
        let scene = build(&doc, page, &view());
        assert!(scene.is_supported(), "{:?}", scene.unsupported_features());
        assert_eq!(scene.texture_count(), 1);
        assert_eq!(scene.shapes().count(), 0, "the texture replaces the fill, so no plain fill is drawn");
        assert!(matches!(scene.items.first(), Some(crate::scene::Item::Texture(_))), "the texture is the only painter");
    }

    #[test]
    fn a_texture_over_children_falls_back() {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let frame = doc.create(page, NodeKind::Frame, "Rough", 10.0, 10.0, 60.0, 40.0).expect("frame");
        let _child = doc.create(frame, NodeKind::Rectangle, "Child", 5.0, 5.0, 20.0, 20.0).expect("child");
        if let Ok(n) = doc.get_mut(frame) {
            n.fills = vec![DocPaint::solid(Color::new(0.2, 0.6, 0.3, 1.0))];
            n.effects = vec![figcraft_doc::Effect { kind: EffectKind::Texture(figcraft_doc::Texture::default()), visible: true }];
        }
        let scene = build(&doc, page, &view());
        assert!(!scene.is_supported(), "the layer's children would be part of what the CPU scatters");
        assert!(scene.unsupported_features().iter().any(|u| u.feature == "effect"));
    }
}
