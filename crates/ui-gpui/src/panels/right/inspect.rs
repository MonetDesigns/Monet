//! Inspect 标签（Dev Mode 手递手），照搬 egui `ui-egui/src/panels/right/inspect.rs`：
//! 图层名 + 属性分组（Layout / Auto layout / Typography / Fills / Strokes / Effects…）+ 资产 +
//! 生成的代码（可复制，可切语言 / 颜色格式 / px-rem）。
//!
//! 全部用冻结通用控件（`PanelSection` / `SectionHeader` / `field_row*` / `Dropdown` /
//! `IconButton` / `swatch`）。语言 / 颜色格式 / rem 是**纯 UI 状态**（egui 存在 `ui.data`），
//! 走 `RightFields` 的 `RefCell` 并唤醒一帧，不推引擎命令。

use super::data::{AssetRow, InspectData, InspectSection, PanelData, PlaygroundData, PluginRunRow};
use super::sections::{icon_row, slot};
use crate::panels::RightFields;
use crate::widgets::{
    Checkbox, Dropdown, Icon, IconButton, MenuRow, PanelSection, SectionHeader, SectionHeaderVariant, field_row, field_row_full, pt, rgba,
    sublabel_row, swatch, tokens,
};
use figcraft_doc::{ExportFormat, NodeId};
use figcraft_i18n::tr;
use gpui::{AnyElement, App, Div, ImageSource, IntoElement, MouseButton, SharedString, Window, div, img, prelude::*, px};
use serde_json::{Value, json};
use std::cell::{Ref, RefCell};
use std::rc::Rc;

/// What the Status section offers next to the status dropdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompareWith<'a> {
    /// "Changed since marked": `dev.compare` finds the version itself.
    Changed,
    /// An unchanged frame / section: compare against the newest saved version.
    Version(&'a str),
}

/// Status actions: Focus only when the target is not already focused, Compare only when there is
/// something to compare with (EGUI `status_dropdown` + `Focus view` + `Compare changes`).
pub(crate) fn status_actions(focused: bool, changed: bool, newest: Option<&str>) -> (bool, Option<CompareWith<'_>>) {
    let focus = !focused;
    let compare = if changed {
        Some(CompareWith::Changed)
    } else {
        newest.map(CompareWith::Version)
    };
    (focus, compare)
}

