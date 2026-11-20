//! The Home screen (A1): a 240 pt sidebar (wordmark, search, Recents / Online / Templates) and the
//! main area (action cards, recent files, Getting started).
//!
//! Structure copied from `crates/ui-egui/src/home.rs`. The `Recents` grid/sort and the Online
//! section are the parts of `home/home_recents.rs` / `home/online.rs` still to come; this file is
//! where they land.

use crate::CanvasApp;
use crate::panels::extra::ExtraState;
use crate::panels::{Cmd, Cmds};
use crate::widgets::{Icon, MenuRow, pt, rgba, tokens};
use figcraft_i18n::tr;
use figcraft_i18n::tr_noop;
use gpui::{App, Bounds, Div, Entity, MouseButton, Pixels, SharedString, Window, div, prelude::*, px};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

/// `ui-egui/src/home.rs::TIPS`: the six Getting-started rows (title, one-line hint).
const HOME_TIPS: [(&str, &str); 6] = [
    (tr_noop("Draw a frame"), tr_noop("Pick the Frame tool, then drag on the canvas or choose a device size")),
    (tr_noop("Add auto layout"), tr_noop("Select layers and they space themselves as their content changes")),
    (tr_noop("Make a component"), tr_noop("Turn a layer into a component, reuse it, and change every copy at once")),
    (tr_noop("Prototype"), tr_noop("Link frames on the Prototype tab, then present the flow")),
    (tr_noop("Share with a code"), tr_noop("Start sharing with the Share button, then pass on the code it gives")),
    (tr_noop("Export"), tr_noop("Export frames and layers as PNG, JPG, SVG or PDF")),
];

