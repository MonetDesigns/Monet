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
//! See `docs/gpu-pipeline-plan.md` for the staged plan (M1 shapes and paints is this commit).

pub mod backend;
pub mod diff;
pub mod doc_scene;
pub mod prefs;
pub mod scene;

#[cfg(feature = "wgpu")]
pub mod render;

pub use backend::{Capability, DeviceClass};
pub use prefs::{Effective, RenderBackend};
pub use scene::{Paint, Scene, ShapeKind, ShapeNode, Viewport};