/// Inspect 标签整页。节序照抄 egui `right/inspect.rs::inspect`：
/// 名字 → Status → Annotations → Dev resources → Measurements → Color format → 属性分组 →
/// Assets → Code（无选中时：Ready for dev → Annotations 分类 → Measurements）。
pub(crate) fn panel(f: &RightFields, d: &PanelData, cx: &mut App, n: &mut usize) -> Div {
    let t = tokens();
    let mut body = div().flex().flex_col().w_full();
    let Some(data) = d.inspect.as_ref() else {
        // 无选中：两条提示 + 就绪列表 + 分类 + 测量（egui 同款顺序）。
        let mut sec = PanelSection::new(tr("Inspect")).variant(SectionHeaderVariant::Dim);
        sec = sec.child(field_row_full(note(tr("Select a layer to see its properties and code"))));
        sec = sec.child(field_row_full(note(tr("Hover other layers to measure the distance"))));
        body = body.child(sec);
        body = body.child(crate::panels::sep()).child(super::devmode::ready_list(f, d));
        body = body.child(crate::panels::sep()).child(super::devmode::categories(f, d, cx, n));
        if let Some(m) = super::devmode::measurements(f, d) {
            body = body.child(crate::panels::sep()).child(m);
        }
        if !d.plugins.is_empty() {
            body = body.child(crate::panels::sep()).child(plugin_section(f, &d.plugins));
        }
        return body;
    };

    // ── 图层名（egui `section_header(&name, &[], false)`，只有一行标题）──
    let name = d.header.as_ref().map(|h| h.label.clone()).unwrap_or_default();
    body = body.child(SectionHeader::new(name));

    // ── Dev Status（egui `dev_mode.rs::status`）：手递手状态 + Focus view / Compare changes ──
    if let Some(s) = d.section.as_ref() {
        let mut sec = PanelSection::new(tr("Status"));
        // 状态走 `node.set {"props": {"devStatus": …}}`：用一个本地下拉选中后推命令。
        let statuses: Vec<(String, &'static str, serde_json::Value)> = [
            (tr("No status"), "node.set", serde_json::json!({"props": {"devStatus": "none"}})),
            (tr("Ready for dev"), "node.set", serde_json::json!({"props": {"devStatus": "readyForDev"}})),
            (tr("Completed"), "node.set", serde_json::json!({"props": {"devStatus": "completed"}})),
        ]
        .into_iter()
        .map(|(l, c, p)| (l.to_string(), c, p))
        .collect();
        let current = if s.changed && s.status == tr("Ready for dev") { tr("Changed").to_string() } else { s.status.clone() };
        sec = sec.child(field_row_full(super::sections::pick_dropdown(f, current, 0.0, statuses)));
        // Focus view / Compare changes. EGUI shows Focus only when not focused, and Compare only
        // when there is something to compare with: the newest saved version for an unchanged
        // frame, "when marked ready" for a changed one. No version, no button — `dev.compare`
        // refuses a frame that is neither ready nor given a version.
        let id = d.ids.first().copied();
        let (show_focus, compare) = status_actions(s.focused, s.changed, s.newest_version.as_deref());
        let mut actions: Vec<AnyElement> = Vec::new();
        if show_focus {
            actions.push(action_button(f, tr("Focus view"), "dev.focus", json!({ "id": id })).into_any_element());
        }
        match compare {
            Some(CompareWith::Changed) => actions.push(
                action_button(f, tr("Compare changes"), "dev.compare", json!({ "id": id, "show": true })).into_any_element(),
            ),
            Some(CompareWith::Version(version)) => actions.push(
                action_button(f, tr("Compare changes"), "dev.compare", json!({ "id": id, "version": version, "show": true }))
                    .into_any_element(),
            ),
            None => {}
        }
        if !actions.is_empty() {
            sec = sec.child(field_row(actions, Vec::<IconButton>::new()));
        }
        body = body.child(sec);
    }

    // ── Annotations / Dev resources / Measurements（egui 顺序在 Color format 之前）──
    if let Some(sec) = super::devmode::annotations(f, d, cx, n) {
        body = body.child(crate::panels::sep()).child(sec);
    }
    if let Some(sec) = super::devmode::dev_resources(f, d, cx, n) {
        body = body.child(crate::panels::sep()).child(sec);
    }
    if let Some(sec) = super::devmode::measurements(f, d) {
        body = body.child(crate::panels::sep()).child(sec);
    }

    // ── Color format：属性值里颜色的写法（Hex / RGB / CSS / HSL / HSB）──
    let fmt = f.inspect_fmt.clone();
    let fmt_rows: Vec<(String, usize)> = data.formats.iter().enumerate().map(|(i, (label, _))| (label.to_string(), i)).collect();
    body = body.child(PanelSection::new(tr("Color format")).variant(SectionHeaderVariant::Dim).child(field_row_full(local_dropdown(
        f,
        data.fmt_label.into(),
        0.0,
        fmt_rows,
        fmt,
    ))));

    // ── 属性分组 ──
    for section in &data.sections {
        body = body.child(inspect_section_with(Some(f), section));
    }

    // ── 资产（egui `inspect_assets`）：缩略图 + 名字 + 格式下拉 + 逐项导出 / Export all ──
    // 导出本身是引擎命令 `file.export` / `file.exportLayers`（UI 只负责选路径和调命令），
    // 缩略图由 `figcraft_render::render_node` 光栅化后在 data.rs 存成 BGRA 字节。
    let all_ids: Vec<NodeId> = data.assets.iter().map(|a| a.id).collect();
    let export_all = {
        let cmds = f.queue();
        let ids = all_ids.clone();
        IconButton::new(Icon::Image).size(24.0).on_click(move |_e, _w, _cx| {
            let Some(dir) = rfd::FileDialog::new().pick_folder() else { return };
            cmds.push(crate::panels::Cmd {
                id: "file.exportLayers",
                params: Some(json!({ "ids": ids, "assets": true, "dir": dir.to_string_lossy() })),
            });
        })
    };
    let mut assets =
        PanelSection::new(tr("Assets")).variant(if data.assets.is_empty() { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal });
    if !data.assets.is_empty() {
        assets = assets.header_trail(icon_row(vec![export_all]));
    }
    if data.assets.is_empty() {
        assets = assets.child(field_row_full(note(tr("No assets: no export settings, icons or images."))));
    } else {
        for a in &data.assets {
            assets = assets.child(field_row_full(asset_row(f, a)));
        }
    }
    body = body.child(assets);

    // ── Playground：组件试值（egui `dev_mode.rs::playground_section`），只改副本，不改文档 ──
    if let Some(pg) = &d.playground {
        body = body.child(playground_section(f, pg, cx, n));
    }

    // ── Code：语言 +（Web 语言）px/rem + 复制 ──
    // Playground tried values: the code below is the copy's, exactly like EGUI's returned snippet.
    let code_text = d.playground.as_ref().filter(|p| p.tried).map_or_else(|| data.code.clone(), |p| p.code.clone());
    let copy = {
        let code = code_text.clone();
        IconButton::new(Icon::Code).size(24.0).on_click(move |_e, _w, cx: &mut App| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(code.clone()));
        })
    };
    let mut code_sec = PanelSection::new(tr("Code")).header_trail(icon_row(vec![copy]));
    let lang = f.inspect_lang.clone();
    let lang_rows: Vec<(String, usize)> = data.langs.iter().enumerate().map(|(i, (label, _))| (label.to_string(), i)).collect();
    let lang_dd = local_dropdown(f, data.lang_label.into(), 0.0, lang_rows, lang);
    let mut trail: Vec<IconButton> = Vec::new();
    if data.web {
        // egui：一个 56 pt 的 px / rem 切换下拉。
        let rem = f.inspect_rem.clone();
        let current = if *rem.borrow() { "rem" } else { "px" };
        let rows: Vec<(String, usize)> = vec![("px".into(), 0), ("rem".into(), 1)];
        let dd = local_dropdown_flag(f, current.into(), 56.0, rows, rem);
        code_sec = code_sec.child(field_row(vec![lang_dd.into_any_element()], Vec::<IconButton>::new()));
        code_sec = code_sec.child(field_row_full(dd));
    } else {
        code_sec = code_sec.child(field_row(vec![lang_dd.into_any_element()], trail.drain(..).collect()));
    }
    // 代码块：等宽字体 + 字段底色（egui `Frame::fill(t.field).corner_radius(6)`）。
    let block = div()
        .w_full()
        .p(pt(10.0))
        .rounded(pt(6.0))
        .bg(rgba(t.field))
        .font_family("JetBrains Mono")
        .text_size(pt(10.5))
        .text_color(rgba(t.text))
        .child(SharedString::from(code_text.clone()));
    code_sec = code_sec.child(field_row_full(block));
    body = body.child(code_sec);

    // ── Plugins：Developer Mode 只列只读插件命令（egui 只读插件区）──
    if !d.plugins.is_empty() {
        body = body.child(plugin_section(f, &d.plugins));
    }

    // ── Connect to code（egui `dev_mode::connect_section`，排在 Code 之后）──
    if let Some(sec) = connect_section(f, data, cx, n) {
        body = body.child(crate::panels::sep()).child(sec);
    }
    body
}

