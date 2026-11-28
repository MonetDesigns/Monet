//! GPU material shaders, evaluated **per layer**.
//!
//! This is the execution side of `wgsl_effect` (which owns the shader-agnostic data: scanning,
//! `@fx` metadata, generic exports and uniform packing). Nothing here knows what a shader looks
//! like, and nothing here touches a GPU: it is the hand-off between the renderer, which knows
//! *where* a material sits in the layer stack, and a backend (the `ui-gpui` `gpu` feature) that
//! owns a device.
//!
//! Contract
//! --------
//! * A material is a **layer's backdrop read**: the shader produces the pixels that sit under the
//!   layer's own content, exactly where the built-in Glass / Background blur effects draw theirs
//!   (`draw::draw_layer`, the `backdrop_layer` call). Children and fills are then drawn over it by
//!   the ordinary recursion — no hole punching, no second render of the subtree.
//! * The renderer never waits. A miss **requests** the layer and draws nothing there this frame;
//!   the finished raster shows up on a later frame through [`ShadeBoard::ready`].
//! * A request carries the backdrop as straight RGBA at the layer's own size, so a backend uploads
//!   a layer, not a page.
//!
//! Why the cache key hashes the backdrop: a shader reads the pixels under its layer, so "same
//! request" must mean "same pixels under it". Hashing the layer-sized backdrop (tens of KB) is
//! cheap next to rasterising it, and it frees the cache from any document-revision plumbing.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use figcraft_doc::{Document, NodeId};
use tiny_skia::{IntSize, Pixmap};

/// The host-box-blur a shader asks for with `@fx blur <input> <field>`. `radius_doc` is the
/// panel value already resolved (its default is used when the layer has no explicit value).
#[derive(Clone, Debug, PartialEq)]
pub struct BlurSpec {
    pub input: crate::wgsl_effect::BlurInput,
    pub field: String,
    pub radius_doc: f64,
}

/// One node's material choice, prepared by the host so this crate needs no JSON.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderSpec {
    /// Registry id the shader is loaded under (`wgsl_effect::WgslEffects`).
    pub effect_id: String,
    /// The `.wgsl` path or inline source id, for diagnostics and reloads.
    pub path: String,
    /// Control values as the panel stores them (`ParamSpec` scale, 0..100).
    pub params: BTreeMap<String, f64>,
    /// `@fx stage`: a backdrop material replaces the paint under the content, a layer material takes
    /// the finished layer (content + children) and replaces it — the same split the built-in effects
    /// use. A layer pass therefore carries `content`, not a backdrop.
    pub stage: crate::wgsl_effect::Stage,
    /// `@fx reach` resolved to document units: how far outside the node the shader reads, used to
    /// grow the layer window so a shadow/blur is not cut at the node's own edges.
    pub reach_doc: f64,
    /// `@fx blur`: the host box-blurs this input before the shader samples it. A backdrop blur
    /// replaces the canvas under the layer; a content blur replaces the layer's finished content.
    pub blur: Option<BlurSpec>,
}

impl ShaderSpec {
    pub fn new(effect_id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            effect_id: effect_id.into(),
            path: path.into(),
            params: BTreeMap::new(),
            stage: crate::wgsl_effect::Stage::Backdrop,
            reach_doc: 0.0,
            blur: None,
        }
    }
}

