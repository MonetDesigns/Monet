# Figma plugin compatibility — plan

**Status:** proposed, 2026-11-26. This settles the "owner decision needed" line under gap 6 in
`gaps.md` (JavaScript plugins). The contract crate exists: `crates/plugin-figma`
(`figcraft-plugin-figma` 0.1.0, 36 tests, no engine and no JavaScript engine dependency). The
runtime, the UI frame and the engine integration are the milestones below.

## 1. What this commits to

1. A **second plugin runtime** next to the WebAssembly one. Figma plugins are JavaScript against a
   synchronous object graph; the existing `figcraft-plugin` ABI (manifest + `figcraft_run` + JSON
   `call`) stays exactly as it is. Nothing in `crates/plugin` or `plugins/sdk` changes.
2. A **contract crate**, `figcraft-plugin-figma`, that owns the Figma-side vocabulary: `manifest.json`,
   node ids, the property table, the calls an operation becomes, the host seam and the coverage
   report. It is versioned on its own and depends on neither the engine nor a JS engine.
3. A **runtime crate**, `figcraft-plugin-figma-quickjs`, that implements the `Backend` seam with
   QuickJS (one realm per session, interrupt-based timeout, heap cap).
4. **1:1 is a number, not a promise.** The measure is `@figma/plugin-typings` parsed into a
   `area / member / status / note` list, plus a corpus of real plugins run against fixed `.fig`
   inputs. We report the percentage and the unsupported list; we do not claim "full support" in
   prose.

Non-negotiable boundaries: the existing plugins keep working, the engine keeps owning permissions,
undo, rollback and limits, and the browser build keeps its current plugin story until a phase says
otherwise.

## 2. What upstream does (checked 2026-11-26)

- `origin/main` and this tree have **no plugin diff** at all (`crates/plugin`, `plugins/`),
  API version 6.
- `upstream/main` differs from this tree only by naming and one browser-directory paragraph:
  7 files, +14/−13 lines, no behaviour. Its plugin API is the same 6, and its docs still say
  "Plugins written for other apps (JavaScript plugins) don't run."
- No ref in the repository contains `rquickjs`, `boa_engine`, `deno_core` or QuickJS: upstream has
  not started a JavaScript runtime.
- One unmerged branch, `upstream/feat/plugin-widgets-depth` (8 commits ahead, 4521 behind, last
  touched 2026-10-13), adds widget timers (`widget_timer` host call), hover, richer panel controls
  (`plugins/sdk/src/ui.rs`) and three examples. It is a parallel take on API 4 that never merged;
  main shipped API 4 as fields + menus, then 5 (panels from widgets) and 6 (lines/SVG). It is a
  reference for timers, not an architectural change.
- Security branches (`fix/sec-plugin-*`) are all merged; no unmerged policy work.

**Conclusion:** there is nothing to adopt or wait for. This work is additive, in new crates, and
should stay out of `crates/plugin` so upstream merges stay clean. If upstream ever grows a JS
runtime, the coverage table is the thing to compare against.

## 3. Why the current interface can't simply be translated

| Difference | What breaks if ignored |
|---|---|
| Synchronous object graph (`node.fills = …`, `parent.appendChild`) | A JSON RPC per property is too slow and reads become snapshots, not live references |
| Node ids are strings (`"12:34"`) | Plugins persist ids in `pluginData` and settings; they must be stable across save/reopen |
| HTML UI frame (`showUI(__html__)`, `postMessage`) | Declarative panels can't host a plugin's own HTML/CSS/JS |
| One undo step per session, `commitUndo` splits | One step per callback gives users a different ⌘Z |
| Long-running scripts with timers and events | The synchronous command model with a fuel budget freezes the editor |
| `documentAccess: "dynamic-page"` | Synchronous getters must throw; pages load on demand |

These are the six places the design below spends its effort.

## 4. Architecture

```text
manifest.json + code.js + ui.html
        │  import: one folder (or a zip), the same entry as Figma's "Import plugin from manifest…"
        ▼
┌───────────────────────────────┐
│ figcraft-plugin-figma-quickjs │  one realm per session; interrupt = time budget; heap cap;
│  (Backend: rquickjs)          │  setTimeout/setInterval; console; no require/process/fs
└───────────────┬───────────────┘
                │  `figma` global: node proxies, figma.*
┌───────────────▼───────────────┐
│ figcraft-plugin-figma         │  ids, property table, command translation, coverage
│  (contract, this crate)       │
└───────────────┬───────────────┘
                │  Host (trait)
┌───────────────▼───────────────┐
│ figcraft-engine integration   │  policy: permissions, one undo step, rollback, limits,
│                               │  settings, UI frame, logs, notify, commitUndo
└───────────────┬───────────────┘
                ▼
          figcraft-doc
```