/// 撞进 Inspect 标签的组件试验台（egui `dev_mode.rs::playground_section`）。
fn playground_section(f: &RightFields, p: &PlaygroundData, cx: &mut App, n: &mut usize) -> PanelSection {
    let t = tokens();
    let mut sec = PanelSection::new(tr("Playground"));
    if p.tried {
        let values = f.playground_values.clone();
        let cache = f.playground_cache.clone();
        let wake = f.cmds.clone();
        let reset = IconButton::new(Icon::Rotate).size(24.0).on_click(move |_e, _w, _cx| {
            values.borrow_mut().clear();
            *cache.borrow_mut() = None;
            wake.wake();
        });
        sec = sec.header_trail(icon_row(vec![reset]));
    }

    // 预览：与 Export Preview 共用 BGRA → ImageSource 管线，四周留 8pt。
    let mut frame =
        div().w_full().h(px(136.0)).rounded(px(6.0)).border_1().border_color(rgba(t.border)).overflow_hidden().relative().my(pt(6.0));
    frame = frame.child(gpui::canvas(|b, _w, _cx| b.size, |b, _, window, _cx| crate::widgets::checker(window, b, 8.0)).absolute().inset_0());
    if let Some((w, h, buf)) = &p.preview {
        let image = img(ImageSource::Render(crate::canvas::render_worker::wrap_bgra(*w, *h, buf.clone())))
            .max_w(px(320.0))
            .max_h(px(120.0));
        frame = frame.child(div().absolute().inset_0().flex().items_center().justify_center().child(image));
    }
    sec = sec.child(field_row_full(frame));

    for prop in &p.properties {
        let (Some(name), Some(kind)) = (prop["name"].as_str(), prop["type"].as_str()) else { continue };
        let value = &prop["value"];
        match kind {
            "variant" => {
                let options: Vec<Value> = prop["options"].as_array().cloned().unwrap_or_default();
                let current = value.as_str().unwrap_or("").to_string();
                let rows: Vec<(String, Value)> =
                    options.into_iter().filter_map(|v| v.as_str().map(|s| (s.to_string(), json!(s)))).collect();
                let dd = playground_dropdown(f, name, current, rows, p.target, 0.0);
                sec = sec.child(field_row(vec![label_element(name), dd.into_any_element()], Vec::<IconButton>::new()));
            }

            "boolean" => {
                let on = value.as_bool().unwrap_or(false);
                let values = f.playground_values.clone();
                let cache = f.playground_cache.clone();
                let wake = f.cmds.clone();
                let key = name.to_string();
                let cb = Checkbox::new(("playground-bool", *n))
                    .label(name.to_string())
                    .checked(on)
                    .on_change(move |next: &bool, _w, _cx| {
                        values.borrow_mut().insert(key.clone(), json!(*next));
                        *cache.borrow_mut() = None;
                        wake.wake();
                    });
                *n += 1;
                sec = sec.child(field_row_full(cb));
            }

            "text" => {
                let edit = f.playground_edit(cx, name, value.as_str().unwrap_or(""));
                sec = sec.child(field_row_full(div().w_full().child(edit)));
            }

            "instanceSwap" => {
                let current = figcraft_engine::params::id_of(value);
                let current_label =
                    current.and_then(|id| p.swapped.iter().find(|(_, c)| *c == id).map(|(n, _)| n.clone())).unwrap_or_else(|| tr("None").into());
                let rows: Vec<(String, Value)> = p.swapped.iter().map(|(n, id)| (n.clone(), json!(id.0))).collect();
                let dd = playground_dropdown(f, name, current_label, rows, p.target, 0.0);
                sec = sec.child(field_row(vec![label_element(name), dd.into_any_element()], Vec::<IconButton>::new()));
            }

            // Slots show as the component has them.
            _ => {}
        }
    }

    sec.child(field_row_full(note(tr("What you try here changes this preview and the code below, not the file"))))
}

