# Contributing

Thanks for helping improve Monet.

Read [`docs/development-model.md`](docs/development-model.md) first: it is the charter for this
repository — what it owns, what it inherits from the Craft suite, what it deliberately does not, and
which rule wins when two documents disagree.

## Where a change belongs

Monet is the **GPUI front-end** of a design tool whose document engine and egui front-end live
upstream at **[storytold/figcraft](https://github.com/storytold/figcraft)**. Send a change to the
repository that owns it:

| Change | Where to submit |
|---|---|
| Document model, layout, rendering, file format, command catalog, engine tests | **[storytold/figcraft](https://github.com/storytold/figcraft)** — engine changes belong upstream |
| egui front-end (`crates/ui-egui`) | **[storytold/figcraft](https://github.com/storytold/figcraft)** — it is the frozen reference here |
| GPUI front-end (`crates/ui-gpui`), the desktop binary (`apps/monet`), GPUI tests | **This repository** (`MonetDesigns/Monet`) |

If an engine change is needed for a GPUI feature, open the engine change upstream first and then port
the resulting command or API here. Do not fork engine logic into this repository: the two front-ends
must keep using the same commands so the 1:1 port stays verifiable.

## Branches and pull requests

`main` is protected. Every change lands through a pull request, including changes made by maintainers:

1. Branch from `main` (`feat/…`, `fix/…`, `ui/…`, `sync/…`).
2. Open a pull request and fill in the template. State which front-end the change touches.
3. CI must pass (`cargo xtask ci` through the shared gate) and the GPUI test suite.
4. AI review (CodeRabbit) comments automatically; address its findings or explain why not.
5. Squash-merge once approved. Never push to `main` directly.

One change per PR. A pure move and a behaviour change are separate PRs. Update the parity plan, the
roadmap row or a progress note in the same PR.

## Parity rule for UI work

Every GPUI panel, dialog, popover, menu and keyboard path is a 1:1 port of the egui front-end.
Before changing UI, read the matching file in `crates/ui-egui`, then copy its controls, order,
sections and dimensions instead of inventing a simpler substitute. The parity contract in
`crates/ui-gpui/src/parity_contract.rs` fails CI if a contracted capability disappears, and
`every_done_status_has_a_contract` refuses a `done` line in the plan's status block without a
contract. `docs/gpui-egui-parity-plan.md` tracks what still differs; its I-section
`PARITY_CONTRACT_STATUS` block is the only machine-readable status.

## Checks

```sh
cargo xtask ci                  # rustfmt on changed files, clippy on monet/xtask, tests, i18n report, layers, assets, docs, parity, wasm
cargo check --workspace --all-targets --features test-support
cargo test -p figcraft-ui-gpui --lib --features test-support
```

`docs/known-failures.md` lists the failures that predate the GPUI work. The count is expected to stay
fixed: a new failure is a regression, and a fixed one is removed in the PR that fixes it.

## Code and comments

- Comments and identifiers are English.
- Describe what the code does, not where it came from.
- New user-visible strings go through the catalogue (`crates/i18n/locales/*.toml`), never inline.
  English is the key; a missing translation reads English and does not block a PR.

## Commit and tag conventions

- Commit subjects use `type(scope): summary` (`feat(ui-gpui)`, `fix(engine)`, `docs`, `perf(export)`),
  with a body that says what was verified. Avoid `Add files via upload`; commit through git so the
  author and message are real.
- Squash-merge keeps one commit per change on `main`.
- Tags are either `vX.Y.Z` (a release) or `sync-YYYYMMDD` (an upstream sync checkpoint). Do not add
  ad-hoc checkpoint tags; existing ones are kept for history but not extended.

## Release

`apps/monet` is the only release target; `crates/ui-egui` and the legacy app shells are excluded from
the workspace and are never packaged. The packaging scripts still carry legacy `figcraft` ids while
the brand migration finishes, so the release workflow is manual (`workflow_dispatch`) and every run
is a draft/dry run until it has been verified end to end.

## Community

Monet is maintained by its owner with agent assistance. External pull requests are welcome once an
issue has been agreed — open an issue describing the change before writing code, so two people do
not build the same fix. There is no issue-claim process or review SLA; reviews happen when
maintainers are available. Contributors are credited through `contributors/people.toml`.

## Upstream sync

The two histories share no common ancestor, so sync is content-level, not `git merge`:

1. `git fetch benchmark` (`benchmark` is `yelintong/FigCraft-Monet`, whose `main` mirrors upstream).
2. `cargo xtask upstream-report` writes `docs/upstream-sync.md`: command ids missing here and files
   that differ, by area.
3. Port on a `sync/YYYYMMDD` branch, verifying each touched crate.
4. Update `upstream.toml` (upstream commit, date, mirror commit, audit date) in the same PR.

The current gap is tracked in [`docs/monet-upstream-diff.md`](docs/monet-upstream-diff.md) and the
generated [`docs/upstream-sync.md`](docs/upstream-sync.md).