/// One layer to shade: its backdrop, geometry, and which shader.
#[derive(Clone)]
pub struct ShadeRequest {
    pub node: NodeId,
    /// [`shade_key`] of this request: the backend stores the raster under it.
    pub key: u64,
    pub spec: Arc<ShaderSpec>,
    /// The layer, in device pixels.
    pub size: (u32, u32),
    /// The node's own size in device pixels (the layer may be a window onto it).
    pub node_px: (f32, f32),
    /// Where this window's top-left sits inside the node, in device pixels.
    pub origin_px: (f32, f32),
    /// Corner radii in device pixels.
    pub radii_px: [f32; 4],
    /// Device pixels per document unit.
    pub scale: f64,
    /// The node's page-space bounds, for the same generic exports the whole-page path packs.
    pub bounds: [f64; 4],
    /// Corner smoothing (the same value `cornerSmoothings` carries on the whole-page path).
    pub smoothing: f64,
    /// **Premultiplied** RGBA, `size.0 × size.1` big: what lies under this layer. Shaders sample it
    /// and call `unpack` before doing straight-alpha math; passing straight bytes here would make
    /// GPU mip generation and edge blending darken transparent borders.
    pub backdrop: Arc<Vec<u8>>,
    /// For `Stage::Layer`: the finished layer as **premultiplied RGBA**, same size as `size`. The
    /// shader sees it through its `content` texture and its result replaces the layer. `None` for a
    /// backdrop material.
    pub content: Option<Arc<Vec<u8>>>,
    /// Which contract the pass runs under (mirrors [`ShaderSpec::stage`]). A backdrop pass paints
    /// under the content; a layer pass takes and replaces the finished layer.
    pub stage: crate::wgsl_effect::Stage,
    /// The window is the node's own **unrotated** frame, not the screen-aligned AABB. The host
    /// rotates the backdrop/content into it before upload and rotates the result back afterwards,
    /// so every shader sees `uv` along the node's axes. That is what makes an arbitrary material
    /// follow a rotated layer without each shader knowing about rotation.
    pub local: bool,
}

/// A finished layer: the shader over that backdrop, premultiplied RGBA at the layer's size.
#[derive(Clone)]
pub struct Shaded {
    pub key: u64,
    pub pixmap: Pixmap,
    /// Where this window's top-left sits inside the node, in device pixels. Two bands of one node
    /// can share a size but sit at different origins; the fallback keeps this so it never hands a
    /// band another band's raster (that would shift the glass by the origin difference).
    pub origin_px: (f32, f32),
}

/// Per-frame bridge between [`crate::draw`] (producer) and a GPU backend (consumer).
///
/// One board lives as long as the surface it belongs to: `ready` is the frame-to-frame cache, while
/// `requests` is cleared by whoever drains it.
///
/// The cache is keyed by **`(layer, key)`**, not by layer. A render is split into horizontal bands,
/// so one tall pane asks for several different rasters of the same node in the same frame — one per
/// band, each with its own window and origin. A per-node slot let those overwrite each other: every
/// band ended up drawing whichever band's raster was installed last, which is exactly the repeating
/// banding users see. `latest` keeps the newest one for misses.
pub struct ShadeBoard {
    specs: Mutex<Arc<HashMap<NodeId, Arc<ShaderSpec>>>>,
    ready: Mutex<HashMap<(NodeId, u64), Arc<Shaded>>>,
    /// Insertion order of `ready`, oldest first, for eviction.
    order: Mutex<VecDeque<(NodeId, u64)>>,
    /// The newest raster of each **window** (`node` + size + `origin_px`), whatever its key — a
    /// miss draws it, so a pane never blinks. Keyed by window, not by node: two bands of one node
    /// can match on size alone while covering different windows, and handing a band another band's
    /// raster shifts the glass. Bounded to nodes that still hold a `ready` entry (see [`Self::put`]).
    latest: Mutex<HashMap<(NodeId, u64), Arc<Shaded>>>,
    requests: Mutex<Vec<ShadeRequest>>,
    stats: Mutex<ShadeStats>,
    /// 同步解析器（由后端安装）。设置后，绘制到材质层时就地解析，不再“登记请求 + 整页重绘”。
    ///
    /// 这是把内置效果和 WGSL 材质统一到同一条管线的接缝：一个层需要像素时，生产者当场交付；
    /// 没有解析器（无 GPU / 测试）时才退回请求-延迟路径。
    resolver: Mutex<Option<Arc<Resolver>>>,
}

/// 一个层请求的同步生产者：给出这一层的像素，或 `None`（后端不可用，退回延迟路径）。
pub type Resolver = dyn Fn(&ShadeRequest, &Document) -> Option<Arc<Shaded>> + Send + Sync;

/// How many layer rasters the board keeps. One material node drawn across a page's bands needs a
/// handful (5–8); a document with several materials still fits. Each raster is one layer (not a
/// page) — a band of a tall pane is about 1 MB — so this bounds the cache near 25 MB.
const CACHE_LIMIT: usize = 24;

/// What one render asked for and got, for the `[shade]` log line and for tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShadeStats {
    /// Layers whose shader was taken from the cache this render.
    pub hits: usize,
    /// Layers requested this render (miss or stale).
    pub misses: usize,
}

