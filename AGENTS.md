# Monet — instructions for agents

Monet is the **GPUI front-end** of a design tool. The document engine and the egui front-end live
upstream at [`storytold/figcraft`](https://github.com/storytold/figcraft); this repository owns
`crates/ui-gpui` and its desktop binary `apps/monet`. `crates/ui-egui` and the two legacy app shells
are read-only reference copies, outside the workspace build.

Read [`docs/development-model.md`](docs/development-model.md) first: it is the charter that says what
this repository inherits from the Craft suite, what it deliberately does not, and which rule wins
when two documents disagree.

## Start every session here

1. Read [`docs/development-model.md`](docs/development-model.md), then
   [`docs/gpui-egui-parity-plan.md`](docs/gpui-egui-parity-plan.md) (the GPUI work queue) and
   [`docs/upstream-sync.md`](docs/upstream-sync.md) with [`upstream.toml`](upstream.toml) (how far
   behind upstream we are). Unless the user gives you a task, pick work from the parity plan.
2. Read `plan/STATUS.md` and `plan/execution-plan.md` (local, gitignored) if present.
3. Orient → plan → implement + test → verify (screenshot) → record (parity plan / progress note) →
   open a PR.

`docs/gaps.md` and `docs/target-app-parity.md` describe the **upstream engine's** Figma parity. They
are reference material, not a Monet task list; do not pick work from them.

## Non-negotiables

### Clean-room

- Behaviour comes from public documentation and black-box observation of the running incumbent app
  with synthetic documents only. Never read, disassemble or copy anything inside the incumbent's
  application bundle, web bundle or network traffic (names and public listings only).
- Never copy GPL/AGPL code.
- Reference screenshots of the incumbent stay local under the gitignored `plan/figma/screenshots/`
  and are never committed, published or used as assets.

### Assets: no third-party product iconography or images — ever

- **Never add, copy, trace, redraw-from, embed or ship any icon, image, illustration, cursor, logo,
  template, UI kit, preset library, font file or screenshot from Figma, Adobe, Avid, Autodesk or any
  other product, or from any source whose licence doesn't allow it.** Breaking this rule is the
  worst thing you can do in this repository.
- Every asset must be original work for Monet, OSI open source, public domain/CC0, or Creative
  Commons with redistribution allowed, and every file needs a row in
  [`ATTRIBUTION.md`](ATTRIBUTION.md). `cargo xtask assets` fails on an unlisted file under
  `assets/`, `docs/brand/` or `docs/images/`.
- UI icons are drawn in code (`crates/ui-gpui/src/icons_panel.rs`); new icons are new code, designed
  from scratch. The app icon is `assets/app-icon/monet.svg`; `cargo xtask icon` renders the PNGs and
  `.ico`.
- Fonts live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), never here. CI
  builds with `craft-fonts-ref`; local builds may point `CRAFT_FONTS_DIR` at a checkout, and a build
  without it must still work with system fonts.
- Demo and test content is synthetic or public domain; never personal photos or data.

### Never crash

- No `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!` in shipped code
  (workspace clippy lints deny them; tests may use them). Return `Result`/`Option`; use `.get()` for
  data-derived indexes; slice strings only at char boundaries; check floats are finite; cap sizes.
- `figcraft_engine::guard` turns a panicking command into `EngineError::Internal` and rolls the
  document back. This front-end guards its entry points; these are safety nets, not substitutes for
  `Result`.
- `unsafe_code = "forbid"` workspace-wide.
- Every crash fix comes with a regression test. Extend the engine's junk-parameter sweep when you
  add parsers or commands.

### Architecture

- **Everything is a command.** The engine owns the command catalog in
  `crates/engine/src/cmd/*` upstream; this front-end does not invent ids. GPUI panels read engine
  state and act through `app.run(id, params)` / `app.set_props(ids, props)`. Menus, shortcuts, the
  Actions palette, CLI, the control channel and the MCP server dispatch the same ids.
- **Programmatic calls never open dialogs.** Commands only *request* UI (`Session::request_ui`); the
  control channel drops dialog requests.
- **Layering** is enforced by `cargo xtask layers`: geom/color (L0) → doc (L1) →
  layout/text/pathops/plugin/collab (L2) → render/format (L3) → svg/pdf (L4) → engine (L5) →
  ui-gpui/mcp (L6) → apps. `crates/ui-egui` is the reference front-end and is outside the workspace;
  nothing in the build may depend on it.
- **The UI is thin**: panels act through commands and read theme tokens (`crates/ui-gpui/src/theme.rs`);
  colours, sizes and fonts never come from ad-hoc constants.
