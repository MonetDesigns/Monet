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
    Gradient { kind: GradientKind, from: [f32; 2], to: [f32; 2], width: [f32; 2], stops: Vec<Stop> },
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
        if x1 <= x0 || y1 <= y0 {
            Self([x0, y0, 0.0, 0.0])
        } else {
            Self([x0, y0, x1 - x0, y1 - y0])
        }
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

/// A feature this milestone cannot draw. Its presence sends the **whole frame** to the CPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    /// A short stable key, e.g. `"stroke"`, `"effect"`, `"text"`.
    pub feature: &'static str,
    /// Where and why, for the log and the tests.
    pub detail: String,
}

/// A frame to draw: size, background, shapes and the list of things it cannot draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    /// The background, straight (non-premultiplied) sRGB.
    pub clear: Rgba,
    pub nodes: Vec<ShapeNode>,
    unsupported: Vec<Unsupported>,
}

impl Scene {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, clear: Rgba::TRANSPARENT, nodes: Vec::new(), unsupported: Vec::new() }
    }

    pub fn with_clear(width: u32, height: u32, clear: Rgba) -> Self {
        Self { width, height, clear, nodes: Vec::new(), unsupported: Vec::new() }
    }

    pub fn push(&mut self, node: ShapeNode) {
        self.nodes.push(node);
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
        figcraft_geom::Rect::new(
            self.origin.0,
            self.origin.1,
            self.origin.0 + f64::from(self.width) / s,
            self.origin.1 + f64::from(self.height) / s,
        )
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
