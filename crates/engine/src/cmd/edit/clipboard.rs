use super::{
    Affine, BTreeMap, BTreeSet, Document, EngineError, ImageData, Node, NodeId, NodeKind, Result, Session, Value, bad, delete, get, id, json, num,
    ok, req_targets, roots, string, targets, vector_clipboard, z_sorted,
};
use figcraft_doc::anim::motion::{AnimProperty, AnimValue, TimelineSettings};

/// The copied form of the selection.
pub fn copy_to_clipboard(s: &mut Session, cmd: &str, ids: &[NodeId]) -> Result<()> {
    let ds = s.ds()?;
    // Pages are not layers: they can't be pasted anywhere.
    let layers: Vec<NodeId> = ids.iter().copied().filter(|i| !ds.doc.node(*i).is_some_and(Node::is_page)).collect();
    if layers.is_empty() && !ids.is_empty() {
        return Err(EngineError::Disabled(cmd.into(), "pages can't be copied; use page.duplicate".into()));
    }
    let (nodes, images, bounds) = copied(&ds.doc, &layers);
    store_clipboard(s, cmd, nodes, images, bounds)
}

/// Make `nodes` (the copied layers, each root's transform in page space) the clipboard's layers.
pub(super) fn store_clipboard(
    s: &mut Session,
    cmd: &str,
    nodes: Vec<(NodeId, Vec<Node>)>,
    images: BTreeMap<String, ImageData>,
    bounds: Option<figcraft_geom::Rect>,
) -> Result<()> {
    let ds = s.ds()?;
    let (source, source_path, uid) = (ds.doc.clone(), ds.path.clone(), ds.uid);
    s.clipboard.source = Some(source);
    s.clipboard.source_path = source_path;
    s.clipboard.source_doc = uid;
    s.clipboard.copied_components = nodes
        .iter()
        .filter(|(_, sub)| {
            cmd == "edit.copy" && sub.first().is_some_and(|n| matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. }))
        })
        .map(|(root, _)| *root)
        .collect();
    s.clipboard.nodes = nodes;
    s.clipboard.images = images;
    s.clipboard.bounds = bounds;
    s.clipboard.layers_generation = s.clipboard.layers_generation.wrapping_add(1);
    Ok(())
}

type Copied = (Vec<(NodeId, Vec<Node>)>, BTreeMap<String, ImageData>, Option<figcraft_geom::Rect>);

fn copied(doc: &Document, ids: &[NodeId]) -> Copied {
    let ids = z_sorted(doc, &roots(doc, ids));
    let mut nodes = Vec::new();
    let mut images = BTreeMap::new();
    for i in &ids {
        let world = doc.world_transform(*i);
        let mut sub: Vec<Node> = doc.subtree(*i).into_iter().filter_map(|x| doc.node(x).cloned()).collect();
        // Store the root's transform in page space so pastes land where it was.
        if let Some(r) = sub.first_mut() {
            r.transform = world;
        }
        collect_images(doc, &sub, &mut images);
        nodes.push((*i, sub));
    }
    let bounds = doc.bounds_of(&ids);
    (nodes, images, bounds)
}

/// The images the paints of `nodes` use, taken from `doc` into `images`.
pub(super) fn collect_images(doc: &Document, nodes: &[Node], images: &mut BTreeMap<String, ImageData>) {
    for n in nodes {
        for p in n.fills.iter().chain(n.strokes.iter()) {
            if let figcraft_color::PaintKind::Image(img) = &p.kind
                && let Some(d) = doc.images.get(&img.image)
            {
                images.insert(img.image.clone(), d.clone());
            }
        }
    }
}

/// The markers around the copied layers in the clipboard's HTML, spelt as that format spells them.
pub const CLIP_OPEN: &str = "(figma)";

pub const CLIP_CLOSE: &str = "(/figma)";

/// Largest design data read from the clipboard: the `.fig` import's own canvas cap (256 MB). Its
/// base64 is checked first, so neither the text nor the decoded bytes can pass it.
const MAX_CLIP_DATA: usize = 256 * 1024 * 1024;

const MAX_CLIP_BASE64: usize = MAX_CLIP_DATA.div_ceil(3) * 4;

/// Base64 `text` (whitespace ignored, padding optional) when it fits [`MAX_CLIP_DATA`].
fn clip_base64(text: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    if text.bytes().filter(|b| !b.is_ascii_whitespace()).count() > MAX_CLIP_BASE64 {
        return None;
    }
    let b64: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD.decode(&b64).or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&b64)).ok()
}