impl std::fmt::Debug for ShadeBoard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShadeBoard")
            .field("specs", &self.specs.lock().map(|s| s.len()).unwrap_or(0))
            .field("cached", &self.cached())
            .field("pending", &self.pending())
            .finish()
    }
}

impl Default for ShadeBoard {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

impl ShadeBoard {
    pub fn new(specs: HashMap<NodeId, Arc<ShaderSpec>>) -> Self {
        Self {
            specs: Mutex::new(Arc::new(specs)),
            ready: Mutex::new(HashMap::new()),
            order: Mutex::new(VecDeque::new()),
            latest: Mutex::new(HashMap::new()),
            requests: Mutex::new(Vec::new()),
            stats: Mutex::new(ShadeStats::default()),
            resolver: Mutex::new(None),
        }
    }

    /// Install the synchronous producer (only if none is installed yet). Safe to call per frame.
    pub fn ensure_resolver(&self, resolver: Arc<Resolver>) {
        let mut slot = self.resolver.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(resolver);
        }
    }

    /// Produce this layer **now** through the installed resolver, or `None` when there is none or it
    /// declined (no adapter, shader not compiled yet…). The caller draws the result this frame; the
    /// delayed request path is only the fallback.
    pub fn resolve(&self, req: &ShadeRequest, doc: &Document) -> Option<Arc<Shaded>> {
        let resolver = self.resolver.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
        resolver(req, doc)
    }

    /// Replace the materials of this board (a document edit changed one). The rasters stay: the
    /// per-layer key decides whether each is still right.
    pub fn set_specs(&self, specs: HashMap<NodeId, Arc<ShaderSpec>>) {
        *self.specs.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(specs);
    }

    /// The material on node `id`, if it has one.
    pub fn spec_for(&self, id: NodeId) -> Option<Arc<ShaderSpec>> {
        self.specs.lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
    }

