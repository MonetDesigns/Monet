//! A16: several real OS windows, ported from `ui-egui/src/windows.rs`.
//!
//! The engine session owns every open document. Rather than duplicating or
//! locking `Session` (475+ call sites expect `&mut Session`), each OS window
//! mounts the **same** `Entity<CanvasView>` as its root view: one shared app,
//! one shared session, one set of tabs per window selected by `docs`.
//! `current` is set at the start of each frame/event so tab and title
//! lookups use the window the user is actually interacting with.

use crate::CanvasView;
use gpui::{AnyWindowHandle, App, Bounds, Context, Entity, Point, Pixels, SharedString, WindowBounds, WindowId, WindowOptions, size, px};
use serde_json::{Value, json};
use std::cell::RefCell;

/// One extra window; id 0 is the main window.
#[derive(Clone)]
pub(crate) struct Entry {
    pub(crate) id: u64,
    pub(crate) window_id: WindowId,
    pub(crate) handle: AnyWindowHandle,
    /// Documents this window owns (others drop the same uid on detach).
    pub(crate) docs: Vec<u64>,
    pub(crate) active: Option<u64>,
    pub(crate) title: String,
}

#[derive(Default)]
pub(crate) struct Windows {
    pub(crate) list: Vec<Entry>,
    pub(crate) current: u64,
    pub(crate) focused: u64,
    /// The main window's active file while another window draws.
    pub(crate) main_active: Option<u64>,
    next_id: u64,
}

thread_local! {
    /// The window whose frame is being built right now (0: main).
    static CURRENT: RefCell<u64> = const { RefCell::new(0) };
}

pub(crate) fn set_current(id: u64) {
    CURRENT.with(|c| *c.borrow_mut() = id);
}

pub(crate) fn current() -> u64 {
    CURRENT.with(|c| *c.borrow())
}

impl Windows {
    pub(crate) fn of(&self, uid: u64) -> u64 {
        self.list.iter().find(|w| w.docs.contains(&uid)).map_or(0, |w| w.id)
    }

    pub(crate) fn exists(&self, id: u64) -> bool {
        id == 0 || self.list.iter().any(|w| w.id == id)
    }

    /// The window drawing now shows this file.
    pub(crate) fn shows(&self, uid: u64) -> bool {
        self.of(uid) == self.current
    }

    pub(crate) fn entry(&self, id: u64) -> Option<&Entry> {
        self.list.iter().find(|w| w.id == id)
    }

    pub(crate) fn entry_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.list.iter_mut().find(|w| w.id == id)
    }
}

fn title_of(name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{name} – Monet"),
        None => "Monet".to_string(),
    }
}

/// Open a real OS window for open file `index`, the file at `path`, or a new
/// file. Mirrors EGUI `windows::open`: the uid is removed from every other
/// window, the new window owns it and the window opens offset from the caller.
pub(crate) fn open(
    windows: &mut Windows,
    cx: &mut App,
    root: Entity<CanvasView>,
    index: Option<usize>,
    path: Option<&str>,
) -> Result<u64, String> {
    let uid = match index {
        Some(i) => doc_uid(cx, &root, i)?,
        None => open_doc(cx, &root, path)?,
    };
    for w in &mut windows.list {
        w.docs.retain(|u| *u != uid);
        if w.active == Some(uid) {
            w.active = w.docs.first().copied();
        }
    }
    let name = root
        .read(cx)
        .app
        .session
        .docs
        .iter()
        .find(|d| d.uid == uid)
        .map(|d| d.doc.name.to_string());
    let title = title_of(name.as_deref());
    windows.next_id += 1;
    let id = windows.next_id;

    let bounds = Bounds::centered(None, size(px(1280.0), px(860.0)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(SharedString::from(title.clone())),
            appears_transparent: true,
            traffic_light_position: Some(Point::new(px(9.0), px(9.0))),
        }),
        show: false,
        ..Default::default()
    };
    let handle: AnyWindowHandle = cx
        .open_window(options, |window, cx| {
            crate::a11y::install(window, cx);
            root.clone()
        })
        .map_err(|e| e.to_string())?
        .into();
    let window_id = handle.window_id();
    windows.list.push(Entry { id, window_id, handle, docs: vec![uid], active: Some(uid), title });
    windows.focused = id;
    Ok(id)
}