fn label_element(label: &str) -> AnyElement {
    div().min_w(px(72.0)).text_size(pt(11.0)).text_color(rgba(tokens().text_secondary)).child(SharedString::from(label.to_string())).into_any_element()
}

/// Playground 里的一个本地下拉：选中的值只写进 `RightFields.playground_values`，不推命令。
fn playground_dropdown(
    f: &RightFields,
    name: &str,
    current: String,
    rows: Vec<(String, Value)>,
    target: NodeId,
    width: f32,
) -> Dropdown {
    let menu: Vec<MenuRow> = rows.iter().enumerate().map(|(i, (l, _))| MenuRow::row(l.clone()).key(i.to_string())).collect();
    let anchor_key = format!("playground:{}:{}", target.0, name);
    let anchor_handle = crate::widgets::dropdown::menu_anchor(&anchor_key);
    let mut d = Dropdown::new(SharedString::from(current), width, None).fill().menu(anchor_key);
    if let Some(root) = f.root.clone() {
        let values = f.playground_values.clone();
        let cache = f.playground_cache.clone();
        let wake = f.cmds.clone();
        let prop = name.to_string();
        d = d.on_click(move |anchor, _window, cx: &mut App| {
            let rows = rows.clone();
            let values = values.clone();
            let cache = cache.clone();
            let wake = wake.clone();
            let prop = prop.clone();
            let on_pick = Rc::new(move |key: &str, _w: &mut Window, _cx: &mut App| {
                if let Some((_, value)) = key.parse::<usize>().ok().and_then(|i| rows.get(i)) {
                    values.borrow_mut().insert(prop.clone(), value.clone());
                    *cache.borrow_mut() = None;
                    wake.wake();
                }
            });
            let anchor_handle = anchor_handle.clone();
            let menu = menu.clone();
            root.update(cx, move |cv, cx| {
                cv.open_dropdown_menu(Some(anchor_handle), anchor, Some(width.max(160.0)), menu, on_pick, cx);
            });
        });
    }
    d
}

