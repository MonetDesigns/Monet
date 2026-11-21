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
    // lizard-main `component_panel.rs::instance_section` 的分派：
    // - 变体（属于 set 的 Component）→ `variant_ui`：Component values 矩阵 + Current variant，
    //   **不显示** 属性定义列表（Properties）。用户图1：选变体时应展示 current variant。
    // - 组件集 / 独立组件 → `properties_ui`：只有属性栏（Properties）。用户图2。
    if c.set_id.is_some() && !c.owner_is_set {
        return Some(variant_section(f, c, cx, n));
    }
    Some(properties_section(f, c, cx, n))
}

/// 选中变体（严格对齐 GitHub lizard main `variant_ui`）：
///   ① Component values：每个变体属性的候选值概览（lizard 用 `matrix_ui` 值表窗口；
///      Monet 暂无值表 UI，先按「属性 → 候选值」列出，不拼成一行）。
///   ② 分隔线 + "Current variant" 标题（带「Go to component set」按钮）。
///   ③ 每属性一行：**属性名可改**（`component.editProperty {newName}`）+ **值可输入/选**（`setVariantValue`），
///      对应 lizard 的 inline text_field + TextFieldWithPresets。之前是静态名 + 下拉，且与 ① 重复。
fn variant_section(f: &RightFields, c: &ComponentData, cx: &mut App, n: &mut usize) -> PanelSection {
    let t = tokens();
    let variant_id = c.main_id.unwrap_or(NodeId(0));
    let set = c.set_id.unwrap_or(NodeId(0));
    let mut sec = PanelSection::new(c.title);

    // ① Component values：属性 → 候选值（每行一个属性，值用 " / " 分隔）。
    if !c.own_variant_props.is_empty() {
        sec = sec.child(field_row_full(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(tr("Component values"))));
        for (name, values, _) in &c.own_variant_props {
            let label = div().w(px(84.0)).flex_none().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(name.clone()));
            let shown = if values.is_empty() { tr("None").to_string() } else { values.join(" / ") };
            let value = div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(shown));
            sec = sec.child(field_row_full(div().flex().flex_row().items_center().gap(pt(8.0)).child(label).child(value)));
        }
    }

    // ② Current variant 标题 + 「Go to component set」按钮（选中该 set）。
    let go = icon_cmd(Icon::ComponentSet, "select.set", json!({ "ids": [set.0] }), f, false);
    sec = sec.child(field_row_full_trailing(
        div().text_size(pt(11.0)).text_color(rgba(t.text)).child(tr("Current variant")),
        vec![go.into_any_element()],
    ));
    if c.own_variant_props.is_empty() {
        sec = sec.child(field_row_full(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("No variant properties"))));
    }

    // ③ 每属性：属性名可改 + 值可输入（两半并排，lizard `variant_ui` 的行布局）。
    for (name, values, current) in &c.own_variant_props {
        let name_edit = f.row_field(
            cx,
            slot(n),
            name,
            RowEditCtx {
                cmd: "component.editProperty",
                params: json!({ "id": set.0, "name": name }),
                key: "newName",
                scale: 1.0,
                string_value: true,
                paint: None,
                nest: None,
                nest_key: None,
                paint_color: None,
            },
        );
        let value_edit = f.row_field(
            cx,
            slot(n),
            current,
            RowEditCtx {
                cmd: "component.setVariantValue",
                params: json!({ "id": variant_id.0, "property": name }),
                key: "value",
                scale: 1.0,
                string_value: true,
                paint: None,
                nest: None,
                nest_key: None,
                paint_color: None,
            },
        );
        let hint = values.join(" / ");
        sec = sec.child(field_row_full(
            div().flex().flex_row().items_center().gap(pt(8.0)).w_full()
                .child(div().flex_1().min_w(px(0.0)).child(name_edit))
                .child(div().flex_1().min_w(px(0.0)).child(value_edit))
                .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(hint))),
        ));
    }
    sec
}

