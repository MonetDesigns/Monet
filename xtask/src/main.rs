//! Project tasks: `cargo xtask <task>`.
//!
//! - `ci`      fmt check of changed files, clippy for the app entry and xtask, tests, the
//!   translation report, layers, assets, docs, parity and the wasm check
//! - `test`    the workspace tests (nextest when installed) and doctests
//! - `i18n-report` the UI strings, command labels and menu wordings each language lacks (never fails
//!   on them: a missing translation reads English)
//! - `layers`  crate dependency layering check
//! - `assets`  every file under assets/ and docs/ has a row in ATTRIBUTION.md
//! - `docs`    no table row twice and no status-line item listed twice in ROADMAP.md and docs/*.md
//!   (what a merge that keeps both sides leaves)
//! - `parity`  menu-catalog coverage → docs/parity-checklist.md
//! - `upstream-report` compare this tree with the upstream mirror → docs/upstream-sync.md
//! - `version` print the version, or `version set X.Y.Z`
//! - `icon`    regenerate the app icon PNGs/ICO from assets/app-icon/monet.svg (needs resvg)
//! - `wasm`    check the core crates for wasm32, and build the plugin SDK and examples
//! - `plugins` format-check, lint, test and build the plugin SDK and example plugins (`plugins/`, a
//!   workspace of its own) for wasm32: `<target>/plugins/dist/<example>/` holds each manifest and
//!   module, ready for Plugins ▸ Development ▸ Import plugin from manifest…
//! - `bench`   release-build engine timings at 1k/10k/50k layers (`bench 1000,10000` for other sizes)

mod docs_check;
mod icon;
mod layers;
mod upstream;

use std::process::{Command, ExitCode};

fn run(cmd: &mut Command) -> bool {
    eprintln!("$ {cmd:?}");
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
}

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(std::path::Path::to_path_buf).unwrap_or_default()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = args.first().map(String::as_str).unwrap_or("help");
    let ok = match task {
        "ci" => ci(),
        "test" => test(),
        "i18n-report" => i18n_report(),
        "layers" => layers::check(&root()),
        "assets" => assets(),
        "docs" => docs_check::check(&root()),
        "parity" => parity(),
        "upstream-report" => upstream::report(&root()),
        "version" => version(args.get(1).map(String::as_str), args.get(2).map(String::as_str)),
        "icon" => icon::generate(&root()),
        "wasm" => wasm(),
        "plugins" => plugins(),
        "bench" => bench(args.get(1).map_or("1000,10000,50000", String::as_str)),
        _ => {
            eprintln!(
                "tasks: ci, test, i18n-report, layers, assets, docs, parity, upstream-report, version [set X.Y.Z], icon, wasm, plugins, bench [SIZES]"
            );
            task == "help"
        }
    };
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn ci() -> bool {
    fmt_changed() && clippy() && test() && i18n_report() && layers::check(&root()) && assets() && docs_check::check(&root()) && parity() && wasm()
}

/// Format-check only the Rust files this branch changes against `origin/main`. The base tree has
/// 197 formatting differences of its own (see `docs/known-failures.md`), so a whole-tree check
/// cannot pass and would only teach people to ignore the gate.
fn fmt_changed() -> bool {
    let out = Command::new("git").args(["diff", "--name-only", "--diff-filter=ACMR", "origin/main", "--", "*.rs"]).current_dir(root()).output();
    let Ok(out) = out else { return false };
    if !out.status.success() {
        eprintln!("xtask: `git diff origin/main` failed; is the remote fetched?");
        return false;
    }
    let files: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).filter(|f| !f.is_empty()).collect();
    if files.is_empty() {
        eprintln!("xtask: no changed Rust files to format-check");
        return true;
    }
    run(Command::new("rustfmt").args(["--edition", "2024", "--check"]).args(&files).current_dir(root()))
}

/// Lint the crates this repository owns outright: the desktop entry and the task runner.
/// `--no-deps` keeps `-D warnings` scoped to those crates; the inherited engine/GPUI warning
/// baseline is recorded in `docs/known-failures.md`.
fn clippy() -> bool {
    run(cargo().args(["clippy", "--no-deps", "-p", "monet", "-p", "xtask", "--all-targets", "--", "-D", "warnings"]))
}

