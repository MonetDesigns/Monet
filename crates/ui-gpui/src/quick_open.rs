//! Quick Open — the search panel in the top-right corner (Figma-style Cmd+K).
//!
//! Three sections, in this order: sections of the current document, recently closed files and
//! recent files. Rows are plain data here; `lib.rs` turns them into the shared menu-card rows.
//! All paths come from the engine's preferences, so the list survives a restart.

use figcraft_engine::Session;
use figcraft_i18n::tr;
use std::path::Path;

/// One pickable row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// `page:<uid>` / `file:<path>` — `CanvasView::pick_quick_open` dispatches on this.
    pub key: String,
    pub label: String,
    /// Right-hand hint (e.g. `Ctrl+Shift+T` for the newest closed file), if any.
    pub hint: Option<String>,
}

/// One named section of the panel.
#[derive(Clone, Debug)]
pub struct Section {
    pub name: String,
    pub rows: Vec<Row>,
}

fn matches(q: &str, label: &str) -> bool {
    q.is_empty() || label.to_lowercase().contains(&q.trim().to_lowercase())
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// Build the three sections for `query`. Newest first; each list is capped by the engine's
/// `MAX_RECENT` (already enforced when the preference is written).
pub fn sections(session: &Session, query: &str) -> Vec<Section> {
    let mut out = Vec::new();

    // 1) Sections of the current document (the pages list).
    if let Ok(doc) = session.doc() {
        let pages: Vec<Row> = doc.pages.iter().filter_map(|&id| {
            let name = doc.node(id)?.name.clone();
            matches(query, &name).then(|| Row { key: format!("page:{}", id.0), label: name, hint: None })
        }).collect();
        out.push(Section { name: tr("Take me to a section").to_string(), rows: pages });
    }

    // 2) Recently closed files — reopening these is the whole point of Quick Open.
    let closed: Vec<Row> = session.prefs.recently_closed.iter().filter(|p| matches(query, &file_name(p))).map(|p| Row {
        key: format!("file:{p}"),
        label: file_name(p),
        hint: None,
    }).collect();
    out.push(Section { name: tr("Recently closed").to_string(), rows: closed });

    // 3) Recent files.
    let recent: Vec<Row> = session.prefs.recent_files.iter().filter(|p| matches(query, &file_name(p))).map(|p| Row {
        key: format!("file:{p}"),
        label: file_name(p),
        hint: None,
    }).collect();
    out.push(Section { name: tr("Recent files").to_string(), rows: recent });

    out
}
