## What changed

<!-- One or two sentences. Link the issue this closes, if any. -->

## Where it belongs

- [ ] This is **UI / GPUI** work (panels, canvas interaction, dialogs, input, theming, `apps/monet`) → this repo
- [ ] This is **engine** work (document model, layout, rendering, format, commands) → upstream project

## Checks

- [ ] `cargo xtask ci` passes (rustfmt on changed files, clippy `-D warnings` on `monet`/`xtask`, tests with the `docs/known-failures.md` skips, i18n report, layers, assets, docs, parity, wasm)
      — or the box below names exactly which subsets ran and why the rest did not
- [ ] `cargo check --workspace --all-targets --features test-support`
- [ ] `cargo test -p figcraft-ui-gpui --lib --features test-support`
- [ ] No new entry is needed in `docs/known-failures.md` (the count stayed fixed)
- [ ] UI work: the change matches the reference implementation in `crates/ui-egui` (no simplified
      substitutes for menus, popovers, drag or list rows)
- [ ] UI work: a `done` claim in the parity plan has a contract in `parity_contract.rs`, or the status
      stays `partial`
- [ ] No new strings without a catalogue entry
- [ ] Parity plan / roadmap row / progress note updated in this PR
- [ ] Screenshots or a short recording for visual changes

Subsets run and skipped:

<!-- e.g. "fmt, clippy, layers, assets, docs, parity ran; wasm skipped: no wasm32 target on this machine" -->