Where each piece lives:

| Crate / module | Contents | Status |
|---|---|---|
| `crates/plugin-figma` | manifest, ids, props, ops, host trait, backend trait, coverage | **0.1.0, done** |
| `crates/plugin-figma-quickjs` | `rquickjs` realm, `figma` global, node proxies, timers, promises | **0.1.0, M0 done**; M1 for the rest |
| `crates/engine/src/extensions/plugins.rs` | `PluginInstance::Script`, import, permissions, undo, logs | M1 |
| app UI | Manage plugins row for Figma plugins: blockers, coverage, network domains | M3 |

Deliberately **not** in the contract crate: the JS engine (a `Backend` implementation), the HTML
frame (a `Host` implementation), any document state. The crate never opens a `Session`, so its
tests are independent and a change there can't break the editor.

## 5. The contract, decision by decision

### 5.1 Manifest

- Read Figma's `manifest.json` when the folder has one and `main` ends `.js`. Keep
  `figcraft-plugin.json` for WebAssembly plugins; a folder that contains both is refused with a
  message (one runtime per folder).
- `api` must be `1.x`; anything else is refused by name. `editorType` is required; `figma` is the
  scope this layer implements first, `figjam`/`dev`/`slides` are reported as blockers, never
  half-run. `containsWidget`/`widgetApi` is the widget API: refused as a blocker, not emulated.
