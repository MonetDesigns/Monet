//! `cargo xtask upstream-report`: compare this tree with the upstream mirror.
//!
//! The two histories share no ancestor (Monet's history was rewritten before the GPUI port), so
//! `git merge-base` and commit counts say nothing. What is meaningful is the content difference:
//! which files differ from the mirror, and which engine commands exist upstream but not here.
//! The report is written to `docs/upstream-sync.md`; `upstream.toml` holds the anchors a human
//! updates after each sync (upstream commit, mirror commit, audit date).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(root).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

fn toml_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(key) else { continue };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        return Some(rest.trim().trim_matches('"').to_string());
    }
    None
}

/// Command ids declared by `spec!(id, …)` in `crates/engine/src/cmd`, read from a tree-ish
/// (`HEAD`, `benchmark/main`) or from the working tree (`None`).
fn command_ids(root: &Path, tree: Option<&str>) -> BTreeSet<String> {
    let files: Vec<String> = match tree {
        Some(tree) => git(root, &["ls-tree", "-r", "--name-only", tree, "--", "crates/engine/src/cmd"])
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default(),
        None => {
            let mut out = Vec::new();
            fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
                let Ok(rd) = std::fs::read_dir(dir) else { return };
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, base, out);
                    } else if p.extension().is_some_and(|x| x == "rs")
                        && let Ok(rel) = p.strip_prefix(base)
                    {
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
            walk(&root.join("crates/engine/src/cmd"), root, &mut out);
            out
        }
    };

    let mut ids = BTreeSet::new();
    for rel in files {
        let text = match tree {
            Some(tree) => git(root, &["show", &format!("{tree}:{rel}")]),
            None => std::fs::read_to_string(root.join(&rel)).ok(),
        };
        let Some(text) = text else { continue };
        let bytes = text.as_bytes();
        let mut at = 0;
        while let Some(pos) = text[at..].find("spec!") {
            let mut i = at + pos + "spec!".len();
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'(' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'"' {
                    let start = i + 1;
                    let mut end = start;
                    while end < bytes.len() && bytes[end] != b'"' {
                        end += 1;
                    }
                    if end < bytes.len() {
                        ids.insert(text[start..end].to_string());
                    }
                }
            }
            at = at + pos + "spec!".len();
        }
    }
    ids
}

/// A compact `dir` → count table for the files that differ from the mirror.
fn group_by_area(lines: &str) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for line in lines.lines() {
        let Some(path) = line.split('\t').next_back() else { continue };
        let area = if let Some(rest) = path.strip_prefix("crates/") {
            let crate_name = rest.split('/').next().unwrap_or(rest);
            format!("crates/{crate_name}")
        } else if let Some(rest) = path.strip_prefix("docs/") {
            let first = rest.split('/').next().unwrap_or(rest);
            format!("docs/{first}")
        } else if let Some(rest) = path.strip_prefix("apps/") {
            format!("apps/{}", rest.split('/').next().unwrap_or(rest))
        } else {
            "(root)".to_string()
        };
        *out.entry(area).or_insert(0) += 1;
    }
    out
}