/// The whole Home screen: sidebar + main. `new_page` is the「+」临时标签页：它画成 参考编辑器 桌面
/// 首页的样子（居中分类胶囊、搜索框、最近文件带缩略图），没有左侧边栏。
pub(crate) fn body(app: &CanvasApp, cmds: &Cmds, state: &ExtraState, root: Option<Entity<crate::CanvasView>>, new_page: bool) -> Div {
    if new_page {
        return new_page_body(app, cmds, state, root);
    }
    let t = tokens();
    let section = *state.home_section.borrow();
    let q = state.home_query.borrow().to_lowercase();

    // ── 240 pt sidebar: wordmark, file/template search, three section tabs ──────
    let mut side = div().flex_none().w(px(240.0)).h_full().flex().flex_col().gap(pt(8.0)).p(pt(12.0)).border_r_1().border_color(rgba(t.border));
    side = side.child(
        div()
            .h(px(40.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(10.0))
            // Brand mark: the three anchor dots in red over the blue path (the mono version is
            // used by the rail / main menu). The renderer only keeps alpha, so each layer is
            // tinted separately instead of relying on the SVG's own colours.
            .child(gpui::svg().path("brand/monet-dots.svg").w(px(24.0)).h(px(24.0)).text_color(gpui::Hsla::from(gpui::rgb(0x007BE5))))
            .child(div().text_size(pt(16.0)).font_weight(crate::panels::UI_FONT_REGULAR).text_color(rgba(t.text)).child(tr("Monet"))),
    );
    side = side.child(state.home_search.clone());
    for (i, (icon, label)) in [(Icon::Page, tr("Recents")), (Icon::LinkOn, tr("Online")), (Icon::ViewGrid, tr("Templates"))].into_iter().enumerate() {
        let on = section == i;
        let slot = state.home_section.clone();
        let wake = cmds.clone();
        side = side.child(
            div()
                .h(px(32.0))
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(10.0))
                .px(pt(10.0))
                .rounded(pt(5.0))
                .bg(rgba(if on { t.selected_row } else { t.panel }))
                .hover(|s| s.bg(rgba(t.hover)))
                .cursor_pointer()
                .child(crate::widgets::icon::paint(icon, rgba(t.icon), 16.0))
                .child(div().text_size(pt(12.0)).text_color(rgba(t.text)).child(tr(label)))
                .on_mouse_down(MouseButton::Left, move |_e, _, _| {
                    *slot.borrow_mut() = i;
                    wake.wake();
                }),
        );
    }

    // ── main: heading + cards + files + tips, scrolled like the egui CentralPanel ─
    let mut main = div().id("home-main").flex_1().min_w(px(0.0)).h_full().flex().flex_col().gap(pt(16.0)).p(pt(24.0)).overflow_y_scroll();
    match section {
        2 => {
            // egui `home.rs::templates`：标题 + 卡片网格（248×234，缩略图 148 高），
            // 点一张就 `file.newFromTemplate`。以前这里是三张动作卡，根本不是模板。
            main = main.child(heading(tr("Templates")));
            let shown: Vec<&figcraft_engine::templates::Template> = figcraft_engine::templates::TEMPLATES
                .iter()
                .filter(|x| q.is_empty() || x.name.to_lowercase().contains(&q) || x.description.to_lowercase().contains(&q))
                .collect();
            if shown.is_empty() {
                main = main.child(note(tr("No templates match your search")));
            } else {
                let mut grid = div().w_full().flex().flex_row().flex_wrap().gap(pt(20.0));
                for tpl in shown {
                    grid = grid.child(template_card(state, tpl, cmds));
                }
                main = main.child(grid);
            }
        }
        1 => {
            // egui `home/online.rs::section`：标题 + 说明 + 本机在线文件列表（空态另说）。
            main = main.child(heading(tr("Online files")));
            main =
                main.child(note(tr("The files you shared or published online from this computer. They stay on their server until you remove them.")));
            let files = &app.session.prefs.online_files;
            if files.is_empty() {
                main = main.child(note(tr("Nothing online yet. Use Share on a file to put it online and it is listed here.")));
            } else {
                let mut list = div().w_full().flex().flex_col();
                for f in files.iter().take(40) {
                    list = list.child(
                        div()
                            .w_full()
                            .h(px(36.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(pt(10.0))
                            .px(pt(10.0))
                            .rounded(pt(5.0))
                            .child(crate::widgets::icon::paint(Icon::LinkOn, rgba(t.icon), 16.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .text_size(pt(12.0))
                                    .text_color(rgba(t.text))
                                    .child(SharedString::from(f.name.clone())),
                            )
                            .child(div().text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(SharedString::from(f.server.clone()))),
                    );
                }
                main = main.child(list);
            }
        }
        _ => {
            main = main.child(heading(tr("Recents")));
            let mut cards = div().flex().flex_row().flex_wrap().gap(pt(12.0));
            cards = cards.child(action_card(Icon::Frame, tr("New design file"), tr("Start from a blank canvas"), "file.new", cmds));
            cards = cards.child(action_card(Icon::Rectangle, tr("New board"), tr("Sticky notes, shapes and connectors"), "file.newBoard", cmds));
            cards = cards.child(action_card(Icon::Page, tr("Open file"), tr("Open .figcraft, .svg or images"), "file.open", cmds));
            main = main.child(cards);
            if !app.session.prefs.getting_started_dismissed {
                main = main.child(subheading(tr("Getting started")));
                let mut tips = div().w_full().flex().flex_col().gap(pt(8.0)).p(pt(16.0)).rounded(pt(8.0)).bg(rgba(t.field));
                for (i, (title, line)) in HOME_TIPS.iter().enumerate() {
                    tips =
                        tips.child(
                            div()
                                .w_full()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(pt(10.0))
                                .child(div().size(pt(20.0)).rounded(pt(10.0)).bg(rgba(t.panel)).flex().items_center().justify_center().child(
                                    div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from((i + 1).to_string())),
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .child(div().text_size(pt(12.0)).text_color(rgba(t.text)).child(tr(title)))
                                        .child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr(line))),
                                ),
                        );
                }
                main = main.child(tips);
            }
            // Toolbar: "Recent files" + a grid/list toggle + a name/edited sort
            // (egui `home_recents::View` + `Sort`).
            let grid = *state.home_view_grid.borrow();
            main = main.child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(8.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(pt(13.0))
                            .font_weight(crate::panels::UI_FONT_REGULAR)
                            .text_color(rgba(t.text))
                            .child(tr("Recent files")),
                    )
                    .child(cycle_bool(if grid { tr("Grid") } else { tr("List") }, state.home_view_grid.clone(), cmds))
                    .child(cycle_u8(if *state.home_sort.borrow() == 0 { tr("Name") } else { tr("Edited") }, state.home_sort.clone(), 2, cmds)),
            );
            let mut files: Vec<String> =
                app.session.prefs.recent_files.iter().filter(|p| q.is_empty() || p.to_lowercase().contains(&q)).cloned().collect();
            if *state.home_sort.borrow() == 0 {
                files.sort_by_key(|p| std::path::Path::new(p).file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default());
            } else {
                files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
            }
            if files.is_empty() {
                main = main.child(note(tr("No recent files yet. Open or save a file to see it here.")));
            } else if grid {
                let mut g = div().w_full().flex().flex_row().flex_wrap().gap(pt(12.0));
                for path in files.iter().take(12) {
                    g = g.child(recent_card(path, cmds, state.dismiss.clone(), root.clone()));
                }
                main = main.child(g);
            } else {
                for path in files.iter().take(12) {
                    main = main.child(recent_row(path, cmds, state.dismiss.clone(), root.clone()));
                }
            }
        }
    }
    div().flex().flex_row().w_full().h_full().child(side).child(main)
}

