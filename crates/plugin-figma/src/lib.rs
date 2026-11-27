//! Figma plugin compatibility for FigCraft.
//!
//! FigCraft's native plugins are WebAssembly modules that talk JSON to the command registry
//! ([`figcraft_plugin`]). Figma's plugins are JavaScript against a synchronous object graph: a
//! `figma` global, node objects whose properties are getters and setters, `figma.showUI(__html__)`
//! with an HTML frame, `figma.clientStorage`, `figma.commitUndo`.
//!
//! This crate holds the part between those two worlds that is **not** a JavaScript engine:
//!
//! - [`manifest`]: a Figma `manifest.json`, parsed and checked (`api` `1.x`, a `.js` main, a
//!   `.html` UI, `editorType`, `documentAccess`, `networkAccess`, parameters, relaunch buttons).
//! - [`ids`]: the bijective mapping between FigCraft's `u64` layer ids and the strings Figma hands
//!   plugins (`"12:34"`), stable across saving and reopening.
//! - [`props`]: the property table (Figma property name → `node.set` key, kind and caveats) and
//!   the enum values that differ between the two APIs.
//! - [`ops`]: the calls a Figma operation turns into (`node.set`, `node.create`, `node.reparent`,
//!   `edit.delete`, `object.group`, …), so a proxy write becomes a batched command, not a
//!   property-by-property round trip.
//! - [`host`]: what the engine answers while a plugin runs (commands, the selection, settings, a
//!   file to save, the UI frame, `commitUndo`), so this crate never depends on `figcraft-engine`.
//! - [`runtime`]: the backend seam (`Backend`): QuickJS today, V8 later, and the invocation
//!   contract shared with the engine.
//! - [`coverage`]: the bookkeeping that turns `@figma/plugin-typings` into a measured
//!   implemented / unsupported / missing report.
//!
//! # Who owns what
//!
//! The engine keeps its policy for WebAssembly plugins: permissions, the denied command prefixes,
//! one undo step per run or callback, rollback on failure, and the sandbox limits. This crate
//! assumes all of that: it never edits the document itself and never keeps a second copy of it. A
//! backend holds one [`host::Host`] for its whole session and calls it while a script runs; the
//! engine wraps each call in its own transaction.
//!
//! # What this crate is not
//!
//! It is not a JavaScript engine, and it does not pretend to be one. [`runtime::Backend`] is the
//! seam a backend implements; the first backend (`rquickjs`) lives in its own crate so that this
//! one stays dependency-light, testable on its own, and usable from the browser build later.
//!
//! It also does not cover Figma's widget API (`useSyncedState`, JSX), which is a different runtime
//! with its own manifest fields (`containsWidget`, `widgetApi`); [`manifest::Manifest::blockers`]
//! reports it instead of half-supporting it.

pub mod coverage;
pub mod host;
pub mod ids;
pub mod manifest;
pub mod ops;
pub mod props;
pub mod runtime;

pub use host::{Host, Limits, UiOptions};
pub use ids::{IdError, figma_id, node_id};
pub use manifest::{Blocker, Manifest, ManifestError, Scope};
pub use runtime::{Backend, Event, Invocation, Outcome, RuntimeError, Script};
