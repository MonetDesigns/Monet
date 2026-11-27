# Known failures

Failures listed here predate the current branch. The count is fixed: a new failure is a
regression, and a fixed one is removed from this table in the PR that fixes it. Do not use this
file to excuse a failure introduced by a change.

## Lint and format baseline

Measured 2026-11-26 on `origin/main` @ `16a1d19cf` with stable 1.99.1 (the toolchain declared in
`rust-toolchain.toml`).

| Command | Result at the base commit | Why it is accepted | How the gate handles it |
|---|---|---|---|
| `cargo fmt --all --check` | 197 files differ | Inherited formatting drift, mostly the ported `crates/ui-gpui` tree. Formatting all of it in one PR would bury every real change. | `cargo xtask ci` rustfmt-checks only the Rust files a branch changes against `origin/main`. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Not green. `figcraft-ui-gpui` test targets need the `test-support` feature, and the port carries unused-import warnings. | `ui-gpui` is a port, not upstream; clearing its lint debt is its own task. | The gate runs clippy `--no-deps` on `monet` and `xtask` only, with `-D warnings`. |
| `cargo clippy -p figcraft-ui-gpui --all-targets` | 229 compile errors: `#[gpui::test]` expands to `BackgroundExecutor::block_test`, which only exists with `test-support`. | Feature wiring, not a lint failure. | UI tests run with `--features test-support`; the workspace lint baseline stays tracked here. |

`figcraft-engine` failed its `-D warnings` gate at the base commit (wildcard imports, collapsible
ifs, complexity, unused imports, `clone` on `Copy`, clamp-like patterns). This branch fixes them:
`cargo clippy -p figcraft-engine --all-targets -- -D warnings` is green.

## Test failures

The same eight `figcraft-engine` library tests fail at `origin/main` @ `16a1d19cf` (verified 2026-11-26). The gate skips them via `KNOWN_FAILING_TESTS` in `xtask/src/main.rs`; delete a name there when its test is fixed.

| Crate / test | Since | Why it is accepted | Owner |
|---|---|---|---|
| `figcraft-engine` `components_behaviour_tests::what_instances_cannot_override_is_refused` | before `16a1d19cf` | Auto-layout differs from the upstream expectation; same at `origin/main`. | GPUI maintainers |
| `figcraft-engine` `fonts::tests::lists_families_of_layers_ranges_styles_and_variables_once_each` | before `16a1d19cf` | Font registry needs the `craft-fonts` checkout; headless CI sees a different family list. | GPUI maintainers |
| `figcraft-engine` `fonts::tests::refit_measures_stale_text_boxes_again` | before `16a1d19cf` | Same missing-font baseline. | GPUI maintainers |
| `figcraft-engine` `import_export_tests::export_rows_add_step_up_and_update` | before `16a1d19cf` | Same at `origin/main`. | GPUI maintainers |
| `figcraft-engine` `live_layer_tests::an_alt_drag_lives_in_the_copies_it_makes` | before `16a1d19cf` | Same at `origin/main`. | GPUI maintainers |
| `figcraft-engine` `shapes_behaviour_tests::corner_smoothing_follows_the_documented_rules` | before `16a1d19cf` | Same at `origin/main`; the port's smoothing differs from upstream expectations. | GPUI maintainers |
| `figcraft-engine` `tests::guides_add_move_remove` | before `16a1d19cf` | Same at `origin/main`. | GPUI maintainers |
| `figcraft-engine` `theme_pages_tests::a_copy_of_a_page_keeps_its_flows_prototype_settings_and_guides` | before `16a1d19cf` | Same at `origin/main`. | GPUI maintainers |

Parallel runs fail four additional engine tests because they share global state; `cargo xtask test` runs single-threaded. `figcraft-ui-gpui` lib baseline: `main_menu aim`, `extra Shift+D`, `panel_resize hit area` (last recorded 2026-11-20).