/// 「+」的临时页：顶部三个动作（新建设计稿 / 新建画板 / 打开文件，和侧栏 Recents 的那三个
/// action card 一致），下面一个搜索框，再下面 "Recents" 和最近文件行（缩略图 + 名字 + 相对时间）。
/// 没有侧边栏。
fn new_page_body(app: &CanvasApp, cmds: &Cmds, state: &ExtraState, root: Option<Entity<crate::CanvasView>>) -> Div {
    let t = tokens();
    let q = state.home_query.borrow().to_lowercase();
    let mut col = div().w(px(448.0)).flex().flex_col().gap(pt(10.0));

    // 三个动作：直接推到引擎命令，和首页侧栏的 action card 是同一批 id。
    let mut actions = div().w_full().flex().flex_row().gap(pt(8.0));
    for (icon, label, id) in [
        (Icon::Frame, tr("New design file"), "file.new"),
        (Icon::Rectangle, tr("New board"), "file.newBoard"),
        (Icon::Page, tr("Open file"), "file.open"),
    ] {
        let c = cmds.clone();
        let dismiss = state.dismiss.clone();
        actions = actions.child(
            div()
                .id(id)
                .flex_1()
                .h(px(34.0))
                .rounded(pt(6.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap(pt(6.0))
                .bg(rgba(t.field))
                .border_1()
                .border_color(rgba(t.border))
                .cursor_pointer()
                .hover(move |s| s.bg(rgba(t.hover)))
                .on_mouse_down(MouseButton::Left, move |_e, _, _| {
                    c.push(Cmd { id, params: None });
                    // 临时页只负责发起动作：命令排进队列后就让位，别盖住新建 / 打开的画布
                    // （取消打开时 `dismiss_new_tabs` 会回首页）。
                    *dismiss.borrow_mut() = true;
                    c.wake();
                })
                .child(crate::widgets::icon::paint(icon, rgba(t.icon), 14.0))
                .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(label))),
        );
    }
    col = col.child(actions).child(state.home_search.clone());

    col = col
        .child(div().w_full().pt(pt(6.0)).text_size(pt(11.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(tr("Recents")));

    // 最近文件：按修改时间从新到旧，缩略图来自文件自带的 thumbnail.png（EGUI 首页的行为）。
    let mut files: Vec<String> = app.session.prefs.recent_files.iter().filter(|p| q.is_empty() || p.to_lowercase().contains(&q)).cloned().collect();
    files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    if files.is_empty() {
        col = col.child(note(tr("No recent files yet. Open or save a file to see it here.")));
    } else {
        let mut rows = div().w_full().flex().flex_col();
        for path in files.iter().take(20) {
            rows = rows.child(new_page_row(state, path, cmds, root.clone()));
        }
        col = col.child(rows);
    }

    let scroll = div()
        .id("home-new-page")
        .w_full()
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .overflow_y_scroll()
        .pt(pt(40.0))
        .pb(pt(24.0))
        .bg(rgba(t.panel))
        .child(col);
    // `overflow_y_scroll` 要 `Stateful<Div>`；外层再包一层普通 `Div` 保持 `body` 的返回类型。
    div().w_full().h_full().child(scroll)
}

/// 临时页的一行最近文件：40×30 缩略图、文件名、右对齐的相对时间。点一行打开该文件。
fn new_page_row(state: &ExtraState, path: &str, cmds: &Cmds, root: Option<Entity<crate::CanvasView>>) -> Div {
    let t = tokens();
    let name = file_name(path);
    let edited = long_ago(path);
    let c = cmds.clone();
    let dismiss = state.dismiss.clone();
    let dismiss_for_menu = dismiss.clone();
    let p = path.to_string();
    let pic = match recent_preview(state, path).0 {
        Some(tex) => div()
            .flex_none()
            .w(px(40.0))
            .h(px(30.0))
            .rounded(pt(4.0))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .child(gpui::img(gpui::ImageSource::Render(tex)).max_w(px(40.0)).max_h(px(30.0))),
        None => div()
            .flex_none()
            .w(px(40.0))
            .h(px(30.0))
            .rounded(pt(4.0))
            .bg(rgba(t.field))
            .flex()
            .items_center()
            .justify_center()
            .child(crate::widgets::icon::paint(Icon::Page, rgba(t.icon), 14.0)),
    };
    let mut row = div()
        .w_full()
        .h(px(40.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(10.0))
        .px(pt(6.0))
        .rounded(pt(5.0))
        .hover(|s| s.bg(rgba(t.hover)))
        .cursor_pointer()
        .child(pic)
        .child(div().flex_1().min_w(px(0.0)).overflow_hidden().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(name)))
        .child(div().text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(SharedString::from(edited)))
        .on_mouse_down(MouseButton::Left, move |_e, _, _| {
            c.push(Cmd { id: "file.openRecent", params: Some(json!({ "path": p.clone() })) });
            *dismiss.borrow_mut() = true;
            c.wake();
        });
    if let Some(root) = root {
        let cmds = cmds.clone();
        let p = path.to_string();
        row = row.on_mouse_down(MouseButton::Right, move |e, _, cx: &mut App| {
            open_recent_menu(root.clone(), cmds.clone(), dismiss_for_menu.clone(), p.clone(), e.position, cx);
            cx.stop_propagation();
        });
    }
    row
}

/// Open a path in the OS file manager with the file selected (egui `services::reveal_path`).
#[cfg(windows)]
fn reveal_path(path: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", path)).spawn();
}
#[cfg(target_os = "macos")]
fn reveal_path(path: &str) {
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
}
#[cfg(all(unix, not(target_os = "macos")))]
fn reveal_path(path: &str) {
    let parent = std::path::Path::new(path).parent().unwrap_or_else(|| std::path::Path::new("."));
    let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
}

/// EGUI `home_recents::interact`: a recent card/row's context menu. Uses the same `MenuRow` +
/// root `open_menu` control as every other panel menu.
fn open_recent_menu(
    root: Entity<crate::CanvasView>,
    cmds: Cmds,
    dismiss: Rc<RefCell<bool>>,
    path: String,
    at: gpui::Point<Pixels>,
    cx: &mut App,
) {
    let open = tr("Open");
    let remove = tr("Remove from recents");
    let reveal = if cfg!(windows) {
        tr("Show in Explorer")
    } else if cfg!(target_os = "macos") {
        tr("Show in Finder")
    } else {
        tr("Show in folder")
    };
    let exists = std::path::Path::new(&path).exists();
    let mut rows = vec![MenuRow::new(open)];
    if exists {
        rows.push(MenuRow::new(reveal));
    }
    rows.push(MenuRow::new(remove).separator());

    let cmds_pick = cmds.clone();
    let on_pick = Rc::new(move |label: &str, _: &mut Window, _: &mut App| {
        if label == open {
            cmds_pick.push(Cmd {
                id: "file.openRecent",
                params: Some(json!({ "path": path.clone() })),
            });
            *dismiss.borrow_mut() = true;
            cmds_pick.wake();
        } else if label == reveal {
            reveal_path(&path);
        } else if label == remove {
            cmds_pick.push(Cmd {
                id: "file.openRecent",
                params: Some(json!({ "remove": path.clone() })),
            });
            cmds_pick.wake();
        }
    });
    let anchor = Bounds::new(at, gpui::size(px(0.0), px(0.0)));
    root.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), rows, on_pick, cx));
}

