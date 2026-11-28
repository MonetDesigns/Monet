//! Prototype 标签（egui `ui-egui/src/panels/proto_panel.rs` 的 GPUI 版）。
//!
//! 三段结构照抄 egui `prototype()`：
//! - **无选中**：`Flow starting points`（起点列表）+ `Prototype settings`（设备 / 旋转 / 机身色 /
//!   自定义尺寸 / Responsive / 背景）。
//! - **有选中**：`Flow starting point`（仅顶层 frame）+ `Interactions` + `Scroll behavior`。
//!
//! 全部走冻结通用控件（`PanelSection` / `field_row*` / `Dropdown` / `IconButton` / `swatch`）；
//! 动态行文本用 `RightFields::row_field` 的编辑器池，没有新建任何控件。
//!
//! 图标差异（冻结 `Icon` 集没有 egui 的 `Play` / `Preview`）：play 用 `ChevronRight`、
//! preview 用 `Eye`、连接字形用 `LinkOn`，语义一一对应。

use super::data::{FlowRow, InteractionRow, PanelData, ProtoData, ScrollData};
use super::sections::{icon_cmd, icon_row, pick_dropdown, pick_dropdown_sized, slot};
use crate::panels::{Cmd, PROTO_EDITOR_SLOT, RightFields, RowEditCtx};
use crate::widgets::popover::{self, popover_body, popover_window, title_text};
use crate::widgets::{
    Icon, IconButton, PanelSection, SectionHeaderVariant, SegItem, Segmented, field_row, field_row_full, field_row_full_trailing, pt, rgba,
    sublabel_row, sublabel_row2, swatch, tokens,
};
use figcraft_i18n::tr;
use gpui::{AnyElement, App, Div, IntoElement, Pixels, SharedString, Size, Window, div, point, prelude::*, px};
use serde_json::{Value, json};