/// Layers copied from another design app travel in the clipboard's HTML as a `.fig` canvas in
/// base64 between [`CLIP_OPEN`] and [`CLIP_CLOSE`]. This gives those bytes, or `None` when the HTML
/// holds no readable design data (it may just mention the marker).
pub fn design_clipboard_buffer(html: &str) -> Option<Vec<u8>> {
    let start = html.find(CLIP_OPEN)? + CLIP_OPEN.len();
    let len = html.get(start..)?.find(CLIP_CLOSE)?;
    let bytes = clip_base64(html.get(start..start + len)?)?;
    figcraft_format::fig::is_fig(&bytes).then_some(bytes)
}

/// Paste design layers from clipboard HTML or base64 `data`: decode and reject missing,
/// invalid or oversized data, load a temporary `.fig`, take its top-level page layers,
/// store them as a foreign-file clipboard source with a bumped layers generation, then
/// delegate to `paste` and append import warnings.
pub(super) fn paste_design_layers(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "edit.pasteDesignLayers";
    let bytes = match (string(p, "html"), string(p, "data")) {
        (Some(html), _) => design_clipboard_buffer(&html).ok_or_else(|| bad(CMD, "the HTML holds no copied design layers"))?,
        (None, Some(d)) => clip_base64(&d).ok_or_else(|| bad(CMD, "`data` is not base64 of at most 256 MB"))?,
        (None, None) => return Err(bad(CMD, "pass `html` or `data`")),
    };
    let other = crate::cmd::file::load_fig(&bytes, "Pasted layers").ok_or_else(|| bad(CMD, "the data is not copied design layers"))??;
    let doc = std::sync::Arc::new(other.doc);
    let tops: Vec<NodeId> = doc.pages.iter().filter_map(|pg| doc.node(*pg)).flat_map(|pg| pg.children.iter().copied()).collect();
    if tops.is_empty() {
        return Err(EngineError::Disabled(CMD.into(), "the copied layers came in empty".into()));
    }
    let (nodes, images, bounds) = copied(&doc, &tops);
    // A design-app paste replaces the copied layers, not the copied rows: a `fill.copy` before it
    // still pastes afterwards.
    let props = s.clipboard.props.take();
    let paint = s.clipboard.paint.take();
    let stroke = s.clipboard.stroke.take();
    let paint_images = std::mem::take(&mut s.clipboard.paint_images);
    let paint_source_doc = s.clipboard.paint_source_doc;
    let effect = s.clipboard.effect.take();
    // No open file has id 0. So paste treats this copy as a copy from a different file.
    let layers_generation = s.clipboard.layers_generation.wrapping_add(1);
    s.clipboard = crate::Clipboard {
        nodes,
        images,
        props,
        paint,
        stroke,
        paint_images,
        paint_source_doc,
        effect,
        bounds,
        source_doc: 0,
        source: Some(doc),
        source_path: None,
        copied_components: BTreeSet::new(),
        layers_generation,
    };
    let mut out = paste(s, p)?;
    let pasted: Vec<NodeId> = out["ids"].as_array().map(|a| a.iter().filter_map(crate::params::id_of).collect()).unwrap_or_default();
    let missing = layers_missing_images(s.doc()?, &pasted);
    if !missing.is_empty() {
        out["missingImages"] = json!(missing);
    }
    if !other.issues.is_empty() {
        out["warnings"] = json!(other.issues);
    }
    Ok(out)
}

/// The most layers [`layers_missing_images`] names.
const MAX_MISSING_NAMED: usize = 20;

/// The names of the layers under `ids` with an image fill or stroke whose image the paste didn't
/// carry (#1646): it shows the importer's missing-image placeholder, or nothing at all.
fn layers_missing_images(doc: &figcraft_doc::Document, ids: &[NodeId]) -> Vec<String> {
    let missing = |p: &figcraft_color::Paint| match &p.kind {
        figcraft_color::PaintKind::Image(img) => doc.images.get(&img.image).is_none_or(figcraft_format::rest::is_missing_image),
        _ => false,
    };
    let mut names: Vec<String> = Vec::new();
    for n in ids.iter().flat_map(|i| doc.subtree(*i)).filter_map(|i| doc.node(i)) {
        if n.fills.iter().chain(&n.strokes).any(missing) && !names.contains(&n.name) {
            names.push(n.name.clone());
            if names.len() == MAX_MISSING_NAMED {
                break;
            }
        }
    }
    names
}

