//! The GPU renderer's drawing IR.
//!
//! Deliberately plain data with **no wgpu and no `Document` inside**: the document front end
//! ([`crate::doc_scene`]) fills it, a test can fill it by hand, and the renderer only reads it. That
//! separation is what makes the CPU↔GPU diff honest — both sides start from the same `Document` and
//! meet again at pixels, never at this struct.
//!
//! Every node is a **filled shape**. A shape with three fills is three nodes, in Figma's order. Strokes,
//! text, paths, effects, masks and non-normal blending arrive as their own [`Item`] nodes; when a frame
//! uses a shape the CPU draws differently, [`crate::doc_scene`] records an [`Unsupported`] entry and the
//! renderer refuses the whole frame (the caller draws it on the CPU instead).

use figcraft_color::{BlendMode, Color};
use figcraft_geom::Affine;

/// Most stops one gradient may carry in this milestone. The renderer packs them into the instance;
/// a gradient with more stops is a fallback, never a truncated gradient.
pub const MAX_STOPS: usize = 6;

/// Straight (non-premultiplied) sRGB + alpha, 0..=1 — the numbers a `figcraft_color::Color` holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const TRANSPARENT: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_color(c: Color) -> Self {
        Self { r: c.r, g: c.g, b: c.b, a: c.a }
    }

    /// The bytes the instance buffer holds for a stop: premultiplied, because a gradient is
    /// interpolated in premultiplied space (that is what the CPU's gradient shader does, and mixing
    /// straight colours would light up the transparent end of a fade).
    pub fn premultiplied(self) -> [f32; 4] {
        [self.r * self.a, self.g * self.a, self.b * self.a, self.a]
    }
}

/// The gradient kinds Figma offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientKind {
    Linear,
    Radial,
    Angular,
    Diamond,
}

/// One gradient stop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    /// 0..=1 along the gradient.
    pub at: f32,
    pub color: Rgba,
}

/// A fill.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Rgba),
    /// The three Figma handles, in the shape's unit box (0..=1 on both axes): `from` is the start
    /// (the centre for radial/angular/diamond), `to` the end of the main axis, `width` the end of the
    /// secondary axis.
    Gradient {
        kind: GradientKind,
        from: [f32; 2],
        to: [f32; 2],
        width: [f32; 2],
        stops: Vec<Stop>,
    },
}

impl Paint {
    /// A gradient needs two stops to interpolate; one stop paints flat, none paints nothing.
    pub fn normalised(self) -> Option<Self> {
        match self {
            Self::Solid(c) if c.a <= 0.0 => None,
            Self::Solid(c) => Some(Self::Solid(c)),
            Self::Gradient { kind, from, to, width, mut stops } => {
                if stops.is_empty() {
                    return None;
                }
                if stops.len() == 1 {
                    let c = stops.remove(0).color;
                    return (c.a > 0.0).then_some(Self::Solid(c));
                }
                if stops.len() > MAX_STOPS {
                    return None;
                }
                Some(Self::Gradient { kind, from, to, width, stops })
            }
        }
    }
}

/// A device-pixel clip rectangle. `w < 0` means "no clip", the same sentinel the shader tests.
///
/// `radii` are the four corner radii in device pixels (top-left, top-right, bottom-right,
/// bottom-left), zero for a plain rectangle. Only SDF shapes and tessellated meshes read them; a
/// frame that puts another kind of item under a rounded clip is recorded as unsupported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip(pub [f32; 4], pub [f32; 4]);

impl Clip {
    pub const NONE: Self = Self([-1.0, -1.0, -1.0, -1.0], [0.0; 4]);

    pub fn is_none(self) -> bool {
        self.0[2] < 0.0
    }

    pub fn has_radii(self) -> bool {
        self.1.iter().any(|r| *r > 0.0)
    }

    pub fn rounded(rect: [f32; 4], radii: [f32; 4]) -> Self {
        Self(rect, radii)
    }

    /// The intersection of this clip and `rect` (`[x, y, w, h]`, device pixels). A rounded clip
    /// cannot be intersected with another rectangle exactly, so callers reject that combination
    /// before asking; the result here is always a plain rectangle.
    pub fn intersect(self, rect: [f32; 4]) -> Self {
        if rect[2] <= 0.0 || rect[3] <= 0.0 {
            // An empty rectangle clips everything away; a zero-width clip still discards every
            // fragment, which is exactly "nothing of this subtree shows".
            return Self([rect[0], rect[1], 0.0, 0.0], [0.0; 4]);
        }
        if self.is_none() {
            return Self(rect, [0.0; 4]);
        }
        let x0 = self.0[0].max(rect[0]);
        let y0 = self.0[1].max(rect[1]);
        let x1 = (self.0[0] + self.0[2]).min(rect[0] + rect[2]);
        let y1 = (self.0[1] + self.0[3]).min(rect[1] + rect[3]);
        if x1 <= x0 || y1 <= y0 { Self([x0, y0, 0.0, 0.0], [0.0; 4]) } else { Self([x0, y0, x1 - x0, y1 - y0], [0.0; 4]) }
    }
}