thread_local! {
    /// 最近一次点开交互行时该行在窗口里的顶边（浮窗用它对齐，egui `editor_popover` 的 `y`）。
    static ANCHOR_Y: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// Prototype 标签整页（`right_panel` 在 `right_tab == 1` 时调用）。
pub(crate) fn panel(f: &RightFields, d: &PanelData, cx: &mut App, n: &mut usize) -> Div {
    let p = &d.proto;
    let mut body = div().flex().flex_col().w_full();
    if d.header.is_none() {
        // ── 无选中：只画重写的原型设置（不再显示 Flow starting points）──
        body = body.child(settings_section(f, p, cx, n));
        return body;
    }
    // ── 有选中：flow（顶层 frame 才有）+ interactions + scroll ──
    if let Some(mine) = &p.frame_flow {
        body = body.child(flow_point_section(f, mine.as_ref(), cx, n)).child(crate::panels::sep());
    }
    body = body.child(interactions_section(f, p));
    if let Some(scroll) = &p.scroll {
        body = body.child(crate::panels::sep()).child(scroll_section(f, scroll));
    }
    body
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Flow starting points（无选中）
// ───────────────────────────────────────────────────────────────────────────────────────────────

fn flows_section(f: &RightFields, p: &ProtoData, cx: &mut App, n: &mut usize) -> PanelSection {
    let empty = p.flows.is_empty();
    let plus = icon_cmd(Icon::Plus, "proto.flow.add", json!({}), f, false);
    let mut sec = PanelSection::new(tr("Flow starting points"))
        .variant(if empty { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal })
        .header_trail(icon_row(vec![plus]));
    if empty {
        return sec.child(field_row_full(note(tr("Select a frame and add a starting point to present it as a flow."))));
    }
    for flow in &p.flows {
        sec = sec.child(field_row_full(flow_line(f, flow, cx, n)));
        if !flow.description.trim().is_empty() {
            sec = sec.child(field_row_full(desc_line(&flow.description)));
        }
    }
    sec
}

/// 一条 flow 行：play / 名字（可编辑）/ preview / remove（egui `flow_row`）。
fn flow_line(f: &RightFields, flow: &FlowRow, cx: &mut App, n: &mut usize) -> Div {
    let play = icon_cmd(Icon::ChevronRight, "proto.present", json!({ "start": flow.start }), f, false);
    let preview = icon_cmd(Icon::Eye, "proto.preview", json!({ "start": flow.start }), f, false);
    let remove = icon_cmd(Icon::Minus, "proto.flow.remove", json!({ "start": flow.start }), f, false);
    let edit = f.row_field(
        cx,
        slot(n),
        &flow.name,
        RowEditCtx {
            cmd: "proto.flow.rename",
            params: json!({ "start": flow.start }),
            key: "name",
            scale: 1.0,
            string_value: true,
            paint: None,
            nest: None,
            nest_key: None,
            map: None,
            paint_color: None,
        },
    );
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(6.0))
        .w_full()
        .child(play)
        .child(div().flex_1().min_w(px(0.0)).child(edit))
        .child(preview)
        .child(remove)
}

/// flow 的描述（名字下面缩进一行的次要文字）。
fn desc_line(text: &str) -> Div {
    let t = tokens();
    div().w_full().pl(pt(30.0)).text_size(pt(10.5)).text_color(rgba(t.text_secondary)).child(SharedString::from(text.to_string()))
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Prototype settings（无选中）
// ───────────────────────────────────────────────────────────────────────────────────────────────

fn settings_section(f: &RightFields, p: &ProtoData, _cx: &mut App, _n: &mut usize) -> PanelSection {
    let s = &p.settings;
    let preview = icon_cmd(Icon::Eye, "proto.preview", json!({}), f, false);
    let mut sec = PanelSection::new(tr("Prototype settings")).header_trail(icon_row(vec![preview]));

    // ── 第一栏：总设备（family）下拉，行尾显示它的屏幕尺寸 ──
    let families: Vec<(String, String, &'static str, Value)> =
        s.devices.iter().map(|d| (d.clone(), figcraft_engine::devices::family_size_label(d), "proto.settings", json!({ "device": d }))).collect();
    let family_dd = pick_dropdown_sized(f, s.device.clone(), 0.0, families);
    sec = sec.child(field_row_full(family_dd));

    // ── 第二栏：分型号下拉，行尾显示该型号 W × H ──
    let is_special = s.device == figcraft_doc::proto::PrototypeSettings::CUSTOM_DEVICE || s.device == "None";
    if !is_special && !s.variants.is_empty() {
        let variants: Vec<(String, String, &'static str, Value)> =
            s.variants.iter().map(|v| (v.clone(), figcraft_engine::devices::size_label(v), "proto.settings", json!({ "variant": v }))).collect();
        let current = if s.variant.is_empty() { s.variants.first().cloned().unwrap_or_default() } else { s.variant.clone() };
        sec = sec.child(field_row_full(pick_dropdown_sized(f, current, 0.0, variants)));
    }

    // ── 第三栏：设备颜色（通用 swatch）+ 竖/横二元选择器，同一行 ──
    let cmds = f.cmds.clone();
    let rotating = s.rotates;
    let mut third: Vec<gpui::AnyElement> = Vec::new();
    if !s.colors.is_empty() {
        let rows: Vec<(String, &'static str, Value)> =
            s.colors.iter().map(|(name, _)| (name.clone(), "proto.settings", json!({ "color": name }))).collect();
        let rgb = s.colors.iter().find(|(name, _)| *name == s.color).map(|(_, c)| *c).unwrap_or([0, 0, 0]);
        let dot = swatch(figcraft_color::Paint::solid(figcraft_color::Color::from_rgb8_const(rgb[0], rgb[1], rgb[2])), 16.0);
        let line = div().flex().flex_row().items_center().gap(pt(8.0)).flex_1().child(dot).child(pick_dropdown(f, s.color.clone(), 0.0, rows));
        third.push(line.into_any_element());
    }
    let orientation = Segmented::new(vec![SegItem::text(tr("Portrait")), SegItem::text(tr("Landscape"))])
        .active(Some(if s.landscape { 1 } else { 0 }))
        .on_pick(move |i: usize, _w: &mut Window, _cx: &mut App| {
            if !rotating {
                return;
            }
            cmds.push(Cmd { id: "proto.settings", params: Some(json!({ "rotation": if i == 1 { "landscape" } else { "portrait" } })) });
            cmds.wake();
        });
    let orientation = orientation.build();
    third.push(orientation.into_any_element());
    sec = sec.child(field_row(third, Vec::<IconButton>::new()));

    // ── 第四栏：预览背景色（固定设置项），通用 hex 输入 ──
    let bg = &s.background;
    let color = figcraft_color::Color::from_hex_or_name(bg).unwrap_or(figcraft_color::Color::from_rgb8_const(0x1E, 0x1E, 0x1E));
    let dot = swatch(figcraft_color::Paint::solid(color), 16.0);
    let hex = f.row_field(
        _cx,
        slot(_n),
        bg,
        RowEditCtx {
            cmd: "proto.settings",
            params: json!({}),
            key: "background",
            scale: 1.0,
            string_value: true,
            map: None,
            paint: None,
            nest: None,
            nest_key: None,
            paint_color: None,
        },
    );
    let line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full().child(dot).child(hex);
    sec = sec.child(field_row_full(line));
    sec
}

fn num(v: f64) -> String {
    if (v.fract()).abs() < 1e-9 { format!("{}", v as i64) } else { format!("{v}") }
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Flow starting point（有选中、且是顶层 frame）
// ───────────────────────────────────────────────────────────────────────────────────────────────

fn flow_point_section(f: &RightFields, mine: Option<&FlowRow>, cx: &mut App, n: &mut usize) -> PanelSection {
    let is_start = mine.is_some();
    let mut buttons: Vec<IconButton> = Vec::new();
    if !is_start {
        // egui：还没成为起点时给「Preview this frame」+「Add starting point」。
        buttons.push(icon_cmd(Icon::Eye, "proto.preview", json!({}), f, false));
        buttons.push(icon_cmd(Icon::Plus, "proto.flow.add", json!({}), f, false));
    }
    let mut sec = PanelSection::new(tr("Flow starting point"))
        .variant(if is_start { SectionHeaderVariant::Normal } else { SectionHeaderVariant::Dim })
        .header_trail(icon_row(buttons));
    if let Some(flow) = mine {
        sec = sec.child(field_row_full(flow_line(f, flow, cx, n)));
        // 描述输入框（egui `flow_section` 第二行）。
        let desc = f.row_field(
            cx,
            slot(n),
            &flow.description,
            RowEditCtx {
                cmd: "proto.flow.rename",
                params: json!({ "start": flow.start }),
                key: "description",
                scale: 1.0,
                string_value: true,
                paint: None,
                nest: None,
                nest_key: None,
                map: None,
                paint_color: None,
            },
        );
        sec = sec.child(field_row_full(desc));
    }
    sec
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Interactions
// ───────────────────────────────────────────────────────────────────────────────────────────────

fn interactions_section(f: &RightFields, p: &ProtoData) -> PanelSection {
    let Some(list) = p.interactions.as_ref() else {
        // 多选且交互不一致（egui `mixed_interactions`）：「+」把全部换成一条点击导航，
        // 「Remove all」清空。
        let replace = json!([
            { "command": "proto.interaction.clear", "params": {} },
            { "command": "proto.interaction.add", "params": { "trigger": { "type": "onClick" }, "action": default_navigate(p) } },
        ]);
        let plus = icon_cmd(Icon::Plus, "command.batch", json!({ "label": "Replace interactions", "commands": replace }), f, false);
        let mut sec = PanelSection::new(tr("Interactions")).header_trail(icon_row(vec![plus]));
        sec = sec.child(field_row_full(note(tr("Mixed interactions: click + to replace them"))));
        let remove = icon_cmd(Icon::Minus, "proto.interaction.clear", json!({}), f, false);
        return sec.child(field_row(vec![plain_label(tr("Remove all"))], vec![remove]));
    };
    let empty = list.is_empty();
    // 「+」= 加一条默认点击导航（egui `action_of("Navigate to")`，没有别的 frame 时退回 Back）。
    // 加完**立刻打开**这条交互的详情浮窗（egui `interactions` 里 `proto.interaction.add`
    // 之后紧跟 `open_editor(ctx, Some((id, index)))`；否则新建后什么设置都看不到）。
    let add_params = json!({ "trigger": { "type": "onClick" }, "action": default_navigate(p) });
    let plus = {
        let cmds = f.cmds.clone();
        let open = f.proto_open.clone();
        let node = p.node;
        let new_index = list.len();
        IconButton::new(Icon::Plus).size(24.0).on_click(move |_e, _w, _cx| {
            cmds.push(Cmd { id: "proto.interaction.add", params: Some(add_params.clone()) });
            if let Some(node) = node {
                // 新交互追加在末尾；先把它记成展开项，下一帧文档更新后浮窗正好指向它。
                *open.borrow_mut() = Some((node, new_index));
                ANCHOR_Y.with(|a| a.set(0.0));
            }
            cmds.wake();
        })
    };
    let mut sec = PanelSection::new(tr("Interactions"))
        .variant(if empty { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal })
        .header_trail(icon_row(vec![plus]));
    if empty {
        return sec.child(field_row_full(note("Add an interaction to make this layer react.")));
    }
    for (i, it) in list.iter().enumerate() {
        let remove = icon_cmd(Icon::Minus, "proto.interaction.remove", json!({ "index": i }), f, false);
        sec = sec.child(interaction_line(f, it, i, p.node, remove));
    }
    sec
}

/// 交互详情浮窗（egui `proto_panel.rs::editor_popover`）：浮在右栏左侧的**通用弹窗控件**
/// （`widgets::popover::Popover`），不是就地展开。`at` 的顶边优先用被点那一行的窗口 y。
pub(crate) fn editor_popover(f: &RightFields, d: &PanelData, cx: &mut App, vp: Size<Pixels>, right_w: f32) -> Vec<AnyElement> {
    let p = &d.proto;
    let Some((node, k)) = f.proto_editor() else {
        return Vec::new();
    };
    if p.node != Some(node) {
        return Vec::new();
    }
    let Some(it) = p.interactions.as_ref().and_then(|l| l.get(k)) else {
        return Vec::new();
    };
    // 浮窗和右栏正文同一帧画，槽位从 `PROTO_EDITOR_SLOT` 起，绝不与正文撞同一个编辑器实体。
    let mut n = PROTO_EDITOR_SLOT;
    let rows = editor_rows(f, it, k, &p.frames, cx, &mut n);
    let y = ANCHOR_Y.with(|a| a.get());
    let anchor = point(px(f32::from(vp.width) - right_w - 8.0), px(if y > 8.0 { y } else { 120.0 }));
    // 外壳先建出来：正文的可用高度要从卡片**真实落点**算（egui `max_h = screen.max.y - pos.y - 64`）。
    // 拿 `vp.height - 80` 算等于假设卡片贴着视口顶；卡片开在被点那一行，顶边可能低得多，
    // 长动作列表就会伸到视口外、底部控件点不到。
    let shell = popover_window(popover::Open::Anchor(anchor)).width(280.0).height(420.0).pad(12.0);
    let origin = shell.origin(vp);
    let avail = (f32::from(vp.height) - f32::from(origin.y) - 16.0 - popover::HEADER_H - 24.0).max(160.0);
    let mut list = popover_body();
    for row in rows {
        list = list.child(row);
    }
    let body = div().id("proto-editor-body").w_full().max_h(px(avail)).overflow_y_scroll().child(list);
    let dismiss: std::rc::Rc<dyn Fn(&mut gpui::Window, &mut App)> = {
        let open = f.proto_open.clone();
        let cmds = f.cmds.clone();
        std::rc::Rc::new(move |_, _| {
            *open.borrow_mut() = None;
            cmds.wake();
        })
    };
    let (_, card) = shell.title(title_text(tr("Interaction"))).body(body).show(vp, dismiss);
    vec![card]
}

/// 浮窗正文的池槽位：从 `PROTO_EDITOR_SLOT` 往后用，上限是整个池的末尾
/// （`sections::slot` 是右栏正文用的，只许 0..`PROTO_EDITOR_SLOT`）。
fn editor_slot(n: &mut usize) -> usize {
    let s = (*n).min(crate::panels::ROW_EDITORS - 1);
    *n += 1;
    s
}

/// 交互详情浮窗的正文：触发器 / 延迟 / 动作列表 / 转场 / 缓动 / 时长
/// （egui `proto_panel.rs::editor_popover` 的 body）。
fn editor_rows(
    f: &RightFields,
    it: &InteractionRow,
    i: usize,
    frames: &[(figcraft_doc::NodeId, String)],
    cx: &mut App,
    n: &mut usize,
) -> Vec<crate::widgets::FieldRow> {
    let mut out = Vec::new();
    // ── Trigger ──
    out.push(sublabel_row(tr("Trigger")));
    let triggers: Vec<(&'static str, &'static str, Value)> = vec![
        (tr("On click"), "onClick", json!({ "type": "onClick" })),
        (tr("While hovering"), "onHover", json!({ "type": "onHover" })),
        (tr("While pressing"), "onPress", json!({ "type": "onPress" })),
        (tr("On drag"), "onDrag", json!({ "type": "onDrag" })),
        (tr("Mouse enter"), "mouseEnter", json!({ "type": "mouseEnter" })),
        (tr("Mouse leave"), "mouseLeave", json!({ "type": "mouseLeave" })),
        (tr("Mouse down"), "mouseDown", json!({ "type": "mouseDown" })),
        (tr("Mouse up"), "mouseUp", json!({ "type": "mouseUp" })),
        (tr("After delay"), "afterDelay", json!({ "type": "afterDelay", "ms": 300 })),
        ("Key / Gamepad", "keyPress", json!({ "type": "keyPress", "key": tr("Space") })),
    ];
    let rows: Vec<(String, &'static str, Value)> =
        triggers.iter().map(|(label, _, v)| (label.to_string(), "proto.interaction.set", json!({ "index": i, "trigger": v }))).collect();
    out.push(field_row_full(pick_dropdown(f, it.trigger.clone(), 0.0, rows)));

    // ── After delay 的毫秒 / 鼠标手势后的延迟 ──
    let needs_ms = it.trigger_kind == "afterDelay";
    let gesture_delay = matches!(it.trigger_kind.as_str(), "mouseEnter" | "mouseLeave" | "mouseDown" | "mouseUp");
    if needs_ms {
        let field = f.row_field(
            cx,
            editor_slot(n),
            &format!("{}", ms_of(&it.trigger_json)),
            RowEditCtx {
                cmd: "proto.interaction.set",
                params: json!({ "index": i, "trigger": it.trigger_json.clone() }),
                key: "ms",
                scale: 1.0,
                string_value: false,
                paint: None,
                nest: Some("trigger"),
                nest_key: None,
                map: None,
                paint_color: None,
            },
        );
        out.push(sublabel_row(tr("Delay")));
        out.push(field_row_full(field));
    } else if gesture_delay {
        let field = f.row_field(
            cx,
            editor_slot(n),
            &format!("{}", it.delay_ms),
            RowEditCtx {
                cmd: "proto.interaction.set",
                params: json!({ "index": i }),
                key: "delayMs",
                scale: 1.0,
                string_value: false,
                paint: None,
                nest: None,
                nest_key: None,
                map: None,
                paint_color: None,
            },
        );
        out.push(sublabel_row(tr("Delay")));
        out.push(field_row_full(field));
    }

    // ── Actions：每条一行（摘要 + 上移 / 下移 / 删除），行下按类型给字段；末尾是类型选择 ──
    out.push(sublabel_row(if it.actions.len() > 1 { tr("Actions") } else { tr("Action") }));
    let first_dest = it.actions.iter().find_map(|(_, a)| a.get("dest").cloned());
    for (k, (label, action)) in it.actions.iter().enumerate() {
        let mut trail: Vec<IconButton> = Vec::new();
        if k > 0 {
            trail.push(icon_cmd(Icon::ChevronUp, "proto.action.move", json!({ "index": i, "path": [k], "to": k - 1 }), f, false));
        }
        if k + 1 < it.actions.len() {
            trail.push(icon_cmd(Icon::ChevronDown, "proto.action.move", json!({ "index": i, "path": [k], "to": k + 1 }), f, false));
        }
        // 一条交互至留一个动作（引擎规则）：只剩一条时不给删除。
        if it.actions.len() > 1 {
            trail.push(icon_cmd(Icon::Minus, "proto.action.remove", json!({ "index": i, "path": [k] }), f, false));
        }
        let line = div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(tokens().text)).child(SharedString::from(label.clone()));
        out.push(field_row_full_trailing(line, trail));

        match action.get("type").and_then(Value::as_str).unwrap_or_default() {
            kind @ ("navigate" | "changeTo" | "scrollTo" | "openOverlay" | "swapOverlay") => {
                let rows = frame_target_rows(frames, action, i, &[k]);
                out.push(field_row_full(pick_dropdown(f, dest_label(frames, action), 0.0, rows)));
                if kind == "openOverlay" {
                    out.extend(overlay_rows(f, action, i, k));
                }
            }
            "openUrl" => {
                let url = action.get("url").and_then(Value::as_str).unwrap_or_default().to_string();
                let field = f.row_field(
                    cx,
                    editor_slot(n),
                    &url,
                    RowEditCtx {
                        cmd: "proto.action.set",
                        params: json!({ "index": i, "path": [k], "action": action.clone() }),
                        key: "url",
                        scale: 1.0,
                        string_value: true,
                        paint: None,
                        nest: Some("action"),
                        nest_key: None,
                        map: None,
                        paint_color: None,
                    },
                );
                out.push(sublabel_row(tr("URL")));
                out.push(field_row_full(field));
            }
            "conditional" => out.extend(conditional_rows(f, action, i, k, cx, n)),
            _ => {}
        }
    }
    // 新增动作：类型选择（eGUI `proto_editor.rs` 的 action kind 菜单）。
    out.push(field_row_full(pick_dropdown(f, tr("Add action"), 0.0, action_kind_rows(first_dest, frames))));

    // ── Transition / Easing / Duration ──
    out.push(sublabel_row(tr("Transition")));
    let transitions: [(&'static str, Value); 8] = [
        (tr("Instant"), json!({ "type": "instant" })),
        (tr("Dissolve"), json!({ "type": "dissolve" })),
        (tr("Smart animate"), json!({ "type": "smartAnimate" })),
        (tr("Move in"), json!({ "type": "moveIn", "direction": "left" })),
        (tr("Move out"), json!({ "type": "moveOut", "direction": "left" })),
        (tr("Push"), json!({ "type": "push", "direction": "left" })),
        (tr("Slide in"), json!({ "type": "slideIn", "direction": "left" })),
        (tr("Slide out"), json!({ "type": "slideOut", "direction": "left" })),
    ];
    let trows: Vec<(String, &'static str, Value)> =
        transitions.iter().map(|(l, v)| (l.to_string(), "proto.interaction.set", json!({ "index": i, "transition": v }))).collect();
    out.push(field_row_full(pick_dropdown(f, it.transition.clone(), 0.0, trows)));

    out.push(sublabel_row(tr("Easing")));
    let easings = [
        tr("Linear"),
        "Ease in",
        "Ease out",
        "Ease in and out",
        "Ease in back",
        "Ease out back",
        "Ease in and out back",
        "Gentle",
        "Quick",
        "Bouncy",
        "Slow",
    ];
    let erows: Vec<(String, &'static str, Value)> = easings
        .iter()
        .zip([
            json!("linear"),
            json!("easeIn"),
            json!("easeOut"),
            json!("easeInOut"),
            json!("easeInBack"),
            json!("easeOutBack"),
            json!("easeInOutBack"),
            json!("gentle"),
            json!("quick"),
            json!("bouncy"),
            json!("slow"),
        ])
        .map(|(l, v)| (l.to_string(), "proto.interaction.set", json!({ "index": i, "easing": v })))
        .collect();
    out.push(field_row_full(pick_dropdown(f, it.easing.clone(), 0.0, erows)));

    out.push(sublabel_row(tr("Duration")));
    let dur = f.row_field(
        cx,
        editor_slot(n),
        &format!("{}", it.duration_ms),
        RowEditCtx {
            cmd: "proto.interaction.set",
            params: json!({ "index": i }),
            key: "durationMs",
            scale: 1.0,
            string_value: false,
            paint: None,
            nest: None,
            nest_key: None,
            map: None,
            paint_color: None,
        },
    );
    out.push(field_row_full(dur));
    out
}

/// `afterDelay` trigger 里的毫秒。
fn ms_of(trigger: &Value) -> u64 {
    trigger.get("ms").and_then(Value::as_u64).unwrap_or(300)
}

/// 新增动作的类型清单（eGUI `proto_editor.rs` 的 kind 菜单）；默认目标取同一条交互已有的 dest。
fn action_kind_rows(dest: Option<Value>, frames: &[(figcraft_doc::NodeId, String)]) -> Vec<(String, &'static str, Value)> {
    let d = || dest.clone().or_else(|| frames.first().map(|(id, _)| json!(id.0))).unwrap_or_else(|| json!(0));
    let kinds: Vec<(&'static str, Value)> = vec![
        (tr("Navigate to"), json!({ "type": "navigate", "dest": d() })),
        (tr("Change to"), json!({ "type": "changeTo", "dest": d() })),
        (tr("Back"), json!({ "type": "back" })),
        (tr("Scroll to"), json!({ "type": "scrollTo", "dest": d() })),
        (tr("Open link"), json!({ "type": "openUrl", "url": "https://" })),
        (tr("Open overlay"), json!({ "type": "openOverlay", "dest": d(), "position": "center" })),
        (tr("Swap overlay"), json!({ "type": "swapOverlay", "dest": d() })),
        (tr("Close overlay"), json!({ "type": "closeOverlay" })),
        (tr("Conditional"), json!({ "type": "conditional", "blocks": [{ "expression": "1 == 1", "actions": [] }] })),
    ];
    kinds.into_iter().map(|(l, v)| (l.to_string(), "proto.action.add", json!({ "action": v }))).collect()
}

/// 目标（`dest`）下拉：本页顶层 frame。
fn frame_target_rows(frames: &[(figcraft_doc::NodeId, String)], action: &Value, i: usize, path: &[usize]) -> Vec<(String, &'static str, Value)> {
    frames
        .iter()
        .map(|(id, name)| {
            let mut a = action.clone();
            a["dest"] = json!(id.0);
            (name.clone(), "proto.action.set", json!({ "index": i, "path": path, "action": a }))
        })
        .collect()
}

/// 当前 `dest` 对应的 frame 名，没有就用提示文案。
fn dest_label(frames: &[(figcraft_doc::NodeId, String)], action: &Value) -> String {
    let dest = action.get("dest").and_then(Value::as_u64);
    frames.iter().find(|(id, _)| Some(id.0) == dest).map(|(_, n)| n.clone()).unwrap_or_else(|| tr("Choose target").to_string())
}

/// `openOverlay` 的 position / anchor / closeOnOutside（eGUI `proto_editor.rs`）。
fn overlay_rows(f: &RightFields, action: &Value, i: usize, k: usize) -> Vec<crate::widgets::FieldRow> {
    let mut out = Vec::new();
    let with = |key: &str, v: Value| {
        let mut a = action.clone();
        a[key] = v;
        json!({ "index": i, "path": [k], "action": a })
    };
    // Position：camelCase 字符串（`overlayPosition` 的 serde 形）。
    let positions = [
        (tr("Center"), "center"),
        (tr("Top left"), "topLeft"),
        (tr("Top center"), "topCenter"),
        (tr("Top right"), "topRight"),
        (tr("Bottom left"), "bottomLeft"),
        (tr("Bottom center"), "bottomCenter"),
        (tr("Bottom right"), "bottomRight"),
        (tr("Manual"), "manual"),
    ];
    let cur = action.get("position").and_then(Value::as_str).unwrap_or("center");
    let prow: Vec<(String, &'static str, Value)> =
        positions.iter().map(|(l, v)| (l.to_string(), "proto.action.set", with("position", json!(v)))).collect();
    let cur_label = positions.iter().find(|(_, v)| *v == cur).map_or(tr("Center").to_string(), |(l, _)| l.to_string());
    out.push(sublabel_row(tr("Position")));
    out.push(field_row_full(pick_dropdown(f, cur_label, 0.0, prow)));
    // Anchor：屏幕 / 触发层内 / 触发层外。
    let anchors = [(tr("Screen"), "screen"), (tr("Trigger inside"), "triggerInside"), (tr("Trigger outside"), "triggerOutside")];
    let acur = action.get("anchor").and_then(Value::as_str).unwrap_or("screen");
    let arow: Vec<(String, &'static str, Value)> =
        anchors.iter().map(|(l, v)| (l.to_string(), "proto.action.set", with("anchor", json!(v)))).collect();
    let a_label = anchors.iter().find(|(_, v)| *v == acur).map_or(tr("Screen").to_string(), |(l, _)| l.to_string());
    out.push(sublabel_row(tr("Anchor")));
    out.push(field_row_full(pick_dropdown(f, a_label, 0.0, arow)));
    // Close on click outside：勾选即写 closeOnOutside。
    let on = action.get("closeOnOutside").and_then(Value::as_bool).unwrap_or(true);
    let toggle = icon_cmd(Icon::Check, "proto.action.set", with("closeOnOutside", json!(!on)), f, on);
    out.push(field_row_full_trailing(plain_label(tr("Close on click outside")), vec![toggle]));
    out
}

/// 条件动作：每个 if / else if / else 分支（条件表达式 + 块内动作 + 删除），以及新增分支。
fn conditional_rows(f: &RightFields, action: &Value, i: usize, k: usize, cx: &mut App, n: &mut usize) -> Vec<crate::widgets::FieldRow> {
    use figcraft_doc::Variables;
    use figcraft_doc::proto::Expr;
    let mut out = Vec::new();
    let blocks: Vec<Value> = action.get("blocks").and_then(Value::as_array).cloned().unwrap_or_default();
    let has_else = blocks.iter().any(|b| b.get("condition").is_none_or(Value::is_null));
    for (b, block) in blocks.iter().enumerate() {
        let cond = block.get("condition").filter(|c| !c.is_null());
        let label = match cond {
            None => tr("Else"),
            Some(_) if b == 0 => tr("If"),
            Some(_) => tr("Else if"),
        };
        let remove = icon_cmd(Icon::Minus, "proto.conditional.removeBlock", json!({ "index": i, "path": [k], "block": b }), f, false);
        out.push(field_row_full_trailing(plain_label(label), vec![remove]));
        if let Some(c) = cond {
            // 条件以文本编辑：`Expr` 反序列化后显示，提交走 `expression`。
            let text = serde_json::from_value::<Expr>(c.clone()).map(|e| e.display(&Variables::default())).unwrap_or_default();
            let field = f.row_field(
                cx,
                editor_slot(n),
                &text,
                RowEditCtx {
                    cmd: "proto.conditional.setCondition",
                    params: json!({ "index": i, "path": [k], "block": b }),
                    key: "expression",
                    scale: 1.0,
                    string_value: true,
                    paint: None,
                    nest: None,
                    nest_key: None,
                    map: None,
                    paint_color: None,
                },
            );
            out.push(field_row_full(field));
        }
        // 块内动作：摘要 + 删除（编辑条件块内部的字段放在后续迭代）。
        let acts = block.get("actions").and_then(Value::as_array).cloned().unwrap_or_default();
        if acts.is_empty() {
            out.push(field_row_full(plain_label(tr("No actions"))));
        }
        for (j, a) in acts.iter().enumerate() {
            let text = a.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
            let remove = icon_cmd(Icon::Minus, "proto.action.remove", json!({ "index": i, "path": [k, b, j] }), f, false);
            let line = div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(tokens().text_secondary)).child(SharedString::from(text));
            out.push(field_row_full_trailing(line, vec![remove]));
        }
    }
    out.push(field_row_full_trailing(
        plain_label(tr("Add else if")),
        vec![icon_cmd(Icon::Plus, "proto.conditional.addBlock", json!({ "index": i, "path": [k] }), f, false)],
    ));
    if !has_else {
        out.push(field_row_full_trailing(
            plain_label(tr("Add else")),
            vec![icon_cmd(Icon::Plus, "proto.conditional.addBlock", json!({ "index": i, "path": [k], "else": true }), f, false)],
        ));
    }
    out
}

/// 一条交互行：连接字形 + 触发器（半粗）/ 动作摘要（次要）+ 删除；点行展开编辑器。
fn interaction_line(
    f: &RightFields,
    it: &InteractionRow,
    i: usize,
    node: Option<figcraft_doc::NodeId>,
    remove: IconButton,
) -> crate::widgets::FieldRow {
    let t = tokens();
    let open = (*f.proto_open.borrow()).is_some_and(|(n, k)| Some(n) == node && k == i);
    let glyph = IconButton::new(Icon::LinkOn).size(20.0);
    // 点文字区域展开 / 收起详情浮窗（egui 是点开出浮窗）；顺便把该行的窗口 y 记下来，
    // 让浮窗顶边贴着这一行出现，而不是每次固定位置。
    let toggler = f.proto_open.clone();
    let cmds = f.cmds.clone();
    let text = div()
        .id(SharedString::from(format!("proto-it-{i}")))
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .cursor_pointer()
        .rounded(pt(5.0))
        .when(open, |d| d.bg(rgba(t.selected_row)))
        .hover(|s| s.bg(rgba(t.hover)))
        .px(pt(4.0))
        .py(pt(2.0))
        .on_mouse_down(gpui::MouseButton::Left, move |e, _w, _cx| {
            ANCHOR_Y.with(|a| a.set(f32::from(e.position.y)));
            {
                let mut cur = toggler.borrow_mut();
                let same = node.is_some() && cur.is_some_and(|(n, k)| Some(n) == node && k == i);
                *cur = if same { None } else { node.map(|n| (n, i)) };
            }
            cmds.wake();
        })
        .child(
            div()
                .text_size(pt(11.0))
                .font_weight(crate::panels::UI_FONT_REGULAR)
                .text_color(rgba(t.text))
                .child(SharedString::from(it.trigger.clone())),
        )
        .child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(it.summary.clone())));
    field_row(vec![glyph.into_any_element(), text.into_any_element()], vec![remove])
}

/// 新交互的默认动作：跳到本 frame 之外的第一个 frame，没有就 Back（egui `action_of`）。
fn default_navigate(p: &ProtoData) -> Value {
    let own = p.frame_flow.as_ref().and_then(|_| p.frames.first().map(|(id, _)| *id));
    let dest = p.frames.iter().map(|(id, _)| *id).find(|f| Some(*f) != own).or_else(|| p.frames.first().map(|(id, _)| *id));
    match dest {
        Some(id) => json!({ "type": "navigate", "dest": id }),
        None => json!({ "type": "back" }),
    }
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Scroll behavior
// ───────────────────────────────────────────────────────────────────────────────────────────────

fn scroll_section(f: &RightFields, s: &ScrollData) -> PanelSection {
    let mut sec = PanelSection::new(tr("Scroll behavior"));
    if s.in_frame {
        let rows: Vec<(String, &'static str, Value)> = [(tr("Scroll with parent"), "scrolls"), (tr("Fixed"), "fixed"), (tr("Sticky"), "sticky")]
            .into_iter()
            .map(|(l, v)| (l.to_string(), "node.set", json!({ "props": { "scrollPosition": v } })))
            .collect();
        let label = match s.position {
            "fixed" => tr("Fixed"),
            "sticky" => tr("Sticky"),
            _ => tr("Scroll with parent"),
        };
        sec = sec.child(field_row(vec![pick_dropdown(f, label, 0.0, rows).into_any_element()], Vec::<IconButton>::new()));
    }
    if s.is_frame {
        let rows: Vec<(String, &'static str, Value)> =
            [(tr("No scrolling"), "none"), (tr("Horizontal"), "horizontal"), (tr("Vertical"), "vertical"), (tr("Both directions"), "both")]
                .into_iter()
                .map(|(l, v)| (l.to_string(), "node.set", json!({ "props": { "overflow": v } })))
                .collect();
        let label = match s.overflow {
            "horizontal" => tr("Horizontal"),
            "vertical" => tr("Vertical"),
            "both" => tr("Both directions"),
            _ => tr("No scrolling"),
        };
        sec = sec.child(field_row(vec![pick_dropdown(f, label, 0.0, rows).into_any_element()], Vec::<IconButton>::new()));
    }
    sec
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// 小工具
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// 一行次要提示文字（egui `widgets::note`）。
fn note(text: &'static str) -> Div {
    let t = tokens();
    div().w_full().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(text))
}

/// 一行普通文字（占满字段列，`Remove all` / `Responsive` 用）。
fn plain_label(text: &'static str) -> Div {
    let t = tokens();
    div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(text))
}