/// Whether the command names its layers (`ids` / `id`): it then acts on them even in vector edit mode.
fn names_layers(p: &Value) -> bool {
    get(p, "ids").is_some() || get(p, "id").is_some()
}

pub(super) fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    // In vector edit mode with points or segments selected, Copy takes just those (#1875).
    if !names_layers(p)
        && let Some(sel) = vector_clipboard::edit_selection(s)
        && sel.any
    {
        // Only lone points selected: nothing to copy, and not the whole layer either.
        return vector_clipboard::copy_selection(s, "edit.copy", sel);
    }
    let ids = req_targets("edit.copy", s, p)?;
    copy_to_clipboard(s, "edit.copy", &ids)?;
    Ok(json!({"copied": s.clipboard.nodes.len()}))
}

pub(super) fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    // In vector edit mode with points or segments selected, Cut takes just those and stays in edit mode (#1875).
    if !names_layers(p)
        && let Some(sel) = vector_clipboard::edit_selection(s)
        && sel.any
    {
        // Lone points with nothing to copy: `cut_selection` refuses, as Copy does.
        return vector_clipboard::cut_selection(s, sel);
    }
    let ids = req_targets("edit.cut", s, p)?;
    copy_to_clipboard(s, "edit.cut", &ids)?;
    let doc = s.doc()?;
    let layers: Vec<NodeId> = ids.into_iter().filter(|i| !doc.node(*i).is_some_and(Node::is_page)).collect();
    delete(s, &json!({"ids": layers}))
}

/// Where a node's parent sits in page space (identity for a page-level node).
fn parent_world(doc: &Document, id: NodeId) -> Affine {
    doc.node(id).and_then(|n| n.parent).map(|p| doc.world_transform(p)).unwrap_or(Affine::IDENTITY)
}

/// A pasted root moves from its old parent's space into a new one: its **own** Position
/// keyframes are parent-local, so without the same placement transform the animation snaps
/// back to where the original was — often outside the destination frame, where the timeline
/// looks empty. Children keep their parent, so only the root needs this.
///
/// Translation only: a rotated/scaled placement changes the layer's base rotation/scale too,
/// and shifting only X/Y would leave those tracks inconsistent, so those keep the old values.
fn rebase_position(node: &mut Node, place: Affine) {
    let [a, b, c, d, e, f] = place.as_coeffs();
    let translated = (a - 1.0).abs() < 1e-6 && b.abs() < 1e-6 && c.abs() < 1e-6 && (d - 1.0).abs() < 1e-6;
    if !translated || (e.abs() < 1e-9 && f.abs() < 1e-9) {
        return;
    }
    let Some(track) = node.motion.track_mut(AnimProperty::Position) else { return };
    for key in &mut track.keys {
        if let AnimValue::Vec2(v) = &mut key.value {
            v.x += e;
            v.y += f;
        }
    }
}

/// Keep a copied frame's own timeline settings with its copy. `duplicate_into` already does
/// this; paste only cloned the nodes, so the pasted frame silently used the destination
/// page's duration instead — long animations then fell past the timeline's right edge.
fn copy_timelines(doc: &mut Document, source: Option<&Document>, copy_local: bool, map: &BTreeMap<NodeId, NodeId>) {
    for (old, new) in map {
        let settings = if copy_local {
            doc.timelines.get(old).copied().or_else(|| source.and_then(|s| s.timelines.get(old).copied()))
        } else {
            source.and_then(|s| s.timelines.get(old).copied())
        };
        if let Some(settings) = settings {
            doc.timelines.insert(*new, settings.sanitized());
        }
    }
}