- `id` is namespaced `figma:<id>` for settings, logs, undo labels and the plugin manager. Two
  plugins with the same `id` are the same plugin (Figma's rule).
- Paths are relative, no `..`, no absolute paths, `main` `.js`, `ui` `.html`; the bundle is read
  before the manifest is accepted.
- `networkAccess.allowedDomains` is enforced (`none`, `*`, host, port, path prefix,
  `*.sub.example.com` strict); `devAllowedDomains` applies only to a plugin imported from a
  folder. No network field means no network.

### 5.2 Ids

`u64` (`client << 32 | counter`) → `"client:counter"`, injective on the whole range and derived
from the id alone: the same string after undo, copy, save and reopen. `"0:0"` is `figma.root`;
layer 0 is refused. Ids are not renumbered when a plugin copies a layer (the copy gets a new
engine id, as in Figma).

### 5.3 Document access

- Reads go through `Host.get` → `node.get`, narrowed by `fields`. The proxy keeps no shadow tree:
  `parent` and `children` are live references resolved at access time, otherwise `appendChild`
  followed by a read gives a stale tree.
- Writes accumulate **per node** and flush as one `node.set` before any read of that node and before
  the callback ends. One property is never one command.
- Coordinates: Figma's `x/y` are relative to the direct parent including groups; the engine's are
  relative to the parent frame, groups not counting. `appendChild`, `insertChild`, `group` and
  `ungroup` convert. This is the single most likely source of "everything is in the wrong place".
- `absoluteBoundingBox`/`absoluteTransform` need a layout pass; the proxy flushes pending writes and
  asks the engine to measure before answering.

### 5.4 Undo

Figma folds a plugin session into one undo entry by default and splits it at `figma.commitUndo()`.
The engine's one-step-per-callback is finer than that and must be merged: consecutive steps written
by the same plugin session become one entry (the engine already keeps a snapshot per step; the merge
is a history operation, not a document one). `commitUndo` closes the open entry, so the next write
starts a new one. The undo label is the plugin's `name`.

### 5.5 Threads

A plugin session runs on its own thread with its own `Session`, loaded from the document's `Arc` at
the start of each callback (the doc is copy-on-write: `s.ds()?.doc.clone()` is an `Arc` clone, and
`Arc::ptr_eq` is how the engine detects "unchanged"). At the callback boundary the result is
committed to the app's session; failure drops the copy. The UI thread never runs script code.

- Trade-off: concurrent user edits inside one callback aren't visible (Figma serializes too);
  between callbacks the plugin sees a fresh document and events (`selectionchange`, `documentchange`).
- M1 may run synchronously behind the current `plugin.run` path with a short budget to get the
  loop working; M2 moves it to the worker before any long-running plugin is supported.

### 5.6 Fonts

`figma.loadFontAsync` must be a real gate: writing `characters` or `fontName` before the font is
loaded throws, as in Figma. The font store resolves `{family, style}`; `figma.listAvailableFontsAsync`
reads the same catalog. A plugin that runs but lays text out differently is worse than one that
throws.

### 5.7 UI

`figma.showUI(__html__, options)` opens an HTML frame: `__html__` is the manifest's `ui` file; the
frame gets `figma.ui.postMessage` / `onmessage`, `resize`, `close`, and, with `themeColors: true`,
Figma's `--figma-color-*` variables (many plugins style themselves with them). Desktop: `wry`
(WebView2 / WKWebView / WebKitGTK) in a panel; browser build: a nested `iframe`.

Policy inside the frame: no `file:` URLs, no navigation away from the plugin's own page, `fetch`
goes through the same `networkAccess` list as the script. JSON only across `postMessage`, as Figma
does.

### 5.8 Async and dynamic pages

A microtask queue and real `Promise`s; `documentAccess: "dynamic-page"` makes the synchronous
getters (`getNodeById`, local-styles getters, `currentPage.children` before load) throw and the
`*Async` forms work; `figma.loadAllPagesAsync()` is required before a document-wide `findAll`.
`figma.mixed` is a singleton, compared with `===`.

### 5.9 Errors and storage

Unsupported members throw a real `Error` with a stable message (`"<member> isn't supported in
FigCraft"`), logged once per session, so plugins that catch errors behave as they do in Figma.
`clientStorage` is async JSON, per `figma:<id>`, bounded; plugin data (`setPluginData`) enforces
Figma's key length and per-node budget instead of silently storing anything.

## 6. Milestones

Each milestone is shippable on its own and has a testable acceptance line.

| Milestone | Work | Acceptance | Estimate |
|---|---|---|---|
| **M0 — spike** | Contract crate (done, 0.1.0); `rquickjs` in a scratch crate: a script that calls `figma.notify`, reads `name`, sets `x`; a realm escape test; 100k property reads benchmark | 10k-layer file, 100k reads < 1 s; no path from JS to `require`/`process`/`fs`/host prototypes; one undo step | 1–2 w |
| **M1 — first plugins** | Manifest import, JS load, `figma` global with read + basic write, `createRectangle/Ellipse/Text`, `appendChild/remove`, `currentPage/selection/getNodeById/findAll`, `notify/closePlugin/clientStorage`, logs, worker thread | 5 real simple plugins run; a bad command rolls back; ⌘Z restores the pre-run document | 3–5 w |
| **M2 — design editing** | Auto layout, components/instances/variants, styles, effects, text + font gate, images/`exportAsync`, `group/ungroup/boolean`, `absoluteBoundingBox`, `insertChild` indices | The corpus' editing plugins produce the same document JSON as Figma for the same `.fig` input (geometry, paints, text, component properties) | 6–10 w |
| **M3 — UI** | `showUI` frame (`wry`), `__html__`, `postMessage`, `resize`, theme CSS variables, `parameters`, `relaunchButtons`, `on("run")` | A plugin with an HTML UI edits through its buttons; theme variables resolve; no navigation/file access | 3–6 w |
| **M4 — dynamic pages, network, events** | `Promise`/`*Async`, `loadAllPagesAsync`, `documentchange`/`selectionchange`/`currentpagechange`, `fetch`/XHR under `networkAccess`, redirect re-checks | A network plugin reaches only its allowlist; a dynamic-page plugin throws exactly where Figma does | 3–5 w |
| **M5 — tail** | Variables and bindings, Dev Mode `codegen`/`inspect` (read-only), FigJam read subset, coverage generator from typings | Coverage table ≥ 85% of the design-plugin members; every unsupported member has a user-facing reason | ongoing |

**Status (2026-11-26).** The contract crate and the QuickJS backend are in tree and green (43 tests
across both; `fmt` and `clippy -D warnings` clean). Against a map host the M0 script line works
(`figma.notify`, read `name`, write `x`), an endless script is interrupted at 50 ms, the realm has
no `process`/`require`/bridge handle, and 100,000 property reads take 229.7 ms. The 10k-layer file
and the one-undo-step measurement need the engine integration, which is the first task of M1.

One experienced developer: 4–6 months to M3, 6–9 months to "most design plugins run", with M4/M5
overlapping. "100% of typings" is not a milestone: Figma's API is a moving target and the widget
API, Slides, payments and the team library are separate surfaces.

## 7. How "1:1" is measured

1. **Typings coverage.** Parse `@figma/plugin-typings`'s `plugin-api.d.ts` (published on npm;
   confirm the licence before vendoring it) into `area<TAB>member<TAB>status<TAB>note`.
   `figcraft-plugin-figma::coverage` reads it, counts it and writes the Markdown report. CI fails
   below the milestone's threshold (60% after M2, 85% after M4) and fails on any `implemented`
   member without a test named after it.