/// 最近文件的缩略图：文件自带的 `thumbnail.png`（EGUI 首页那条链路），读盘 + 解码一次就缓存。
fn recent_preview(state: &ExtraState, path: &str) -> (Option<std::sync::Arc<gpui::RenderImage>>, bool) {
    if let Some(hit) = state.recent_thumbs.borrow().get(path) {
        return hit.clone();
    }
    let p = figcraft_format::thumbnail::file_preview(std::path::Path::new(path), 640);
    let is_board = p.kind == figcraft_doc::FileKind::Board;
    let drawn = p.picture.map(|thumb| crate::canvas::render_worker::wrap_rgba(thumb.width, thumb.height, thumb.rgba));
    let hit = (drawn, is_board);
    state.recent_thumbs.borrow_mut().insert(path.to_string(), hit.clone());
    hit
}

/// 「24 minutes ago」式的相对时间（用词条表里的复数形式，和 参考编辑器 首页显示的一致）。
fn long_ago(path: &str) -> String {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else { return String::new() };
    let Ok(since) = modified.elapsed() else { return String::new() };
    let d = since.as_secs();
    if d < 60 {
        tr("Just now").to_string()
    } else if d < 3600 {
        figcraft_i18n::trn("{n} minutes ago", (d / 60) as usize, &[])
    } else if d < 86_400 {
        figcraft_i18n::trn("{n} hours ago", (d / 3600) as usize, &[])
    } else if d < 2_592_000 {
        figcraft_i18n::trn("{n} days ago", (d / 86_400) as usize, &[])
    } else if d < 31_536_000 {
        figcraft_i18n::trn("{n} months ago", (d / 2_592_000) as usize, &[])
    } else {
        figcraft_i18n::trn("{n} years ago", (d / 31_536_000) as usize, &[])
    }
}