/// The shape a node's box is filled as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeKind {
    /// A rectangle with independent corner radii, in document units (clamped to half the box by the
    /// front end).
    RoundedRect { radii: [f32; 4] },
    /// A full ellipse in the box.
    Ellipse,
    /// A regular polygon, point up, inscribed in the box's smaller side.
    Polygon { count: u32 },
    /// A regular star, point up, inscribed in the box's smaller side.
    Star { count: u32, inner_ratio: f32 },
}

/// One filled shape in device space.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeNode {
    /// Local → device pixels, row-major `[a, b, c, d, e, f]`: `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
    pub mat: [f32; 6],
    /// The shape's box in document units; the local space is `0..=w` × `0..=h`.
    pub size: [f32; 2],
    pub shape: ShapeKind,
    pub paint: Paint,
    pub clip: Clip,
    /// The node's opacity, 0..=1 (fills already fold their own paint opacity in).
    pub opacity: f32,
}

/// A tessellated path in **device pixels** (the document transform is applied when the scene is
/// built, so the shader needs no matrix for these). Used for any fill or stroke the SDF shapes cannot
/// cover: vector networks, booleans, lines, and every stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshNode {
    pub verts: std::sync::Arc<[[f32; 2]]>,
    /// Triangles into `verts`; a non-empty list means at least one triangle.
    pub indices: std::sync::Arc<[u32]>,
    /// The paint, straight sRGB; the renderer premultiplies and folds `opacity` in.
    pub color: Rgba,
    /// The node's opacity, 0..=1.
    pub opacity: f32,
    pub clip: Clip,
}

/// A shadow cast by an SDF shape: the shape's silhouette, offset, blurred and tinted. A drop shadow
/// paints **before** the shape (it sits behind the node); an inner shadow paints **after** the fills
/// and is masked to the shape's own coverage.
///
/// Only shapes whose silhouette is exactly their box outline are eligible (a filled or empty
/// primitive with no stroke); anything else is recorded as unsupported, because the CPU blurs the
/// node's rendered pixels and a box would not be the same alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadowNode {
    /// The caster's box in document units and local → device matrix, like [`ShapeNode`].
    pub mat: [f32; 6],
    pub size: [f32; 2],
    pub shape: ShapeKind,
    /// Offset in **device pixels** (the document offset times the node's device scale).
    pub offset: [f32; 2],
    /// Gaussian-equivalent blur radius in device pixels (the CPU's `radius / 2 * scale`).
    pub sigma: f32,
    /// The tint, straight sRGB (its alpha is the shadow's strength).
    pub color: Rgba,
    pub clip: Clip,
    /// Inside the shape (`InnerShadow`) rather than behind it (`DropShadow`).
    pub inner: bool,
}

/// A layer blur of an SDF box: the box's coverage blurred by the CPU's box approximation and repainted
/// with the box's own fill. Painted in the box's place (the fill is not drawn sharp), before the node's
/// children and stroke, which a blurred layer would have included (this build only takes boxes whose
/// fills are the whole layer).
#[derive(Clone, Debug, PartialEq)]
pub struct LayerBlurNode {
    /// The box in document units and local → device matrix, like [`ShapeNode`].
    pub mat: [f32; 6],
    pub size: [f32; 2],
    pub shape: ShapeKind,
    /// Gaussian-equivalent blur radius in device pixels (the CPU's `radius / 2 * scale`).
    pub sigma: f32,
    /// The fill, straight sRGB, its alpha already folded with the node's opacity.
    pub color: Rgba,
    pub clip: Clip,
}

/// A progressive layer blur: the box is blurred with a different sigma along a line in its own
/// bounds (`s0` at `start`, `s1` at `end`), the CPU's eight-level weighted mix.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressiveBlurNode {
    /// The box in document units and local → device matrix, like [`ShapeNode`].
    pub mat: [f32; 6],
    pub size: [f32; 2],
    pub shape: ShapeKind,
    /// The two endpoint sigmas in device pixels (the CPU's `radius / 2 * scale`).
    pub s0: f32,
    pub s1: f32,
    /// The ramp endpoints in device pixels, derived from the blur's unit-box points.
    pub start: [f32; 2],
    pub end: [f32; 2],
    /// The CPU's `levels(lo, hi)`; one entry for a degenerate ramp.
    pub levels: Vec<f32>,
    /// The fill, straight sRGB, its alpha already folded with the node's opacity.
    pub color: Rgba,
    pub clip: Clip,
}