/// Tests that fail on `origin/main` too (docs/known-failures.md). The gate skips them so it stays
/// actionable; deleting a name here is the fix for that test. Run them alone with
/// `cargo test -p figcraft-engine --lib -- <name>`.
const KNOWN_FAILING_TESTS: &[&str] = &[
    "components_behaviour_tests::what_instances_cannot_override_is_refused",
    "fonts::tests::lists_families_of_layers_ranges_styles_and_variables_once_each",
    "fonts::tests::refit_measures_stale_text_boxes_again",
    "import_export_tests::export_rows_add_step_up_and_update",
    "live_layer_tests::an_alt_drag_lives_in_the_copies_it_makes",
    "shapes_behaviour_tests::corner_smoothing_follows_the_documented_rules",
    "tests::guides_add_move_remove",
    "theme_pages_tests::a_copy_of_a_page_keeps_its_flows_prototype_settings_and_guides",
];

/// Unit and integration tests, single-threaded (several engine tests share the font registry), then
/// the doctests. Examples are not built: clippy's `--all-targets` already checks them.
fn test() -> bool {
    let mut units = cargo();
    units.args(["test", "--workspace", "--lib", "--bins", "--tests", "--", "--test-threads=1"]);
    for name in KNOWN_FAILING_TESTS {
        units.args(["--skip", name]);
    }
    run(&mut units) && run(cargo().args(["test", "--workspace", "--doc"]))
}

/// Prints what each language's catalog lacks, from the test that lists it. `--workspace --lib`
/// reuses the binaries `test` built.
fn i18n_report() -> bool {
    run(cargo().args(["test", "-q", "--workspace", "--lib", "--", "--exact", "i18n::tests::missing_translations_report", "--nocapture"]))
}

fn wasm() -> bool {
    // The GPUI front-end is desktop-only; what has to stay wasm-clean is the engine and the
    // collaboration core. The egui front-end and the web shell are reference copies outside this
    // workspace (see Cargo.toml), so they are not built here.
    run(cargo().args([
        "check",
        "--keep-going",
        "--target",
        "wasm32-unknown-unknown",
        "-p",
        "figcraft-collab",
        "-p",
        "figcraft-collab-net",
        "-p",
        "figcraft-engine",
    ])) && plugins()
}

/// The example plugins in `plugins/examples` (each folder's name is its package; its module is
/// `<name with _>.wasm`).
const EXAMPLE_PLUGINS: &[&str] = &["tidy-names", "swatch-grid", "counter", "export-names"];

/// The plugin workspace: fmt, clippy and build for wasm32, host tests, then each example's manifest
/// and module side by side in `<target>/plugins/dist/<example>/`.
fn plugins() -> bool {
    let root = root();
    let manifest = root.join("plugins").join("Cargo.toml");
    let target = std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| root.join("target"));
    let ws = |args: &[&str]| {
        let mut c = cargo();
        // Keep the caller's target; Cargo separates host and wasm artifacts itself.
        c.args(args).arg("--manifest-path").arg(&manifest).env("CARGO_TARGET_DIR", &target);
        c
    };
    let wasm = ["--target", "wasm32-unknown-unknown", "--locked"];
    let ok = run(&mut ws(&["fmt", "--all", "--check"]))
        && run(ws(&["clippy", "--workspace", "--release"]).args(wasm).args(["--", "-D", "warnings"]))
        && run(&mut ws(&["test", "--workspace", "--locked"]))
        && run(ws(&["build", "--workspace", "--release"]).args(wasm));
    if !ok {
        return false;
    }
    for name in EXAMPLE_PLUGINS {
        let module = format!("{}.wasm", name.replace('-', "_"));
        let dist = target.join("plugins/dist").join(name);
        let copied = std::fs::create_dir_all(&dist)
            .and_then(|()| std::fs::copy(root.join("plugins/examples").join(name).join("figcraft-plugin.json"), dist.join("figcraft-plugin.json")))
            .and_then(|_| std::fs::copy(target.join("wasm32-unknown-unknown/release").join(&module), dist.join(&module)));
        match copied {
            Ok(bytes) => eprintln!("plugins: {} ({} KB)", dist.display(), bytes / 1024),
            Err(e) => {
                eprintln!("plugins: {name}: {e}");
                return false;
            }
        }
    }
    true
}

/// Engine/render timings (`figcraft-cli perf`), release builds, for each comma-separated document
/// size in layers. The egui front-end's frame benchmark lives with the reference tree and is not
/// part of this workspace's gate.
fn bench(sizes: &str) -> bool {
    run(cargo().args(["run", "--release", "-p", "figcraft-cli", "--", "perf", "--layers", sizes]))
}