fn cycle_bool(label: &'static str, slot: Rc<RefCell<bool>>, cmds: &Cmds) -> Div {
    let t = tokens();
    let wake = cmds.clone();
    div()
        .h(px(24.0))
        .px(pt(8.0))
        .rounded(pt(4.0))
        .flex()
        .items_center()
        .bg(rgba(t.panel))
        .hover(|s| s.bg(rgba(t.hover)))
        .cursor_pointer()
        .text_size(pt(11.0))
        .text_color(rgba(t.text))
        .child(SharedString::from(label))
        .on_mouse_down(MouseButton::Left, move |_e, _, _| {
            // ⛔ 先读再写：`*slot.borrow_mut() = !*slot.borrow()` 是一条表达式，左侧的 `RefMut`
            // 还活着时右侧又借了一次，直接 panic（RefCell already borrowed），整个 app 会崩。
            let next = !*slot.borrow();
            *slot.borrow_mut() = next;
            wake.wake();
        })
}

fn cycle_u8(label: &'static str, slot: Rc<RefCell<u8>>, n: u8, cmds: &Cmds) -> Div {
    let t = tokens();
    let wake = cmds.clone();
    div()
        .h(px(24.0))
        .px(pt(8.0))
        .rounded(pt(4.0))
        .flex()
        .items_center()
        .bg(rgba(t.panel))
        .hover(|s| s.bg(rgba(t.hover)))
        .cursor_pointer()
        .text_size(pt(11.0))
        .text_color(rgba(t.text))
        .child(SharedString::from(label))
        .on_mouse_down(MouseButton::Left, move |_e, _, _| {
            // 同上：先读再写。
            let next = (*slot.borrow() + 1) % n.max(1);
            *slot.borrow_mut() = next;
            wake.wake();
        })
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

fn edited_ago(path: &str) -> String {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else { return String::new() };
    let Ok(since) = modified.elapsed() else { return String::new() };
    let d = since.as_secs();
    if d < 60 {
        tr("Just now").to_string()
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86_400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86_400)
    }
}