/// Paste the clipboard under `parent` with the copied top-left moved to `at` (page coords).
pub fn paste_into(s: &mut Session, parent: NodeId, at: Option<(f64, f64)>) -> Result<Vec<NodeId>> {
    let clip = s.clipboard.clone();
    if clip.nodes.is_empty() {
        return Err(EngineError::Disabled("edit.paste".into(), "the clipboard is empty".into()));
    }
    // From another file: its instances follow the mains this file can have.
    let foreign = clip.source.as_ref().filter(|_| s.ds().is_ok_and(|d| d.uid != clip.source_doc));
    let copy_local = s.ds()?.uid == clip.source_doc;
    let doc = s.doc_mut()?;
    for (k, v) in &clip.images {
        doc.images.entry(k.clone()).or_insert_with(|| v.clone());
    }
    // Sections go on the page (or in sections), never in the frame being pasted into.
    let page = doc.page_of(parent).unwrap_or(parent);
    let shift = match (at, clip.bounds) {
        (Some((x, y)), Some(b)) => Affine::translate((x - b.x0, y - b.y0)),
        _ => Affine::IDENTITY,
    };
    let mut out = Vec::new();
    let mut all = BTreeMap::new();
    for (root, nodes) in &clip.nodes {
        if copy_local
            && clip.copied_components.contains(root)
            && let Some(n) = nodes.first().filter(|n| matches!(n.kind, NodeKind::Component { .. }))
            && doc.node(*root).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. }))
        {
            let old_parent = parent_world(doc, *root);
            let instance = crate::components::create_instance(doc, *root, parent, 0.0, 0.0)?;
            let place = doc.world_transform(parent).inverse() * shift;
            doc.get_mut(instance)?.transform = place * n.transform;
            if let Some(inst) = doc.node_mut(instance) {
                rebase_position(inst, place * old_parent);
            }
            out.push(instance);
            continue;
        }
        let mut map = BTreeMap::new();
        for n in nodes {
            map.insert(n.id, doc.alloc_id());
        }
        let mut nodes = nodes.clone();
        let section = nodes.first().is_some_and(|r| matches!(r.kind, NodeKind::Section { .. }));
        let into = if section && !doc.takes_sections(parent) { page } else { parent };
        let old_parent = clip.source.as_deref().map(|d| parent_world(d, *root)).unwrap_or(Affine::IDENTITY);
        let place = doc.world_transform(into).inverse() * shift;
        if let Some(r) = nodes.first_mut() {
            r.transform = place * r.transform;
            rebase_position(r, place * old_parent);
        }
        out.push(doc.paste_nodes(&nodes, &map, *root, into, None)?);
        copy_timelines(doc, clip.source.as_deref(), copy_local, &map);
        all.extend(map);
    }
    // A pasted animation can be longer than the destination frame's timeline. The timeline
    // maps time to x, so its bar then starts past the right edge and the layer looks like it
    // has no animation (and its clips paint over the shaded past-duration area). Grow the
    // scope the pasted layers will be shown in — never shrink it — so the copy is visible
    // where it was pasted, the manual "set the frame's duration" workaround done for the user.
    // Group by scope: two frames pasted at once each need their own duration, while layers
    // that landed in the same frame/page share one combined maximum.
    let mut scopes: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for root in &out {
        let scope = doc.top_level_frame(*root).or_else(|| doc.page_of(*root)).unwrap_or(*root);
        scopes.entry(scope).or_default().push(*root);
    }
    for (scope, roots) in scopes {
        // An instance follows its main's clock and must not get an instance-owned entry:
        // `timeline_settings_for` would then stop forwarding later main duration/fps/loop
        // changes. There is no unambiguous destination to extend here, so leave the main alone.
        if doc.node(scope).is_some_and(|n| matches!(n.kind, NodeKind::Instance { .. })) {
            continue;
        }
        let mut settings = doc.timeline_settings_for(scope).sanitized();
        let mut end = settings.duration_ms;
        for root in &roots {
            for id in doc.subtree(*root) {
                if let Some((_, hi)) = doc.node(id).and_then(|n| n.motion.span_ms(settings.duration_ms)) {
                    end = end.max(hi);
                }
            }
        }
        if end > settings.duration_ms {
            settings.duration_ms = end.clamp(1, TimelineSettings::MAX_DURATION_MS);
            doc.timelines.insert(scope, settings);
        }
    }
    // Board connectors pasted with what they join join the pasted copies.
    let originals: Vec<(NodeId, figcraft_doc::BoardItem)> =
        clip.nodes.iter().flat_map(|(_, nodes)| nodes).filter_map(|n| Some((*all.get(&n.id)?, n.board.clone()?))).collect();
    crate::board::rejoin(doc, &originals, &all, !copy_local);
    if let Some(src) = foreign {
        crate::library::adopt_pasted(doc, src, clip.source_path.as_deref(), &all)?;
    }
    crate::library::count_inserted(doc, &out);
    Ok(out)
}

