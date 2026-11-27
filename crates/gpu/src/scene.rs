//! The GPU renderer's drawing IR.
//!
//! Deliberately plain data with **no wgpu and no `Document` inside**: the document front end
//! ([`crate::doc_scene`]) fills it, a test can fill it by hand, and the renderer only reads it. That
//! separation is what makes the CPU↔GPU diff honest — both sides start from the same `Document` and
//! meet again at pixels, never at this struct.
//!
//! Every node is a **filled shape**. A shape with three fills is three nodes, in Figma's order. Strokes,
//! text, paths, effects, masks and non-normal blending are later milestones; when a frame uses them,
//! [`crate::doc_scene`] records an [`Unsupported`] entry and the renderer refuses the whole frame
//! (the caller draws it on the CPU instead).

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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip(pub [f32; 4]);

impl Clip {
    pub const NONE: Self = Self([-1.0, -1.0, -1.0, -1.0]);

    pub fn is_none(self) -> bool {
        self.0[2] < 0.0
    }

    /// The intersection of this clip and `rect` (`[x, y, w, h]`, device pixels).
    pub fn intersect(self, rect: [f32; 4]) -> Self {
        if rect[2] <= 0.0 || rect[3] <= 0.0 {
            // An empty rectangle clips everything away; a zero-width clip still discards every
            // fragment, which is exactly "nothing of this subtree shows".
            return Self([rect[0], rect[1], 0.0, 0.0]);
        }
        if self.is_none() {
            return Self(rect);
        }
        let x0 = self.0[0].max(rect[0]);
        let y0 = self.0[1].max(rect[1]);
        let x1 = (self.0[0] + self.0[2]).min(rect[0] + rect[2]);
        let y1 = (self.0[1] + self.0[3]).min(rect[1] + rect[3]);
        if x1 <= x0 || y1 <= y0 { Self([x0, y0, 0.0, 0.0]) } else { Self([x0, y0, x1 - x0, y1 - y0]) }
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
}

/// A feature this milestone cannot draw. Its presence sends the **whole frame** to the CPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    /// A short stable key, e.g. `"stroke"`, `"effect"`, `"text"`.
    pub feature: &'static str,
    /// Where and why, for the log and the tests.
    pub detail: String,
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

    /// The SDF shapes, in draw order (tests and the log use this; the renderer walks `items`).
    pub fn shapes(&self) -> impl Iterator<Item = &ShapeNode> {
        self.items.iter().filter_map(|item| match item {
            Item::Shape(shape) => Some(shape),
            Item::Mesh(_) | Item::Shadow(_) | Item::Blur(_) | Item::ProgressiveBlur(_) | Item::Backdrop(_) | Item::Noise(_) | Item::Texture(_) => {
                None
            }
        })
    }

    /// How many tessellated paths the frame carries.
    pub fn mesh_count(&self) -> usize {
        self.items.iter().filter(|item| matches!(item, Item::Mesh(_))).count()
    }

    /// How many drop/inner shadows the frame carries.
    pub fn shadow_count(&self) -> usize {
        self.items.iter().filter(|item| matches!(item, Item::Shadow(_))).count()
    }

    /// How many layer blurs the frame carries.
    pub fn blur_count(&self) -> usize {
        self.items.iter().filter(|item| matches!(item, Item::Blur(_) | Item::ProgressiveBlur(_))).count()
    }

    pub fn progressive_blur_count(&self) -> usize {
        self.items.iter().filter(|item| matches!(item, Item::ProgressiveBlur(_))).count()
    }

    pub fn backdrop_count(&self) -> usize {
        self.items.iter().filter(|item| matches!(item, Item::Backdrop(_))).count()
    }

    pub fn noise_count(&self) -> usize {
        self.items.iter().filter(|item| matches!(item, Item::Noise(_))).count()
    }

    pub fn texture_count(&self) -> usize {
        self.items.iter().filter(|item| matches!(item, Item::Texture(_))).count()
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
        let a = Clip([0.0, 0.0, 10.0, 10.0]);
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
}
