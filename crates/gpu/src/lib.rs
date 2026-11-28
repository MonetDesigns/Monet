//! Monet's GPU render channel — the parallel path to the CPU renderer (`figcraft-render`, tiny-skia).
//!
//! The contract, in one sentence: **the same `Document` produces the same pixels on either path, and
//! a frame the GPU cannot draw faithfully falls back to the CPU instead of drawing something almost
//! right.**
//!
//! Pieces:
//!
//! * [`backend`] — does this machine have a GPU worth drawing on? (adapter probe, hardware vs software)
//! * [`prefs`] — the user's choice (`auto` / `gpu` / `cpu`) and what it resolves to right now.
//! * [`scene`] — the drawing IR: shapes, paints, clips, group and mask layers, and the explicit
//!   list of features a frame uses that this path cannot express.
//! * [`doc_scene`] — `Document` → [`scene::Scene`], the only place document semantics are read.
//!   It implements SDF fills/strokes, text and paths as meshes, layer/backdrop/progressive blurs,
//!   drop and inner shadows, noise/texture effects, isolated group layers (opacity and non-Normal
//!   blends), alpha/luminance masks, and tiled document export.
//! * [`render`] (feature `wgpu`) — device, pipelines, offscreen layers, draw, readback. Frames
//!   with nested group masks, unbounded blur radii, over-budget offscreen layers, or effects that
//!   read outside the frame edge return a [`render::RenderError`] and fall back to the CPU.
//! * [`diff`] — CPU↔GPU pixel comparison for the regression tests.
//!
//! See `docs/gpu-pipeline-plan.md` for the staged plan. All document-level effects are transitional
//! implementations: built-in effects are planned for deprecation and future effects move to
//! external WGSL, so this branch adds no more.

pub mod backend;
pub mod diff;
pub mod doc_scene;
pub mod geometry;
pub mod prefs;
pub mod scene;

#[cfg(feature = "wgpu")]
pub mod render;

pub use backend::{Capability, DeviceClass};
pub use prefs::{Effective, RenderBackend};
pub use scene::{
    BackdropNode, GrainMode, Item, LayerBlurNode, MeshNode, NoiseNode, Paint, ProgressiveBlurNode, Scene, ShadowNode, ShapeKind, ShapeNode,
    TextureNode, Viewport,
};