pub(super) fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    // While editing a vector, the copied layers join its network (#1875); `parent` asks for layers.
    if id(p, "parent").is_none() && s.ds()?.vector_edit.is_some() {
        return vector_clipboard::paste_into_vector(s, p);
    }
    let page = s.page()?;
    // Figma pastes into the selected frame (if one is selected), at the same position when it fits,
    // else in the frame's centre; otherwise in place on the page (or the viewport centre).
    let sel = s.selection();
    let doc = s.doc()?;
    let copy_local = s.ds()?.uid == s.clipboard.source_doc;
    let target = id(p, "parent").or_else(|| {
        sel.first()
            .copied()
            .filter(|i| {
                sel.len() == 1
                    && doc.node(*i).is_some_and(|n| n.kind.is_frame_like() && !matches!(n.kind, NodeKind::Instance { .. }) && !n.is_board_atom())
            })
            .filter(|i| !copy_local || !s.clipboard.copied_components.iter().any(|root| doc.is_ancestor(*root, *i)))
    });
    // One layer selected that isn't a frame target: the copy joins its parent, just above it (#2355).
    let sibling_of = if target.is_none() && id(p, "parent").is_none() { sibling_anchor(s, &sel, copy_local) } else { None };
    let target = target.or_else(|| sibling_of.and_then(|i| doc.node(i).and_then(|n| n.parent)));
    let parent = target.unwrap_or(page);
    let at = match (num(p, "x"), num(p, "y")) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => {
            let b = s.clipboard.bounds;
            match (target, b) {
                (Some(t), Some(b)) => {
                    let tb = doc.world_bounds(t);
                    match tb {
                        Some(tb) if tb.contains(b.origin()) && tb.contains(figcraft_geom::Point::new(b.x1, b.y1)) => None,
                        Some(tb) => Some((tb.center().x - b.width() / 2.0, tb.center().y - b.height() / 2.0)),
                        None => None,
                    }
                }
                (None, Some(b)) => {
                    // Off-screen originals paste at the viewport centre.
                    let v = &s.ds()?.view;
                    let (ox, oy) = v.origin();
                    let vr = figcraft_geom::Rect::new(ox, oy, ox + v.vw / v.zoom.max(1e-9), oy + v.vh / v.zoom.max(1e-9));
                    if vr.intersect(b).is_zero_area() { Some((v.cx - b.width() / 2.0, v.cy - b.height() / 2.0)) } else { None }
                }
                _ => None,
            }
        }
    };
    let ids = paste_into(s, parent, at)?;
    if let Some(sel) = sibling_of {
        let above = s.doc()?.index_in_parent(sel).map(|ix| ix + 1);
        move_above(s, &ids, parent, above)?;
    }
    s.set_selection(ids.clone());
    Ok(json!({"ids": ids}))
}

/// The one selected layer a plain paste goes next to: not a page-level layer, not inside an instance
/// (its children are fixed), not a board atom, a component set (a variant's parent) or inside a copied
/// component. Else the page, as before.
fn sibling_anchor(s: &Session, sel: &[NodeId], copy_local: bool) -> Option<NodeId> {
    let [only] = sel else { return None };
    let doc = s.doc().ok()?;
    let parent = doc.node(*only)?.parent?;
    let pn = doc.node(parent)?;
    let ok = !pn.is_page()
        && pn.kind.is_frame_like()
        && !pn.is_board_atom()
        && !crate::components::inside_instance(doc, *only)
        && !matches!(pn.kind, NodeKind::Instance { .. } | NodeKind::ComponentSet { .. })
        && (!copy_local || !s.clipboard.copied_components.iter().any(|root| doc.is_ancestor(*root, parent)));
    ok.then_some(*only)
}

/// Move the pasted copies that landed in `parent` to `above` and after, in the copied order.
fn move_above(s: &mut Session, ids: &[NodeId], parent: NodeId, above: Option<usize>) -> Result<()> {
    let Some(above) = above else { return Ok(()) };
    let doc = s.doc_mut()?;
    for (k, i) in ids.iter().filter(|i| doc.node(**i).is_some_and(|n| n.parent == Some(parent))).copied().collect::<Vec<_>>().into_iter().enumerate()
    {
        doc.reorder(i, above + k)?;
    }
    Ok(())
}

pub(super) fn paste_over(s: &mut Session, _p: &Value) -> Result<Value> {
    let sel = s.selection();
    let page = s.page()?;
    let doc = s.doc()?;
    let (parent, at, above) = match sel.first() {
        Some(i) => {
            let parent = doc.node(*i).and_then(|n| n.parent).unwrap_or(page);
            (parent, doc.world_bounds(*i).map(|b| (b.x0, b.y0)), doc.index_in_parent(*i).map(|ix| ix + 1))
        }
        None => (page, None, None),
    };
    let ids = paste_into(s, parent, at)?;
    // The pasted layers sit above the layer they replace, as the incumbent stacks them.
    move_above(s, &ids, parent, above)?;
    s.set_selection(ids.clone());
    Ok(json!({"ids": ids}))
}