/// Inspect 的只读插件命令：每行一个按钮，`plugin.run` 只带 id/command，参数表单暂不支持。
fn plugin_section(f: &RightFields, rows: &[PluginRunRow]) -> PanelSection {
    let mut sec = PanelSection::new(tr("Plugins")).variant(SectionHeaderVariant::Normal);
    for row in rows {
        let label = if row.plugin_name.is_empty() { row.label.clone() } else { format!("{} · {}", row.plugin_name, row.label) };
        let id = row.plugin.clone();
        let command = row.command.clone();
        let cmds = f.cmds.clone();
        let button = Dropdown::new(SharedString::from(label), 0.0, None).fill().on_click(move |_anchor, _w, _cx| {
            cmds.push(crate::panels::Cmd { id: "plugin.run", params: Some(json!({ "id": id, "command": command })) });
        });
        sec = sec.child(field_row_full(button));
    }
    sec.child(field_row_full(note(tr("Developer Mode lists read-only plugin commands; plugin panels are not supported yet."))))
}

/// 「Connect to code」：把组件（组件集 / 实例落到主组件）连到它的代码组件。
/// 连上后给 Connected 徽标 + import 路径 / 名字 / 模板（模板就是 Code 里看到的片段）。
fn connect_section(f: &RightFields, data: &InspectData, cx: &mut App, n: &mut usize) -> Option<PanelSection> {
    let t = tokens();
    let owner = data.connect_owner?;
    let base = serde_json::json!({ "id": owner, "language": data.lang });
    let mut sec =
        PanelSection::new(tr("Connect to code")).variant(if data.link.is_none() { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal });
    let Some(link) = &data.link else {
        let btn = if data.connectable {
            super::sections::icon_cmd(Icon::Plus, "codeConnect.set", base.clone(), f, false)
        } else {
            // 不可连：不给按钮（egui 同样只在可连语言下给 +）。
            IconButton::new(Icon::Plus).size(24.0)
        };
        sec = sec.header_trail(icon_row(vec![btn]));
        let text = if data.connectable {
            tr("Link this component to its code component: instances then show your real snippet with their values.")
        } else {
            tr("Choose React, Leptos, Vue, SwiftUI, Compose, ArkUI or Flutter above to connect this component to its code.")
        };
        return Some(sec.child(field_row_full(note(text))));
    };
    let btn = super::sections::icon_cmd(Icon::Minus, "codeConnect.remove", base.clone(), f, false);
    sec = sec.header_trail(icon_row(vec![btn]));

    // ── Connected 徽标 + 片段来源 ──
    let from = if link.import_path.is_empty() { link.name.clone() } else { format!("{} from {}", link.name, link.import_path) };
    let badge = div()
        .flex_none()
        .px(pt(5.0))
        .py(pt(2.0))
        .rounded(pt(4.0))
        .bg(gpui::rgb(0x087D4F))
        .text_size(pt(10.0))
        .font_weight(crate::panels::UI_FONT_REGULAR)
        .text_color(gpui::white())
        .child(SharedString::from(tr("Connected")));
    sec = sec.child(field_row_full(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .w_full()
            .child(badge)
            .child(div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(from))),
    ));

    // ── Import path / 组件名 ──
    let import = link_field(f, cx, n, &base, "importPath", &link.import_path, tr("Import path"));
    let name = link_field(f, cx, n, &base, "name", &link.name, tr("Component name"));
    sec = sec.child(field_row_full(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .w_full()
            .child(div().flex_1().min_w(px(0.0)).child(import))
            .child(div().flex_1().min_w(px(0.0)).child(name)),
    ));

    // ── 模板（egui 是 3 行 multiline；GPUI 只有单行框）──
    let template = link_field(f, cx, n, &base, "template", &link.template, "<Button size={size}>{label}</Button>");
    sec = sec.child(field_row_full(div().w_full().child(template)));
    Some(sec.child(field_row_full(note(tr("{property} is its value; {property ? \"a\" : \"b\"} picks a string. Other {…} stays as written.")))))
}