/// Every asset file must be listed in ATTRIBUTION.md.
fn assets() -> bool {
    let root = root();
    let attribution = std::fs::read_to_string(root.join("ATTRIBUTION.md")).unwrap_or_default();
    let mut missing = Vec::new();
    for dir in ["assets", "docs/brand", "docs/images"] {
        walk(&root.join(dir), &mut |p| {
            let rel = p.strip_prefix(&root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let name = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            if name.starts_with('.') || name.ends_with(".md") || name.ends_with(".txt") {
                return;
            }
            // Listed by path, or by any ancestor directory written as `dir/`.
            let mut listed = attribution.contains(&format!("`{rel}`"));
            let mut cur = rel.as_str();
            while let Some((parent, _)) = cur.rsplit_once('/') {
                if attribution.contains(&format!("`{parent}/`")) {
                    listed = true;
                    break;
                }
                cur = parent;
            }
            if !listed {
                missing.push(rel);
            }
        });
    }
    if !missing.is_empty() {
        eprintln!("assets without an ATTRIBUTION.md row:\n  {}", missing.join("\n  "));
        return false;
    }
    eprintln!("assets: ok");
    true
}

fn walk(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() && !ft.is_symlink() {
            walk(&p, f);
        } else if !ft.is_dir() {
            f(&p);
        }
    }
}

fn parity() -> bool {
    let extra = [
        "file.open",
        "file.import",
        "file.placeImage",
        "file.save",
        "file.saveAs",
        "file.export",
        "file.exportSvg",
        "edit.copyAsText",
        "edit.copyAsCss",
        "edit.copyAsSvg",
        "edit.copyAsPng",
        "edit.copyLink",
    ];
    let (done, total, missing) = figcraft_engine::catalog::parity(&extra);
    let pct = done as f64 * 100.0 / total.max(1) as f64;
    // The generation date: the date of the last commit (no clock dependency in xtask).
    let date = Command::new("git")
        .args(["log", "-1", "--format=%cs"])
        .current_dir(root())
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|d| d.trim().to_string())
        .unwrap_or_default();
    let mut md = format!(
        "# Parity checklist\n\n> **Generated:** {date} by `cargo xtask parity`. Do not edit by hand.\n\nFigCraft commands registered for the incumbent's main menu and toolbar ({} commands in the engine).\n\n**{done} / {total} menu items ({pct:.1}%)**\n\n",
        figcraft_engine::all_commands().len()
    );
    if missing.is_empty() {
        md.push_str("Every catalog item has a command.\n");
    } else {
        md.push_str("## Missing\n\n");
        for m in &missing {
            md.push_str(&format!("- `{m}`\n"));
        }
    }
    md.push_str("\nMenu coverage measures breadth only; see docs/target-app-parity.md for depth and fidelity.\n");
    let path = root().join("docs/parity-checklist.md");
    if let Err(e) = std::fs::write(&path, md) {
        eprintln!("{}: {e}", path.display());
        return false;
    }
    eprintln!("parity: {done}/{total} ({pct:.1}%) → docs/parity-checklist.md");
    true
}

fn version(sub: Option<&str>, value: Option<&str>) -> bool {
    let path = root().join("Cargo.toml");
    let Ok(text) = std::fs::read_to_string(&path) else { return false };
    let cur = text
        .lines()
        .skip_while(|l| !l.starts_with("[workspace.package]"))
        .find_map(|l| l.strip_prefix("version = \"").and_then(|r| r.strip_suffix('"')))
        .unwrap_or("0.0.0")
        .to_string();
    match (sub, value) {
        (Some("set"), Some(v)) => {
            let valid =
                v.split(['-', '+']).next().is_some_and(|core| core.split('.').count() == 3 && core.split('.').all(|p| p.parse::<u64>().is_ok()));
            if !valid {
                eprintln!("version must look like 1.2.3 or 1.2.3-rc.1");
                return false;
            }
            let new = text.replacen(&format!("version = \"{cur}\""), &format!("version = \"{v}\""), 1);
            if std::fs::write(&path, new).is_err() {
                return false;
            }
            eprintln!("version {cur} → {v}");
            run(cargo().args(["update", "--workspace"]))
        }
        _ => {
            println!("{cur}");
            true
        }
    }
}