    pub fn specs_len(&self) -> usize {
        self.specs.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn has_specs(&self) -> bool {
        !self.specs.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// A raster for this layer under this key, if a previous frame produced one.
    pub fn ready(&self, node: NodeId, key: u64) -> Option<Arc<Shaded>> {
        let found = self.ready.lock().unwrap_or_else(|e| e.into_inner()).get(&(node, key)).cloned();
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        match found {
            Some(s) => {
                stats.hits += 1;
                Some(s)
            }
            None => {
                stats.misses += 1;
                None
            }
        }
    }

    /// Ask for this layer. Repeats of the same `(node, key)` in one frame are dropped, so a layer
    /// drawn into several bands (or a retried render) submits once.
    pub fn request(&self, req: ShadeRequest) {
        let mut q = self.requests.lock().unwrap_or_else(|e| e.into_inner());
        if q.iter().any(|q| q.node == req.node && q.key == req.key) {
            return;
        }
        q.push(req);
    }

    /// Take everything requested since the last drain (the backend calls this once a frame).
    pub fn take_requests(&self) -> Vec<ShadeRequest> {
        std::mem::take(&mut *self.requests.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn pending(&self) -> usize {
        self.requests.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Install a finished raster under its own key. Several rasters of one node coexist (one per
    /// band); the oldest is evicted once [`CACHE_LIMIT`] is reached. Eviction also drops the node's
    /// fallback entries once its last `ready` raster is gone, so `latest` cannot keep a pixel buffer
    /// alive after the cache stopped accounting for it.
    pub fn put(&self, node: NodeId, shaded: Shaded) {
        self.put_shared(node, Arc::new(shaded));
    }

    /// [`put`](Self::put) for an already shared raster (the synchronous resolver's result), so the
    /// cache and the frame that just drew it share one pixel buffer instead of copying it.
    pub fn put_shared(&self, node: NodeId, shaded: Arc<Shaded>) {
        let key = shaded.key;
        let win = window_id((shaded.pixmap.width(), shaded.pixmap.height()), shaded.origin_px);
        self.ready.lock().unwrap_or_else(|e| e.into_inner()).insert((node, key), Arc::clone(&shaded));
        self.latest.lock().unwrap_or_else(|e| e.into_inner()).insert((node, win), shaded);
        // Never hold `order` while taking `ready` (or the two lock orders deadlock): collect what
        // fell out of the queued window first, then drop it from the map.
        let evicted: Vec<(NodeId, u64)> = {
            let mut order = self.order.lock().unwrap_or_else(|e| e.into_inner());
            order.push_back((node, key));
            let mut out = Vec::new();
            while order.len() > CACHE_LIMIT {
                if let Some(old) = order.pop_front() {
                    out.push(old);
                }
            }
            out
        };
        if !evicted.is_empty() {
            // Nodes that still have a `ready` raster after eviction keep their fallback windows;
            // the rest lose every fallback, bounding `latest` together with `ready`.
            let still: std::collections::HashSet<NodeId> = {
                let mut ready = self.ready.lock().unwrap_or_else(|e| e.into_inner());
                for old in &evicted {
                    ready.remove(old);
                }
                ready.keys().map(|(n, _)| *n).collect()
            };
            let mut latest = self.latest.lock().unwrap_or_else(|e| e.into_inner());
            latest.retain(|(n, _), _| still.contains(n));
        }
    }

    /// Drop a layer's rasters (its node was deleted, or the material was removed).
    pub fn forget(&self, node: NodeId) {
        self.ready.lock().unwrap_or_else(|e| e.into_inner()).retain(|(n, _), _| *n != node);
        self.order.lock().unwrap_or_else(|e| e.into_inner()).retain(|(n, _)| *n != node);
        self.latest.lock().unwrap_or_else(|e| e.into_inner()).retain(|(n, _), _| *n != node);
    }

    /// The newest raster for `node` **of this exact window** (size and origin). A stale layer is
    /// what a miss draws: the pane keeps its last glass for the frame or two a new one takes,
    /// instead of blinking out.
    ///
    /// Both the size and the origin have to match: with several bands of one node in flight, a
    /// raster of another band is the same size at a different origin, and blitting it here would
    /// shift the glass. An equal size does not mean the same window.
    pub fn latest(&self, node: NodeId, size: (u32, u32), origin: (f32, f32)) -> Option<Arc<Shaded>> {
        let win = window_id(size, origin);
        let s = self.latest.lock().unwrap_or_else(|e| e.into_inner()).get(&(node, win)).cloned()?;
        (s.pixmap.width() == size.0 && s.pixmap.height() == size.1).then_some(s)
    }

    /// Take (and reset) this render's hit/miss counters.
    pub fn take_stats(&self) -> ShadeStats {
        std::mem::take(&mut *self.stats.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn cached(&self) -> usize {
        self.ready.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// Fold a layer's content into an existing [`shade_key`]. A layer material's result depends on the
/// pixels it is given, exactly like a backdrop material's depends on the backdrop; two frames whose
/// content differs must not share a raster.
pub fn with_content(key: u64, content: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    content.hash(&mut h);
    h.finish()
}

/// Mark a key as a node-local window. A screen-aligned window and a node-local one can have the
/// same size/origin numbers on an unrotated node; they must not share a raster.
pub fn with_local(key: u64) -> u64 {
    key ^ 0x9E37_79B9_7F4A_7C15
}

/// The cache key of one layer's shade: the shader, its parameters, the geometry it is drawn into,
/// and the pixels underneath it.
pub fn shade_key(
    spec: &ShaderSpec,
    size: (u32, u32),
    node_px: (f32, f32),
    bounds: [f64; 4],
    smoothing: f64,
    origin_px: (f32, f32),
    radii_px: [f32; 4],
    scale: f64,
    backdrop: &[u8],
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    spec.effect_id.hash(&mut h);
    spec.path.hash(&mut h);
    for (k, v) in &spec.params {
        k.hash(&mut h);
        v.to_bits().hash(&mut h);
    }
    size.hash(&mut h);
    // CodeRabbit PR#10: the geometry drawn into is part of the result; without these a grown rect
    // (fixed layer size, changed bounds/smoothing) hits an old raster.
    (node_px.0).to_bits().hash(&mut h);
    (node_px.1).to_bits().hash(&mut h);
    smoothing.to_bits().hash(&mut h);
    for v in bounds {
        v.to_bits().hash(&mut h);
    }
    for v in [origin_px.0, origin_px.1, radii_px[0], radii_px[1], radii_px[2], radii_px[3], scale as f32] {
        v.to_bits().hash(&mut h);
    }
    backdrop.hash(&mut h);
    h.finish()
}

/// Identity of a material **window** — a node's layer size plus where its top-left sits inside the
/// node. `latest` keys the fallback by this instead of by size alone: two bands of one node can be
/// the same size at different origins, and a fallback must never cross them.
fn window_id(size: (u32, u32), origin_px: (f32, f32)) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    size.hash(&mut h);
    origin_px.0.to_bits().hash(&mut h);
    origin_px.1.to_bits().hash(&mut h);
    h.finish()
}

/// A layer's pixels as straight (un-premultiplied) RGBA — what a shader samples.
pub fn straight(pix: &Pixmap) -> Vec<u8> {
    let mut out = pix.data().to_vec();
    for p in out.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        if a == 0 || a == 255 {
            continue;
        }
        for c in &mut p[0..3] {
            *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
        }
    }
    out
}

/// Straight RGBA → a pixmap (premultiplied, as tiny-skia wants it). `None` for a size mismatch,
/// which a backend bug can produce and which must not take the render down.
pub fn premultiplied(rgba: &[u8], w: u32, h: u32) -> Option<Pixmap> {
    let want = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    if rgba.len() < want || w == 0 || h == 0 {
        return None;
    }
    let mut bytes = rgba[..want].to_vec();
    for p in bytes.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        if a == 255 {
            continue;
        }
        for c in &mut p[0..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
    Pixmap::from_vec(bytes, IntSize::from_wh(w, h)?)
}

/// Wrap already-premultiplied RGBA bytes as a pixmap. A layer shader's output comes back
/// premultiplied (it was alpha-blended over a transparent canvas), and multiplying it again would
/// darken every antialiased edge; this is the counterpart of [`premultiplied`] for that case.
pub fn from_premultiplied(bytes: &[u8], w: u32, h: u32) -> Option<Pixmap> {
    Pixmap::from_vec(bytes.to_vec(), IntSize::from_wh(w, h)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Arc<ShaderSpec> {
        let mut s = ShaderSpec::new("material:abc", "glass.wgsl");
        s.params.insert("u.frost".into(), 4.0);
        Arc::new(s)
    }

    fn board() -> ShadeBoard {
        ShadeBoard::new(HashMap::from([(NodeId(7), spec())]))
    }

    fn request(key: u64) -> ShadeRequest {
        ShadeRequest {
            node: NodeId(7),
            key,
            spec: spec(),
            size: (4, 2),
            node_px: (4.0, 2.0),
            origin_px: (0.0, 0.0),
            radii_px: [0.0; 4],
            scale: 1.0,
            bounds: [0.0, 0.0, 4.0, 2.0],
            smoothing: 0.0,
            backdrop: Arc::new(vec![255; 4 * 2 * 4]),
            content: None,
            stage: crate::wgsl_effect::Stage::Backdrop,
            local: false,
        }
    }

    #[test]
    fn a_drawn_layer_is_requested_once_per_key() {
        let b = board();
        b.request(request(1));
        b.request(request(1));
        b.request(request(2));
        assert_eq!(b.pending(), 2, "same (node, key) in one frame submits once");
        assert_eq!(b.take_requests().len(), 2);
        assert_eq!(b.pending(), 0, "draining clears the queue");
    }

    #[test]
    fn only_the_matching_key_counts_as_a_hit() {
        let b = board();
        let n = NodeId(7);
        let pixmap = Pixmap::new(4, 2).unwrap();
        assert!(b.ready(n, 1).is_none());
        b.put(n, Shaded { key: 1, pixmap, origin_px: (0.0, 0.0) });
        assert!(b.ready(n, 1).is_some(), "same key: cached");
        assert!(b.ready(n, 2).is_none(), "a new backdrop invalidates the raster");
        assert_eq!(b.take_stats(), ShadeStats { hits: 1, misses: 2 });
        b.forget(n);
        assert!(b.ready(n, 1).is_none());
    }

    #[test]
    fn a_miss_hands_back_the_layer_s_last_raster() {
        let b = board();
        let n = NodeId(7);
        assert!(b.latest(n, (4, 2), (0.0, 0.0)).is_none(), "nothing drawn yet: the first frame is empty");
        b.put(n, Shaded { key: 1, pixmap: Pixmap::new(4, 2).unwrap(), origin_px: (0.0, 0.0) });
        assert!(b.ready(n, 2).is_none(), "a new key is a miss");
        assert_eq!(b.latest(n, (4, 2), (0.0, 0.0)).map(|s| s.key), Some(1), "a stale raster beats a blank frame");
        assert!(b.latest(n, (4, 9), (0.0, 0.0)).is_none(), "another band's window is a different raster");
    }

    /// A tall pane is rendered in bands, so one node asks for several rasters in one frame, at
    /// different windows. They must not evict each other: each band draws its own.
    #[test]
    fn bands_of_one_layer_keep_their_own_rasters() {
        let b = board();
        let n = NodeId(7);
        b.put(n, Shaded { key: 1, pixmap: Pixmap::new(4, 2).unwrap(), origin_px: (0.0, 0.0) });
        b.put(n, Shaded { key: 2, pixmap: Pixmap::new(4, 3).unwrap(), origin_px: (0.0, 0.0) });
        assert_eq!(b.ready(n, 1).map(|s| s.pixmap.height()), Some(2), "the first band is still there");
        assert_eq!(b.ready(n, 2).map(|s| s.pixmap.height()), Some(3), "and so is the second");
        assert_eq!(b.cached(), 2, "two windows, two entries");
    }

    #[test]
    fn the_key_moves_with_parameters_geometry_and_backdrop() {
        let s = spec();
        let base = shade_key(&s, (8, 8), (0.0, 0.0), [0.0; 4], 0.0, (0.0, 0.0), [2.0; 4], 1.0, &[1, 2, 3, 4]);
        assert_eq!(base, shade_key(&s, (8, 8), (0.0, 0.0), [0.0; 4], 0.0, (0.0, 0.0), [2.0; 4], 1.0, &[1, 2, 3, 4]), "stable");
        assert_ne!(base, shade_key(&s, (8, 8), (0.0, 0.0), [0.0; 4], 0.0, (0.0, 0.0), [2.0; 4], 1.0, &[1, 2, 3, 5]), "backdrop");
        assert_ne!(base, shade_key(&s, (8, 8), (9.0, 8.0), [0.0; 4], 0.0, (0.0, 0.0), [2.0; 4], 1.0, &[1, 2, 3, 4]), "node_px");
        assert_ne!(base, shade_key(&s, (8, 8), (0.0, 0.0), [0.0, 0.0, 9.0, 9.0], 0.0, (0.0, 0.0), [2.0; 4], 1.0, &[1, 2, 3, 4]), "bounds");
        assert_ne!(base, shade_key(&s, (8, 8), (0.0, 0.0), [0.0; 4], 0.5, (0.0, 0.0), [2.0; 4], 1.0, &[1, 2, 3, 4]), "smoothing");
        assert_ne!(base, shade_key(&s, (9, 8), (0.0, 0.0), [0.0; 4], 0.0, (0.0, 0.0), [2.0; 4], 1.0, &[1, 2, 3, 4]), "size");
        assert_ne!(base, shade_key(&s, (8, 8), (0.0, 0.0), [0.0; 4], 0.0, (1.0, 0.0), [2.0; 4], 1.0, &[1, 2, 3, 4]), "window");
        assert_ne!(base, shade_key(&s, (8, 8), (0.0, 0.0), [0.0; 4], 0.0, (0.0, 0.0), [3.0; 4], 1.0, &[1, 2, 3, 4]), "radii");
        let mut other = ShaderSpec::new("material:abc", "glass.wgsl");
        other.params.insert("u.frost".into(), 5.0);
        assert_ne!(base, shade_key(&other, (8, 8), (0.0, 0.0), [0.0; 4], 0.0, (0.0, 0.0), [2.0; 4], 1.0, &[1, 2, 3, 4]), "params");
    }

    #[test]
    fn straight_and_premultiplied_round_trip() {
        // Half-alpha colour next to an opaque one.
        let straight_in = [128u8, 64, 32, 128, 255, 255, 255, 255];
        let pix = premultiplied(&straight_in, 2, 1).unwrap();
        assert_eq!(&pix.data()[0..4], &[64, 32, 16, 128], "colour is scaled by alpha");
        assert_eq!(&pix.data()[4..8], &[255, 255, 255, 255], "an opaque pixel is untouched");
        assert_eq!(straight(&pix), straight_in, "un-premultiplying restores the input");
        assert!(premultiplied(&straight_in, 3, 1).is_none(), "short buffer is refused, not panicked on");
    }
}