/// 「Connect to code」里的一行文本编辑（import 路径 / 组件名 / 模板），提交走 `codeConnect.set`。
fn link_field(
    f: &RightFields,
    cx: &mut App,
    n: &mut usize,
    base: &serde_json::Value,
    key: &'static str,
    value: &str,
    placeholder: &'static str,
) -> gpui::Entity<crate::widgets::TextEdit> {
    let field = f.row_field(
        cx,
        slot(n),
        value,
        crate::panels::RowEditCtx {
            cmd: "codeConnect.set",
            params: base.clone(),
            key,
            scale: 1.0,
            string_value: true,
            paint: None,
            map: None,
            paint_color: None,
            nest: None,
            nest_key: None,
        },
    );
    field.update(cx, |te, _| te.set_placeholder(placeholder));
    field
}

/// 一项资产：40pt 缩略图 + 名字 / 导出摘要 + 格式下拉 + 导出按钮。
///
/// 导出按钮走 `file.export {ids, format, path}`（引擎命令）；路径由 `rfd` 的「另存为」给，
/// 与 egui `save_one` 的 `app.pick_save → app.run("file.export", …)` 一一对应。
fn asset_row(f: &RightFields, a: &AssetRow) -> Div {
    let t = tokens();
    let thumb = match &a.thumb {
        Some((w, h, buf)) => img(ImageSource::Render(crate::canvas::render_worker::wrap_bgra(*w, *h, buf.clone())))
            .w(px(40.0))
            .h(px(40.0))
            .rounded(pt(4.0))
            .into_any_element(),
        None => div().flex_none().w(px(40.0)).h(px(40.0)).rounded(pt(4.0)).bg(rgba(t.field)).into_any_element(),
    };
    let cmds = f.queue();
    let store = f.asset_formats();
    let id = a.id;
    let default_idx = a.format_index;
    let name = a.name.clone();
    let export = IconButton::new(Icon::Image).size(24.0).on_click(move |_e, _w, _cx| {
        let idx = store.borrow().iter().find(|(k, _)| *k == id).map(|(_, i)| *i).unwrap_or(default_idx);
        let fmt = ExportFormat::ALL[idx.min(ExportFormat::ALL.len() - 1)];
        let file = format!("{}.{}", name.rsplit('/').next().unwrap_or(&name), fmt.ext());
        let Some(path) = rfd::FileDialog::new().set_file_name(&file).add_filter(fmt.label(), &[fmt.ext()]).save_file() else {
            return;
        };
        cmds.push(crate::panels::Cmd {
            id: "file.export",
            params: Some(json!({ "ids": [id], "format": fmt.ext(), "path": path.to_string_lossy() })),
        });
    });
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(8.0))
        .w_full()
        .child(thumb)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(a.name.clone())))
                .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(a.exports.join(", ")))),
        )
        .child(asset_format_dropdown(f, a))
        .child(export)
}

/// 格式下拉只改本地选择（egui `AssetUi::formats`），导出按钮按下时才带上它。
fn asset_format_dropdown(f: &RightFields, a: &AssetRow) -> Dropdown {
    let store = f.asset_formats();
    let idx = store.borrow().iter().find(|(k, _)| *k == a.id).map(|(_, i)| *i).unwrap_or(a.format_index);
    let cur = ExportFormat::ALL[idx.min(ExportFormat::ALL.len() - 1)];
    let rows: Vec<(String, usize)> = ExportFormat::ALL.iter().enumerate().map(|(i, f)| (f.label().to_string(), i)).collect();
    let id = a.id;
    local_dropdown_impl(f, cur.label().into(), 74.0, rows, move |i| {
        let mut m = store.borrow_mut();
        match m.iter_mut().find(|(k, _)| *k == id) {
            Some(e) => e.1 = i,
            None => m.push((id, i)),
        }
    })
}

