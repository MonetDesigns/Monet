# Monet development model

This document is the charter the other rule files defer to. It records **where Monet sits in the
Craft ecosystem**, **what this repository inherits**, **what it deliberately does not**, and **which
rule wins when two documents disagree**. Changing it is a normal pull request; when it changes,
`AGENTS.md` and `CONTRIBUTING.md` change in the same PR.

## Position

Monet is the **GPUI front-end** of a design tool whose document engine and egui front-end live
upstream at [`storytold/figcraft`](https://github.com/storytold/figcraft). This repository owns:

- `crates/ui-gpui` — panels, canvas interaction, dialogs, input, theming, GPUI tests;
- `apps/monet` — the desktop binary (`cargo run -p monet --`);
- the GPUI parity plan and its contract (`crates/ui-gpui/src/parity_contract.rs`);
- the upstream sync anchor and report (`upstream.toml`, `docs/upstream-sync.md`).

It does **not** own the document model, layout, rendering, file format, command catalog or engine
tests. An engine change needed by a GPUI feature goes upstream first and is then ported here as the
resulting command or API. Do not fork engine logic into this repository.

`crates/ui-egui`, `apps/figcraft` and `apps/figcraft-web` are **reference copies**: kept in the tree
for the 1:1 port, excluded from the workspace (see the root `Cargo.toml`), never built or released
here. They are read, not maintained.

## Inherited from the Craft suite

| What | How it is consumed |
|---|---|
| `storytold/craft-libs` | Git dependencies pinned to **one rev** in the root `Cargo.toml`; a rev move is its own PR |
| `storytold/craft-common-actions` | `ci.yml` and `release.yml` call its `gate.yml` / `release.yml` |
| `storytold/craft-fonts` | CI passes `craft-fonts-ref` and `require-craft-fonts: true`; fonts are never committed here |
| Rust layout and command rules | The engine keeps the L0–L6 layering and the “everything is a command” contract |

## Deliberately not inherited

| Upstream practice | Why not here | What Monet does instead |
|---|---|---|
| `craft-contrib` process standards (issue claims, 3-day review SLA, RFC to change rules) | One maintainer plus agents; there is no second claimant to coordinate | PR-only, one change per PR, CodeRabbit review, squash merge |
| Translations added in `storytold/craft-i18n` | The GPUI front-end ships its own catalogue | `crates/i18n/locales/*.toml`; English is the key and a missing translation reads English |
| Append-only progress fragment per change | Noise at this size | `docs/audits/` (dated snapshots) and `docs/progress/` (weekly notes) |
| Menu-command coverage as the main KPI | Monet's value is GPUI parity, not command count | `parity_contract` coverage and upstream lag (see below) |
| Maintaining both front-ends | Would double the work and guarantee divergence | `ui-egui` is read-only reference; replace whole directories from upstream when needed |

## Rule precedence

When documents disagree, this order wins:

1. `AGENTS.md` — how to work in this repository;
2. `CONTRIBUTING.md` — where a change belongs and how it lands;
3. `docs/*` — plans, parity tables, audits.

A conflict is a bug: fix it in the PR that notices it, starting with the higher document. Do not
silently follow the lower one.

## Work queue

`docs/gpui-egui-parity-plan.md` is the queue for GPUI work; its I-section
`<!-- PARITY_CONTRACT_STATUS … -->` block is the only machine-readable status, and the contract test
refuses a `done` line without a contract. `docs/gaps.md` and `docs/target-app-parity.md` describe the
**upstream** engine's Figma parity; they are reference material here, not a source of tasks.

## Quality gate

`cargo xtask ci` is the gate: rustfmt on the files a branch changes, clippy `-D warnings` on the
packages Monet owns (`monet` and `xtask`, with `--no-deps`), workspace tests with the
`docs/known-failures.md` skips, the translation report,
layers, assets, docs, menu parity and the wasm check. `docs/known-failures.md` lists the failures
that predate the GPUI work; the count is expected to stay fixed and a new failure is a regression.
The pull-request template mirrors the gate rather than promising a smaller check.

## Upstream sync

The two histories have **no common ancestor** (Monet's history was rewritten before the port), so
commit counts and `git merge` are meaningless. Sync is content-level:

- `upstream.toml` records the upstream commit, its date, the mirror commit and the last audit date;
- `cargo xtask upstream-report` compares this tree with `upstream/main` (falling back to
  `benchmark/main` when the upstream remote is absent) and writes `docs/upstream-sync.md` (command
  ids missing here, files that differ, by area);
- `benchmark/main` mirrors `storytold/figcraft` main; `git fetch benchmark` before a report;
- a sync lands on a `sync/YYYYMMDD` branch and updates the anchor after the port.

## Release

`apps/monet` is the only desktop release target. `packaging/` builds `-p monet` and names Linux,
macOS and Windows artifacts `monet-*`; the reference shells are excluded from the workspace and
must not be packaged. The web front-end, headless CLI and collaboration server are still the
upstream `figcraft-*` crates and stay disabled in `release.yml` (`has-web`, `has-server`,
`has-freebsd` are false). The release workflow remains manual (`workflow_dispatch`) until one
end-to-end dry run installs and starts a `monet` package; only then is the `release` branch
trigger re-enabled.

## Metrics

| Metric | Definition | Target |
|---|---|---|
| Upstream lag | Days between the upstream commit in `upstream.toml` and today | ≤ 14 |
| Parity evidence | Share of `done` rows in the parity plan backed by a contract | 100% |
| Gate health | `cargo xtask ci` result; entries in `docs/known-failures.md` | Green / fixed list |
| Deliverable | Can `main` produce an installable Monet package? | Dry run first; `Yes` once a desktop `monet` package installs and starts |
| Sync cost | Files that differ from the mirror per sync | Explainable, trending down |

## Divergence register

These differences from upstream are intentional; do not “fix” them by copying upstream:

1. No issue-claim process, no review SLA, no RFC for rule changes.
2. Local i18n catalogue instead of `craft-i18n`.
3. `ui-egui` and the legacy app shells are read-only reference.
4. No per-change progress fragments.
5. No menu-command-coverage KPI.
6. GPUI accessibility is asserted through the AccessKit tree, not egui's `request_focus` model.
