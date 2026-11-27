# figcraft-plugin-figma-quickjs

The first JavaScript backend for Figma-compatible plugins: one QuickJS realm per plugin session,
the `figma` global over the [`figcraft-plugin-figma`](../plugin-figma) `Host` seam.

```text
manifest.json + code.js + ui.html
        │
        ▼  this crate: QuickJS realm, figma global, node proxies, bridge
   figma.notify / node.x = 40 / console.log
        │  figcraft-plugin-figma: ids, property table, ops
        ▼
   Host (the engine: Session::execute, selection, settings, UI, commitUndo)
        │
        ▼
   Session::grouped → figcraft-doc   (one undo step per callback)
```

## Status

`0.1.0`, the M0 spike of `docs/figma-plugin-compat-plan.md`.

- **done** — a realm loaded once per session; a script that calls `figma.notify`, reads `name`,
  sets `x`; a batch of writes per node becomes one `node.set`; `create*`, `appendChild`, `remove`,
  `currentPage`/`selection`, `getNodeById`, `showUI`/`ui.postMessage`/`commitUndo`; `console.*`;
  `figma.on` event dispatch; time, memory, call-count, message and log limits; a timeout that
  interrupts an endless loop; a test that the realm has no `process`/`require`/bridge handle.
- **next** — engine integration (a `Host` over the worker's `Session`, import flow, permissions,
  undo), geometry and children on node proxies, `findAll`, async/promises, clientStorage, fonts,
  images, components. Until then an unsupported member throws a catchable `Error` rather than
  silently doing nothing.

## Why it is its own crate

- Depends on the contract crate and `rquickjs`; **not** on `figcraft-engine`. The engine implements
  `Host`; this crate never opens a `Session`. Swapping QuickJS for V8 (or testing with a map host)
  changes no code here.
- One owner, one version. A regression is a failing test in this crate, not an editor crash.
- `rquickjs-sys` compiles QuickJS from source; the crate needs a C toolchain (MSVC on Windows,
  `cc`/`clang` elsewhere). No network at runtime and no WASI.

## Session model

`QuickJs::load(script, limits, host)` creates the realm and its bridge, then evaluates the prelude.
The plugin's `main` runs on the first `Backend::invoke`, so a load error is reported with the run
that caused it. The realm — module state, `figma.on` handlers, `figma.ui.onmessage` — survives
between invocations.

The backend owns the `Box<dyn Host>` for the whole session: a JavaScript function can outlive one
callback, so the object it calls into must live as long as the realm. The engine constructs the
host over the worker's document session and commits at the callback boundary
(`Session::grouped`).

## Limits

`Limits` replaces the WebAssembly sandbox's fuel:

| Limit | Default | Enforced by |
|---|---|---|
| `time_ms` | 30 s | QuickJS interrupt handler (uncatchable) |
| `memory_bytes` | 256 MiB | `Runtime::set_memory_limit` |
| `max_calls` | 1,000,000 | every host call, reads and writes |
| `max_message` | 32 MiB | every string across the bridge |
| `max_log_bytes` | 1 MiB | `console.*` |

A thrown plugin error keeps the realm alive (like Figma); the invocation returns `Failed`, and the
engine rolls the callback back. A timeout returns `Timeout`; memory exhaustion returns
`OutOfMemory`.

## Tests and the M0 measurement

```sh
cargo test -p figcraft-plugin-figma-quickjs
```

Seven tests run against a map host: batched writes, create + selection, UI/undo plumbing, an escape
test, an unsupported-property error, a 50 ms timeout on `while (true) {}`, and the M0 benchmark:

```
100k property reads: 229.7 ms
```

That is 100,000 `node.width` reads through the bridge on one node (mock host), well under the
plan's one-second line. The real engine's `node.get` is heavier; the worker-session integration
test in M1 is the definitive measurement.

## What the `figma` object does today

| Surface | Behaviour |
|---|---|
| `figma.currentPage` | `page.list`, picks `current: true` |
| `page.selection` (get/set) | `Host::selection` / `select.set` |
| `figma.getNodeById(id)` | `node.get type`; `null` when the id is unknown |
| `figma.createRectangle/Ellipse/Frame/Text/Line/Vector/Polygon/Star/Component/Section/Slice` | `node.create`, not selected |
| `node.name/x/y/width/height/rotation/opacity/visible/locked/type` | property table, batched `node.set` |
| other single-key properties (`fills`, `strokes`, `characters`, …) | property table, batched |
| enums (`layoutMode`, `strokeAlign`, `textAlign`, …) | converted to the engine's spelling |
| `node.appendChild`, `node.remove` | `node.reparent`, `edit.delete` |
| `figma.notify`, `closePlugin`, `commitUndo`, `showUI`, `ui.postMessage`, `ui.close` | `Host` |
| `figma.on/off/once`, `figma.ui.onmessage` | dispatched from `Event::{Message, Run, SelectionChange, DocumentChange}` |
| `console.log/info/warn/error/debug` | collected into `Outcome.logs` |
| `figma.mixed`, `figma.apiVersion` | sentinel and `"1.0.0"` |

Everything else (`children`, `parent`, `absoluteBoundingBox`, `findAll`, `clientStorage`,
`exportAsync`, `loadFontAsync`, variables, …) throws a catchable error. The coverage report in
`figcraft-plugin-figma` is where the distance to 1:1 is measured, not hidden.

## Maintenance

- `0.x`, one owner, released independently of the app. Tests use a map host, so this crate's CI
  needs no editor and no GUI.
- A change in `figcraft-plugin-figma` (a new property, a new op) is adopted here by using it; a
  change in QuickJS is contained to this crate.
- The engine integration is a separate module; it implements `Host` and owns the thread/snapshot
  model. This crate stays usable by anything that can answer `Host`.