/// A background blur: the backdrop under the box, blurred and shown through the box before the
/// node's own fills and children paint over it.
#[derive(Clone, Debug, PartialEq)]
pub struct BackdropNode {
    /// The box in document units and local → device matrix, like [`ShapeNode`].
    pub mat: [f32; 6],
    pub size: [f32; 2],
    pub shape: ShapeKind,
    /// Gaussian-equivalent blur radius in device pixels (the CPU's `radius / 2 * scale`).
    pub sigma: f32,
    pub clip: Clip,
}

/// Which colour rule the grains use: one colour, one of two by the hash's top bit, or a colour per
/// grain from three 16-bit hash fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrainMode {
    Mono,
    Duo,
    Multi,
}

/// A grain layer: `density` of the cells of `cell` (document units) show a colour over the node's
/// own fill, at the fill's alpha (the CPU's `grain::grains`).
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseNode {
    pub mat: [f32; 6],
    pub size: [f32; 2],
    pub shape: ShapeKind,
    pub clip: Clip,
    /// The node id the CPU hashes cells with.
    pub seed: u32,
    pub density: f32,
    /// Cell size in document units (the CPU's twice the grain size).
    pub cell: [f32; 2],
    /// Mono and Duo: the grain colour, straight sRGB.
    pub color: Rgba,
    /// Duo: the other grains' colour, straight sRGB.
    pub secondary: Rgba,
    /// Multi: the grains' alpha; ignored by Mono and Duo.
    pub opacity: f32,
    pub mode: GrainMode,
    /// The fill's alpha (paint × node opacity): the layer alpha the grains ride on.
    pub fill_alpha: f32,
}

/// A texture layer: each destination pixel is replaced by the pixel of the node's own fill at a
/// hashed whole-pixel offset (the CPU's `grain::texture`), so edges turn rough and grainy.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureNode {
    pub mat: [f32; 6],
    pub size: [f32; 2],
    pub shape: ShapeKind,
    pub clip: Clip,
    /// The node id the CPU hashes cells with (XOR 0x7E57).
    pub seed: u32,
    /// Scatter distance in device pixels (the CPU's `radius * scale`, clamped to 2048).
    pub radius: f32,
    /// The CPU's `clip_to_shape`: multiply the result by the layer alpha at the destination.
    pub clip_to_shape: bool,
    /// Cell size in document units (twice the texture size).
    pub cell: [f32; 2],
    /// The fill colour, straight sRGB; the shader premultiplies it by its own alpha and coverage.
    pub fill: Rgba,
}

/// A group whose children are composited into an offscreen layer and then painted at `opacity`.
///
/// This is the painter's-order escape hatch for a group that cannot be flattened onto one leaf:
/// the children draw into a transparent full-frame layer, and one composite multiplies the layer
/// alpha exactly once. The layer content must not read the backdrop (no background blur), because
/// inside an isolated layer there is no document behind it.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupNode {
    pub items: Vec<Item>,
    /// The group's alpha, straight (not folded into the children).
    pub opacity: f32,
    /// How the resolved layer composits over the frame. `Normal`/`PassThrough` is plain
    /// source-over; the other modes read the frame under the layer and blend in `blend.wgsl`.
    pub blend: BlendMode,
}

/// One thing to draw, in painter's order: an SDF shape, a tessellated path, a shadow, a layer blur,
/// a background blur, a mono grain layer or a texture layer.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Shape(ShapeNode),
    Mesh(MeshNode),
    Shadow(ShadowNode),
    Blur(LayerBlurNode),
    ProgressiveBlur(ProgressiveBlurNode),
    Backdrop(BackdropNode),
    Noise(NoiseNode),
    Texture(TextureNode),
    Group(GroupNode),
    Mask(MaskNode),
}

/// How a mask layer's pixels turn into coverage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskKind {
    Alpha,
    Luminance,
}

/// A mask run: the siblings after a mask node, clipped through the mask layer's alpha or
/// luminance. Both sides are already full scenes; the renderer draws each into its own layer and
/// composites the content through the mask once.
#[derive(Clone, Debug, PartialEq)]
pub struct MaskNode {
    pub content: Vec<Item>,
    pub mask: Vec<Item>,
    pub kind: MaskKind,
}