fn recent_card(path: &str, cmds: &Cmds, dismiss: Rc<RefCell<bool>>, root: Option<Entity<crate::CanvasView>>) -> Div {
    let t = tokens();
    let name = file_name(path);
    let edited = edited_ago(path);
    let c = cmds.clone();
    let p = path.to_string();
    let mut card = div()
        .flex_none()
        .w(px(200.0))
        .h(px(154.0))
        .flex()
        .flex_col()
        .gap(pt(6.0))
        .p(pt(10.0))
        .rounded(pt(8.0))
        .bg(rgba(t.field))
        .border_1()
        .border_color(rgba(t.border))
        .hover(|s| s.bg(rgba(t.hover)))
        .cursor_pointer()
        .child(
            div().w_full().h(px(96.0)).rounded(pt(6.0)).bg(rgba(t.panel)).flex().items_center().justify_center().child(crate::widgets::icon::paint(
                Icon::Page,
                rgba(t.icon),
                28.0,
            )),
        )
        .child(div().text_size(pt(12.0)).text_color(rgba(t.text)).child(SharedString::from(name)))
        .child(div().text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(SharedString::from(edited)))
        .on_mouse_down(MouseButton::Left, move |_e, _, _| {
            c.push(Cmd { id: "file.openRecent", params: Some(json!({ "path": p.clone() })) });
        });
    if let Some(root) = root {
        let cmds = cmds.clone();
        let dismiss = dismiss.clone();
        let p = path.to_string();
        card = card.on_mouse_down(MouseButton::Right, move |e, _, cx: &mut App| {
            open_recent_menu(root.clone(), cmds.clone(), dismiss.clone(), p.clone(), e.position, cx);
            cx.stop_propagation();
        });
    }
    card
}

fn recent_row(path: &str, cmds: &Cmds, dismiss: Rc<RefCell<bool>>, root: Option<Entity<crate::CanvasView>>) -> Div {
    let t = tokens();
    let name = file_name(path);
    let c = cmds.clone();
    let p = path.to_string();
    let mut row = div()
        .w_full()
        .h(px(36.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(10.0))
        .px(pt(10.0))
        .rounded(pt(5.0))
        .hover(|s| s.bg(rgba(t.hover)))
        .cursor_pointer()
        .child(crate::widgets::icon::paint(Icon::Page, rgba(t.icon), 16.0))
        .child(div().text_size(pt(12.0)).text_color(rgba(t.text)).child(SharedString::from(name)))
        .on_mouse_down(MouseButton::Left, move |_e, _, _| {
            c.push(Cmd { id: "file.openRecent", params: Some(json!({ "path": p.clone() })) });
        });
    if let Some(root) = root {
        let cmds = cmds.clone();
        let dismiss = dismiss.clone();
        let p = path.to_string();
        row = row.on_mouse_down(MouseButton::Right, move |e, _, cx: &mut App| {
            open_recent_menu(root.clone(), cmds.clone(), dismiss.clone(), p.clone(), e.position, cx);
            cx.stop_propagation();
        });
    }
    row
}

/// A template card (egui `home.rs::template_card`): a 148 pt thumbnail over its name and one
/// line about it, 248 × 234. Clicking it opens a new document made from the template.
fn template_card(state: &ExtraState, tpl: &figcraft_engine::templates::Template, cmds: &Cmds) -> gpui::Stateful<Div> {
    let t = tokens();
    let thumb = template_thumb(state, tpl);
    let c = cmds.clone();
    let id = tpl.id;
    let name = figcraft_i18n::tr(tpl.name).to_string();
    let name_for_cmd = name.clone();
    let mut card = div()
        .id(gpui::ElementId::from(SharedString::from(format!("home-tpl-{}", tpl.id))))
        .flex_none()
        .w(px(248.0))
        .h(px(234.0))
        .flex()
        .flex_col()
        .rounded(pt(8.0))
        .bg(rgba(t.field))
        .border_1()
        .border_color(rgba(t.border))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, move |_e, _, _| {
            c.push(Cmd { id: "file.newFromTemplate", params: Some(json!({ "template": id, "name": name_for_cmd.clone() })) });
        });
    // The picture: the template drawn small, or its mark when it couldn't be drawn.
    let mut pic = div().w_full().h(px(148.0)).flex().items_center().justify_center().bg(rgba(t.panel)).border_b_1().border_color(rgba(t.border));
    pic = match thumb {
        Some(tex) => pic.child(gpui::img(gpui::ImageSource::Render(tex)).max_w(px(226.0)).max_h(px(148.0))),
        None => pic.child(crate::widgets::icon::paint(Icon::Frame, rgba(t.icon), 32.0)),
    };
    card = card.child(pic).child(
        div()
            .flex()
            .flex_col()
            .gap(pt(4.0))
            .p(pt(12.0))
            .child(div().text_size(pt(12.0)).font_weight(crate::panels::UI_FONT_REGULAR).text_color(rgba(t.text)).child(SharedString::from(name)))
            .child(
                div()
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text_secondary))
                    .child(SharedString::from(figcraft_i18n::tr(tpl.description).to_string())),
            ),
    );
    card
}

