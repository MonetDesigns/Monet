# figcraft-plugin-figma

The Figma-compatibility layer for FigCraft's plugin system: everything between a Figma plugin's
`manifest.json` / `code.js` and the engine's command registry, **except** the JavaScript engine.

```text
manifest.json + code.js + ui.html
        │
        ▼  figcraft-plugin-figma-quickjs (rquickjs, one realm per session)
   figma global (node proxies, figma.*)
        │  this crate: ids, the property table, the calls a write makes
        ▼
   Host (implemented by figcraft-engine: call/get/selection/settings/UI/commitUndo)
        │
        ▼
   Session::grouped → figcraft-doc        (one undo step per callback, rollback on failure)
```

## Status

0.1.0 is the foundation of `docs/figma-plugin-compat-plan.md`, not the finished feature:

- **done** — `manifest.json` parsing and checking (API 1.x, `.js` main, `.html` UI, `editorType`,
  `documentAccess`, `networkAccess` incl. domain matching, parameters, menu, relaunch buttons,
  widget/board/Dev Mode/Slides blockers); the id mapping; the property table and enum conversion;
  the op → command translation; the `Host` and `Backend` seams; the coverage report format.
- **backend** — [`figcraft-plugin-figma-quickjs`](../plugin-figma-quickjs) implements `Backend` with
  QuickJS, owns one realm per session, and builds the `figma` global over `Host`.
- **not here** — the JavaScript engine, the HTML frame, document events, fonts, images, variables,
  components. Those live in the backend crate and the engine's integration module, in the
  milestones below.

## Modules

| Module | What it owns |
|---|---|
| `manifest` | Figma's `manifest.json`: parse, validate, scopes, blockers, network allowlist |
| `ids` | `u64` ↔ `"12:34"`, injective and stable across save/reopen; `"0:0"` is the document |
| `props` | the property table (Figma name → `node.set` key / derived), enum conversion, read-only set |
| `ops` | the command one Figma operation becomes (`node.set`, `node.create`, `node.reparent`, …) |
| `host` | what the engine answers: commands, selection, settings, files, the UI frame, `commitUndo` |
| `runtime` | the object-safe `Backend` seam and the invocation contract |
| `coverage` | `area<TAB>member<TAB>status<TAB>note` lists and the Markdown coverage report |

## Why it is its own crate

- **No engine dependency.** `Host` is a trait; the engine implements it. This crate never opens a
  `Session`, never touches a document, never keeps a second copy of one. It compiles and tests in
  seconds, and a change here can't break the editor.
- **No JavaScript engine dependency.** `Backend` is a trait. QuickJS (or V8) is a backend crate;
  swapping it changes no code here.
- **One owner, one version.** The compatibility contract (this crate) is versioned `0.x` on its own;
  the engine integration follows it, not the other way round. The plan's coverage report is the
  contract's test suite.

## Tests

```sh
cargo test -p figcraft-plugin-figma
```

The tests cover the manifest (new-style and legacy, bad paths, API versions, network allowlists,
blockers), the id round trip over the whole `u64` range, the property table (unique names, renames,
composed properties, read-only members, enum spellings), every op's JSON, and the coverage format.

## Next

1. Engine integration: a second `PluginRuntime` next to the WebAssembly one, a `Host` over the
   worker's `Session`, the import flow (manifest.json → plugin folder), the permission sheet,
   `commitUndo` plumbing.
2. More of the `figma` object in the backend: children/parent, geometry (`absoluteBoundingBox`),
   `findAll`, `clientStorage`, async and promises.
3. Coverage generator from `@figma/plugin-typings`'s `plugin-api.d.ts`.

Milestones, acceptance criteria and non-goals: `docs/figma-plugin-compat-plan.md`.