/// 选中组件集 / 独立组件（lizard `properties_ui`）：只有属性栏（描述 + Properties + 属性行）。
fn properties_section(f: &RightFields, c: &ComponentData, cx: &mut App, n: &mut usize) -> PanelSection {
    // lizard `instance_section`: ComponentSet 直接进入 `properties_ui` —— 标题由 Design panel
    // header 画，这里**没有** description 输入框（之前我塞了一个在 Properties 上面，是错的）。
    let sec = PanelSection::new(c.title);
    let sec = property_rows(f, c, cx, n).into_iter().fold(sec, |s, row| s.child(row));
    sec
}

/// 组件的 Properties 节（严格对齐 GitHub brava-group/lizard main 的 `properties_ui`）：
/// - 变体属性：从节点名字解析（`c.variants`），不在 PropDef 里 —— 之前漏了这一节，
///   导致点 + 加的 Variant 属性命令成功、UI 却纹丝不动。
/// - 共享属性：从 `c.defs` 读，按 Text/Boolean/InstanceSwap/Slot 排序。
/// 每行一个 PropertyController 式控件：类型图标 + 行内可改名 + 值 + 设置/删除按钮。
fn property_rows(f: &RightFields, c: &ComponentData, cx: &mut App, n: &mut usize) -> Vec<crate::widgets::FieldRow> {
    let owner = c.main_id.unwrap_or(NodeId(0));
    let t = tokens();
    let mut out: Vec<crate::widgets::FieldRow> = Vec::new();
    out.push(field_row_full_trailing(
        div().text_size(pt(11.0)).text_color(rgba(t.text)).child(tr("Properties")),
        vec![add_property_button(f, c)],
    ));

    // ── ① 变体属性（lizard `properties_ui` 的 variants 循环）────────────────────
    // 默认值取左上角默认变体；每行可改属性名/默认值、删除、展开值列表。
    for (name, values) in &c.variants {
        // lizard: 默认值取左上角默认变体；本快照未逐变体存值，退到候选首项。
        let default = values.first().cloned().unwrap_or_default();
        let name_edit = f.row_field(
            cx,
            slot(n),
            name,
            RowEditCtx {
                cmd: "component.editProperty",
                params: json!({ "id": owner.0, "name": name }),
                key: "newName",
                scale: 1.0,
                string_value: true,
                paint: None,
                nest: None,
                nest_key: None,
                paint_color: None,
            },
        );
        let rows: Vec<(String, &'static str, Value)> = values
            .iter()
            .map(|v| (v.clone(), "component.editProperty", json!({ "id": owner.0, "name": name, "default": v })))
            .collect();
        let dd = pick_dropdown(f, default.clone(), 0.0, rows);
        let trigger = format!("component-prop-{}", name);
        let settings = property_settings_button(f, &trigger, owner.0, name, "variant", default, values.iter().map(|v| (v.clone(), 0)).collect());
        let delete = icon_cmd(Icon::Minus, "component.deleteProperty", json!({ "id": owner.0, "name": name }), f, false);
        let mut line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full();
        line = line
            .child(crate::widgets::icon::paint_in_slot(Icon::ComponentSet, 14.0, Some(rgba(t.text_secondary))))
            .child(div().w(px(84.0)).flex_none().child(name_edit))
            .child(div().flex_1().min_w(px(0.0)).child(dd));
        out.push(field_row_full_trailing(line, vec![settings, delete.into_any_element()]));
    }

    // ── ② 共享属性（lizard `shared` 循环）───────────────────────────────────────
    // variant 定义在节点名字里、已由 ① 承载，这里只画非 variant 的 PropDef。
    let mut shared: Vec<_> = c.defs.iter().filter(|d| d.kind != "variant").collect();
    shared.sort_by_key(|d| match d.kind {
        "text" => 0u8,
        "boolean" => 1,
        "instanceSwap" => 2,
        "slot" => 3,
        _ => 4,
    });
    for def in shared {
        let name_edit = f.row_field(
            cx,
            slot(n),
            &def.name,
            RowEditCtx {
                cmd: "component.editProperty",
                params: json!({ "id": owner.0, "name": def.name }),
                key: "newName",
                scale: 1.0,
                string_value: true,
                paint: None,
                nest: None,
                nest_key: None,
                paint_color: None,
            },
        );
        let icon = match def.kind {
            "boolean" => Icon::Boolean,
            "text" => Icon::Text,
            "instanceSwap" => Icon::Instance,
            "slot" => Icon::Slot,
            _ => Icon::ComponentSet,
        };
        // 行内值控件：Boolean 复选框 / Text 输入框 / InstanceSwap 下拉 / Slot 静态。
        let value: gpui::AnyElement = match def.kind {
            "boolean" => {
                let on = def.default == tr("True");
                icon_cmd(
                    Icon::Check,
                    "component.editProperty",
                    json!({ "id": owner.0, "name": def.name, "default": !on }),
                    f,
                    on,
                )
                .into_any_element()
            }
            "text" => f
                .row_field(
                    cx,
                    slot(n),
                    &def.default,
                    RowEditCtx {
                        cmd: "component.editProperty",
                        params: json!({ "id": owner.0, "name": def.name }),
                        key: "default",
                        scale: 1.0,
                        string_value: true,
                        paint: None,
                        nest: None,
                        nest_key: None,
                        paint_color: None,
                    },
                )
                .into_any_element(),
            "instanceSwap" => {
                let rows: Vec<(String, &'static str, Value)> = def
                    .options
                    .iter()
                    .map(|(label, id)| (label.clone(), "component.editProperty", json!({ "id": owner.0, "name": def.name, "default": id })))
                    .collect();
                pick_dropdown(f, def.default.clone(), 0.0, rows).into_any_element()
            }
            _ => div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(def.default.clone())).into_any_element(),
        };
        let trigger = format!("component-prop-{}", def.name);
        let settings = property_settings_button(f, &trigger, owner.0, &def.name, def.kind, def.default.clone(), def.options.clone());
        let delete = icon_cmd(Icon::Minus, "component.deleteProperty", json!({ "id": owner.0, "name": def.name }), f, false);
        let mut line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full();
        line = line
            .child(crate::widgets::icon::paint_in_slot(icon, 14.0, Some(rgba(t.text_secondary))))
            .child(div().w(px(84.0)).flex_none().child(name_edit))
            .child(div().flex_1().min_w(px(0.0)).child(value));
        out.push(field_row_full_trailing(line, vec![settings, delete.into_any_element()]));
    }

    // ── ③ Add variant（lizard `can_add_variant` → 按钮）────────────────────────
    if c.can_add_variant && c.owner_is_set {
        let add_variant = icon_cmd(
            Icon::Plus,
            "component.addVariant",
            json!({ "id": owner.0 }),
            f,
            false,
        );
        out.push(field_row_full_trailing(
            div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Add variant")),
            vec![add_variant.into_any_element()],
        ));
    }
    out
}

/// 属性行的设置按钮：打开通用 `WindowState::component_property` 弹窗，编辑名称/默认值。
fn property_settings_button(
    f: &RightFields,
    trigger: &str,
    owner: u64,
    name: &str,
    kind: &'static str,
    default: String,
    options: Vec<(String, u64)>,
) -> gpui::AnyElement {
    let trigger: String = trigger.to_string();
    let is_open = f.popups().is_open(&trigger);
    let popups = f.popups();
    let cmds = f.cmds.clone();
    let name = name.to_string();
    let trig2 = trigger.clone();
    window_open_button(&trigger, crate::widgets::Icon::Settings, is_open, move |window, cx| {
        let top = crate::popovers::anchored_top(&trig2, window);
        let trig3 = trig2.clone();
        let popups = popups.clone();
        let cmds = cmds.clone();
        let name = name.clone();
        let default = default.clone();
        let options = options.clone();
        open_detached_with(popups, trig3, Some(top), cx, move |anchor, cx| {
            crate::popovers::WindowState::component_property(cmds.clone(), cx, owner, name.clone(), kind, default.clone(), options.clone(), anchor)
        })
    })
}

/// 生成一个不与现有属性重名的名字（引擎 `component.addProperty` 拒绝重名）。
fn unique_prop_name(used: &[String]) -> String {
    let base = tr("Property");
    if !used.iter().any(|n| n == &base) {
        return base.to_string();
    }
    let mut i = 2;
    loop {
        let name = format!("{} {}", base, i);
        if !used.iter().any(|n| n == &name) {
            return name;
        }
        i += 1;
    }
}

/// + 按钮：弹菜单选属性类型，推 `component.addProperty`（引擎已有命令）。
fn add_property_button(f: &RightFields, c: &ComponentData) -> gpui::AnyElement {
    let owner = c.main_id.unwrap_or(NodeId(0));
    let cmds = f.cmds.clone();
    let key = "component-add-property";
    let root = f.root.clone();
    let t = tokens();
    let candidates: Vec<(String, u64)> = c.component_candidates.iter().map(|(id, name)| (name.clone(), id.0)).collect();
    let add_popups = f.popups();
    let used_names: Vec<String> = c.defs.iter().map(|d| d.name.clone()).chain(c.variants.iter().map(|(n, _)| n.clone())).collect();
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
            let candidates = candidates.clone();
            let used_names = used_names.clone();
            let add_popups = add_popups.clone();
            let p = window.mouse_position();
            let anchor = gpui::Bounds { origin: p, size: gpui::size(px(0.0), px(0.0)) };
            // Slot creation stays on the Frame row's "Make slot" flow: the component-level
            // add menu has no Frame to bind, and engine add_prop requires a `layer`.
            let rows: Vec<crate::widgets::menu::MenuRow> = [
                ("Variant", "variant"),
                ("Text", "text"),
                ("Boolean", "boolean"),
                ("Instance swap", "instanceSwap"),
                ("Slot", "slot"),
            ]
            .into_iter()
            .map(|(label, ty)| crate::widgets::menu::MenuRow::row(tr(label)).key(ty))
            .collect();
            let cmds = cmds.clone();
            let on_pick = std::rc::Rc::new(move |ty: &str, window: &mut gpui::Window, cx: &mut App| {
                // lizard `add_property_menu`：Variant 点一下直接建、不弹窗；其余 4 种
                // (Text/Boolean/Instance swap/Slot) 才写 draft 打开带 Create 按钮的弹窗。
                if ty == "variant" {
                    cmds.push(crate::panels::Cmd {
                        id: "component.addProperty",
                        params: Some(json!({ "id": owner.0, "name": unique_prop_name(&used_names), "type": "variant", "default": "Default" })),
                    });
                    return;
                }
                let trigger = format!("component-create-property-{}", ty);
                let top = crate::popovers::anchored_top(&trigger, window);
                let base = match ty {
                    "variant" => tr("Variant"),
                    "text" => tr("Text"),
                    "boolean" => tr("Boolean"),
                    "instanceSwap" => tr("Instance"),
                    _ => tr("Slot"),
                };
                let name = if used_names.iter().any(|n| n == &base.to_string()) { unique_prop_name(&used_names) } else { base.to_string() };
                let kind: &'static str = match ty {
                    "variant" => "variant",
                    "text" => "text",
                    "boolean" => "boolean",
                    "instanceSwap" => "instanceSwap",
                    _ => "slot",
                };
                let cmds = cmds.clone();
                let candidates = candidates.clone();
                let add_popups = add_popups.clone();
                crate::panels::right::sections::open_detached_with(add_popups, &trigger, Some(top), cx, move |anchor, cx| {
                    crate::popovers::WindowState::component_property_create(
                        cmds.clone(),
                        cx,
                        owner.0,
                        kind,
                        name.clone(),
                        candidates.clone(),
                        anchor,
                    )
                });
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(180.0), rows, on_pick, cx));
        })
        .into_any_element()
}

