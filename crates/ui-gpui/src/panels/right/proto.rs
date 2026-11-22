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
use super::sections::{icon_cmd, icon_row, pick_dropdown, slot};
use crate::panels::{Cmd, PROTO_EDITOR_SLOT, RightFields, RowEditCtx};
use crate::widgets::popover::{self, popover_body, popover_window, title_text};
use crate::widgets::{
    Icon, IconButton, PanelSection, SectionHeaderVariant, field_row, field_row_full, field_row_full_trailing, pt, rgba, sublabel_row, sublabel_row2,
    swatch, tokens,
};
use figcraft_i18n::tr;
use gpui::{AnyElement, App, Div, IntoElement, Pixels, SharedString, Size, div, point, prelude::*, px};
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
        // ── 无选中：flows + settings（egui `prototype()` 前半段）──
        body = body.child(flows_section(f, p, cx, n)).child(crate::panels::sep()).child(settings_section(f, p, cx, n));
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

fn settings_section(f: &RightFields, p: &ProtoData, cx: &mut App, n: &mut usize) -> PanelSection {
    let s = &p.settings;
    let preview = icon_cmd(Icon::Eye, "proto.preview", json!({}), f, false);
    let mut sec = PanelSection::new(tr("Prototype settings")).header_trail(icon_row(vec![preview]));

    // ── Device + 旋转（只有会转的设备才给旋转按钮）──
    sec = sec.child(sublabel_row(tr("Device")));
    let devices: Vec<(String, &'static str, Value)> = s.devices.iter().map(|d| (d.to_string(), "proto.settings", json!({ "device": d }))).collect();
    let device_dd = pick_dropdown(f, s.device.clone(), 0.0, devices);
    let mut trail: Vec<IconButton> = Vec::new();
    if s.rotates {
        let next = if s.landscape { "portrait" } else { "landscape" };
        trail.push(icon_cmd(Icon::Rotate, "proto.settings", json!({ "rotation": next }), f, s.landscape));
    }
    sec = sec.child(field_row(vec![device_dd.into_any_element()], trail));

    // ── 机身色 ──
    if !s.colors.is_empty() {
        sec = sec.child(sublabel_row(tr("Color")));
        let rows: Vec<(String, &'static str, Value)> =
            s.colors.iter().map(|(name, _)| (name.to_string(), "proto.settings", json!({ "color": name }))).collect();
        let rgb = s.colors.iter().find(|(name, _)| *name == s.color).map(|(_, c)| *c).unwrap_or([0, 0, 0]);
        let dot = swatch(figcraft_color::Paint::solid(figcraft_color::Color::from_rgb8_const(rgb[0], rgb[1], rgb[2])), 16.0);
        let line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full().child(dot).child(pick_dropdown(f, s.color.clone(), 0.0, rows));
        sec = sec.child(field_row_full(line));
    }

    // ── 自定义尺寸（W/H + 交换）──
    if let Some((w, h)) = s.custom {
        sec = sec.child(sublabel_row(tr("Size")));
        let wf = f.row_field(
            cx,
            slot(n),
            &num(w),
            RowEditCtx {
                cmd: "proto.settings",
                params: json!({}),
                key: "width",
                scale: 1.0,
                string_value: false,
                paint: None,
                nest: None,
                nest_key: None,
                paint_color: None,
            },
        );
        let hf = f.row_field(
            cx,
            slot(n),
            &num(h),
            RowEditCtx {
                cmd: "proto.settings",
                params: json!({}),
                key: "height",
                scale: 1.0,
                string_value: false,
                paint: None,
                nest: None,
                nest_key: None,
                paint_color: None,
            },
        );
        let swap = icon_cmd(Icon::Rotate, "proto.settings", json!({ "width": h, "height": w }), f, false);
        sec = sec.child(field_row(vec![wf.into_any_element(), hf.into_any_element()], vec![swap]));
    }

    // ── Responsive ──
    let resp = icon_cmd(Icon::Check, "proto.settings", json!({ "responsive": !s.responsive }), f, s.responsive);
    sec = sec.child(field_row(vec![plain_label(tr("Responsive"))], vec![resp]));

    // ── Background（色块 + hex 输入）──
    sec = sec.child(sublabel_row(tr("Background")));
    let bg = &s.background;
    let color = figcraft_color::Color::from_hex_or_name(bg).unwrap_or(figcraft_color::Color::from_rgb8_const(0x1E, 0x1E, 0x1E));
    let hex = f.row_field(
        cx,
        slot(n),
        bg,
        RowEditCtx {
            cmd: "proto.settings",
            params: json!({}),
            key: "background",
            scale: 1.0,
            string_value: true,
            paint: None,
            nest: None,
            nest_key: None,
            paint_color: None,
        },
    );
    let dot = swatch(figcraft_color::Paint::solid(color), 16.0);
    let line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full().child(dot).child(hex);
    sec.child(field_row_full(line))
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
    let rows = editor_rows(f, it, k, cx, &mut n);
    let avail = (f32::from(vp.height) - 16.0 - popover::HEADER_H - 24.0).max(160.0);
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
    let y = ANCHOR_Y.with(|a| a.get());
    let anchor = point(px(f32::from(vp.width) - right_w - 8.0), px(if y > 8.0 { y } else { 120.0 }));
    let (_, card) = popover_window(popover::Open::Anchor(anchor))
        .width(280.0)
        .height(420.0)
        .pad(12.0)
        .title(title_text(tr("Interaction")))
        .body(body)
        .show(vp, dismiss);
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
fn editor_rows(f: &RightFields, it: &InteractionRow, i: usize, cx: &mut App, n: &mut usize) -> Vec<crate::widgets::FieldRow> {
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
                paint_color: None,
            },
        );
        out.push(sublabel_row(tr("Delay")));
        out.push(field_row_full(field));
    }

    // ── Actions：每条一行（摘要 + 删除），另有「+ Navigate to」加一条 ──
    out.push(sublabel_row(if it.actions.len() > 1 { tr("Actions") } else { tr("Action") }));
    for (k, (label, action)) in it.actions.iter().enumerate() {
        // 删掉第 k 条：把剩下的整组写回（`proto.interaction.set {actions}`）。
        let mut rest: Vec<Value> = it.actions.iter().map(|(_, a)| a.clone()).collect();
        rest.remove(k);
        let remove = icon_cmd(Icon::Minus, "proto.interaction.set", json!({ "index": i, "actions": rest }), f, false);
        let line = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(6.0))
            .w_full()
            .child(div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(tokens().text)).child(SharedString::from(label.clone())));
        out.push(field_row_full_trailing(line, vec![remove]));
    }
    // 追加一条导航（目标 = 本 frame 之外的第一个 frame）。
    let add_params = match it.actions.first().and_then(|(_, a)| a.get("dest")).cloned() {
        Some(dest) => json!({ "index": i, "action": { "type": "navigate", "dest": dest } }),
        None => json!({ "index": i, "action": { "type": "back" } }),
    };
    let add = icon_cmd(Icon::Plus, "proto.interaction.set", add_params, f, false);
    out.push(field_row_full_trailing(plain_label(tr("Add action")), vec![add]));

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