/// A template's thumbnail, drawn once and kept on the view (five documents' worth of rendering
/// is far too slow to do per frame).
fn template_thumb(state: &ExtraState, tpl: &figcraft_engine::templates::Template) -> Option<std::sync::Arc<gpui::RenderImage>> {
    if let Some(hit) = state.template_thumbs.borrow().get(tpl.id) {
        return hit.clone();
    }
    let drawn = draw_template_thumb(tpl);
    state.template_thumbs.borrow_mut().insert(tpl.id, drawn.clone());
    drawn
}

/// `ui-egui/src/home.rs::thumbnail`: the template built on a page of its own and drawn to fit
/// a 226 × 126 area, cropped to its top when it is much taller than it is wide.
fn draw_template_thumb(tpl: &figcraft_engine::templates::Template) -> Option<std::sync::Arc<gpui::RenderImage>> {
    let doc = tpl.build(figcraft_color::Color::WHITE).ok()?;
    let frames = doc.node(doc.first_page()?)?.children.clone();
    let bounds = frames.iter().filter_map(|id| figcraft_render::export_bounds(&doc, *id)).reduce(|a, b| a.union(b))?;
    if !(bounds.width() > 0.0 && bounds.height() > 0.0) {
        return None;
    }
    let (w, h) = (226.0f64, 126.0f64);
    let area = figcraft_geom::Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y1.min(bounds.y0 + bounds.width() * h / w * 1.5));
    let scale = (w / area.width()).min(h / area.height());
    let pix = crate::render_stack::on_render_stack(|| figcraft_render::render_area(&doc, &frames, area, scale.min(1.0)))??;
    Some(crate::canvas::render_worker::screen_image(&pix, doc.color_profile))
}

fn note(text: &'static str) -> Div {
    let t = tokens();
    div().w_full().p(pt(16.0)).text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(text))
}

fn heading(text: &'static str) -> Div {
    div().w_full().text_size(pt(20.0)).font_weight(crate::panels::UI_FONT_REGULAR).text_color(rgba(tokens().text)).child(SharedString::from(text))
}

fn subheading(text: &'static str) -> Div {
    div().w_full().text_size(pt(13.0)).font_weight(crate::panels::UI_FONT_REGULAR).text_color(rgba(tokens().text)).child(SharedString::from(text))
}

/// `ui-egui/src/home.rs::action_card`: 260x72, a 32 pt accent icon tile, title + sub line.
fn action_card(icon: Icon, title: &'static str, sub: &'static str, id: &'static str, cmds: &Cmds) -> Div {
    let t = tokens();
    let c = cmds.clone();
    div()
        .flex_none()
        .w(px(260.0))
        .h(px(72.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(12.0))
        .px(pt(16.0))
        .rounded(pt(8.0))
        .bg(rgba(t.field))
        .border_1()
        .border_color(rgba(t.border))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .child(
            div()
                .w(px(32.0))
                .h(px(32.0))
                .rounded(pt(6.0))
                .bg(rgba(t.accent))
                .flex()
                .items_center()
                .justify_center()
                .child(crate::widgets::icon::paint(icon, rgba(t.accent_on), 16.0)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(pt(4.0))
                .child(
                    div().text_size(pt(12.0)).font_weight(crate::panels::UI_FONT_REGULAR).text_color(rgba(t.text)).child(SharedString::from(title)),
                )
                .child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(sub))),
        )
        .on_mouse_down(MouseButton::Left, move |_e, _, _| c.push(Cmd { id, params: None }))
}