pub fn report(root: &Path) -> bool {
    let Ok(anchor_text) = std::fs::read_to_string(root.join("upstream.toml")) else {
        eprintln!("upstream-report: upstream.toml is missing");
        return false;
    };
    let repository = toml_value(&anchor_text, "repository").unwrap_or_default();
    let anchor_commit = toml_value(&anchor_text, "commit").unwrap_or_default();
    let anchor_date = toml_value(&anchor_text, "commit_date").unwrap_or_default();
    let mirror_ref = toml_value(&anchor_text, "mirror").unwrap_or_else(|| "benchmark/main".into());
    let compare_ref = toml_value(&anchor_text, "compare").unwrap_or_else(|| "upstream/main".into());

    let Some(local_head) = git(root, &["rev-parse", "HEAD"]) else {
        eprintln!("upstream-report: not a git checkout");
        return false;
    };
    let local_branch = git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let base_commit = git(root, &["rev-parse", &format!("{compare_ref}^{{commit}}")])
        .or_else(|| git(root, &["rev-parse", &format!("{mirror_ref}^{{commit}}")]))
        .unwrap_or_else(|| {
            eprintln!("upstream-report: neither {compare_ref} nor {mirror_ref} is available; run `git fetch upstream benchmark`");
            std::process::exit(1);
        });
    let base_date = git(root, &["log", "-1", "--format=%cs", &base_commit]).unwrap_or_default();
    let mirror_commit = git(root, &["rev-parse", &format!("{mirror_ref}^{{commit}}")]).unwrap_or_default();
    let mirror_date = git(root, &["log", "-1", "--format=%cs", &mirror_commit]).unwrap_or_default();

    let local_ids = command_ids(root, None);
    let mirror_ids = command_ids(root, Some(&base_commit));
    let missing_here: Vec<&String> = mirror_ids.difference(&local_ids).collect();
    let only_here: Vec<&String> = local_ids.difference(&mirror_ids).collect();

    let diff = git(root, &["diff", "--name-status", &base_commit, "--", "crates", "apps", "docs", "Cargo.toml", "AGENTS.md", "CONTRIBUTING.md"])
        .unwrap_or_default();
    let changed = diff.lines().count();
    let areas = group_by_area(&diff);

    let mut md = String::new();
    md.push_str("# Upstream sync report\n\n");
    md.push_str("> Generated by `cargo xtask upstream-report`. Do not edit by hand; update\n");
    md.push_str("> `upstream.toml` after a sync and re-run the task.\n\n");
    md.push_str("| Anchor | Value |\n|---|---|\n");
    md.push_str(&format!("| Upstream repository | `{repository}` |\n"));
    md.push_str(&format!("| Upstream commit (anchor) | `{anchor_commit}` ({anchor_date}) |\n"));
    md.push_str(&format!("| Compared against | `{compare_ref}` @ `{base_commit}` ({base_date}) |\n"));
    md.push_str(&format!("| Mirror | `{mirror_ref}` @ `{mirror_commit}` ({mirror_date}) |\n"));
    md.push_str(&format!("| This tree | `{local_branch}` @ `{local_head}` |\n"));
    md.push_str(&format!("| Files that differ from upstream | {changed} |\n\n"));

    md.push_str("## Engine commands\n\n");
    md.push_str(&format!(
        "- This tree: **{}** commands\n- Upstream: **{}** commands\n- Present upstream, missing here: **{}**\n- Present here, absent upstream: **{}**\n\n",
        local_ids.len(),
        mirror_ids.len(),
        missing_here.len(),
        only_here.len()
    ));
    if !missing_here.is_empty() {
        md.push_str("### Present upstream, missing here\n\n");
        for id in &missing_here {
            md.push_str(&format!("- `{id}`\n"));
        }
        md.push('\n');
    }

    md.push_str("## Files that differ from the mirror, by area\n\n");
    md.push_str("| Area | Files |\n|---|---|\n");
    let mut areas_sorted: Vec<(&String, &usize)> = areas.iter().collect();
    areas_sorted.sort_by(|a, b| b.1.cmp(a.1));
    for (area, count) in &areas_sorted {
        md.push_str(&format!("| `{area}` | {count} |\n"));
    }

    md.push_str("\n## How to read this\n\n");
    md.push_str("No common ancestor exists between the two histories, so commit counts are meaningless. ");
    md.push_str("A file listed here may differ only because this repository carries its own work ");
    md.push_str("(`crates/ui-gpui`, `apps/monet`); the ones to inspect first are `crates/engine`, ");
    md.push_str("`crates/doc`, `crates/format` and `docs/`. The command list above is the hard signal: ");
    md.push_str("a missing id is a feature the engine knows upstream and this tree does not.\n");

    let path = root.join("docs/upstream-sync.md");
    if let Err(e) = std::fs::write(&path, md) {
        eprintln!("upstream-report: {}: {e}", path.display());
        return false;
    }
    println!(
        "upstream-report: {} files differ; commands local {} / mirror {} (missing here {}, only here {}) → docs/upstream-sync.md",
        changed,
        local_ids.len(),
        mirror_ids.len(),
        missing_here.len(),
        only_here.len()
    );
    true
}
