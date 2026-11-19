//! A16: several real OS windows, ported from `ui-egui/src/windows.rs`.
//!
//! The engine session owns every open document. Rather than duplicating or
//! locking `Session` (475+ call sites expect `&mut Session`), each OS window
//! mounts the **same** `Entity<CanvasView>` as its root view: one shared app,
//! one shared session, one set of tabs per window selected by `docs`.
//! `current` is set at the start of each frame/event so tab and title
//! lookups use the window the user is actually interacting with.

use crate::CanvasView;
use gpui::{AnyWindowHandle, App, Bounds, Context, Entity, Pixels, Point, SharedString, WindowBounds, WindowId, WindowOptions, point, px, size};
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
    /// **借用**的文档：演示拖出成的独立预览窗口只显示播放器，要有源文档才能渲染，
    /// 但它不是这个窗口的设计标签（`docs` 里不放，标签栏就不会冒出设计页）
    /// （用户 2026-11-17：拖出预览时设计页不能被一起带出来）。
    pub(crate) borrowed: Option<u64>,
    pub(crate) active: Option<u64>,
    pub(crate) title: String,
}

/// 正在创建、还没拿到 `AnyWindowHandle` 的窗口。`cx.open_window` 会在返回前同步跑一次
/// 首帧绘制，那一帧已经需要知道「这个 OS 窗口是哪个窗口 id、拥有哪些文档」——否则新窗口
/// 的首帧会被当成主窗口（把别人的 uid 写进 `main_active`、面板状态也换错）。
/// 拿到 handle 后立刻转成真正的 [`Entry`] 并从这里删除。
#[derive(Clone)]
pub(crate) struct Pending {
    pub(crate) window_id: WindowId,
    pub(crate) id: u64,
    pub(crate) uid: u64,
    /// `true` = 只借用（预览窗口），不能因此抢走文档归属 / 冒出设计标签。
    pub(crate) borrow: bool,
}

