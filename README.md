<p align="center">
  <img alt="Monet" src="assets/app-icon/hicolor/256x256/apps/app.monet.design.png" width="128" height="128">
</p>

<h1 align="center">Monet</h1>

<p align="center">
  <b>A native design tool front-end built with GPUI.</b><br>
  Frames, auto layout, components and variants, vector networks, styles, variables, prototypes<br>
  and a motion timeline — driven by a Rust document engine, in an app that opens offline.
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">中文</a>
</p>

<p align="center">
  <img alt="100% Rust" src="https://img.shields.io/badge/100%25-Rust-b7410e?style=flat-square&logo=rust">
  <img alt="UI: GPUI" src="https://img.shields.io/badge/UI-GPUI-0EA5C6?style=flat-square">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-3a3a3a?style=flat-square">
  <img alt="Status: early alpha" src="https://img.shields.io/badge/status-early%20alpha-d69e2e?style=flat-square">
  <br>
  <a href="https://sonarcloud.io/project/overview?id=MonetDesigns_Monet"><img alt="Scanned by SonarCloud" src="https://img.shields.io/badge/Scanned_by-SonarCloud-4b9fd5?style=flat-square&logo=sonarqubecloud&logoColor=white"></a>
  <a href="https://coderabbit.ai"><img alt="Reviewed by CodeRabbit" src="https://img.shields.io/coderabbit/prs/github/MonetDesigns/Monet?style=flat-square&label=Reviewed%20by%20CodeRabbit&logo=coderabbit"></a>
</p>

<br>

> [!NOTE]
> **This is the GPUI front-end.** The document engine and the original egui front-end are developed
> in the upstream project, **[storytold/figcraft](https://github.com/storytold/figcraft)**. Engine
> changes belong upstream; this repository only accepts GPUI UI work. See
> [CONTRIBUTING.md](CONTRIBUTING.md).

<p align="center">
  <a href="#a-look-around">A look around</a> ·
  <a href="#what-this-is">What this is</a> ·
  <a href="#features">Features</a> ·
  <a href="#get-started">Get started</a> ·
  <a href="#contributing">Contributing</a>
</p>

<br>

## A look around

All screenshots below are of the running GPUI application (`cargo run -p figcraft-ui-gpui --example run`).

<table>
<tr>
<td width="50%" valign="top">
  <img src="docs/images/screenshots/01-design-light.png" alt="Monet in its light theme: the left rail and File panel list Pages and a Layers tree with a Hello text layer, auto layout frames and seams; the canvas shows an auto layout frame; the floating toolbar sits over the canvas; the Design panel on the right shows Layout, Dimensions (Hug/Hug), Resizing, Appearance and Typography sections" width="100%">
  <p align="center"><sub><b>Design panel, light theme</b>: layers, canvas and auto layout properties</sub></p>
</td>
<td width="50%" valign="top">
  <img src="docs/images/screenshots/02-design-dark.png" alt="The same Monet editor in its dark theme: dark rail, panels and Design controls, with the canvas and selection unchanged" width="100%">
  <p align="center"><sub><b>Dark theme</b>: the same document, themed interface</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
  <img src="docs/images/screenshots/05-assets.png" alt="The left File panel switched to Assets, showing the Components and Styles tabs and a search box for assets" width="100%">
  <p align="center"><sub><b>Assets</b>: components and styles library</sub></p>
</td>
<td width="50%" valign="top">
  <img src="docs/images/screenshots/04-devmode.png" alt="Developer Mode: the right panel shows the Development section for the selected layer, with the canvas and the floating toolbar visible" width="100%">
  <p align="center"><sub><b>Developer Mode</b>: inspect and hand-off controls</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
  <img src="docs/images/screenshots/06-motion-timeline.png" alt="The motion timeline at the bottom of the window: a 1.5 second looping clip on a Hello layer with a Position track, three diamond keyframes at 0, 0.75 and 1.5 seconds, and playback controls on the left" width="100%">
  <p align="center"><sub><b>Motion timeline</b>: keyframes on a looping Position track</sub></p>
</td>
<td width="50%" valign="top">
  <img src="docs/images/screenshots/07-variables.png" alt="The Variables window open over the editor, showing variable collections and their modes" width="100%">
  <p align="center"><sub><b>Variables</b>: collections and modes</sub></p>
</td>
</tr>
</table>

## What this is

Monet is a desktop design tool front-end. Its lower half is a document engine: a scene graph,
layout, vector networks, text, components, variables and prototypes. Its upper half is the
interface that draws and drives that engine.

The engine's original front-end was written with **egui**, an immediate-mode toolkit. This
repository ports that front-end to **GPUI**, the retained-mode, GPU-composited framework behind
Zed: a declarative element tree with persistent state, taffy layout, and panels, popovers and the
canvas composited as one scene on the GPU.

The port is intentionally one-for-one. Every panel, dialog, popover, menu and keyboard path is
rebuilt to match the egui front-end's layout, dimensions and behaviour, and
[`docs/gpui-egui-parity-plan.md`](docs/gpui-egui-parity-plan.md) tracks what still differs. The egui
front-end stays in the tree as the frozen reference the port is checked against; engine changes
are made upstream, not here.

## Features

<table>
  <tr>
    <td width="50%" valign="top">
      <h3>🎛️ One GPU scene</h3>
      Panels, popovers, the floating toolbar and the canvas share one render tree, so overlapping
      menus, drag previews and the motion timeline composite in the order you see on screen.
    </td>
    <td width="50%" valign="top">
      <h3>⚡ Smooth canvas</h3>
      Pan, zoom and selection stay responsive on large documents. Tiles are rasterised off the UI
      thread and streamed in, with a low-resolution overview first.
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <h3>🧱 Engine-backed editing</h3>
      Auto layout, component instances and overrides, styles, variables and modes, boolean
      operations, vector networks, prototyping and boards all live in the engine, shared with the
      upstream project.
    </td>
    <td width="50%" valign="top">
      <h3>🎬 Motion</h3>
      Keyframes, easing and a timeline under the canvas that drives the same scene the canvas
      draws, with looping clips and per-property tracks.
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <h3>🖌️ Local brushes</h3>
      Brushes are external <code>.abr</code> files imported into a local library; strokes keep the
      brush tip's soft edge and are stamped along the path.
    </td>
    <td width="50%" valign="top">
      <h3>🔒 Offline and local-first</h3>
      No account, no server and no network round trip to open a file. The UI follows the system
      language and every string comes from the catalogue.
    </td>
  </tr>
</table>

## Get started

Requires a recent stable Rust toolchain (edition 2024, `rust-version = 1.95`). On Windows the MSVC
toolchain is expected.

```sh
cargo run -p figcraft-ui-gpui --example run
```

Tests:

```sh
cargo test -p figcraft-ui-gpui --lib --features test-support
```

## Contributing

Engine and egui changes belong in **[storytold/figcraft](https://github.com/storytold/figcraft)**;
this repository accepts GPUI front-end work only. See [CONTRIBUTING.md](CONTRIBUTING.md) for the
full rule, the 1:1 parity requirement and the checks that run on every PR.

The gap to upstream is tracked in [`docs/monet-upstream-diff.md`](docs/monet-upstream-diff.md).

## Attribution and license

Monet's GPUI front-end is original work in this repository. The document engine and egui
front-end come from [storytold/figcraft](https://github.com/storytold/figcraft); see
[ATTRIBUTION.md](ATTRIBUTION.md) for third-party assets and credits.

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
Brand assets have their own terms in [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt).