fn doc_uid(cx: &App, root: &Entity<CanvasView>, index: usize) -> Result<u64, String> {
    root.read(cx)
        .app
        .session
        .docs
        .get(index)
        .map(|d| d.uid)
        .ok_or_else(|| format!("no open file {index}"))
}

fn open_doc(cx: &mut App, root: &Entity<CanvasView>, path: Option<&str>) -> Result<u64, String> {
    let (command, params) = match path {
        Some(path) => ("file.open", json!({ "path": path })),
        None => ("file.new", json!({})),
    };
    let uid = root
        .update(cx, |view, cx| {
            view.app.session.execute(command, &params).map_err(|e| e.to_string())?;
            view.app.session.ds().ok().map(|d| d.uid).ok_or_else(|| "no file opened".to_string())
        })
        .map_err(|e| e.to_string())?;
    Ok(uid)
}

/// Bring window `id` to the front (engine `window.focus`).
pub(crate) fn focus(windows: &Windows, cx: &mut App, id: u64) -> Result<Value, String> {
    let entry = windows.entry(id).ok_or_else(|| format!("no window {id}"))?;
    entry.handle.update(cx, |_, window, _| window.activate_window()).map_err(|e| e.to_string())?;
    Ok(json!({"focused": id}))
}

/// Close an extra window; its files close with it (engine/file lifecycle).
pub(crate) fn close(windows: &mut Windows, cx: &mut App, id: u64) {
    if let Some(entry) = windows.entry(id) {
        let _ = entry.handle.update(cx, |_, window, _| window.remove_window());
    }
    windows.list.retain(|w| w.id != id);
    if windows.focused == id {
        windows.focused = 0;
    }
}

/// Handle engine `window.new` / `window.focus` requests.
pub(crate) fn requests(
    windows: &mut Windows,
    cx: &mut Context<CanvasView>,
    root: Entity<CanvasView>,
    kind: &str,
    data: &Value,
) -> Result<(), String> {
    match kind {
        "window.new" => {
            let index = data.get("index").and_then(Value::as_u64).map(|i| i as usize);
            let path = data.get("path").and_then(Value::as_str);
            open(windows, cx, root, index, path).map(|_| ())
        }
        "window.focus" => {
            let id = data.get("id").and_then(Value::as_u64).unwrap_or(u64::MAX);
            focus(windows, cx, id).map(|_| ())
        }
        _ => Ok(()),
    }
}

/// Control-channel `window.list`: id, main flag, title, files, active, focus.
pub(crate) fn list(windows: &Windows, cx: &App, root: &Entity<CanvasView>) -> Value {
    let active_of = |id: u64| -> Option<u64> {
        if id == windows.current {
            root.read(cx).app.session.ds().ok().map(|d| d.uid)
        } else if id == 0 {
            windows.main_active
        } else {
            windows.entry(id).and_then(|w| w.active)
        }
    };
    let names = root
        .read(cx)
        .app
        .session
        .docs
        .iter()
        .map(|d| (d.uid, d.doc.name.to_string()))
        .collect::<Vec<_>>();
    let ids = std::iter::once(0).chain(windows.list.iter().map(|w| w.id));
    Value::Array(
        ids.map(|id| {
            let files = if id == windows.current {
                names.iter().filter(|(uid, _)| windows.shows(*uid)).map(|(_, n)| n.clone()).collect::<Vec<_>>()
            } else {
                names.iter().filter(|(uid, _)| windows.of(*uid) == id).map(|(_, n)| n.clone()).collect::<Vec<_>>()
            };
            let title = if id == 0 {
                title_of(files.first().map(String::as_str))
            } else {
                windows.entry(id).map(|w| w.title.clone()).unwrap_or_else(|| "Monet".into())
            };
            let active = active_of(id).and_then(|uid| names.iter().find(|(u, _)| *u == uid).map(|(_, n)| n.clone()));
            json!({
                "id": id,
                "main": id == 0,
                "title": title,
                "files": files,
                "active": active,
                "focused": id == windows.focused,
            })
        })
        .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_follows_the_active_file() {
        assert_eq!(title_of(Some("Board")), "Board – Monet");
        assert_eq!(title_of(None), "Monet");
    }
}
