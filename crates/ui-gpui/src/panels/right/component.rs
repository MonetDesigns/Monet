//! Component / Instance 面板（lizard `component_panel.rs::instance_section`）。
//!
//! Design 标签里，header 之后、Position 之前：
//! - **Instance**：标题（Instance / Nested instance）+ Reset / Detach 按钮、主组件 swap 下拉、
//!   主组件描述、Expose 开关、组件属性（变体值 / 布尔 / 文本 / 实例互换）。
//! - **Component / Component set**：属性列表（每行带设置按钮 → 通用弹窗编辑名称/默认值），
//!   属性下附「当前变体」（组件的变体属性值 / 组件集的变体属性概览）。
//!
//! 属性设置走 `WindowState::component_property`（通用弹窗卡 + Setting 行），不再用行下内联框。
//! 全部走冻结通用控件与列表行编辑器池，没有新建控件。

use super::data::{ComponentData, ComponentDef, ComponentProp, PanelData};
use super::sections::{icon_cmd, icon_row, open_detached_with, pick_dropdown, slot, window_open_button};
use crate::panels::{RightFields, RowEditCtx};
use crate::widgets::{Icon, IconButton, PanelSection, field_row, field_row_full, field_row_full_trailing, pt, rgba, sublabel_row, tokens};
use figcraft_doc::NodeId;
use figcraft_i18n::tr;
use gpui::{App, Div, IntoElement, SharedString, div, prelude::*, px};
use serde_json::{Value, json};

/// Component / Instance 节（`None` = 选中的不是组件相关节点）。
pub(crate) fn section(f: &RightFields, d: &PanelData, cx: &mut App, n: &mut usize) -> Option<PanelSection> {
    let c = d.component.as_ref()?;
    if c.is_instance {
        return Some(instance_section(f, c, cx, n));
    }
    if !c.editable_description {
        return None;
    }
    // 组件 / 组件集：描述 + 属性列表 + 当前变体。
    let mut sec = PanelSection::new(c.title);
    if !c.description.is_empty() || c.editable_description {
        let field = f.row_field(
            cx,
            slot(n),
            &c.description,
            RowEditCtx {
                cmd: "node.set",
                params: json!({}),
                key: "description",
                scale: 1.0,
                string_value: true,
                paint: None,
                nest: Some("props"),
                nest_key: None,
                paint_color: None,
            },
        );
        sec = sec.child(field_row_full(field));
    }
    for row in property_rows(f, c, cx) {
        sec = sec.child(row);
    }
    for row in current_variant_rows(f, c) {
        sec = sec.child(row);
    }
    Some(sec)
}

/// 组件的 Properties 节：每行一个属性定义，行尾设置按钮打开通用弹窗；header 的 + 建新属性。
fn property_rows(f: &RightFields, c: &ComponentData, cx: &mut App) -> Vec<crate::widgets::FieldRow> {
    let owner = c.main_id.unwrap_or(NodeId(0));
    let mut out: Vec<crate::widgets::FieldRow> = Vec::new();
    out.push(field_row_full(div().text_size(pt(11.0)).text_color(rgba(tokens().text)).child(tr("Properties"))));
    for def in &c.defs {
        let trigger: String = format!("component-prop-{}", def.name);
        let is_open = f.popups().is_open(&trigger);
        let popups = f.popups();
        let cmds = f.cmds.clone();
        let d = def.clone();
        let trig2 = trigger.clone();
        let settings = window_open_button(&trigger, crate::widgets::Icon::Settings, is_open, move |window, cx| {
            let top = crate::popovers::anchored_top(&trig2, window);
            let d = d.clone();
            let def = d.clone();
            let trig3 = trig2.clone();
            let popups = popups.clone();
            let cmds = cmds.clone();
            open_detached_with(popups, trig3, Some(top), cx, |anchor, cx| {
                crate::popovers::WindowState::component_property(
                    cmds.clone(),
                    cx,
                    owner.0,
                    def.name.clone(),
                    def.kind,
                    def.default.clone(),
                    def.options.iter().map(|(label, _)| label.clone()).collect(),
                    anchor,
                )
            });
        });
        let label = div()
            .w(px(84.0))
            .flex_none()
            .text_size(pt(11.0))
            .text_color(rgba(tokens().text_secondary))
            .child(SharedString::from(def.name.clone()));
        let kind = div()
            .flex_1()
            .min_w(px(0.0))
            .text_size(pt(11.0))
            .text_color(rgba(tokens().text))
            .child(SharedString::from(format!("{} · {}", def.kind, def.default)));
        out.push(field_row_full_trailing(div().flex().flex_row().items_center().gap(pt(8.0)).child(label).child(kind), vec![settings]));
    }
    out
}