/// 属性下面的「当前变体」：组件/组件集列出了变体属性 → 每个属性的默认值/候选。
fn current_variant_rows(f: &RightFields, c: &ComponentData) -> Vec<crate::widgets::FieldRow> {
    let owner = c.main_id.unwrap_or(NodeId(0));
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
            .map(|v| (v.clone(), "component.editProperty", json!({ "id": owner.0, "name": name, "default": v })))
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
        // 每个属性行左侧的类型图标（lizard `prop_label`）。
        let p_icon = match p.kind {
            "boolean" => Icon::Boolean,
            "text" => Icon::Text,
            "variant" => Icon::ComponentSet,
            "swap" => Icon::Instance,
            "slot" => Icon::Slot,
            _ => Icon::ComponentSet,
        };
        let label = div()
            .w(px(84.0))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(6.0))
            .child(crate::widgets::icon::paint_in_slot(p_icon, 14.0, Some(rgba(t.text_secondary))))
            .child(SharedString::from(p.name.clone()));
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
                // options 已经是（显示名, 提交值），且 preferred 已排在前面（lizard 的
                // Preferred / All components 顺序）。
                let rows: Vec<(String, &'static str, Value)> = p
                    .options
                    .iter()
                    .map(|(label, value)| (label.clone(), "instance.setProperties", json!({ "properties": { p.name.clone(): value.clone() } })))
                    .collect();
                pick_dropdown(f, p.value.clone(), 0.0, rows).into_any_element()
            }
            // Slot：lizard `instance_props` 的 `PropDef::Slot` → `slot_fill`。下拉里列出
            // preferred 组件 + 全部组件（`instance.slotInsert`），再给「Reset slot」。
            // 之前这里只画了一个静态 "Slot" 文字，等于完全没接。
            "slot" => slot_fill_control(f, c, p),
            _ => div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(p.value.clone())).into_any_element(),
        };
        let line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full().child(label).child(div().flex_1().min_w(px(0.0)).child(control));
        sec = sec.child(field_row_full(line));
    }
    sec
}

