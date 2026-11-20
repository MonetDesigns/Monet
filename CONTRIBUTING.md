# Contributing

Thanks for helping improve Monet.

## Where a change belongs

Monet is the **GPUI front-end** of a design tool whose document engine and egui front-end live
upstream at **[storytold/figcraft](https://github.com/storytold/figcraft)**. Please send a change
to the repository that owns it:

| Change | Where to submit |
|---|---|
| Document model, layout, rendering, file format, command catalog, engine tests | **[storytold/figcraft](https://github.com/storytold/figcraft)** — engine changes belong upstream |
| egui front-end (`crates/ui-egui`) | **[storytold/figcraft](https://github.com/storytold/figcraft)** — it is the frozen reference here |
| GPUI front-end (`crates/ui-gpui`): panels, canvas interaction, dialogs, input, theming, GPUI tests | **This repository** (`MonetDesigns/Monet`) |

If an engine change is needed for a GPUI feature, open the engine change upstream first and then
port the resulting command or API here. Do not fork engine logic into this repository: the two
front-ends must keep using the same commands so the 1:1 port stays verifiable.

## Branches and pull requests

`main` is protected. Every change lands through a pull request, including changes made by
maintainers:

1. Branch from `main` (`feat/…`, `fix/…`, `ui/…`).
2. Open a pull request and fill in the template. State which front-end the change touches.
3. CI must pass (`cargo check --workspace` and the GPUI test suite).
4. AI review (CodeRabbit) comments automatically; address its findings or explain why not.
5. Squash-merge once approved.

## Parity rule for UI work

Every GPUI panel, dialog, popover, menu and keyboard path is a 1:1 port of the egui front-end.
Before changing UI, read the matching file in `crates/ui-egui`, then copy its controls, order,
sections and dimensions instead of inventing a simpler substitute. The parity contract in
`crates/ui-gpui/src/parity_contract.rs` fails CI if a contracted capability disappears.
`docs/gpui-egui-parity-plan.md` tracks what still differs.

## Checks

```sh
cargo check --workspace --all-targets --features test-support
cargo test -p figcraft-ui-gpui --lib --features test-support
```

A few baseline test failures predate this repository's UI work; they are tracked in the issue
tracker and are not caused by a change unless the count moves.

## Code and comments

- Comments and identifiers are English.
- Describe what the code does, not where it came from.
- New user-visible strings go through the catalogue (`crates/i18n/locales/*.toml`), never inline.

## Upstream sync

[`yelintong/FigCraft-Monet`](https://github.com/yelintong/FigCraft-Monet) is a fork of
`storytold/figcraft` whose `main` mirrors upstream. Check it to see how far behind upstream we are;
after porting the missing features here, update that mirror to the new upstream `main`.

The current gap is tracked in [`docs/monet-upstream-diff.md`](docs/monet-upstream-diff.md).
