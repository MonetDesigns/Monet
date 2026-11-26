//! 键盘框选 / 点选的回归测试（`select.area` → `selectArea` → `key_box` / `pick`）。
//!
//! 用户 2026-11-26：右栏画布项无法用快捷键多选 —— `select.area` 在 GPUI 端只发 UI 请求，
//! 而 `sync_ui_requests` 没有 `selectArea` 分支，请求被丢掉。这里锁住整条链路：
//! 无坐标的 `select.area` 一定产生请求；UI 建框围绕选中层；方向键 / Shift / Enter / Esc
//! 的状态机与 egui 一致。

use crate::app::{CanvasApp, sample_doc};
use crate::menu_card::Item;
use crate::{key_box_key, key_pick, menu_row_step};
use gpui::Modifiers;

fn plain() -> Modifiers {
    Modifiers::default()
}

fn shift() -> Modifiers {
    Modifiers { shift: true, ..Default::default() }
}

fn auto_layout_h(app: &CanvasApp) -> figcraft_doc::NodeId {
    let ds = app.session.ds().expect("document session");
    let mut ids = ds.doc.find_by_name("AutoLayout · H", Some(ds.page)).into_iter();
    ids.next().expect("AutoLayout · H in sample_doc")
}

fn zoom(app: &CanvasApp) -> f64 {
    app.session.ds().map(|d| d.view.zoom.max(1e-6)).unwrap_or(1.0)
}

#[test]
fn select_area_without_coordinates_asks_the_ui() {
    let mut app = CanvasApp::new(sample_doc());
    assert!(app.session.execute("select.area", &serde_json::json!({})).is_ok(), "select.area runs");
    assert!(
        app.session.ui_requests.iter().any(|r| r.kind.as_str() == "selectArea"),
        "`select.area` with no box must ask the UI for one; otherwise Alt+Space / F8 do nothing"
    );
}

#[test]
fn start_key_box_wraps_the_selection() {
    let mut app = CanvasApp::new(sample_doc());
    let id = auto_layout_h(&app);
    app.session.set_selection(vec![id]);
    app.start_key_box();
    let [x0, y0, x1, y1] = app.key_box.expect("box");
    let b = app.session.ds().expect("session").doc.world_bounds(id).expect("bounds");
    assert!(x0 <= b.x0 && y0 <= b.y0 && x1 >= b.x1 && y1 >= b.y1, "box {x0},{y0},{x1},{y1} must contain {b:?}");
    assert!((b.x0 - x0 - 8.0 / zoom(&app)).abs() < 1e-6, "8px padding left");
}

#[test]
fn arrows_move_the_box_and_shift_sizes_it() {
    let mut app = CanvasApp::new(sample_doc());
    app.start_key_box();
    let before = app.key_box.expect("box");
    let step = 10.0 / zoom(&app);

    assert!(key_box_key(&mut app, "ArrowRight", &plain()));
    let moved = app.key_box.expect("box");
    assert!((moved[0] - before[0] - step).abs() < 1e-6 && (moved[2] - before[2] - step).abs() < 1e-6, "plain arrow moves both edges");

    assert!(key_box_key(&mut app, "ArrowRight", &shift()));
    let sized = app.key_box.expect("box");
    assert!((sized[0] - moved[0]).abs() < 1e-6, "Shift keeps the near corner");
    assert!((sized[2] - moved[2] - step).abs() < 1e-6, "Shift moves the far corner");
}

#[test]
fn enter_selects_what_the_box_covers_and_escape_drops_it() {
    let mut app = CanvasApp::new(sample_doc());
    let id = auto_layout_h(&app);
    app.session.set_selection(vec![id]);
    app.start_key_box();
    assert!(key_box_key(&mut app, "Enter", &plain()), "Enter is consumed by the box");
    assert!(app.key_box.is_none(), "box closes after selecting");
    assert_eq!(app.selection(), vec![id], "the box selects the layer inside it");

    app.start_key_box();
    assert!(key_box_key(&mut app, "Escape", &plain()));
    assert!(app.key_box.is_none(), "Escape drops the box");
    assert_eq!(app.selection(), vec![id], "Escape leaves the selection alone");
}

#[test]
fn the_chord_while_the_box_shows_switches_to_pick_select() {
    let mut app = CanvasApp::new(sample_doc());
    let id = auto_layout_h(&app);
    app.session.set_selection(vec![id]);
    app.start_key_box();
    app.start_key_box();
    assert!(app.key_box.is_none(), "second chord drops the box");
    assert_eq!(app.picked(), Some(id), "pick cursor starts on the selected layer");

    assert!(key_pick(&mut app, "Tab", &plain()), "Tab moves the pick cursor");
    let walked = app.picked().expect("picked sibling");
    assert_ne!(walked, id, "Tab walks to a sibling layer");

    assert!(key_pick(&mut app, "Space", &plain()), "Space selects the picked layer");
    assert!(app.pick.is_none(), "pick cursor goes away after selecting");
    assert_eq!(app.selection(), vec![walked], "selection follows the pick cursor");
}

#[test]
fn a_box_belongs_to_the_document_and_page_it_was_opened_in() {
    let mut app = CanvasApp::new(sample_doc());
    let ds = app.session.ds().expect("document session");
    let (uid, page) = (ds.uid, ds.page);
    app.start_key_box();
    let before = app.live_key_box().expect("box");
    // Same document, different page: the page-space rectangle must not be reused there.
    app.key_box_doc = Some((uid, figcraft_doc::NodeId(page.0.wrapping_add(1))));
    assert!(app.live_key_box().is_none(), "a box from another page is stale");
    assert!(!key_box_key(&mut app, "ArrowRight", &plain()), "a stale box must not move or swallow arrows");
    assert_eq!(app.key_box, Some(before), "the stale value stays inert until replaced");
    app.start_key_box();
    assert!(app.live_key_box().is_some(), "a new box is owned by the current page");
}

#[test]
fn menu_keyboard_highlight_skips_disabled_rows_and_wraps() {
    let items = vec![
        Item::new("a", "A"),
        Item::new("b", "B").disabled(true),
        Item::new("c", "C"),
    ];
    assert_eq!(menu_row_step(&items, None, true), Some(0), "Down from nothing goes to the first row");
    assert_eq!(menu_row_step(&items, Some(0), true), Some(2), "skip the disabled middle row");
    assert_eq!(menu_row_step(&items, Some(2), true), Some(0), "wrap at the bottom");
    assert_eq!(menu_row_step(&items, Some(0), false), Some(2), "Up wraps over the disabled row");
    assert_eq!(menu_row_step(&items, None, false), Some(2), "Up from nothing goes to the last row");

    let all_disabled = vec![Item::new("a", "A").disabled(true)];
    assert_eq!(menu_row_step(&all_disabled, None, true), None, "a menu with no runnable row has no highlight");
}

#[test]
fn pick_escape_cancels_without_changing_the_selection() {
    let mut app = CanvasApp::new(sample_doc());
    let id = auto_layout_h(&app);
    app.session.set_selection(vec![id]);
    app.start_key_box();
    app.start_key_box();
    assert!(key_pick(&mut app, "Escape", &plain()));
    assert!(app.pick.is_none());
    assert_eq!(app.selection(), vec![id]);
}