/// 占满字段列的普通文字。
/// 实例里一个 slot 属性的填充控件（lizard `slot_fill`）：显示「Empty / Default content / N layers」，
/// 下拉列出 preferred 组件 + 全部组件（`instance.slotInsert`），已编辑时再给「Reset slot」。
fn slot_fill_control(f: &RightFields, c: &ComponentData, p: &ComponentProp) -> gpui::AnyElement {
    let t = tokens();
    let label = if !p.slot_edited {
        if p.slot_layers == 0 { tr("Empty").to_string() } else { tr("Default content").to_string() }
    } else {
        format!("{} layers", p.slot_layers)
    };
    let Some(slot_id) = p.slot_id else {
        // 找不到实例里的槽副本（主组件里没有对应 frame）：只读显示当前状态。
        return div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(label)).into_any_element();
    };
    // preferred 组件排前面，其余组件随后（与 lizard 的分组顺序一致）。
    let mut ordered: Vec<&(NodeId, String)> =
        p.preferred.iter().filter_map(|id| c.components.iter().find(|(cid, _)| cid.0 == *id)).collect();
    for comp in &c.components {
        if !ordered.iter().any(|(cid, _)| cid == &comp.0) {
            ordered.push(comp);
        }
    }
    let mut rows: Vec<(String, &'static str, Value)> = ordered
        .into_iter()
        .map(|(cid, name)| (name.clone(), "instance.slotInsert", json!({ "id": slot_id.0, "component": cid.0 })))
        .collect();
    if p.slot_edited {
        rows.push((tr("Reset slot").to_string(), "instance.resetSlot", json!({ "id": slot_id.0 })));
    }
    pick_dropdown(f, label, 0.0, rows).into_any_element()
}