#[derive(Default)]
pub(crate) struct Windows {
    pub(crate) list: Vec<Entry>,
    pub(crate) pending: Vec<Pending>,
    pub(crate) current: u64,
    pub(crate) focused: u64,
    /// The main window's active file while another window draws.
    pub(crate) main_active: Option<u64>,
    /// 演示标签被拖出成新窗口时寄存在这里：新窗口建 pane 时接手（用户 2026-11-17）。
    /// `(源文档 uid, 目标窗口 id, 标签)`：借用模式下 `of(uid)` 仍指向原窗口，只能靠
    /// 目标窗口 id 判断谁该接手（否则新窗口永远接不到，显示成原来的设计画布）。
    pub(crate) pending_present: Option<(u64, Option<u64>, crate::present::PresentTab)>,
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
        // Pending 优先：文档刚要转到新窗口时，其它窗口的 `docs` 还没清理，但归属必须
        // 已经是新窗口（否则首帧里新窗口写不进 active、旧窗口还会认领它）。
        // 借用型 pending（预览窗口）不参与归属：原窗口继续拥有，预览窗口也不冒出设计标签。
        if let Some(p) = self.pending.iter().find(|p| p.uid == uid && !p.borrow) {
            return p.id;
        }
        self.list.iter().find(|w| w.docs.contains(&uid)).map_or(0, |w| w.id)
    }

    /// 窗口 `id` 是否**显示**这份文档：自己拥有，或作为预览窗口借用它。
    /// 与 [`Self::of`] 的区别：借用不算归属，但绑定 `session.active` / 画布仍然需要它。
    pub(crate) fn shows_uid(&self, id: u64, uid: u64) -> bool {
        if self.pending.iter().any(|p| p.id == id && p.uid == uid) {
            return true;
        }
        if id == 0 {
            // 主窗口没有 Entry：没被任何额外窗口拥有的文档就是它显示的。
            return self.of(uid) == 0;
        }
        self.entry(id).is_some_and(|e| e.docs.contains(&uid) || e.borrowed == Some(uid))
    }

    pub(crate) fn exists(&self, id: u64) -> bool {
        id == 0 || self.list.iter().any(|w| w.id == id) || self.pending.iter().any(|p| p.id == id)
    }

    /// OS 窗口 id → 我们登记的窗口 id（pending 也算，0 = 主窗口 / 未登记）。
    pub(crate) fn id_for(&self, window_id: WindowId) -> u64 {
        if let Some(w) = self.list.iter().find(|w| w.window_id == window_id) {
            return w.id;
        }
        if let Some(p) = self.pending.iter().find(|p| p.window_id == window_id) {
            return p.id;
        }
        0
    }

    /// 窗口自己的活动文档（pending 的首帧期间取它将要拥有的那份）。
    pub(crate) fn active_of(&self, id: u64) -> Option<u64> {
        if id == 0 {
            return None;
        }
        self.list.iter().find(|w| w.id == id).and_then(|w| w.active).or_else(|| self.pending.iter().find(|p| p.id == id).map(|p| p.uid))
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
/// Create the OS window **outside** any `CanvasView` lease.
///
/// ⛔ `cx.open_window` 会同步 `window.draw()` 一次（gpui `app.rs:963`），而新窗口挂的是
/// **同一个** `CanvasView` 实体。如果这步发生在 `CanvasView::render` / `update` 里面
/// （`sync_ui_requests` 正是如此），draw 就会二次租借该实体 →
/// `cannot read CanvasView while it is already being updated` → 整个应用 abort。
/// 所以由调用方用 `App::defer` 把这一步推到本次更新之后。
fn open_now(
    windows: &std::rc::Rc<RefCell<Windows>>,
    cx: &mut App,
    root: Entity<CanvasView>,
    uid: u64,
    title: String,
    // 主窗口在文档被抢走后应该显示的另一份文档（没有就 None）。
    main_fallback: Option<u64>,
    // 是否把文档从原窗口**搬走**；false = 只借用（演示拖出成独立窗口，原窗口保留设计标签）。
    steal: bool,
) {
    // 新窗口**必须**和原窗口错开：`Bounds::centered` 会把新窗口正好盖在同样居中的主窗口上，
    // 看起来像「切不过去」（点哪里都还是同一个位置），也看不出新窗口出来了。
    // 按已开窗口数级联 24pt（EGUI 也是相对调用窗口偏移）。
    let cascade = windows.borrow().list.len() as f32 + 1.0;
    let base = Bounds::centered(None, size(px(1280.0), px(860.0)), cx);
    let bounds = Bounds { origin: base.origin + point(px(24.0 * cascade), px(24.0 * cascade)), size: base.size };
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
    // id 在开窗之前分配：`open_window` 返回前会同步跑一次首帧绘制，那帧里
    // `bind_window_document` 就要用这个 id（见 `Pending`）。
    let id = {
        let mut w = windows.borrow_mut();
        w.next_id += 1;
        w.next_id
    };
    let build_windows = windows.clone();
    let handle: AnyWindowHandle = match cx.open_window(options, move |window, cx| {
        crate::a11y::install(window, cx);
        {
            let mut w = build_windows.borrow_mut();
            // 文档从其它窗口转过来：首帧之前完成转移，否则首帧里 `of(uid)` 还指向旧窗口，
            // 新窗口绑不上自己的文档；旧窗口也会继续认领它。
            // `steal == false` 时只登记 pending，原窗口保留这份文档（演示窗口借用）。
            if steal {
                for e in &mut w.list {
                    e.docs.retain(|u| *u != uid);
                    if e.active == Some(uid) {
                        e.active = e.docs.first().copied();
                    }
                }
            }
            // 寄存在 `pending_present` 的演示标签：**首帧之前**写好目标窗口 id，
            // 否则新窗口的 `enter_pane` 认不出该由谁接手（借用模式下归属仍是原窗口）。
            if let Some(p) = w.pending_present.as_mut()
                && p.0 == uid
            {
                p.1 = Some(id);
            }
            w.pending.push(Pending { window_id: window.window_handle().window_id(), id, uid, borrow: !steal });
        }
        root.clone()
    }) {
        Ok(h) => h.into(),
        Err(e) => {
            eprintln!("open_window: {e}");
            windows.borrow_mut().pending.retain(|p| p.id != id);
            return;
        }
    };
    let window_id = handle.window_id();
    // 注意：`open_window` 的首次 draw 已经跑完（也读写过 `self.windows`），此刻才能借。
    let mut w = windows.borrow_mut();
    w.pending.retain(|p| p.id != id);
    // `steal == false`（预览窗口）只**借用**：文档不进 `docs`，原窗口的设计标签保持不动，
    // 预览窗口的标签栏也不会冒出设计页（用户 2026-11-17）。
    w.list.push(Entry {
        id,
        window_id,
        handle,
        docs: if steal { vec![uid] } else { Vec::new() },
        borrowed: (!steal).then_some(uid),
        active: Some(uid),
        title,
    });
    // 首帧期间归属已经正确（pending），但主窗口若在这之前就被写脏，这里兜一次底。
    if steal && w.main_active == Some(uid) {
        w.main_active = main_fallback;
    }
    w.focused = id;
    drop(w);
    // `show: false` + Windows 后端：gpui 只在 `activate()` 里 `SetWindowPlacement`/显示窗口，
    // 所以少了这一步新窗口一直隐藏、用户「切不过去」。登记完 Entry 后再激活。
    let _ = handle.update(cx, |_, window, _| window.activate_window());
}

fn doc_uid(session: &figcraft_engine::Session, index: usize) -> Result<u64, String> {
    session.docs.get(index).map(|d| d.uid).ok_or_else(|| format!("no open file {index}"))
}

fn open_doc(session: &mut figcraft_engine::Session, path: Option<&str>) -> Result<u64, String> {
    let (command, params) = match path {
        Some(path) => ("file.open", json!({ "path": path })),
        None => ("file.new", json!({})),
    };
    session.execute(command, &params).map_err(|e| e.to_string())?;
    session.ds().ok().map(|d| d.uid).ok_or_else(|| "no file opened".to_string())
}

/// Bring window `id` to the front (engine `window.focus`).
pub(crate) fn focus(windows: &std::rc::Rc<RefCell<Windows>>, cx: &mut App, id: u64) -> Result<Value, String> {
    // 取 handle 后立刻放开借用：`handle.update` 会租借同一个 `CanvasView`，不能和 `borrow` 重叠。
    let handle = windows.borrow().entry(id).map(|e| e.handle).ok_or_else(|| format!("no window {id}"))?;
    handle.update(cx, |_, window, _| window.activate_window()).map_err(|e| e.to_string())?;
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
    windows: &std::rc::Rc<RefCell<Windows>>,
    cx: &mut App,
    root: Entity<CanvasView>,
    session: &mut figcraft_engine::Session,
    kind: &str,
    data: &Value,
) -> Result<(), String> {
    match kind {
        "window.new" => {
            let index = data.get("index").and_then(Value::as_u64).map(|i| i as usize);
            let uid_by_id = data.get("uid").and_then(Value::as_u64);
            let path = data.get("path").and_then(Value::as_str);
            // 文档先备好：这一步只动 `session`，在 render 里做是安全的。
            let uid = match (uid_by_id, index, path) {
                (Some(uid), _, _) => uid,
                (None, Some(i), _) => doc_uid(session, i)?,
                (None, None, path) => open_doc(session, path)?,
            };
            let title = title_of(session.docs.iter().find(|d| d.uid == uid).map(|d| d.doc.name.to_string()).as_deref());
            // `keep` = 新窗口**只借用**这份文档（演示拖出成独立窗口）：原窗口的标签要留着，
            // 不能把设计一起搬过去（用户 2026-11-17：拖出演示时设计窗口跟着跑进新窗口）。
            let keep = data.get("keep").and_then(Value::as_bool).unwrap_or(false);
            if !keep
                && session.docs.get(session.active).map(|d| d.uid) == Some(uid)
                && let Some(i) = session.docs.iter().position(|d| d.uid != uid)
            {
                session.active = i;
            }
            // 建窗必须推迟到本次更新之后（见 `open_now` 的说明）。
            let main_fallback = if keep { None } else { session.docs.iter().find(|d| d.uid != uid).map(|d| d.uid) };
            let wins = windows.clone();
            cx.defer(move |cx| open_now(&wins, cx, root, uid, title, main_fallback, !keep));
            Ok(())
        }
        "window.merge" => {
            // 把文件从额外窗口塞回主窗口（用户 2026-11-16：拖出去之后没法塞回去）。
            let Some(index) = data.get("index").and_then(Value::as_u64).map(|i| i as usize) else { return Ok(()) };
            let Some(uid) = session.docs.get(index).map(|d| d.uid) else { return Ok(()) };
            {
                let mut w = windows.borrow_mut();
                for e in &mut w.list {
                    e.docs.retain(|u| *u != uid);
                    if e.active == Some(uid) {
                        e.active = e.docs.first().copied();
                    }
                }
                w.main_active = Some(uid);
                // 空掉的额外窗口直接关掉，不留一个空白窗口。但「借用」窗口的 docs 本来就恒空
                // （它只借一份演示播放器），合并别的文件不能把它关掉（CodeRabbit PR#32）。
                // 关窗会租借 `CanvasView`，必须在 `Windows` 借用结束后 defer 执行，否则重入
                // panic（CodeRabbit PR#30/#31）。
                let empties: Vec<_> =
                    w.list.iter().filter(|e| e.docs.is_empty() && e.borrowed.is_none()).filter_map(|e| w.entry(e.id).map(|en| en.handle)).collect();
                w.list.retain(|e| !e.docs.is_empty() || e.borrowed.is_some());
                w.focused = 0;
                cx.defer(move |cx| {
                    for handle in empties {
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                    }
                });
            }
            if let Some(pos) = session.docs.iter().position(|d| d.uid == uid) {
                session.active = pos;
            }
            Ok(())
        }
        "window.focus" => {
            let id = data.get("id").and_then(Value::as_u64).unwrap_or(u64::MAX);
            // `handle.update` 会租借同一个 `CanvasView`，同样必须推迟。
            let wins = windows.clone();
            cx.defer(move |cx| {
                let _ = focus(&wins, cx, id);
            });
            Ok(())
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
    let names = root.read(cx).app.session.docs.iter().map(|d| (d.uid, d.doc.name.to_string())).collect::<Vec<_>>();
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