/// A feature this milestone cannot draw. Its presence sends the **whole frame** to the CPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    /// A short stable key, e.g. `"stroke"`, `"effect"`, `"text"`.
    pub feature: &'static str,
    /// Where and why, for the log and the tests.
    pub detail: String,
}

fn count_items(items: &[Item], pred: &dyn Fn(&Item) -> bool) -> usize {
    let mut count = 0;
    for item in items {
        if pred(item) {
            count += 1;
        }
        match item {
            Item::Group(group) => count += count_items(&group.items, pred),
            Item::Mask(mask) => {
                count += count_items(&mask.content, pred);
                count += count_items(&mask.mask, pred);
            }
            _ => {}
        }
    }
    count
}

/// A frame to draw: size, background, draw items and the list of things it cannot draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    /// The background, straight (non-premultiplied) sRGB.
    pub clear: Rgba,
    pub items: Vec<Item>,
    unsupported: Vec<Unsupported>,
}

impl Scene {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, clear: Rgba::TRANSPARENT, items: Vec::new(), unsupported: Vec::new() }
    }

    pub fn with_clear(width: u32, height: u32, clear: Rgba) -> Self {
        Self { width, height, clear, items: Vec::new(), unsupported: Vec::new() }
    }

    /// Collapse a run of consecutive, same-paint, axis-aligned plain rectangles that **abut** along an
    /// edge. Two separate SDF rectangles each return partial coverage on the shared pixel; source-over
    /// then leaves `1 - (1-a)(1-b)` instead of full coverage, a one-pixel seam at fractional zoom.
    /// Painting the union once has no internal edge, matching the CPU's merged fill.
    ///
    /// A union that is still a rectangle stays an SDF shape. Anything else — an L, a cross, a frame
    /// with a hole, unequal spans — becomes one tessellated mesh so the internal edges disappear too.
    /// Rects join only through another rect of the same run; a gap, a positive-area overlap, a
    /// different paint, clip or opacity, or any item in between ends the run and keeps painter's
    /// order.
    pub fn merge_adjacent_plain_rectangles(&mut self) {
        const EPS: f32 = 1e-3;

        fn plain(shape: &ShapeNode) -> Option<([f32; 4], Rgba)> {
            let ShapeKind::RoundedRect { radii } = shape.shape else { return None };
            let Paint::Solid(color) = &shape.paint else { return None };
            if radii.iter().any(|r| *r > 0.0) || shape.mat[1].abs() > EPS || shape.mat[2].abs() > EPS {
                return None;
            }
            let (sx, sy) = (shape.mat[0], shape.mat[3]);
            if sx <= 0.0 || sy <= 0.0 {
                return None;
            }
            let rect = [shape.mat[4], shape.mat[5], shape.size[0] * sx, shape.size[1] * sy];
            (rect[2] > 0.0 && rect[3] > 0.0 && rect.iter().all(|v| v.is_finite())).then_some((rect, *color))
        }

        fn overlap(a: [f32; 4], b: [f32; 4]) -> (f32, f32) {
            let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
            let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
            (w, h)
        }

        /// Touches along one edge with a positive-length shared span. A corner-only touch does not
        /// count, and a positive-area overlap is rejected separately: compositing two translucent
        /// copies over each other must stay two paints.
        fn abuts(a: [f32; 4], b: [f32; 4]) -> bool {
            let (w, h) = overlap(a, b);
            let x_touch = (a[0] + a[2] - b[0]).abs() <= EPS || (b[0] + b[2] - a[0]).abs() <= EPS;
            let y_touch = (a[1] + a[3] - b[1]).abs() <= EPS || (b[1] + b[3] - a[1]).abs() <= EPS;
            (x_touch && h > EPS) || (y_touch && w > EPS)
        }

        fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
            let (w, h) = overlap(a, b);
            w > EPS && h > EPS
        }

        fn edges(mut values: Vec<f32>) -> Vec<f32> {
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            values.dedup_by(|a, b| (*a - *b).abs() <= EPS);
            values
        }

        /// The rectilinear union on a compressed grid. `covered` is row-major over `xs × ys` cells.
        struct Union {
            xs: Vec<f32>,
            ys: Vec<f32>,
            covered: Vec<bool>,
        }

        impl Union {
            fn new(rects: &[[f32; 4]]) -> Self {
                let xs = edges(rects.iter().flat_map(|r| [r[0], r[0] + r[2]]).collect());
                let ys = edges(rects.iter().flat_map(|r| [r[1], r[1] + r[3]]).collect());
                let cols = xs.len() - 1;
                let mut covered = vec![false; cols * (ys.len() - 1)];
                for r in rects {
                    // Every rect edge is in `xs`/`ys`; the cell range inside it is a contiguous slab.
                    let x0 = xs.partition_point(|v| *v < r[0] - EPS);
                    let x1 = (xs.partition_point(|v| *v <= r[0] + r[2] + EPS) - 1).min(cols);
                    let y0 = ys.partition_point(|v| *v < r[1] - EPS);
                    let y1 = (ys.partition_point(|v| *v <= r[1] + r[3] + EPS) - 1).min(ys.len() - 1);
                    for row in y0..y1 {
                        covered[row * cols + x0..row * cols + x1].fill(true);
                    }
                }
                Self { xs, ys, covered }
            }

            fn is_rectangle(&self) -> bool {
                self.covered.iter().all(|c| *c)
            }

            /// One quad per horizontal run of covered cells; a hole stays uncovered.
            fn quads(&self) -> Vec<[f32; 4]> {
                let cols = self.xs.len() - 1;
                let mut quads = Vec::new();
                for row in 0..self.ys.len() - 1 {
                    let mut run: Option<usize> = None;
                    for col in 0..=cols {
                        let covered = col < cols && self.covered[row * cols + col];
                        match (run, covered) {
                            (None, true) => run = Some(col),
                            (Some(start), false) => {
                                quads.push([self.xs[start], self.ys[row], self.xs[col] - self.xs[start], self.ys[row + 1] - self.ys[row]]);
                                run = None;
                            }
                            _ => {}
                        }
                    }
                }
                quads
            }
        }

        fn mesh(quads: &[[f32; 4]], color: Rgba, opacity: f32, clip: Clip) -> MeshNode {
            let mut verts = Vec::with_capacity(quads.len() * 4);
            let mut indices = Vec::with_capacity(quads.len() * 6);
            for [x, y, w, h] in quads.iter().copied() {
                let base = verts.len() as u32;
                verts.extend_from_slice(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]]);
                indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            MeshNode { verts: std::sync::Arc::from(verts), indices: std::sync::Arc::from(indices), color, opacity, clip }
        }

        let old = std::mem::take(&mut self.items);
        let mut out = Vec::with_capacity(old.len());
        let mut i = 0;
        while i < old.len() {
            let Item::Shape(seed) = &old[i] else {
                out.push(old[i].clone());
                i += 1;
                continue;
            };
            let Some((seed_rect, seed_color)) = plain(seed) else {
                out.push(old[i].clone());
                i += 1;
                continue;
            };
            let mut rects = vec![seed_rect];
            let mut j = i + 1;
            while j < old.len() {
                let Item::Shape(next) = &old[j] else { break };
                let Some((next_rect, _)) = plain(next) else { break };
                if seed.paint != next.paint || seed.clip != next.clip || seed.opacity != next.opacity {
                    break;
                }
                if rects.iter().any(|rect| overlaps(*rect, next_rect)) || !rects.iter().any(|rect| abuts(*rect, next_rect)) {
                    break;
                }
                rects.push(next_rect);
                j += 1;
            }
            if rects.len() == 1 {
                out.push(old[i].clone());
            } else {
                let union = Union::new(&rects);
                let (x, y) = (union.xs[0], union.ys[0]);
                let w = union.xs[union.xs.len() - 1] - x;
                let h = union.ys[union.ys.len() - 1] - y;
                if union.is_rectangle() {
                    out.push(Item::Shape(ShapeNode {
                        mat: [1.0, 0.0, 0.0, 1.0, x, y],
                        size: [w, h],
                        shape: ShapeKind::RoundedRect { radii: [0.0; 4] },
                        paint: Paint::Solid(seed_color),
                        clip: seed.clip,
                        opacity: seed.opacity,
                    }));
                } else {
                    out.push(Item::Mesh(mesh(&union.quads(), seed_color, seed.opacity, seed.clip)));
                }
            }
            i = j;
        }
        self.items = out;
    }

    pub fn push(&mut self, node: ShapeNode) {
        self.items.push(Item::Shape(node));
    }

    pub fn push_mesh(&mut self, mesh: MeshNode) {
        self.items.push(Item::Mesh(mesh));
    }

    pub fn push_shadow(&mut self, shadow: ShadowNode) {
        self.items.push(Item::Shadow(shadow));
    }

    /// A blurred layer in the box's place.
    pub fn push_blur(&mut self, blur: LayerBlurNode) {
        self.items.push(Item::Blur(blur));
    }

    /// A progressive blurred layer in the box's place.
    pub fn push_progressive_blur(&mut self, blur: ProgressiveBlurNode) {
        self.items.push(Item::ProgressiveBlur(blur));
    }

    /// A blurred backdrop under the box, painted before the node's own content.
    pub fn push_backdrop(&mut self, backdrop: BackdropNode) {
        self.items.push(Item::Backdrop(backdrop));
    }

    /// A mono grain layer over the node's fill.
    pub fn push_noise(&mut self, noise: NoiseNode) {
        self.items.push(Item::Noise(noise));
    }

    /// A rough-edge texture over the node's fill.
    pub fn push_texture(&mut self, texture: TextureNode) {
        self.items.push(Item::Texture(texture));
    }

    /// A group's children, composited once at `opacity`.
    pub fn push_group(&mut self, group: GroupNode) {
        self.items.push(Item::Group(group));
    }

    pub fn push_mask(&mut self, mask: MaskNode) {
        self.items.push(Item::Mask(mask));
    }

    /// Carry the reasons a nested group cannot draw into this frame's list.
    pub fn absorb_unsupported(&mut self, other: &[Unsupported]) {
        self.unsupported.extend_from_slice(other);
    }

    /// The SDF shapes, in draw order (tests and the log use this; the renderer walks `items`).
    pub fn shapes(&self) -> impl Iterator<Item = &ShapeNode> {
        fn collect<'a>(items: &'a [Item], out: &mut Vec<&'a ShapeNode>) {
            for item in items {
                match item {
                    Item::Shape(shape) => out.push(shape),
                    Item::Group(group) => collect(&group.items, out),
                    Item::Mask(mask) => {
                        collect(&mask.content, out);
                        collect(&mask.mask, out);
                    }
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        collect(&self.items, &mut out);
        out.into_iter()
    }

    /// How many tessellated paths the frame carries.
    pub fn mesh_count(&self) -> usize {
        count_items(&self.items, &|item| matches!(item, Item::Mesh(_)))
    }

    /// How many drop/inner shadows the frame carries.
    pub fn shadow_count(&self) -> usize {
        count_items(&self.items, &|item| matches!(item, Item::Shadow(_)))
    }

    /// How many layer blurs the frame carries.
    pub fn blur_count(&self) -> usize {
        count_items(&self.items, &|item| matches!(item, Item::Blur(_) | Item::ProgressiveBlur(_)))
    }

    pub fn progressive_blur_count(&self) -> usize {
        count_items(&self.items, &|item| matches!(item, Item::ProgressiveBlur(_)))
    }

    pub fn backdrop_count(&self) -> usize {
        count_items(&self.items, &|item| matches!(item, Item::Backdrop(_)))
    }

    pub fn noise_count(&self) -> usize {
        count_items(&self.items, &|item| matches!(item, Item::Noise(_)))
    }

    pub fn texture_count(&self) -> usize {
        count_items(&self.items, &|item| matches!(item, Item::Texture(_)))
    }

    pub fn group_count(&self) -> usize {
        count_items(&self.items, &|item| matches!(item, Item::Group(_)))
    }

    pub fn mask_count(&self) -> usize {
        count_items(&self.items, &|item| matches!(item, Item::Mask(_)))
    }

    /// Whether the frame reads pixels outside its own bounds (blur, shadow, backdrop) or anchors
    /// a pattern to device coordinates (grain, texture). A tiled export would create seams or
    /// shifted patterns, so those frames are rendered whole and fall back to the CPU when they
    /// are larger than the device limit.
    pub fn has_tile_unsafe_effects(&self) -> bool {
        count_items(&self.items, &|item| {
            matches!(item, Item::Shadow(_) | Item::Blur(_) | Item::ProgressiveBlur(_) | Item::Backdrop(_) | Item::Noise(_) | Item::Texture(_))
        }) > 0
    }

    /// Record a feature this frame uses that this milestone cannot draw.
    pub fn unsupported(&mut self, feature: &'static str, detail: impl Into<String>) {
        self.unsupported.push(Unsupported { feature, detail: detail.into() });
    }

    pub fn unsupported_features(&self) -> &[Unsupported] {
        &self.unsupported
    }

    /// Whether every feature in this frame is expressible (the renderer's precondition).
    pub fn is_supported(&self) -> bool {
        self.unsupported.is_empty()
    }

    /// The scene when it is fully expressible, else the reasons it is not.
    pub fn into_supported(self) -> Result<Self, Vec<Unsupported>> {
        if self.unsupported.is_empty() { Ok(self) } else { Err(self.unsupported) }
    }
}

/// The document space → device pixel mapping a canvas or export asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Document coordinate shown at the top-left pixel.
    pub origin: (f64, f64),
    /// Device pixels per document unit.
    pub scale: f64,
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    /// The viewport a canvas render asks for: page origin, zoom and device-pixel size.
    pub fn from_view(view: &figcraft_render::View) -> Self {
        Self { origin: view.origin, scale: view.scale, width: view.width, height: view.height }
    }

    pub fn affine(&self) -> Affine {
        Affine::scale(self.scale) * Affine::translate((-self.origin.0, -self.origin.1))
    }

    /// The area this viewport shows, in document units.
    pub fn page_rect(&self) -> figcraft_geom::Rect {
        let s = self.scale.max(1e-9);
        figcraft_geom::Rect::new(self.origin.0, self.origin.1, self.origin.0 + f64::from(self.width) / s, self.origin.1 + f64::from(self.height) / s)
    }
}