fn plain_label(text: &'static str) -> Div {
    let t = tokens();
    div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(text))
}

#[cfg(test)]
mod tests {
    use super::unique_prop_name;

    #[test]
    fn variant_value_edit_rewrites_only_its_segment() {
        use super::super::data::variant_name_with_value;
        assert_eq!(variant_name_with_value("Size=Large, State=Hover", "Size", "Small"), "Size=Small, State=Hover");
        assert_eq!(variant_name_with_value("Size=Large, State=Hover", "State", "Press"), "Size=Large, State=Press");
        // 名字里没有该属性时追加。
        assert_eq!(variant_name_with_value("Size=Large", "State", "On"), "Size=Large, State=On");
    }

    #[test]
    fn new_property_name_never_collides_with_existing() {
        let base = figcraft_i18n::tr("Property");
        assert_eq!(unique_prop_name(&[]), base);
        assert_eq!(unique_prop_name(&[base.to_string()]), format!("{} 2", base));
        assert_eq!(unique_prop_name(&[base.to_string(), format!("{} 2", base)]), format!("{} 3", base));
        // 非连续占用也跳过（删过属性后留下的空号不复用，行为确定即可）。
        assert_eq!(unique_prop_name(&[base.to_string(), format!("{} 3", base)]), format!("{} 2", base));
    }
}
