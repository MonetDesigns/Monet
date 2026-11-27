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
//! * [`scene`] — the drawing IR: shapes, paints, clips, and the explicit list of features a frame uses
//!   that this path cannot express.
//! * [`doc_scene`] — `Document` → [`scene::Scene`], the only place document semantics are read.
//! * [`render`] (feature `wgpu`) — device, pipeline, draw, readback.
//! * [`diff`] — CPU↔GPU pixel comparison for the regression tests.
//!
//! See `docs/gpu-pipeline-plan.md` for the staged plan. Stages 1–3 (shapes and paints, paths and
//! strokes, text) and stage 4's shadows, uniform and progressive blurs, grains and rough-edge
//! texture are in; glass is out of scope (the product deprecates it), composition and the export
//! path follow.

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