/// 一个「点了就推命令」的文字按钮（`Dropdown` 的填充态，没有菜单）。
fn action_button(f: &RightFields, label: &'static str, cmd: &'static str, params: serde_json::Value) -> Dropdown {
    let cmds = f.cmds.clone();
    Dropdown::new(SharedString::from(label), 0.0, None)
        .fill()
        .on_click(move |_anchor, _w, _cx| cmds.push(crate::panels::Cmd { id: cmd, params: Some(params.clone()) }))
}

/// 只改本地 `usize` 状态的下拉（语言 / 颜色格式）：选中后写回并唤醒一帧。
pub(super) fn local_dropdown(f: &RightFields, label: SharedString, width: f32, rows: Vec<(String, usize)>, slot: Rc<RefCell<usize>>) -> Dropdown {
    local_dropdown_impl(f, label, width, rows, move |i| *slot.borrow_mut() = i)
}

/// 只改本地 `bool` 状态的下拉（px / rem）。
fn local_dropdown_flag(f: &RightFields, label: SharedString, width: f32, rows: Vec<(String, usize)>, slot: Rc<RefCell<bool>>) -> Dropdown {
    local_dropdown_impl(f, label, width, rows, move |i| *slot.borrow_mut() = i == 1)
}

fn local_dropdown_impl(f: &RightFields, label: SharedString, width: f32, rows: Vec<(String, usize)>, set: impl Fn(usize) + 'static) -> Dropdown {
    let menu: Vec<MenuRow> = rows.iter().map(|(l, _)| MenuRow::row(l.clone())).collect();
    let mut d = Dropdown::new(label, width, None).fill();
    if let Some(root) = f.root.clone() {
        let cmds = f.cmds.clone();
        let set = Rc::new(set);
        d = d.on_click(move |anchor, _window, cx: &mut App| {
            let rows = rows.clone();
            let cmds = cmds.clone();
            let set = set.clone();
            let on_pick = Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some((_, i)) = rows.iter().find(|(l, _)| l == label) {
                    set(*i);
                    cmds.wake();
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(width.max(120.0)), menu.clone(), on_pick, cx));
        });
    }
    d
}

/// A property group in the presentation's read-only Inspect panel (no copy target).
pub(crate) fn inspect_section(s: &InspectSection) -> PanelSection {
    inspect_section_with(None, s)
}

/// 一组属性（标题 + 若干行：标签 | 色块? 值 | 来源 chip）。点一行复制它的值（egui `prop_row`）。
pub(crate) fn inspect_section_with(f: Option<&RightFields>, s: &InspectSection) -> PanelSection {
    let t = tokens();
    let mut sec = PanelSection::new(s.title);
    let root = f.and_then(RightFields::root);
    for row in &s.rows {
        let label = div().w(px(88.0)).flex_none().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(row.label.clone()));
        let mut value = div().flex().flex_row().items_center().gap(pt(6.0)).flex_1().min_w(px(0.0));
        if let Some(hex) = &row.color
            && let Some(c) = figcraft_color::Color::from_hex(hex)
        {
            value = value.child(swatch(figcraft_color::Paint::solid(c), 12.0));
        }
        value = value.child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(row.value.clone())));
        let mut row_el = div().flex().flex_row().items_center().gap(pt(6.0)).w_full().child(label).child(value);
        if let Some(token) = &row.token {
            row_el = row_el.child(
                div()
                    .px(pt(6.0))
                    .rounded(pt(4.0))
                    .bg(rgba(t.field))
                    .text_size(pt(10.0))
                    .text_color(rgba(t.text_secondary))
                    .child(SharedString::from(token.clone())),
            );
        }
        // Click copies the value, like EGUI's `prop_row` (the pin rows call the same helper).
        let value = row.value.clone();
        let copy_value = row.value.clone();
        let row_root = root.clone();
        row_el = row_el.cursor_pointer().on_mouse_down(MouseButton::Left, move |_e, _w, cx: &mut App| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(value.clone()));
            let text = figcraft_i18n::trf("Copied {value}", &[("value", &copy_value)]);
            if let Some(root) = row_root.clone() {
                root.update(cx, |cv, cx| cv.toast(text.clone(), cx));
            }
        });
        sec = sec.child(field_row_full(row_el));
    }
    sec
}

/// 一行次要提示文字。
fn note(text: &'static str) -> Div {
    let t = tokens();
    div().w_full().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(text))
}

#[allow(dead_code)]
fn _ref_check(_: Ref<'_, usize>) {}
