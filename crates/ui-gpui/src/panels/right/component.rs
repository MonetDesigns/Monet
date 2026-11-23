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

use super::data::{BindingData, ComponentData, ComponentDef, ComponentProp, PanelData};
use super::sections::{icon_cmd, icon_row, open_detached_with, pick_dropdown, slot, window_open_button};
use crate::panels::{RightFields, RowEditCtx};
use crate::widgets::{Icon, IconButton, PanelSection, field_row, field_row_full, field_row_full_trailing, pt, rgba, sublabel_row, tokens};
use figcraft_doc::NodeId;
use figcraft_i18n::tr;
use gpui::{App, Div, IntoElement, SharedString, div, prelude::*, px};
use serde_json::{Value, json};

/// Component / Instance 节（`None` = 选中的不是组件相关节点）。
pub(crate) fn section(f: &RightFields, d: &PanelData, cx: &mut App, n: &mut usize) -> Option<PanelSection> {
    // 组件内 / 实例内的普通图层：显示属性绑定与覆盖（lizard 的 text_value_ui /
    // visibility_value_ui / scoped_property_row / overrides_ui）。之前这里直接 `?` 返回 None，
    // 所以选组件里的图层什么都不显示。
    let Some(c) = d.component.as_ref() else {
        if let Some(b) = d.binding.as_ref() {
            return Some(binding_section(f, b));
        }
        // 多选主组件 → 「Combine as variants」（lizard `combine_ui`）。
        if !d.combine_ids.is_empty() {
            return Some(combine_section(f, d));
        }
        return None;
    };
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

    // ① Component values：属性定义列表（lizard `component_matrix_ui::show` 的 axis 行）。
    // 之前这里只是「属性名 + 候选值」的静态文本，选中变体时看不到也改不了属性定义；
    // 现在每行 = 类型图标 + 可改名 + N values 摘要 + 设置（展开值列表）+ 删除，
    // 与组件集 `property_rows` 的变体属性行同构，命令落在组件集上。
    if !c.own_variant_props.is_empty() {
        sec = sec.child(field_row_full(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(tr("Component values"))));
        for (name, values, _) in &c.own_variant_props {
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
            let expanded = f.variant_expanded.borrow().iter().any(|x| x == name);
            let expanded_ref = f.variant_expanded.clone();
            let toggle_name = name.clone();
            let wake = f.cmds.clone();
            let toggle = div()
                .id(SharedString::from(format!("variant-def-expand-{name}")))
                .size(pt(24.0))
                .rounded(pt(5.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(expanded, |d| d.bg(rgba(t.selected_row)))
                .when(!expanded, |d| d.hover(|s| s.bg(rgba(t.hover))))
                .child(crate::widgets::icon::paint(Icon::Settings, rgba(t.icon), 16.0))
                .on_mouse_down(gpui::MouseButton::Left, move |_e, _w, _cx| {
                    let mut list = expanded_ref.borrow_mut();
                    if let Some(pos) = list.iter().position(|x| x == &toggle_name) {
                        list.remove(pos);
                    } else {
                        list.push(toggle_name.clone());
                    }
                    drop(list);
                    wake.wake();
                });
            let delete = icon_cmd(Icon::Minus, "component.deleteProperty", json!({ "id": set.0, "name": name }), f, false);
            let mut line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full();
            line = line
                .child(crate::widgets::icon::paint_in_slot(Icon::ComponentSet, 14.0, Some(rgba(t.text_secondary))))
                .child(div().w(px(84.0)).flex_none().child(name_edit))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(pt(11.0))
                        .text_color(rgba(t.text_secondary))
                        .child(SharedString::from(format!("{} values", values.len()))),
                );
            sec = sec.child(field_row_full_trailing(line, vec![toggle.into_any_element(), delete.into_any_element()]));

            // 展开：候选值可改名 + 加新值（lizard 值表窗口的内联等价）。
            if expanded {
                let mut vrow = div().flex().flex_row().flex_wrap().gap(pt(6.0)).w_full().pl(pt(22.0)).pb(pt(4.0));
                for v in values {
                    let edit = f.row_field(
                        cx,
                        slot(n),
                        v,
                        RowEditCtx {
                            cmd: "component.editProperty",
                            params: json!({ "id": set.0, "name": name, "value": v }),
                            key: "newValue",
                            scale: 1.0,
                            string_value: true,
                            paint: None,
                            nest: None,
                            nest_key: None,
                            paint_color: None,
                        },
                    );
                    vrow = vrow.child(div().w(px(86.0)).child(edit));
                }
                let fresh_value = (values.len() + 1..)
                    .map(|k| format!("Value {k}"))
                    .find(|cand| !values.iter().any(|v| v == cand))
                    .unwrap_or_else(|| format!("Value {}", values.len() + 1));
                let add = icon_cmd(
                    Icon::Plus,
                    "component.addVariant",
                    json!({ "id": set.0, "property": name, "value": fresh_value }),
                    f,
                    false,
                );
                vrow = vrow.child(add);
                sec = sec.child(field_row_full(vrow));
            }
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
    let t = tokens();
    let sec = PanelSection::new(c.title);
    let sec = property_rows(f, c, cx, n).into_iter().fold(sec, |s, row| s.child(row));
    // 变体值冲突警告（lizard `variant_conflicts`：同一组合出现多个变体时给黄条）。
    let sec = if c.variant_conflicts.is_empty() {
        sec
    } else {
        sec.child(field_row_full(
            div()
                .text_size(pt(11.0))
                .text_color(rgba(t.warning))
                .child(tr("Some variants have the same property values: give each one a unique combination")),
        ))
    };
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
    // 行：类型图标 + 可改名 + 默认值下拉 + 展开按钮 + 删除。
    // 展开后是**值列表**：每个值一个可改名输入框（改 → editProperty value/newValue），
    // 末尾 + 加一个新值（addVariant property/value）。之前只做了弹窗改默认值，
    // 值的重命名 / 新增完全没接（用户反复指出）。
    for (name, values) in &c.variants {
        // 默认值：取默认变体（左上角）里该属性的值。快照未逐变体存值，用候选首项兜底。
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
        // 展开按钮（lizard `summary.settings_clicked`：切换展开，不弹窗）。
        let expanded = f.variant_expanded.borrow().iter().any(|n| n == name);
        let expanded_ref = f.variant_expanded.clone();
        let toggle_name = name.clone();
        let wake = f.cmds.clone();
        let toggle = div()
            .id(SharedString::from(format!("variant-expand-{name}")))
            .size(pt(24.0))
            .rounded(pt(5.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(expanded, |d| d.bg(rgba(t.selected_row)))
            .when(!expanded, |d| d.hover(|s| s.bg(rgba(t.hover))))
            .child(crate::widgets::icon::paint(Icon::Settings, rgba(t.icon), 16.0))
            .on_mouse_down(gpui::MouseButton::Left, move |_e, _w, _cx| {
                let mut list = expanded_ref.borrow_mut();
                if let Some(pos) = list.iter().position(|n| n == &toggle_name) {
                    list.remove(pos);
                } else {
                    list.push(toggle_name.clone());
                }
                drop(list);
                wake.wake();
            });
        let delete = icon_cmd(Icon::Minus, "component.deleteProperty", json!({ "id": owner.0, "name": name }), f, false);
        let mut line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full();
        line = line
            .child(crate::widgets::icon::paint_in_slot(Icon::ComponentSet, 14.0, Some(rgba(t.text_secondary))))
            .child(div().w(px(84.0)).flex_none().child(name_edit))
            .child(div().flex_1().min_w(px(0.0)).child(dd));
        out.push(field_row_full_trailing(line, vec![toggle.into_any_element(), delete.into_any_element()]));

        // 展开：值列表（可改名）+ 加值。
        if expanded {
            let mut vrow = div().flex().flex_row().flex_wrap().gap(pt(6.0)).w_full().pl(pt(22.0)).pb(pt(4.0));
            for v in values {
                let edit = f.row_field(
                    cx,
                    slot(n),
                    v,
                    RowEditCtx {
                        cmd: "component.editProperty",
                        params: json!({ "id": owner.0, "name": name, "value": v }),
                        key: "newValue",
                        scale: 1.0,
                        string_value: true,
                        paint: None,
                        nest: None,
                        nest_key: None,
                        paint_color: None,
                    },
                );
                vrow = vrow.child(div().w(px(86.0)).child(edit));
            }
            // 加值：新值名 "Value N"，避开已有值。
            let fresh_value = (values.len() + 1..)
                .map(|k| format!("Value {k}"))
                .find(|cand| !values.iter().any(|v| v == cand))
                .unwrap_or_else(|| format!("Value {}", values.len() + 1));
            let add = icon_cmd(
                Icon::Plus,
                "component.addVariant",
                json!({ "id": owner.0, "property": name, "value": fresh_value }),
                f,
                false,
            );
            vrow = vrow.child(add);
            out.push(field_row_full(vrow));
        }
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
                checkbox_cmd(on, "component.editProperty", json!({ "id": owner.0, "name": def.name, "default": !on }), f)
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
            // instanceSwap 默认值：带搜索的组件列表（组件多时必须能搜）。
            "instanceSwap" => {
                let trigger = format!("idef-{}", def.name);
                let is_open = f.popups().is_open(&trigger);
                let popups = f.popups();
                let cmds = f.cmds.clone();
                let dname = def.name.clone();
                let rows: Vec<crate::popovers::Row> = def
                    .options
                    .iter()
                    .map(|(label, id)| {
                        crate::popovers::Row::new(
                            label.clone(),
                            "component.editProperty",
                            json!({ "id": owner.0, "name": dname.clone(), "default": id }),
                        )
                    })
                    .collect();
                let trig2 = trigger.clone();
                let shown = def.default.clone();
                window_open_button(&trigger, Icon::Instance, is_open, move |window, cx| {
                    let top = crate::popovers::anchored_top(&trig2, window);
                    let trig3 = trig2.clone();
                    let popups = popups.clone();
                    let cmds = cmds.clone();
                    let rows = rows.clone();
                    let dname = dname.clone();
                    open_detached_with(popups, trig3, Some(top), cx, move |anchor, cx| {
                        crate::popovers::WindowState::component_search(cmds.clone(), cx, dname.clone(), rows.clone(), anchor)
                    })
                })
                .into_any_element()
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

        // instanceSwap / slot 的 preferred 组件行（lizard `properties_ui` 里
        // `PropDef::Slot { preferred } | InstanceSwap { preferred, .. }` 那段）：
        // 勾选后随属性定义一起保存（`component.editProperty { preferred: [ids] }`）。
        if matches!(def.kind, "slot" | "instanceSwap") {
            let current_pref = def.preferred.clone();
            let label = if current_pref.is_empty() {
                tr("No preferred components").to_string()
            } else {
                format!("{} preferred", current_pref.len())
            };
            let mut pref_rows: Vec<(String, &'static str, Value)> = Vec::new();
            for (cid, cname) in &c.components {
                let on = current_pref.contains(&cid.0);
                let mut next: Vec<u64> = current_pref.iter().filter(|x| **x != cid.0).cloned().collect();
                if !on {
                    next.push(cid.0);
                }
                pref_rows.push((
                    format!("{}{}", if on { "✓ " } else { "" }, cname),
                    "component.editProperty",
                    json!({ "id": owner.0, "name": def.name, "preferred": next }),
                ));
            }
            let dd = pick_dropdown(f, label, 0.0, pref_rows);
            out.push(field_row_full(
                div().flex().flex_row().items_center().gap(pt(8.0)).w_full().pl(pt(22.0))
                    .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(tr("Preferred")))
                    .child(div().flex_1().min_w(px(0.0)).child(dd)),
            ));
        }
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
    // lizard `instance_ui` 头部：Dots 菜单（Go to main / Push changes / Restore / Reset / Detach）
    // + Detach + Reset。之前只有 Reset / Detach，pushChanges 完全没有入口。
    let mut trail: Vec<gpui::AnyElement> = vec![instance_more_button(f, c)];
    trail.push(reset.into_any_element());
    if !c.nested {
        trail.push(icon_cmd(Icon::Detach, "component.detach", json!({}), f, false).into_any_element());
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
        sec = sec.child(instance_prop_row(f, c, cx, n, p));
    }
    // 暴露属性的嵌套实例（lizard `exposed_copies`）：名字小标题 + 它们各自的属性行。
    // 之前这块完全没接，实例里暴露的嵌套实例属性在右栏看不到。
    for (name, props) in &c.exposed_instances {
        sec = sec.child(field_row_full(
            div().mt(pt(6.0)).text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(name.clone())),
        ));
        for p in props {
            sec = sec.child(instance_prop_row(f, c, cx, n, p));
        }
    }
    sec
}

/// 实例属性的一行（lizard `instance_props` 的 row）：类型图标 + 属性名 + 值控件。
fn instance_prop_row(f: &RightFields, c: &ComponentData, cx: &mut App, n: &mut usize, p: &ComponentProp) -> crate::widgets::FieldRow {
    let t = tokens();
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
        "boolean" => checkbox_cmd(p.value == "true", "instance.setProperties", json!({ "properties": { p.name.clone(): p.value != "true" } }), f),
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
        "variant" => {
            let rows: Vec<(String, &'static str, Value)> = p
                .options
                .iter()
                .map(|(label, value)| (label.clone(), "instance.setProperties", json!({ "properties": { p.name.clone(): value.clone() } })))
                .collect();
            pick_dropdown(f, p.value.clone(), 0.0, rows).into_any_element()
        }
        // swap：带搜索的组件列表（lizard `swap_picker` / `comp_search`）。组件多时
        // 平铺下拉根本没法用（图标库几百个组件），所以换成搜索窗。
        "swap" => {
            let trigger = format!("swap-{}", p.name);
            let is_open = f.popups().is_open(&trigger);
            let popups = f.popups();
            let cmds = f.cmds.clone();
            let pname = p.name.clone();
            let rows: Vec<crate::popovers::Row> = p
                .options
                .iter()
                .map(|(label, value)| {
                    crate::popovers::Row::new(
                        label.clone(),
                        "instance.setProperties",
                        json!({ "properties": { pname.clone(): value.clone() } }),
                    )
                })
                .collect();
            let trig2 = trigger.clone();
            let shown = p.value.clone();
            window_open_button(&trigger, Icon::Instance, is_open, move |window, cx| {
                let top = crate::popovers::anchored_top(&trig2, window);
                let trig3 = trig2.clone();
                let popups = popups.clone();
                let cmds = cmds.clone();
                let rows = rows.clone();
                let pname = pname.clone();
                open_detached_with(popups, trig3, Some(top), cx, move |anchor, cx| {
                    crate::popovers::WindowState::component_search(cmds.clone(), cx, pname.clone(), rows.clone(), anchor)
                })
            })
            .into_any_element()
        }
        "slot" => slot_fill_control(f, c, p),
        _ => div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(p.value.clone())).into_any_element(),
    };
    let line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full().child(label).child(div().flex_1().min_w(px(0.0)).child(control));
    field_row_full(line)
}

/// 实例头部的 More 菜单（lizard `instance_ui` 的 `Icon::Dots`）：
/// Go to main component / Push changes to main / Restore main / Reset all changes / Detach。
/// pushChanges 之前没有任何 UI 入口。
fn instance_more_button(f: &RightFields, c: &ComponentData) -> gpui::AnyElement {
    let root = f.root.clone();
    let cmds = f.cmds.clone();
    let t = tokens();
    let missing = c.missing;
    let nested = c.nested;
    let node = c.main_id.map(|i| i.0).unwrap_or(0);
    let instance_node = c.instance_id.unwrap_or(NodeId(0));
    div()
        .id("window-open-instance-more")
        .size(pt(24.0))
        .rounded(pt(5.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .child(crate::widgets::icon::paint(Icon::Menu, rgba(t.icon), 16.0))
        .on_mouse_down(gpui::MouseButton::Left, move |_e, window, cx: &mut App| {
            let Some(root) = root.clone() else { return };
            let p = window.mouse_position();
            let anchor = gpui::Bounds { origin: p, size: gpui::size(px(0.0), px(0.0)) };
            let mut items: Vec<(&'static str, &'static str)> = Vec::new();
            if !missing {
                items.push(("Go to main component", "goToMain"));
                items.push(("Push changes to main component", "pushChanges"));
            }
            if missing && !nested {
                items.push(("Restore main component", "restoreMain"));
            }
            items.push(("Reset all changes", "resetInstance"));
            if !nested {
                items.push(("Detach instance", "detach"));
            }
            let rows: Vec<crate::widgets::menu::MenuRow> =
                items.iter().map(|(label, key)| crate::widgets::menu::MenuRow::row(tr(*label)).key(*key)).collect();
            let cmds = cmds.clone();
            let on_pick = std::rc::Rc::new(move |key: &str, _w: &mut gpui::Window, _cx: &mut App| {
                let id = instance_node.0;
                let (cmd, params): (&'static str, Value) = match key {
                    "goToMain" => ("component.goToMain", json!({ "id": id })),
                    "pushChanges" => ("component.pushChanges", json!({ "id": id })),
                    "restoreMain" => ("component.restoreMain", json!({ "id": id })),
                    "resetInstance" => ("component.resetInstance", json!({ "ids": [id] })),
                    "detach" => ("component.detach", json!({ "ids": [id] })),
                    _ => return,
                };
                let _ = node;
                cmds.push(crate::panels::Cmd { id: cmd, params: Some(params) });
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(220.0), rows, on_pick, cx));
        })
        .into_any_element()
}

/// 多选主组件时的「Combine as variants」按钮（lizard `combine_ui`）。
fn combine_section(f: &RightFields, d: &PanelData) -> PanelSection {
    let t = tokens();
    let ids: Vec<u64> = d.combine_ids.iter().map(|i| i.0).collect();
    let label = tr("Combine as variants");
    let btn = div()
        .id("combine-as-variants")
        .h(pt(24.0))
        .px(pt(10.0))
        .rounded(pt(5.0))
        .flex()
        .items_center()
        .gap(pt(6.0))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .child(crate::widgets::icon::paint_in_slot(Icon::ComponentSet, 14.0, Some(rgba(t.icon))))
        .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(label)))
        .on_mouse_down(gpui::MouseButton::Left, {
            let cmds = f.cmds.clone();
            move |_e, _w, _cx| {
                cmds.push(crate::panels::Cmd { id: "component.combineVariants", params: Some(json!({ "ids": ids.clone() })) });
            }
        });
    PanelSection::new(tr("Component")).child(field_row_full(btn))
}

/// 组件内 / 实例内图层的属性绑定与覆盖（lizard `component_panel.rs` 末尾）：
/// - 在实例里 → Overrides 节（每组一行 + 单组重置 + 全部重置）。
/// - 在组件里 → Component property 节：Visible / Text / Instance / Slot 各一行的绑定下拉。
fn binding_section(f: &RightFields, b: &BindingData) -> PanelSection {
    let t = tokens();
    let mut sec = PanelSection::new(tr("Component property"));

    // ── Slot（选中实例里的槽，lizard `slot_section`）────────────────────────────
    // 显示槽的属性名 + 填充下拉（preferred 组件在前 + 全部组件 + Reset slot）。
    if b.in_instance && b.is_slot {
        let name = b.slot_name.clone().unwrap_or_default();
        sec = sec.child(field_row_full(
            div().flex().flex_row().items_center().gap(pt(6.0))
                .child(crate::widgets::icon::paint_in_slot(Icon::Slot, 14.0, Some(rgba(t.text_secondary))))
                .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(name.clone()))),
        ));
        let edited = b.overrides.iter().any(|g| g == "slot");
        let label = if edited { tr("Own content").to_string() } else { tr("Default content").to_string() };
        let mut rows: Vec<(String, &'static str, Value)> = b
            .components
            .iter()
            .map(|(cid, cname)| (cname.clone(), "instance.slotInsert", json!({ "id": b.node.0, "component": cid.0 })))
            .collect();
        rows.push((tr("Reset slot").to_string(), "instance.resetSlot", json!({ "id": b.node.0 })));
        let dd = pick_dropdown(f, label, 0.0, rows);
        sec = sec.child(field_row_full(div().flex().flex_row().items_center().gap(pt(8.0)).w_full().pl(pt(20.0)).child(dd)));
    }

    // ── Overrides（实例内图层）──────────────────────────────────────────────
    if b.in_instance && !b.overrides.is_empty() {
        let reset_all = icon_cmd(Icon::Rotate, "component.resetOverride", json!({ "id": b.node.0 }), f, false);
        sec = sec.child(field_row_full_trailing(
            div().text_size(pt(11.0)).text_color(rgba(t.text)).child(tr("Overrides")),
            vec![reset_all.into_any_element()],
        ));
        for g in &b.overrides {
            let reset = icon_cmd(Icon::Close, "component.resetOverride", json!({ "id": b.node.0, "group": g }), f, false);
            sec = sec.child(field_row_full_trailing(
                div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(override_group_label(g))),
                vec![reset.into_any_element()],
            ));
        }
        return sec;
    }

    if b.in_instance {
        return sec;
    }

    // ── Visible 绑定（所有组件内图层，lizard `visibility_value_ui`）───────────
    sec = sec.child(bind_row(f, b, "visible", tr("Visible"), "boolean", b.visible.to_string()));
    // ── Text 绑定（文本图层，lizard `text_value_ui`）────────────────────────
    if b.kind == "text" {
        sec = sec.child(bind_row(f, b, "characters", tr("Text content"), "text", String::new()));
    }
    // ── Instance 绑定（嵌套实例，lizard `scoped_property_row(mainComponent)`）─
    if b.show_main_component_row {
        sec = sec.child(bind_row(f, b, "mainComponent", tr("Instance"), "instanceSwap", String::new()));
    }
    // ── Slot 绑定（Frame，lizard `scoped_property_row(slot)` → Make slot / Remove slot）─
    if b.show_slot_row {
        let label = if b.is_slot { tr("Slot") } else { tr("Convert to slot") };
        sec = sec.child(bind_row(f, b, "slot", label, "slot", b.slot_name.clone().unwrap_or_default()));
    }
    sec
}

/// 一行绑定控件：字段标签 + 下拉（列出类型匹配的属性、解除绑定、创建属性；slot 另有 Make/Remove）。
fn bind_row(
    f: &RightFields,
    b: &BindingData,
    field: &'static str,
    label: &'static str,
    ty: &'static str,
    current: String,
) -> crate::widgets::FieldRow {
    let t = tokens();
    let bound = b.bound.iter().find(|(k, _)| k == field).map(|(_, v)| v.clone());
    let shown = bound.clone().unwrap_or_else(|| current.clone());
    let shown = if shown.is_empty() { tr("Not bound").to_string() } else { shown };
    let mut rows: Vec<(String, &'static str, Value)> = Vec::new();
    // 类型匹配的属性。
    for (name, kind) in &b.props {
        if *kind == ty {
            rows.push((name.clone(), "component.bindProperty", json!({ "layer": b.node.0, "field": field, "property": name })));
        }
    }
    if ty == "slot" {
        if b.is_slot {
            rows.push((tr("Remove slot").to_string(), "component.removeSlot", json!({ "id": b.node.0 })));
        } else {
            rows.push((tr("Make slot").to_string(), "component.makeSlot", json!({ "id": b.node.0 })));
        }
    } else {
        // 新建一个同类型属性并绑到本图层（lizard `Create property…`）。
        rows.push((tr("Create property").to_string(), "component.addProperty", json!({ "id": b.node.0, "name": label, "type": ty })));
        if bound.is_some() {
            rows.push((tr("Detach property").to_string(), "component.bindProperty", json!({ "layer": b.node.0, "field": field, "property": null })));
        }
    }
    let dd = pick_dropdown(f, shown, 0.0, rows);
    field_row(
        vec![div().w(px(84.0)).flex_none().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(label)).into_any_element(),
             dd.into_any_element()],
        Vec::<gpui::AnyElement>::new(),
    )
}

/// 覆盖组的显示名（lizard `group_label`）。
fn override_group_label(g: &str) -> String {
    match g {
        "fills" => tr("Fill"),
        "strokes" => tr("Stroke"),
        "effects" => tr("Effects"),
        "opacity" => tr("Opacity & blend"),
        "visible" => tr("Visibility"),
        "name" => tr("Layer name"),
        "corners" => tr("Corner radius"),
        "characters" => tr("Text content"),
        "textStyle" => tr("Text style"),
        "padding" => tr("Padding"),
        "gap" => tr("Gap"),
        "alignment" => tr("Alignment"),
        "grids" => tr("Layout grids"),
        "exports" => tr("Export settings"),
        "interactions" => tr("Interactions"),
        "size" => tr("Size"),
        "resizing" => tr("Resizing"),
        "position" => tr("Position"),
        "component" => tr("Swapped instance"),
        "props" => tr("Instance properties"),
        "slot" => tr("Slot content"),
        other => other,
    }
    .to_string()
}

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

/// 布尔属性的勾选框（lizard `widgets::checkbox`）：方框 + 勾 + True/False。
/// GPUI 没有通用 checkbox 控件，这里按项目现有图标按钮的尺寸画一个。
fn checkbox_cmd(on: bool, cmd: &'static str, params: Value, f: &RightFields) -> gpui::AnyElement {
    let cmds = f.cmds.clone();
    let t = tokens();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(6.0))
        .cursor_pointer()
        .on_mouse_down(gpui::MouseButton::Left, move |_e, _w, _cx| {
            cmds.push(crate::panels::Cmd { id: cmd, params: Some(params.clone()) });
        })
        .child(
            div()
                .size(pt(14.0))
                .rounded(pt(3.0))
                .flex()
                .items_center()
                .justify_center()
                .when(on, |d| d.bg(rgba(t.accent)))
                .when(!on, |d| d.border_1().border_color(rgba(t.border)))
                .when(on, |d| d.child(crate::widgets::icon::paint(Icon::Check, rgba(t.text), 10.0))),
        )
        .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(if on { tr("True") } else { tr("False") })))
        .into_any_element()
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