/// + 按钮：弹菜单选属性类型，推 `component.addProperty`（引擎已有命令）。
fn add_property_button(f: &RightFields, c: &ComponentData, _cx: &mut App) -> gpui::AnyElement {
    let owner = c.main_id.unwrap_or(NodeId(0));
    let cmds = f.cmds.clone();
    let key = "component-add-property";
    let root = f.root.clone();
    let t = tokens();
    div()
        .id(SharedString::from(format!("window-open-{key}")))
        .size(pt(24.0))
        .rounded(pt(5.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .child(crate::widgets::icon::paint(Icon::Plus, rgba(t.icon), 16.0))
        .on_mouse_down(gpui::MouseButton::Left, move |_e, window, cx: &mut App| {
            let Some(root) = root.clone() else { return };
            let p = window.mouse_position();
            let anchor = gpui::Bounds { origin: p, size: gpui::size(px(0.0), px(0.0)) };
            let rows: Vec<crate::widgets::menu::MenuRow> = [("Variant", "variant"), ("Text", "text"), ("Boolean", "boolean"), ("Instance swap", "instanceSwap"), ("Slot", "slot")]
                .into_iter()
                .map(|(label, ty)| crate::widgets::menu::MenuRow::row(tr(label)).key(ty))
                .collect();
            let cmds = cmds.clone();
            let on_pick = std::rc::Rc::new(move |ty: &str, _w: &mut gpui::Window, _cx: &mut App| {
                let default = match ty {
                    "boolean" => json!(true),
                    "variant" => json!("Default"),
                    _ => Value::Null,
                };
                cmds.push(crate::panels::Cmd {
                    id: "component.addProperty",
                    params: Some(json!({ "id": owner.0, "name": tr("Property"), "type": ty, "default": default })),
                });
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(180.0), rows, on_pick, cx));
        })
        .into_any_element()
}

/// 属性下面的「当前变体」：组件/组件集列出了变体属性 → 每个属性的默认值/候选。
fn current_variant_rows(f: &RightFields, c: &ComponentData) -> Vec<crate::widgets::FieldRow> {
    let mut out: Vec<crate::widgets::FieldRow> = Vec::new();
    out.push(field_row_full(div().text_size(pt(11.0)).text_color(rgba(tokens().text_secondary)).child(tr("Current variant"))));
    for (name, values) in &c.variants {
        let label = div()
            .w(px(84.0))
            .flex_none()
            .text_size(pt(11.0))
            .text_color(rgba(tokens().text_secondary))
            .child(SharedString::from(name.clone()));
        let current = values.first().cloned().unwrap_or_default();
        let rows: Vec<(String, &'static str, Value)> = values
            .iter()
            .map(|v| (v.clone(), "component.editProperty", json!({ "name": name, "default": v })))
            .collect();
        let dd = pick_dropdown(f, current, 0.0, rows);
        out.push(field_row_full(div().flex().flex_row().items_center().gap(pt(8.0)).child(label).child(dd)));
    }
    out
}

fn instance_section(f: &RightFields, c: &ComponentData, cx: &mut App, n: &mut usize) -> PanelSection {
    let t = tokens();
    let reset = icon_cmd(Icon::Rotate, "component.resetInstance", json!({}), f, false);
    let mut trail = vec![reset];
    if !c.nested {
        trail.push(icon_cmd(Icon::Detach, "component.detach", json!({}), f, false));
    }
    let mut sec = PanelSection::new(SharedString::from(c.title)).header_trail(icon_row(trail));

    // ── 主组件（swap 下拉）+ 「去主组件 / 恢复主组件」──
    let swap_rows: Vec<(String, &'static str, Value)> =
        c.components.iter().map(|(id, name)| (name.clone(), "instance.swap", json!({ "component": id }))).collect();
    let main_dd = pick_dropdown(f, c.main_name.clone(), 0.0, swap_rows);
    let mut main_trail: Vec<IconButton> = Vec::new();
    if c.missing {
        main_trail.push(icon_cmd(Icon::Component, "component.restoreMain", json!({}), f, true));
    } else {
        main_trail.push(icon_cmd(Icon::Component, "component.goToMain", json!({}), f, false));
    }
    sec = sec.child(field_row(vec![main_dd.into_any_element()], main_trail));

    // ── 主组件描述（次要文字）──
    if !c.description.is_empty() {
        sec = sec.child(field_row_full(
            div().w_full().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(c.description.clone())),
        ));
    }

    // ── 嵌在主组件里的实例：把属性暴露给实例 ──
    if c.nested_in_main {
        let expose = icon_cmd(Icon::Check, "component.exposeInstance", json!({ "exposed": !c.exposed }), f, c.exposed);
        sec = sec.child(field_row(vec![plain_label(tr("Expose properties to instances"))], vec![expose]));
    }

    // ── 组件属性 ──
    if !c.props.is_empty() {
        sec = sec.child(sublabel_row(tr("Properties")));
    }
    for p in &c.props {
        let label = div().w(px(84.0)).flex_none().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(p.name.clone()));
        let control: gpui::AnyElement = match p.kind {
            "boolean" => {
                let on = p.value == "true";
                let btn = icon_cmd(Icon::Check, "instance.setProperties", json!({ "properties": { p.name.clone(): !on } }), f, on);
                div().flex().flex_row().items_center().child(btn).into_any_element()
            }
            "text" => f
                .row_field(
                    cx,
                    slot(n),
                    &p.value,
                    RowEditCtx {
                        cmd: "instance.setProperties",
                        params: json!({}),
                        key: "value",
                        scale: 1.0,
                        string_value: true,
                        paint: None,
                        nest: Some("properties"),
                        nest_key: Some(p.name.clone()),
                        paint_color: None,
                    },
                )
                .into_any_element(),
            "variant" | "swap" => {
                // options 已经是（显示名, 提交值）。
                let rows: Vec<(String, &'static str, Value)> = p
                    .options
                    .iter()
                    .map(|(label, value)| (label.clone(), "instance.setProperties", json!({ "properties": { p.name.clone(): value.clone() } })))
                    .collect();
                pick_dropdown(f, p.value.clone(), 0.0, rows).into_any_element()
            }
            _ => div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(tr("Slot"))).into_any_element(),
        };
        let line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full().child(label).child(div().flex_1().min_w(px(0.0)).child(control));
        sec = sec.child(field_row_full(line));
    }
    sec
}

/// 占满字段列的普通文字。
fn plain_label(text: &'static str) -> Div {
    let t = tokens();
    div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(text))
}