- **GPUI control vocabulary lives in `crates/ui-gpui/src/widgets/`**: `SectionHeader` (`Normal`/`Dim`
  with pre-reserved lead/trail slots), `PanelSection`/`panel_section` (the 16/8 pt inset grid),
  `IconButton`, and the input family (`Slot`, `TextField`, `text_field`, `search_field`,
  `combo_field`, `number_field`). Use these instead of hand-rolling a section, icon button or input.
- **Retired names:** the developer panel is **开发**. "Dev Mode" and "Inspect" are retired (trademark
  plus the 2026-11-10 request to label it 开发); the command id `view.devMode` is an identifier and
  stays unchanged.

### ⛔ Unified grid — permanent rule

- **Every panel row is laid out by the unified grid in `crates/ui-gpui/src/widgets/panel_section.rs`.
  There is exactly one way to build a row: `field_row` / `field_row_full` / `field_row_h` → passed to
  `PanelSection::child`.**
- **Never hand-roll a row.** No `div().flex().flex_row()…` in a panel to lay out fields, and no
  hand-computed column widths. Column widths are owned by `panel_section.rs` alone.
- The grid is **`1fr | 1fr | 24`** — exactly **one** 24 pt column. A control taller than one row uses
  `field_row_h(fields, trailing, height)`; do not route around the grid with a free block.
- Breaking any of the above misaligns the whole right panel and counts as an unacceptable
  regression. Read the FROZEN header of `panel_section.rs` before touching a panel.
- **Time goes through `chrono`** (workspace dependency). Never hand-roll UTC/calendar conversion.
- **Rust only** (no handwritten JS/TS). **Never break wasm** for the engine and collaboration crates
  (`cargo xtask wasm`).

### Quality gates

`cargo xtask ci` is the gate: rustfmt on the files a branch changes, clippy `-D warnings` on
`monet` and `xtask` (`--no-deps`), workspace tests with the `docs/known-failures.md` skips, the translation report,
layers, assets, docs, menu parity and the wasm check. Run it (or name in the PR exactly which
subsets ran and which did not). `docs/known-failures.md` lists failures that predate the GPUI work;
the count is expected to stay fixed and a new failure is a regression. A `done` row in the parity
plan needs a `parity_contract` entry (`crates/ui-gpui/src/parity_contract.rs`); the contract test
fails otherwise.

- Work on a `feat/…`, `fix/…`, `ui/…` or `sync/…` branch. **Never commit or push directly to the
  protected `main` branch**; every change lands through a pull request with the template filled in,
  and `main` is protected.
- One change per PR. Moving code and changing behaviour are separate PRs.
- Update the rows you touch in the parity plan, `ROADMAP.md` or a progress note in the same PR.
- Do not commit scratch files or agent tool files; do not pass `--no-verify`.

## Running and looking at the app

- `cargo run -p monet -- --control <port>` — the desktop binary. Pick a free port, then drive it with
  JSON lines (`engine.execute`, `ui.screenshot`) as in
  [`docs/control-protocol.md`](docs/control-protocol.md).
- `cargo run -p figcraft-ui-gpui --example run [-- --control <port>]` — the development entry point
  (same UI, thinner shell).
- Headless: `figcraft-cli` (`info`, `export`, `run`, `commands`, `mcp`). MCP:
  `figcraft-cli mcp --automation-read-root DIR --automation-write-root DIR`
  (see [`docs/mcp.md`](docs/mcp.md)).
- **For UI work, look at the result** (take a screenshot and read the PNG), and read the AccessKit
  tree for the controls you changed. A visual claim without a screenshot is not done.

## Upstream sync

- An engine change goes upstream first (`storytold/figcraft`), then the resulting command or API is
  ported here. Do not fork engine logic.
- The two histories share no ancestor: sync is content-level. `git fetch benchmark`, run
  `cargo xtask upstream-report`, port on a `sync/YYYYMMDD` branch, then update `upstream.toml`.
- `crates/ui-egui` is the frozen 1:1 reference; read it, do not edit it. When it must move, replace
  whole directories from upstream rather than patching file by file.

## Roadmap and progress

[`ROADMAP.md`](ROADMAP.md) is the one-page summary. [`docs/gpui-egui-parity-plan.md`](docs/gpui-egui-parity-plan.md)
is the authoritative GPUI assessment and work queue; [`docs/audits/`](docs/audits/) holds dated audit
snapshots (read-only) and [`docs/progress/`](docs/progress/) weekly notes. Grade by behaviour against
the egui reference, not by whether a menu item exists; err low.

## Contributor credits

About ▸ Contributors is compiled from `contributors/contributors.json` (generated, never hand-edit)
and `contributors/people.toml` (names people chose). If you prepare a PR for a human whose GitHub
username has no `[people.<username>]` entry, ask them once whether they want a display name; add only
their own entry. Never guess or copy anyone else's.