/// An affine transform as the six floats the instance buffer carries.
pub fn matrix(affine: Affine) -> [f32; 6] {
    let [a, b, c, d, e, f] = affine.as_coeffs();
    [a as f32, b as f32, c as f32, d as f32, e as f32, f as f32]
}

/// How uniform the linear part is: a similarity (rotation + uniform scale + translation) maps a circle
/// to a circle, which is what the SDF's circular corner radii and round joins assume. A skew or a
/// non-uniform scale does not, so a rounded shape under one is a fallback.
pub fn is_similarity(m: [f32; 6]) -> bool {
    let (a, b, c, d) = (m[0], m[1], m[2], m[3]);
    let lx = (a * a + b * b).sqrt();
    let ly = (c * c + d * d).sqrt();
    let longest = lx.max(ly).max(1e-6);
    (lx - ly).abs() <= longest * 1e-3 && (a * c + b * d).abs() <= longest * longest * 1e-3
}

/// Whether a frame may use layers at all: this milestone draws one flat pass, so a group opacity,
/// another blend mode, or a mask is a fallback.
pub fn flat_compositing(node_opacity: f32, blend: BlendMode, mask: bool) -> bool {
    node_opacity >= 0.999 && matches!(blend, BlendMode::Normal | BlendMode::PassThrough) && !mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_one_stop_gradient_is_a_solid_and_none_is_nothing() {
        let g = Paint::Gradient {
            kind: GradientKind::Linear,
            from: [0.0, 0.0],
            to: [1.0, 0.0],
            width: [0.0, 1.0],
            stops: vec![Stop { at: 0.5, color: Rgba::new(1.0, 0.0, 0.0, 1.0) }],
        };
        assert!(matches!(g.normalised(), Some(Paint::Solid(c)) if c.r == 1.0));
        let empty = Paint::Gradient { kind: GradientKind::Radial, from: [0.5, 0.5], to: [0.5, 1.0], width: [1.0, 0.5], stops: vec![] };
        assert!(empty.normalised().is_none());
    }

    #[test]
    fn clips_intersect_and_an_empty_one_stays_empty() {
        let a = Clip([0.0, 0.0, 10.0, 10.0], [0.0; 4]);
        let b = a.intersect([5.0, 2.0, 20.0, 20.0]);
        assert_eq!(b.0, [5.0, 2.0, 5.0, 8.0]);
        let empty = a.intersect([20.0, 20.0, 5.0, 5.0]);
        assert_eq!(empty.0[2], 0.0);
    }

    #[test]
    fn similarities_are_told_apart_from_skews_and_squashes() {
        assert!(is_similarity(matrix(Affine::rotate(0.4) * Affine::scale(3.0))));
        assert!(!is_similarity(matrix(Affine::scale_non_uniform(2.0, 1.0))));
    }

    #[test]
    fn a_scene_with_an_unsupported_feature_refuses_to_render() {
        let mut s = Scene::new(4, 4);
        s.unsupported("text", "a text layer");
        assert!(!s.is_supported());
        assert_eq!(s.into_supported().unwrap_err()[0].feature, "text");
    }

    fn plain(x: f32, y: f32, w: f32, h: f32, paint: Paint) -> ShapeNode {
        ShapeNode {
            mat: [1.0, 0.0, 0.0, 1.0, x, y],
            size: [w, h],
            shape: ShapeKind::RoundedRect { radii: [0.0; 4] },
            paint,
            clip: Clip::NONE,
            opacity: 1.0,
        }
    }

    #[test]
    fn abutting_same_paint_rectangles_merge_along_their_edge() {
        let green = Paint::Solid(Rgba::new(0.2, 0.6, 0.4, 1.0));
        let mut s = Scene::new(100, 100);
        s.push(plain(10.0, 20.0, 30.0, 40.0, green.clone()));
        s.push(plain(40.0, 20.0, 20.0, 40.0, green));
        s.merge_adjacent_plain_rectangles();
        assert_eq!(s.items.len(), 1);
        let [Item::Shape(merged)] = s.items.as_slice() else {
            assert_eq!(s.items.len(), 1, "one shape");
            return;
        };
        assert_eq!((merged.mat[4], merged.mat[5]), (10.0, 20.0));
        assert_eq!(merged.size, [50.0, 40.0]);
    }

    #[test]
    fn unequal_spans_merge_into_one_mesh() {
        let green = Paint::Solid(Rgba::new(0.2, 0.6, 0.4, 1.0));
        let mut s = Scene::new(100, 100);
        s.push(plain(10.0, 20.0, 30.0, 40.0, green.clone()));
        s.push(plain(40.0, 20.0, 20.0, 30.0, green));
        s.merge_adjacent_plain_rectangles();
        assert_eq!(s.items.len(), 1);
        let [Item::Mesh(mesh)] = s.items.as_slice() else {
            assert_eq!(s.items.len(), 1, "one mesh");
            return;
        };
        assert!(mesh.indices.len() >= 12, "the L is at least two quads");
    }

    #[test]
    fn only_plain_paint_merges() {
        let green = Paint::Solid(Rgba::new(0.2, 0.6, 0.4, 1.0));
        let gradient = Paint::Gradient {
            kind: GradientKind::Linear,
            from: [0.0, 0.0],
            to: [1.0, 0.0],
            width: [0.0, 1.0],
            stops: vec![Stop { at: 0.0, color: Rgba::new(0.0, 0.0, 0.0, 1.0) }, Stop { at: 1.0, color: Rgba::new(1.0, 1.0, 1.0, 1.0) }],
        };
        let mut gradient_edge = Scene::new(100, 100);
        gradient_edge.push(plain(10.0, 20.0, 30.0, 40.0, gradient.clone()));
        gradient_edge.push(plain(40.0, 20.0, 30.0, 40.0, gradient));
        gradient_edge.merge_adjacent_plain_rectangles();
        assert_eq!(gradient_edge.items.len(), 2);

        let mut clipped = Scene::new(100, 100);
        clipped.push(plain(10.0, 20.0, 30.0, 40.0, green.clone()));
        let mut other = plain(40.0, 20.0, 30.0, 40.0, green);
        other.clip = Clip([10.0, 10.0, 50.0, 50.0], [0.0; 4]);
        clipped.push(other);
        clipped.merge_adjacent_plain_rectangles();
        assert_eq!(clipped.items.len(), 2);
    }

    #[test]
    fn a_gap_keeps_two_rectangles() {
        let green = Paint::Solid(Rgba::new(0.2, 0.6, 0.4, 1.0));
        let mut s = Scene::new(100, 100);
        s.push(plain(10.0, 20.0, 20.0, 40.0, green.clone()));
        s.push(plain(40.0, 20.0, 20.0, 40.0, green));
        s.merge_adjacent_plain_rectangles();
        assert_eq!(s.items.len(), 2);
    }

    #[test]
    fn overlapping_same_paint_rectangles_stay_separate() {
        let green = Paint::Solid(Rgba::new(0.2, 0.6, 0.4, 0.5));
        let mut s = Scene::new(100, 100);
        s.push(plain(10.0, 20.0, 30.0, 40.0, green.clone()));
        s.push(plain(30.0, 20.0, 30.0, 40.0, green));
        s.merge_adjacent_plain_rectangles();
        assert_eq!(s.items.len(), 2, "two translucent fills overlap and must compose, not become one");
    }

    #[test]
    fn stacked_same_paint_rectangles_merge_too() {
        let green = Paint::Solid(Rgba::new(0.2, 0.6, 0.4, 1.0));
        let mut s = Scene::new(100, 100);
        s.push(plain(10.0, 20.0, 30.0, 40.0, green.clone()));
        s.push(plain(10.0, 60.0, 30.0, 20.0, green));
        s.merge_adjacent_plain_rectangles();
        assert_eq!(s.items.len(), 1);
        let [Item::Shape(merged)] = s.items.as_slice() else {
            assert_eq!(s.items.len(), 1, "one shape");
            return;
        };
        assert_eq!(merged.size, [30.0, 60.0]);
    }
}