2. **Corpus.** 20–30 open-source plugins with fixed `.fig` inputs; run in Figma (by hand, once) and
   here; compare exported `.fig`/document JSON. The existing `figcraft-format` parity work is the
   comparator.
3. **Undo and error tests.** Compare undo-stack depth and thrown messages for the same script on
   both sides; these are the semantics users notice first.
4. **Visible honesty.** Manage plugins shows "needs X, not supported yet" per plugin, and the
   coverage report ships in the repository, not only in a PR.

## 8. Integration points

- **Import:** the same flow as Figma's "Import plugin from manifest…": pick `manifest.json`, read
  the folder (or a zip), check the manifest, confirm permissions. A second `PluginInstance` variant
  (`Script`) next to `Module`.
- **Permissions:** the existing sheet plus two lines Figma makes explicit: document read/write and
  the network allowlist with its `reasoning`.
- **Manage plugins:** a row per Figma plugin with the blockers (widget, board, Dev Mode, Slides),
  the coverage percentage, the network domains and a "run" menu. Unsupported calls appear in the
  plugin's log with the member name.
- **Settings:** keyed `figma:<id>` in the same store the WebAssembly plugins use
  (`setting_get/set`); `clientStorage` is a separate namespace per plugin.
- **Commands/CLI/MCP:** unchanged. `plugin.run` dispatches to whichever runtime the installed
  plugin uses; nothing else in the automation surface needs to know.
- **Docs:** `docs/plugins.md` is upstream's; this plan and the crate README are ours. Do not edit
  upstream's plugin doc for this work.

## 9. Risks

| Risk | Mitigation |
|---|---|
| QuickJS semantics differ from Figma's V8 (Intl, newer syntax, `Error` shapes) | Measure against the corpus in M0; if syntax coverage is the blocker, the `Backend` seam makes a V8 (`deno_core`) backend a crate swap |
| Webview dependency on three platforms | `wry` is the same stack Tauri uses; M3 is its own milestone so M0–M2 don't depend on it |
| Thread/snapshot model breaks live reads | Commit at callback boundaries only; synthesize `documentchange` from the engine's change events; fall back to the synchronous path for panels that read during a callback |
| Undo merging holds big snapshots for long sessions | Merge lazily and cap the session; `commitUndo` lets plugins split without us guessing |
| Font metrics differ | Gate on `loadFontAsync`; compare text bounds in the corpus before claiming M2 |
| API drift | Coverage report is generated from typings, so drift shows up as `missing`, not as a silent lie |
| Legal | Typings are published for plugin authors (check the licence); do not ship Figma's fonts, icons or assets; "Figma-compatible" is descriptive, not an endorsement |

## 10. Maintenance

- `figcraft-plugin-figma` is `0.x`, one owner, its own README and tests, released independently of
  the app. It depends on `serde`, `serde_json`, `thiserror` and `url` only.
- `figcraft-plugin-figma-quickjs` depends on the contract and `rquickjs`; it can be swapped for a
  V8 backend without touching the contract or the engine.
- The engine integration is the only place that knows both; it stays inside the existing plugin
  policy module so the WebAssembly path is untouched.
- Coverage and the corpus are the regression suite. A new Figma API version starts as a new TSV
  and a diff, the way `gaps.md` treats other parity work.

## References

- Figma plugin manifest (`documentAccess`, `networkAccess`, `editorType`, `parameters`):
  <https://developers.figma.com/docs/plugins/manifest/>
- `figma.commitUndo()` and the default one-entry-per-plugin undo behaviour:
  <https://developers.figma.com/docs/plugins/api/properties/figma-commitundo/>
- `@figma/plugin-typings` (the member list the coverage generator reads):
  <https://www.npmjs.com/package/@figma/plugin-typings>