pub(super) fn paste_replace(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = req_targets("edit.pasteReplace", s, p)?;
    let mut out = Vec::new();
    for t in targets {
        let doc = s.doc()?;
        let Some(n) = doc.node(t) else { continue };
        let parent = n.parent.ok_or_else(|| bad("edit.pasteReplace", "can't replace a page"))?;
        let idx = doc.index_in_parent(t);
        let at = doc.world_bounds(t).map(|b| (b.x0, b.y0));
        let ids = paste_into(s, parent, at)?;
        let doc = s.doc_mut()?;
        for (k, i) in ids.iter().enumerate() {
            if let Some(ix) = idx {
                let _ = doc.reorder(*i, ix + k);
            }
        }
        doc.remove(t)?;
        out.extend(ids);
    }
    s.set_selection(out.clone());
    Ok(json!({"ids": out}))
}

pub(super) fn copy_props_of(doc: &Document, id: NodeId) -> Value {
    let Some(n) = doc.node(id) else { return Value::Null };
    let mut v = json!({
        "fills": n.fills, "strokes": n.strokes, "strokeWeight": n.stroke.weight, "strokeAlign": n.stroke.align,
        "strokeSides": n.stroke.sides, "startCap": n.stroke.cap_start, "endCap": n.stroke.cap_end,
        "strokeJoin": n.stroke.join, "miterLimit": n.stroke.miter_limit, "dashes": n.stroke.dashes,
        "dashCap": n.stroke.dash_cap,
        "widthProfile": super::super::props::width_points_json(n.stroke.width_profile.as_ref()), "brush": n.stroke.brush,
        "effects": n.effects, "opacity": n.opacity, "blend": n.blend,
    });
    // A vector rounds each anchor on its own (`vector.edit.cornerRadius` keeps the radii in the
    // network's vertices, its layer corners stay empty): there is no style radius to carry, and
    // pasting the empty one would flatten every anchor's radius.
    if !matches!(n.kind, NodeKind::Vector { .. }) {
        v["cornerRadius"] = json!(n.corners.radii);
        v["cornerSmoothing"] = json!(n.corners.smoothing);
    }
    if let NodeKind::Text(t) = &n.kind {
        v["fontFamily"] = json!(t.style.font_family);
        v["fontStyle"] = json!(t.style.font_style);
        v["fontSize"] = json!(t.style.font_size);
        v["fontVariations"] = json!(t.style.variations);
        v["lineHeight"] = serde_json::to_value(t.style.line_height).unwrap_or(Value::Null);
        v["letterSpacing"] = serde_json::to_value(t.style.letter_spacing).unwrap_or(Value::Null);
    }
    v
}

pub(super) fn copy_props(s: &mut Session, p: &Value) -> Result<Value> {
    let id = targets(s, p).first().copied().ok_or_else(|| bad("edit.copyProperties", "select one layer"))?;
    let v = copy_props_of(s.doc()?, id);
    s.clipboard.props = Some(v.clone());
    Ok(v)
}

/// Give layer `id` the properties [`copy_props_of`] copied (`props`), those it can take: the text
/// properties only to text, the corners only to a layer with corners.
pub(crate) fn apply_copied_props(cmd: &str, doc: &mut Document, id: NodeId, props: &Value) -> Result<()> {
    let mut m = props.as_object().cloned().unwrap_or_default();
    let is_text = matches!(doc.get(id)?.kind, NodeKind::Text(_));
    if !is_text {
        m.retain(|k, _| !k.starts_with("font") && k != "lineHeight" && k != "letterSpacing");
    }
    if !doc.get(id)?.kind.has_corner_radius() {
        m.remove("cornerRadius");
        m.remove("cornerSmoothing");
    }
    crate::cmd::props::apply(cmd, doc, id, &m)
}

pub(super) fn paste_props(s: &mut Session, p: &Value) -> Result<Value> {
    let props = s.clipboard.props.clone().ok_or_else(|| EngineError::Disabled("edit.pasteProperties".into(), "no copied properties".into()))?;
    let ids = req_targets("edit.pasteProperties", s, p)?;
    let doc = s.doc_mut()?;
    for i in ids {
        apply_copied_props("edit.pasteProperties", doc, i, &props)?;
    }
    ok()
}
