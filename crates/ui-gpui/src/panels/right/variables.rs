//! 变量编辑器（`ui-egui/src/dialogs/variables_panel.rs` 的 GPUI 移植）。
//!
//! 范围（用户 2026-11-06）：整个面板 —— 顶部 header、左侧 sidebar（集合 / 分组）、右侧表格、
//! 底栏、Floating / Full 两种呈现、以及通用拖拽件 `crate::drag` 接进来的排序。
//!
//! 与右栏其它面板一样，这是**只读快照 + 命令队列**：`CanvasView::panel_data()` 每帧把
//! `Document` 读成 [`VariablesData`]，窗口持有它；所有编辑先乐观更新本地快照、再推 `Cmd`，
//! 由 `sync_panel` 统一送到引擎。窗口内部状态（选中、搜索、菜单、拖拽、输入框）是 retained 的。
//!
//! 这是**表格 / 弹窗**，不是右栏 `PanelSection` 行，所以不受「统一网格 / 禁止手算列宽」约束；
//! 列宽来自 egui 的 `NAME_W = 240` / `ADD_W = 40`，mode 列等分剩余宽度。

use crate::canvas::geom::Color32;
use crate::drag::{self, Config as DragCfg, Row as DragRow, Target as DragTarget};
use crate::panels::right::virtual_list::{VirtualListScrollHandle, v_virtual_list};
use crate::panels::{Cmd, Cmds};
use crate::widgets::icon::paint as paint_icon;
use crate::widgets::{
    Checkbox, Icon, IconButton, PopoverOpen, Slot, Surface, TextField, number_field, popover_window, pt, rgba, search_field, text_field, title_text,
    tokens,
};
use figcraft_i18n::tr;
use figcraft_i18n::tr_noop;
use gpui::{
    AnyElement, App, Bounds, CursorStyle, Div, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollHandle,
    SharedString, Size, Stateful, Window, div, point, prelude::*, px, size,
};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::Instant;

/// 整卡拖动的按下回调（由通用窗口传入；只绑在头部左侧，不绑整张卡片）。
pub type DragStart = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;

// ── egui `variables_panel.rs:25-50` 的常量 ────────────────────────────────────────────────
pub const SIDEBAR_W: f32 = 264.0;
pub const NAME_W: f32 = 240.0;
pub const ADD_W: f32 = 40.0;
pub const ROW_H: f32 = 40.0;
pub const HEAD_ROW_H: f32 = 40.0;
pub const GROUP_H: f32 = 40.0;
pub const GROUP_GAP: f32 = 16.0;
pub const BOTTOM_H: f32 = 40.0;
pub const HEADER_H: f32 = 40.0;
pub const SIDE_ROW_H: f32 = 32.0;
pub const FLOAT_W: f32 = 880.0;
pub const FLOAT_H: f32 = 480.0;
pub const WINDOW_W: f32 = FLOAT_W;
pub const WINDOW_H: f32 = FLOAT_H;
/// egui `Geom::MIN_W/MIN_H`：浮动面板 resize 的下限。
pub const MIN_W: f32 = 480.0;
pub const MIN_H: f32 = 240.0;
/// 列宽 / 侧栏宽的拖动边界（egui `SIDEBAR_MIN/MAX`、`NAME_MIN/MAX`、`MODE_MIN/MAX`）。
pub const SIDEBAR_MIN: f32 = 180.0;
pub const SIDEBAR_MAX: f32 = 420.0;
// 用户 2026-11-17：列最小宽度太小，拖到极限后列名/值完全看不清。
// Name 至少容得下缩进后的完整名称，mode 列至少容得下「#123456」这类常见值。
pub const NAME_MIN: f32 = 180.0;
pub const NAME_MAX: f32 = 420.0;
pub const MODE_MIN: f32 = 140.0;
pub const MODE_MAX: f32 = 420.0;
pub const MODE_W: f32 = 200.0;
/// 水平滚动条的条带高度（egui `HSCROLL_H`）；需要横滚时从行区高度里扣掉。
pub const HSCROLL_H: f32 = 12.0;
const SEARCH_W: f32 = 174.0;
/// 过滤按钮与搜索框拼接后的总宽（egui `SEARCH_W + FILTER_W`）。
const FILTER_W: f32 = 25.0;
const VALUE_FIELD_W: f32 = 150.0;
const TYPES: [(&str, VarKind); 4] = [
    (tr_noop("Color"), VarKind::Color),
    (tr_noop("Number"), VarKind::Number),
    (tr_noop("String"), VarKind::String),
    (tr_noop("Boolean"), VarKind::Boolean),
];

/// `figcraft_doc::VariableType` 的显示形态（egui `kind_glyph`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VarKind {
    #[default]
    Color,
    Number,
    String,
    Boolean,
}

impl VarKind {
    fn id(self) -> &'static str {
        match self {
            VarKind::Color => "color",
            VarKind::Number => "number",
            VarKind::String => "string",
            VarKind::Boolean => "boolean",
        }
    }
    fn index(self) -> usize {
        match self {
            VarKind::Color => 0,
            VarKind::Number => 1,
            VarKind::String => 2,
            VarKind::Boolean => 3,
        }
    }

    fn doc_type(self) -> figcraft_doc::VariableType {
        match self {
            VarKind::Color => figcraft_doc::VariableType::Color,
            VarKind::Number => figcraft_doc::VariableType::Number,
            VarKind::String => figcraft_doc::VariableType::String,
            VarKind::Boolean => figcraft_doc::VariableType::Boolean,
        }
    }
}

/// 一行里某个 mode 的值（egui `value_cell`）。
#[derive(Clone, Debug, PartialEq)]
pub enum VarCell {
    Empty,
    Color { color: Color32, hex: String, alpha: u8 },
    Number(String),
    Text(String),
    Bool(bool),
    Alias(String),
}

impl VarCell {
    /// 值格输入框里显示的文本。
    fn text(&self) -> String {
        match self {
            VarCell::Color { hex, .. } => hex.clone(),
            VarCell::Number(s) | VarCell::Text(s) | VarCell::Alias(s) => s.clone(),
            VarCell::Bool(b) => b.to_string(),
            VarCell::Empty => String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VarRow {
    pub id: String,
    /// 文档里的完整名（`Brand/Primary`）；详情面板改名、复制都用它，不能用短名。
    pub full_name: String,
    /// 分组内显示的短名（`rsplit('/')` 的最后一段）。
    pub name: String,
    /// 所属分组（去掉名字前缀），空串 = 未分组。
    pub group: String,
    pub kind: VarKind,
    /// 与集合 `modes` 一一对应。
    pub values: Vec<VarCell>,
    /// 每个 mode 是否为 override（扩展集合继承标记，画左侧 2pt 竖条）。
    pub overrides: Vec<bool>,
    /// 详情面板：描述、发布时隐藏、各平台的代码名、作用范围。
    pub description: String,
    pub hidden: bool,
    pub code: Vec<(String, String)>,
    pub scopes: Option<Vec<String>>,
    /// 扩展集合里继承来的变量：不可拖拽、改名、删除（egui `v.extends.is_some()`）。
    pub inherited: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModeSnap {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollectionSnap {
    pub id: String,
    pub name: String,
    pub modes: Vec<ModeSnap>,
    /// 文档顺序的全部变量（过滤 / 排序在渲染时做）。
    pub rows: Vec<VarRow>,
    /// 由变量名前缀推出的分组，按字母序。
    pub groups: Vec<String>,
    /// 来源库（`Some` = 库里的只读集合）。
    pub library: Option<String>,
    /// 扩展自哪个集合（egui `coll.extends`：值格右侧多留按钮位、显示 notice_row）。
    pub extends: Option<String>,
    /// 本地集合可编辑；库集合只读。
    pub editable: bool,
}

/// 一个已启用库（egui `View::of(lib)`）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LibrarySnap {
    pub id: String,
    pub name: String,
    pub collections: Vec<CollectionSnap>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VariablesData {
    pub collections: Vec<CollectionSnap>,
    pub libraries: Vec<LibrarySnap>,
}

fn kind_of(k: figcraft_doc::VariableType) -> VarKind {
    match k {
        figcraft_doc::VariableType::Color => VarKind::Color,
        figcraft_doc::VariableType::Number => VarKind::Number,
        figcraft_doc::VariableType::String => VarKind::String,
        figcraft_doc::VariableType::Boolean => VarKind::Boolean,
    }
}

/// Toggling one scope checkbox in the details panel: `None` means "all supported properties"
/// (every scope checked), `Some([])` means hidden from pickers. Turning one off from `None`
/// removes just that scope; checking one from an empty list starts a single-scope list.
fn toggle_scope(kind: VarKind, current: Option<&[String]>, scope: &str, on: bool) -> Vec<String> {
    let supported: Vec<&str> = figcraft_doc::styles::scopes_for(kind.doc_type()).iter().map(|(id, _)| *id).collect();
    let mut list: Vec<String> = match current {
        Some(s) => s.to_vec(),
        None => supported.iter().map(|s| s.to_string()).collect(),
    };
    if on {
        if supported.contains(&scope) && !list.iter().any(|s| s == scope) {
            list.push(scope.to_string());
        }
    } else {
        list.retain(|s| s != scope);
    }
    list
}

/// One undoable `command.batch` for bulk details: set the same publishing flag on every row.
fn bulk_hidden_batch(rows: &[String], hidden: bool) -> Value {
    let steps: Vec<Value> = rows
        .iter()
        .map(|id| json!({"command": "variable.update", "params": {"variable": id, "hiddenFromPublishing": hidden}}))
        .collect();
    json!({"label": "Edit variables", "commands": steps})
}

/// One undoable `command.batch` for bulk details: toggle `scope` on every row, each starting
/// from its own current scope list (egui `bulk_details` applies the click per variable).
fn bulk_scope_batch(rows: &[(String, VarKind, Option<Vec<String>>)], scope: &str, on: bool) -> Value {
    let steps: Vec<Value> = rows
        .iter()
        .map(|(id, kind, scopes)| {
            let scopes = toggle_scope(*kind, scopes.as_deref(), scope, on);
            json!({"command": "variable.update", "params": {"variable": id, "scopes": scopes}})
        })
        .collect();
    json!({"label": "Edit variables", "commands": steps})
}

fn cell_of(vars: &figcraft_doc::Variables, val: Option<&figcraft_doc::VariableValue>) -> VarCell {
    use figcraft_doc::VariableValue as V;
    let Some(val) = val else { return VarCell::Empty };
    match val {
        V::Color(c) => {
            let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            VarCell::Color {
                color: Color32::from_rgba_unmultiplied(ch(c.r), ch(c.g), ch(c.b), ch(c.a)),
                hex: c.to_hex(),
                alpha: (c.a.clamp(0.0, 1.0) * 100.0).round() as u8,
            }
        }
        V::Number(n) => {
            let s = if n.is_finite() && n.fract() == 0.0 { format!("{n:.0}") } else { format!("{n}") };
            VarCell::Number(s)
        }
        V::String(s) => VarCell::Text(s.clone()),
        V::Boolean(b) => VarCell::Bool(*b),
        V::Alias(id) => {
            let name = vars.get(id).map(|v| v.name.clone()).unwrap_or_else(|| format!("→ {id}"));
            VarCell::Alias(name)
        }
        _ => VarCell::Empty,
    }
}

/// Renaming group `group` to `new` (egui `group_renames`): `new` replaces the group's last path
/// segment, and a `/` in it nests the group under a new parent. Each path part is trimmed and
/// empty parts are dropped. Moves are returned deepest remainder first, so a variable moving into
/// the group's own new path (`Orange` → `Orange/Warm`) never takes a name another still holds.
/// `None` when nothing changes.
fn group_renames(members: &[(String, String)], group: &str, new: &str) -> Option<(String, Vec<(String, String)>)> {
    let new = new.split('/').map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("/");
    if new.is_empty() {
        return None;
    }
    let renamed = match group.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{new}"),
        None => new,
    };
    if renamed == group {
        return None;
    }
    let prefix = format!("{group}/");
    let mut moved: Vec<(&str, &str)> = members.iter().filter_map(|(id, rest)| Some((id.as_str(), rest.as_str()))).collect();
    moved.sort_by_key(|(_, rest)| std::cmp::Reverse(rest.len()));
    Some((renamed.clone(), moved.into_iter().map(|(id, rest)| (id.to_string(), format!("{renamed}/{rest}"))).collect()))
}

/// The first of `base`, `base 2`, `base 3`… that `taken` doesn't hold (egui `unique`).
fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..10_000).map(|k| if k == 1 { base.to_string() } else { format!("{base} {k}") }).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

/// `command.batch` params copying the local variables under `group` into the next free sibling
/// group (`Brand` → `Brand 2`), every mode and scope included. Mirrors egui `duplicate_group`:
/// each copy is `variable.duplicate` then a rename through the `$ref` tag, so one undo step
/// restores the whole group. Inherited (library-extension) members are skipped: the engine
/// rejects them and one failure would roll the entire batch back.
fn duplicate_group_batch(rows: &[VarRow], groups: &[String], group: &str) -> Option<Value> {
    let (parent, leaf) = group.rsplit_once('/').map_or(("", group), |(p, l)| (p, l));
    let copy_leaf = unique_name(leaf, |n| {
        let full = if parent.is_empty() { n.to_string() } else { format!("{parent}/{n}") };
        full == group || groups.contains(&full) || rows.iter().any(|r| r.full_name.starts_with(&format!("{full}/")))
    });
    let copy = if parent.is_empty() { copy_leaf } else { format!("{parent}/{copy_leaf}") };
    let prefix = format!("{group}/");
    let mut steps: Vec<Value> = Vec::new();
    for (k, r) in rows.iter().filter(|r| !r.inherited && r.full_name.starts_with(&prefix)).enumerate() {
        let tag = format!("copy{k}");
        steps.push(json!({"command": "variable.duplicate", "params": {"variable": r.id}, "as": tag}));
        steps.push(json!({
            "command": "variable.update",
            "params": {"variable": {"$ref": tag}, "name": format!("{copy}/{}", &r.full_name[prefix.len()..])},
        }));
    }
    (!steps.is_empty()).then(|| json!({"label": tr("Duplicate group"), "commands": steps}))
}

/// 把分组连同它的所有祖先分组一起收集（`Varia/bl/k` → `Varia`、`Varia/bl`）。
/// 用户 2026-11-17：只有叶子组会被 `rsplit_once('/')` 命中，中间层在侧栏里会凭空消失。
fn push_group(groups: &mut Vec<String>, group: &str) {
    let mut upto = String::new();
    for seg in group.split('/') {
        if !upto.is_empty() {
            upto.push('/');
        }
        upto.push_str(seg);
        if !groups.contains(&upto) {
            groups.push(upto.clone());
        }
    }
}

/// 从 `Document` 读快照（本地集合；库变量只读与 extends 继承留给后续）。
pub fn read(doc: &figcraft_doc::Document) -> VariablesData {
    let vars = &doc.variables;
    let collections = vars
        .collections
        .iter()
        .map(|c| {
            let modes: Vec<ModeSnap> = c.modes.iter().map(|m| ModeSnap { id: m.id.clone(), name: m.name.clone() }).collect();
            let mut groups: Vec<String> = Vec::new();
            let rows: Vec<VarRow> = vars
                .variables
                .iter()
                .filter(|v| v.collection == c.id)
                .map(|v| {
                    let (group, short) = match v.name.rsplit_once('/') {
                        Some((g, s)) => (g.to_string(), s.to_string()),
                        None => (String::new(), v.name.clone()),
                    };
                    if !group.is_empty() {
                        push_group(&mut groups, &group);
                    }
                    VarRow {
                        id: v.id.clone(),
                        full_name: v.name.clone(),
                        name: short,
                        group,
                        kind: kind_of(v.kind),
                        values: c.modes.iter().map(|m| cell_of(vars, v.values.get(&m.id))).collect(),
                        overrides: c.modes.iter().map(|m| v.overrides.contains(&m.id)).collect(),
                        description: v.description.clone(),
                        hidden: v.hidden_from_publishing,
                        code: ["WEB", tr("iOS"), "ANDROID"]
                            .iter()
                            .map(|k| (k.to_string(), v.code_syntax.get(*k).cloned().unwrap_or_default()))
                            .collect(),
                        scopes: v.scopes.clone(),
                        inherited: v.extends.is_some(),
                    }
                })
                .collect();
            groups.sort();
            CollectionSnap { id: c.id.clone(), name: c.name.clone(), modes, rows, groups, library: None, extends: c.extends.clone(), editable: true }
        })
        .collect();
    VariablesData { collections, libraries: Vec::new() }
}

/// 已启用库的只读集合（egui `View::of(lib)` + `published_collections`）。
pub fn read_libraries(cache: &figcraft_engine::library::Cache, doc: &figcraft_doc::Document) -> Vec<LibrarySnap> {
    use figcraft_engine::library;
    library::read_libraries(cache, doc)
        .map(|(link, lib)| {
            let vars = &lib.doc.variables;
            let collections = library::published_collections(lib)
                .map(|c| {
                    let modes: Vec<ModeSnap> = c.modes.iter().map(|m| ModeSnap { id: m.id.clone(), name: m.name.clone() }).collect();
                    let mut groups: Vec<String> = Vec::new();
                    let rows: Vec<VarRow> = library::published_variables(lib)
                        .filter(|v| v.collection == c.id)
                        .map(|v| {
                            let (group, short) = match v.name.rsplit_once('/') {
                                Some((g, s)) => (g.to_string(), s.to_string()),
                                None => (String::new(), v.name.clone()),
                            };
                            if !group.is_empty() {
                                push_group(&mut groups, &group);
                            }
                            VarRow {
                                id: v.id.clone(),
                                full_name: v.name.clone(),
                                name: short,
                                group,
                                kind: kind_of(v.kind),
                                values: c.modes.iter().map(|m| cell_of(vars, v.values.get(&m.id))).collect(),
                                overrides: c.modes.iter().map(|m| v.overrides.contains(&m.id)).collect(),
                                description: v.description.clone(),
                                hidden: v.hidden_from_publishing,
                                code: ["WEB", tr("iOS"), "ANDROID"]
                                    .iter()
                                    .map(|k| (k.to_string(), v.code_syntax.get(*k).cloned().unwrap_or_default()))
                                    .collect(),
                                scopes: v.scopes.clone(),
                                inherited: v.extends.is_some(),
                            }
                        })
                        .collect();
                    groups.sort();
                    CollectionSnap {
                        id: c.id.clone(),
                        name: c.name.clone(),
                        modes,
                        rows,
                        groups,
                        library: Some(link.id.clone()),
                        extends: c.extends.clone(),
                        editable: false,
                    }
                })
                .collect();
            LibrarySnap { id: link.id.clone(), name: link.name.clone(), collections }
        })
        .collect()
}

/// `FC_DEBUG_POPUP=variables_table` 用的合成数据。
pub fn sample() -> VariablesData {
    let color = |r: u8, g: u8, b: u8, hex: &str| VarCell::Color { color: Color32::from_rgb(r, g, b), hex: hex.to_string(), alpha: 100 };
    let row = |id: &str, name: &str, group: &str, kind: VarKind, values: Vec<VarCell>| VarRow {
        id: id.to_string(),
        full_name: if group.is_empty() { name.to_string() } else { format!("{group}/{name}") },
        name: name.to_string(),
        group: group.to_string(),
        kind,
        overrides: vec![false; values.len()],
        description: String::new(),
        hidden: false,
        code: vec![("WEB".into(), String::new()), (tr("iOS").into(), String::new()), ("ANDROID".into(), String::new())],
        scopes: None,
        inherited: false,
        values,
    };
    VariablesData {
        libraries: Vec::new(),
        collections: vec![CollectionSnap {
            id: "coll-sample".into(),
            name: "Design tokens".into(),
            modes: vec![ModeSnap { id: "m-light".into(), name: tr("Light").into() }, ModeSnap { id: "m-dark".into(), name: tr("Dark").into() }],
            library: None,
            extends: None,
            editable: true,
            groups: vec!["Brand".into(), tr("Text").into()],
            rows: vec![
                row("v-radius", tr("Radius"), "", VarKind::Number, vec![VarCell::Number("8".into()), VarCell::Number("12".into())]),
                row("v-primary", "Primary", "Brand", VarKind::Color, vec![color(0x0D, 0x99, 0xFF, "0D99FF"), color(0x60, 0xA5, 0xFA, "60A5FA")]),
                row("v-on", "On primary", "Brand", VarKind::Color, vec![color(0xFF, 0xFF, 0xFF, "FFFFFF"), VarCell::Alias("Brand/Primary".into())]),
                row("v-font", "Font family", tr("Text"), VarKind::String, vec![VarCell::Text("Inter".into()), VarCell::Text("Inter".into())]),
                row("v-bold", "Use bold", tr("Text"), VarKind::Boolean, vec![VarCell::Bool(true), VarCell::Bool(false)]),
            ],
        }],
    }
}

// ── 编辑器状态 ────────────────────────────────────────────────────────────────────────────

/// 内联菜单（不占用全局 `Popup` 槽：变量窗口本身就是 `Popup::Window`，再开全局菜单会把它顶掉）。
#[derive(Clone)]
pub struct MenuItem {
    pub label: SharedString,
    pub cmd: &'static str,
    pub params: Value,
}

impl MenuItem {
    fn new(label: impl Into<SharedString>, cmd: &'static str, params: Value) -> Self {
        Self { label: label.into(), cmd, params }
    }
}

/// The context menu of a row that is part of a selection of several (egui `bulk_menu`):
/// edit / delete / move to a group / scope all act on every selected variable in one undo step.
fn bulk_menu_items(rows: &[(String, String, VarKind)], groups: &[String]) -> Vec<MenuItem> {
    let ids: Vec<Value> = rows.iter().map(|(id, ..)| json!(id)).collect();
    let mut items = vec![MenuItem::new(tr("Edit variables"), "ui.details", json!({ "variables": ids }))];
    let n = rows.len();
    let n_text = n.to_string();
    let delete_steps: Vec<Value> =
        rows.iter().map(|(id, ..)| json!({"command": "variable.delete", "params": {"variable": id}})).collect();
    items.push(MenuItem::new(
        figcraft_i18n::trf("Delete {n} variables", &[("n", &n_text)]),
        "command.batch",
        json!({"label": "Delete variables", "commands": delete_steps}),
    ));
    let moves = std::iter::once(None).chain(groups.iter().map(|g| Some(g.as_str())));
    for g in moves {
        let label = g.map_or_else(|| tr("Move out of any group").to_string(), |g| figcraft_i18n::trf("Move to group {group}", &[("group", &g)]));
        let steps: Vec<Value> = rows
            .iter()
            .filter_map(|(id, full, _)| {
                let short = full.rsplit('/').next().unwrap_or(full);
                let name = g.map_or_else(|| short.to_string(), |g| format!("{g}/{short}"));
                (name != *full).then(|| json!({"command": "variable.update", "params": {"variable": id, "name": name}}))
            })
            .collect();
        if !steps.is_empty() {
            items.push(MenuItem::new(label, "command.batch", json!({"label": "Move variables", "commands": steps})));
        }
    }
    let mut scopes: Vec<(&'static str, &'static str)> =
        rows.first().map(|(_, _, kind)| figcraft_doc::styles::scopes_for(kind.doc_type()).to_vec()).unwrap_or_default();
    for (_, _, kind) in rows.iter().skip(1) {
        let kind_scopes = figcraft_doc::styles::scopes_for(kind.doc_type());
        scopes.retain(|(id, _)| kind_scopes.iter().any(|(sid, _)| sid == id));
    }
    let scope_item = |label: String, value: Option<&'static str>| {
        let steps: Vec<Value> = rows
            .iter()
            .filter_map(|(id, _, kind)| {
                let scopes = match value {
                    None => Value::Null,
                    Some("") => json!([]),
                    Some(s) => {
                        if figcraft_doc::styles::scopes_for(kind.doc_type()).iter().any(|(id, _)| *id == s) {
                            json!([s])
                        } else {
                            return None;
                        }
                    }
                };
                Some(json!({"command": "variable.update", "params": {"variable": id, "scopes": scopes}}))
            })
            .collect();
        MenuItem::new(label, "command.batch", json!({"label": "Change scope", "commands": steps}))
    };
    items.push(scope_item(tr("Scope: all supported properties").to_string(), None));
    items.push(scope_item(tr("Scope: hidden from pickers").to_string(), Some("")));
    for (id, label) in scopes {
        items.push(scope_item(
            figcraft_i18n::trf("Scope: only {scope}", &[("scope", &tr(label))]),
            Some(id),
        ));
    }
    items
}

#[derive(Clone)]
pub struct Menu {
    pub x: f32,
    pub y: f32,
    pub items: Vec<MenuItem>,
    /// 对齐到某个控件的真实矩形（窗口坐标）：创建变量钮 / 过滤眼睛用它把菜单贴到按钮上。
    /// `None` = 右键 / 省略号菜单，直接以按下点 `(x, y)` 为锚。
    pub anchor: Option<Bounds<Pixels>>,
    /// 相对锚点矩形的对齐方式（`RectAlign::TopStart` = 按钮上方左对齐）。
    pub align: crate::widgets::RectAlign,
    pub gap: f32,
    pub width: crate::widgets::Width,
}

impl Menu {
    /// 按下点锚定的菜单（右键 / 旧调用）。
    fn at(x: f32, y: f32, items: Vec<MenuItem>) -> Self {
        Self { x, y, items, anchor: None, align: crate::widgets::RectAlign::BottomStart, gap: 0.0, width: crate::widgets::Width::Min(180.0) }
    }

    /// 贴到某个控件矩形上的菜单：`align` 决定在哪一侧，`gap` 是它和控件的距离。
    fn on(rect: Option<Bounds<Pixels>>, align: crate::widgets::RectAlign, gap: f32, items: Vec<MenuItem>) -> Self {
        Self { x: 0.0, y: 0.0, items, anchor: rect, align, gap, width: crate::widgets::Width::Fit }
    }
}

/// 一次列表拖拽（集合 / 分组 / 变量行共用），落点由 `crate::drag::resolve` 算。
pub struct ListDragState {
    pub kind: DragList,
    pub from: usize,
    /// 可见行的稳定标识：`ids[gap]` 用来拼 `before` 参数。
    pub ids: Vec<String>,
    pub rows: Vec<DragRow>,
    pub pointer: Point<Pixels>,
    pub target: Option<DragTarget>,
    pub armed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DragList {
    Collections,
    Groups,
    Variables,
}

/// 本地剪贴板：按 **mode 名** 存值（egui `CopiedVariable`），跨集合粘贴才不会串列。
#[derive(Clone, Debug)]
pub struct ClipRow {
    pub name: String,
    pub kind: VarKind,
    pub values: Vec<(String, VarCell)>,
}

/// 行内联编辑（改名 / mode 改名 / 值编辑）用到的 retained `TextField` 实体缓存。
///
/// 虚拟列表的渲染闭包是 `'static`、拿不到 `&mut VariablesEditor`，所以这些实体必须放在
/// `Rc<RefCell<..>>` 里，由闭包共享读写（参照通用虚拟列表的做法，只是把
/// `Entity<V>`/`V: Render` 的约束换成闭包捕获）。
#[derive(Default)]
struct RowCaches {
    /// 就地改名控件（照搬左栏样式重命名）。**每次开始改名都新建一个**：
    /// `InlineInput` 自己文档里写死了复用会留下上一次的草稿，且旧编辑器若仍在编辑态，
    /// `begin_edit` 里的 `finish_edit` 会把旧草稿提交到刚设置的新目标上（表现为要连点
    /// 三四次、输入框空白或串名）。
    rename_input: Option<Entity<crate::inline_input::InlineInput>>,
    mode_rename_input: Option<Entity<crate::inline_input::InlineInput>>,
    /// sidebar 集合 / 分组改名，同样是 InlineInput（不各造 TextField）。
    collection_rename_input: Option<Entity<crate::inline_input::InlineInput>>,
    group_rename_input: Option<Entity<crate::inline_input::InlineInput>>,
    /// 分组改名需要成员（变量 id + 组内剩余名），提交时逐条重命名。
    group_rename_members: Rc<RefCell<Vec<(String, String)>>>,
    /// 每个可拖行在窗口里的真实矩形（prepaint 写入，按下时读）。
    /// 插入线 / 落点判定都用它校准，而不是拿按下点当行顶 —— 后者会把「点在行内的偏移」
    /// 当成整列的偏移，插入线因此画错、行也拖不动（用户 2026-11-17）。
    drag_bounds: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
    value_editors: HashMap<String, Entity<TextField>>,
    /// 颜色值格用通用 `ColorField`：它的 hex / alpha 是裸 `TextEdit`，按 key 缓存以保住焦点。
    color_editors: HashMap<String, (Entity<crate::widgets::input::TextEdit>, Entity<crate::widgets::input::TextEdit>)>,
    /// 颜色格最新 RGB（alpha 由 opacity 输入框给）：缓存 editor 的 opacity 回调读这里，
    /// 否则取色器把红改成蓝后，改透明度会把旧红色写回去（CodeRabbit PR#29）。
    color_rgb: HashMap<String, Rc<Cell<[u8; 4]>>>,
    /// 打开取色器用的请求队列（由 `WindowState` 注入；独立渲染测试里可能没有）。
    popups: Option<crate::panels::PopupReqs>,
}

/// 虚拟列表中的「条目」：分组间隔、分组标题、或一条变量行。
enum RowItem {
    Gap,
    /// 分组标题。`first` 是该组第一个变量：拖拽落在这里 = 移到该组**上方**（组外），
    /// 提交时用它当 `before` 并显式 `group: ""`。
    GroupHeader {
        name: String,
        first: String,
    },
    Row {
        index: usize,
        row: VarRow,
    },
}

/// 拖拽行里的分组标题锚点前缀：`\u{1}g\u{1}{组名}\u{1}{组首变量 id}`。
const DRAG_GROUP: &str = "\u{1}g\u{1}";

/// 分组标题锚点的字符串键（不会与变量 id 或名字撞）。
fn drag_group_key(group: &str, first: &str) -> String {
    format!("{DRAG_GROUP}{group}\u{1}{first}")
}

/// 从锚点键里取出「组首变量 id」；普通变量 id 返回 `None`。
fn drag_group_first(key: &str) -> Option<&str> {
    key.strip_prefix(DRAG_GROUP)?.split_once('\u{1}').map(|(_, first)| first)
}
impl RowItem {
    fn height(&self) -> f32 {
        match self {
            RowItem::Gap => GROUP_GAP,
            RowItem::GroupHeader { .. } => GROUP_H,
            RowItem::Row { .. } => ROW_H,
        }
    }
}

pub struct EditorState {
    /// 当前来源库（`None` = 本文件）。
    pub library: Option<String>,
    pub collection: usize,
    pub query: String,
    pub hidden: [bool; 4],
    pub group: Option<String>,
    pub sidebar: bool,
    pub full: bool,
    /// 选中的变量行（egui `Editor::selected`）；空 = 未选。
    pub selected: Vec<String>,
    /// Shift+click 的起点（egui `Editor::anchor`）。
    pub anchor: Option<String>,
    /// 选中的整列（mode id）；与 `selected` 互斥。
    pub selected_mode: Option<String>,
    /// sidebar 里 "All variables" 是否被选中（与按组过滤区分）。
    pub selected_all: bool,
    /// 侧栏宽度与 Collections/Groups 的上下分割比例（egui `sidebar_w` / `sidebar_split`）。
    pub sidebar_w: f32,
    pub sidebar_split: f32,
    pub sidebar_resize: Option<(f32, f32)>,
    pub split_resize: Option<(f32, f32)>,
    /// 表格的水平滚动句柄（12pt 条带读它的 offset 画滑块）。
    pub hscroll: ScrollHandle,
    /// 横向滚动条拖动中：`(按下 x, 起始 offset, 滑块可移动行程, 最大滚动量)`。
    pub hscroll_drag: Option<(f32, f32, f32, f32)>,
    pub rename_collection: Option<String>,
    pub rename_group: Option<String>,
    pub rename: Option<String>,
    /// 正在内联改名的 mode id（`variable.renameMode`）。
    pub rename_mode: Option<String>,
    /// 该 mode 所属 collection（`variable.renameMode` 需要 collection + mode 两个 id）。
    pub rename_mode_coll: Option<String>,
    pub menu: Option<Menu>,
    pub drag: Option<ListDragState>,
    /// mode 列水平拖动：`(起始列下标, 按下 x, 当前 x)`。
    pub mode_drag: Option<(usize, f32, f32)>,
    /// Name 列宽（egui `NAME_W`，可拖 120..420）。
    pub name_w: f32,
    /// Name 列宽拖动中：`(按下 x, 起始宽度)`。
    pub col_resize: Option<(f32, f32)>,
    /// 每个 mode 列的宽度（egui `MODE_W`，可拖 100..420）。
    pub mode_ws: Vec<f32>,
    /// mode 列宽拖动中：`(列下标, 按下 x, 起始宽度)`。
    pub mode_resize: Option<(usize, f32, f32)>,
    /// 本地剪贴板：复制的整行（按 mode 名）。
    pub clipboard: Option<ClipRow>,
    /// 复制的整列：`(变量 id, 值)`，Paste column 时逐条 `variable.set`。
    pub clipboard_column: Option<Vec<(String, VarCell)>>,
    /// 分组显示顺序（egui 的 `group_order`，会话视图偏好，不进文档）。
    pub group_order: Vec<String>,
    /// 被折叠的分组（其下所有变量不在表格里显示）。
    pub collapsed: BTreeSet<String>,
    /// 正在编辑详情的变量 id（egui `details`）。
    pub details: Option<String>,
    /// 多选时「Edit variables」批量详情的变量 id 列表（egui `bulk_details`）。
    pub bulk_details: Option<Vec<String>>,
    /// 批量详情当前页签：0 = Details，1 = Scope。
    pub bulk_details_tab: usize,
    /// 「+ Create variable」按钮的真实矩形（prepaint 写入，开菜单时读，用作锚点）。
    pub add_anchor: Option<Bounds<Pixels>>,
    /// 搜索旁过滤眼睛按钮的真实矩形（同上）。
    pub filter_anchor: Option<Bounds<Pixels>>,
    /// 「Edit variable」浮层被拖到的位置（窗口坐标；`None` = 默认右上）。
    pub details_pos: Option<Point<Pixels>>,
    /// 浮层标题栏拖动中：按下点相对卡片原点的偏移。
    pub details_drag: Option<Point<Pixels>>,
    /// 「Edit variable」浮层上一帧量到的真实高度：clamp 用它而非默认 72pt，
    /// 否则拖到视口底部时下方控件出屏（CodeRabbit PR#29）。
    pub details_h: Option<f32>,
    /// 用户拖出来的窗口尺寸（None = 默认 FLOAT_W/FLOAT_H）。
    pub size: Option<(f32, f32)>,
    /// resize 拖动中：`(方向位, 按下 x, 按下 y, 起始宽, 起始高)`。
    pub resize: Option<(u8, f32, f32, f32, f32)>,
    /// 双击检测：上次按下的时间与目标 id。
    pub last_down: Option<(Instant, String)>,
    /// mode 表头的双击检测。
    pub last_mode_down: Option<(Instant, String)>,
    /// 变量表格的虚拟列表滚动句柄（跨帧保留，滚动位置不丢失）。
    pub scroll: VirtualListScrollHandle,
    /// 侧栏 Groups 列表的虚拟列表滚动句柄（与表格 / 左栏图层同一套虚拟化）。
    pub groups_scroll: VirtualListScrollHandle,
    /// root 在窗口里的真实矩形（由每帧的 canvas prepaint 写入）。插入线要把窗口坐标换算成
    /// root 局部坐标，而 prepaint 在本帧构建之后才跑——所以这个 `Rc` 必须**跨帧保留**，
    /// 每帧新建一个的话读到的永远是 `None`（插入线因此一直画不出来）。
    pub root_bounds: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
}

impl EditorState {
    fn new() -> Self {
        Self {
            library: None,
            collection: 0,
            query: String::new(),
            hidden: [false; 4],
            group: None,
            sidebar: true,
            full: false,
            selected: Vec::new(),
            anchor: None,
            selected_mode: None,
            selected_all: true,
            sidebar_w: SIDEBAR_W,
            sidebar_split: 0.5,
            sidebar_resize: None,
            split_resize: None,
            hscroll: ScrollHandle::default(),
            hscroll_drag: None,
            rename_collection: None,
            rename_group: None,
            rename: None,
            rename_mode: None,
            rename_mode_coll: None,
            menu: None,
            drag: None,
            mode_drag: None,
            name_w: NAME_W,
            col_resize: None,
            mode_ws: Vec::new(),
            mode_resize: None,
            clipboard: None,
            clipboard_column: None,
            group_order: Vec::new(),
            collapsed: BTreeSet::new(),
            details: None,
            bulk_details: None,
            bulk_details_tab: 0,
            details_pos: None,
            details_drag: None,
            details_h: None,
            add_anchor: None,
            filter_anchor: None,
            size: None,
            resize: None,
            last_down: None,
            last_mode_down: None,
            scroll: VirtualListScrollHandle::new(),
            groups_scroll: VirtualListScrollHandle::new(),
            root_bounds: Rc::new(std::cell::Cell::new(None)),
        }
    }
}

pub type State = Rc<RefCell<EditorState>>;

/// A click on a variable row (egui `variables_panel.rs::select_clicked`): plain selects only it,
/// Shift extends from the anchor through the shown rows (added to the selection with Cmd/Ctrl),
/// Cmd/Ctrl (or Shift without a valid anchor) toggles it.
fn select_var(st: &mut EditorState, id: &str, order: &[String], m: &Modifiers) {
    let pos = |x: &str| order.iter().position(|o| o == x);
    let accel = m.platform || m.control;
    let anchor = st.anchor.as_deref().filter(|a| st.selected.iter().any(|s| s == a)).and_then(pos);
    if m.shift
        && let (Some(ai), Some(bi)) = (anchor, pos(id))
    {
        let (lo, hi) = (ai.min(bi), ai.max(bi));
        let mut s: Vec<String> = if accel { st.selected.clone() } else { Vec::new() };
        for x in order.get(lo..=hi).unwrap_or_default() {
            if !s.iter().any(|v| v == x) {
                s.push(x.clone());
            }
        }
        st.selected = s;
    } else if m.shift || accel {
        if let Some(i) = st.selected.iter().position(|x| x == id) {
            st.selected.remove(i);
        } else {
            st.selected.push(id.to_string());
        }
        st.anchor = Some(id.to_string());
    } else {
        st.selected = vec![id.to_string()];
        st.anchor = Some(id.to_string());
    }
}

/// 完整的变量编辑器。`WindowState` 持有它，`render` 每帧调用。
#[derive(Clone)]
pub struct VariablesEditor {
    pub state: State,
    pub data: VariablesData,
    cmds: Cmds,
    search: Entity<TextField>,
    /// 行内联编辑（改名 / mode 改名 / 值编辑）用到的 retained `TextField` 实体缓存。
    /// 放进 `Rc<RefCell<..>>` 以便虚拟列表的 `'static` 渲染闭包也能读写它们。
    row_caches: Rc<RefCell<RowCaches>>,
    details_fields: HashMap<String, Entity<TextField>>,
    on_full: Rc<dyn Fn(bool, &mut App)>,
    on_close: Rc<dyn Fn(&mut App)>,
}

/// 表格里的分组标题行（egui `group_rows` 的表头）：左 16pt 起、11pt 次要色、底部 1px 线。
/// **没有折叠三角** —— 折叠只在 sidebar 的 Groups 里做（egui 同）。
///
/// 子组标题按层级缩进（一级 16、二级 40、三级 64 …），缩进**只作用于文字**：
/// 标题行本身仍然满宽，底线通栏 —— 用户 2026-11-17「子层级的左边距没有了」。
fn group_header_div(name: &str) -> Div {
    let t = tokens();
    let level = name.matches('/').count() + 1;
    div()
        .relative()
        .flex_none()
        .w_full()
        .h(pt(GROUP_H))
        .flex()
        .items_center()
        .pl(pt(16.0 + (level.saturating_sub(1)) as f32 * 24.0))
        .text_size(pt(11.0))
        .text_color(rgba(t.text_secondary))
        .child(name.to_string())
        .child(div().absolute().bottom_0().left_0().w_full().h(pt(1.0)).bg(rgba(t.border)))
}

/// 内联菜单（`menu_overlay`）按内容自适应时的**预计宽度**：最宽一项的文字宽 +
/// 行左右各 8 + 卡片外框左右各 [`crate::menu_metrics::MENU_PAD`]。
/// 通用 `Menu` 只按**左缘**定位；需要「右缘对齐触发按钮」时先用它反推锚点左缘。
fn menu_fit_width(window: &Window, items: &[MenuItem]) -> f32 {
    let text_w = items.iter().map(|i| crate::bidi::measure(window, &i.label, 11.0)).fold(0.0_f32, f32::max);
    (text_w + 16.0 + 2.0 * crate::menu_metrics::MENU_PAD).ceil()
}

impl VariablesEditor {
    pub fn new(
        cmds: Cmds,
        data: VariablesData,
        cx: &mut App,
        popups: Option<crate::panels::PopupReqs>,
        on_full: Rc<dyn Fn(bool, &mut App)>,
        on_close: Rc<dyn Fn(&mut App)>,
    ) -> Self {
        let state: State = Rc::new(RefCell::new(EditorState::new()));
        let search = search_field(cx, SEARCH_W, "", "Search variables");
        let q = state.clone();
        // 边打边过滤：`on_change` 立即写查询词并唤醒根视图重绘（原来是回车才过滤）。
        let wake = cmds.clone();
        search.update(cx, move |tf, _| {
            tf.set_on_change(move |text, _, _| {
                q.borrow_mut().query = text.trim().to_lowercase();
                wake.wake();
            });
        });
        // 改名控件不在这里预建：InlineInput 要求每次进入重命名都新建一个（见其文档），
        // 否则会沿用上一次的草稿，并让 start_*_rename 的旧提交写到新目标上。
        let mut caches = RowCaches::default();
        caches.group_rename_members = Rc::new(RefCell::new(Vec::new()));
        caches.popups = popups;
        Self { state, data, cmds, search, row_caches: Rc::new(RefCell::new(caches)), details_fields: HashMap::new(), on_full, on_close }
    }

    // ── 就地改名：每次新建 InlineInput（与左栏 Pages / Layers / 画布标签同款）────────────

    /// 变量名改名。旧编辑器先提交（此时 `rename` 仍指向旧变量），再换新编辑器 + 新目标。
    ///
    /// 与左栏 `begin_inline_rename` 完全同一套做法：**只建控件、不在这里 begin_edit**。
    /// 焦点与全选由下一帧渲染时的 `InlineInput::poll_active` 完成 —— 若在按下事件里
    /// `begin_edit`，同一手势的抬起会落到刚出现的输入框上、把 `select_all_pending` 吃掉，
    /// 表现为双击后没有焦点 / 没有全选（用户 2026-11-17：要点三四次才进输入态）。
    fn start_variable_rename(caches: &Rc<RefCell<RowCaches>>, st: &State, cmds: &Cmds, id: String, name: String, window: &mut Window, cx: &mut App) {
        Self::finish_previous(&caches.borrow().rename_input, window, cx);
        let input = crate::inline_input::InlineInput::build(cx, ROW_H, 0.0, name, {
            let q = st.clone();
            let c = cmds.clone();
            move |text: &str, _, _| {
                let name = text.trim().to_string();
                // 先 take 到局部变量：`if let Some(id) = q.borrow_mut()...` 的临时 RefMut
                // 会活到整个 if-let 块结束，块内再 borrow_mut 就是 BorrowMutError
                // （GPUI 里表现为提交改名时进程 abort —— 用户 2026-11-17「提交时卡死」）。
                let id = q.borrow_mut().rename.take();
                if let Some(id) = id {
                    if !name.is_empty() {
                        c.push(Cmd { id: "variable.update", params: Some(json!({ "variable": id, "name": name })) });
                    }
                    q.borrow_mut().last_down = None;
                }
            }
        });
        caches.borrow_mut().rename_input = Some(input);
        st.borrow_mut().rename = Some(id);
    }

    fn start_mode_rename(
        caches: &Rc<RefCell<RowCaches>>,
        st: &State,
        cmds: &Cmds,
        coll: String,
        mode: String,
        name: String,
        window: &mut Window,
        cx: &mut App,
    ) {
        Self::finish_previous(&caches.borrow().mode_rename_input, window, cx);
        let input = crate::inline_input::InlineInput::build(cx, HEAD_ROW_H, 0.0, name, {
            let q = st.clone();
            let c = cmds.clone();
            move |text: &str, _, _| {
                let name = text.trim().to_string();
                // 同 `start_variable_rename`：先把 RefMut 临时收掉，否则块内的后两次
                // borrow_mut 会 panic（提交 mode 改名时 abort）。
                let mid = q.borrow_mut().rename_mode.take();
                if let Some(mid) = mid {
                    if !name.is_empty() {
                        let coll = q.borrow_mut().rename_mode_coll.take();
                        if let Some(coll) = coll {
                            c.push(Cmd { id: "variable.renameMode", params: Some(json!({ "collection": coll, "mode": mid, "name": name })) });
                        }
                    }
                    q.borrow_mut().last_mode_down = None;
                }
            }
        });
        {
            let mut s = st.borrow_mut();
            s.rename_mode = Some(mode);
            s.rename_mode_coll = Some(coll);
        }
        caches.borrow_mut().mode_rename_input = Some(input);
    }

    fn start_collection_rename(
        caches: &Rc<RefCell<RowCaches>>,
        st: &State,
        cmds: &Cmds,
        id: String,
        name: String,
        window: &mut Window,
        cx: &mut App,
    ) {
        Self::finish_previous(&caches.borrow().collection_rename_input, window, cx);
        let input = crate::inline_input::InlineInput::build(cx, 24.0, 0.0, name, {
            let q = st.clone();
            let c = cmds.clone();
            move |text: &str, _, _| {
                let name = text.trim().to_string();
                if let Some(id) = q.borrow_mut().rename_collection.take() {
                    if !name.is_empty() {
                        c.push(Cmd { id: "variable.renameCollection", params: Some(json!({ "collection": id, "name": name })) });
                    }
                }
            }
        });
        caches.borrow_mut().collection_rename_input = Some(input);
        st.borrow_mut().rename_collection = Some(id);
    }

    fn start_group_rename(caches: &Rc<RefCell<RowCaches>>, st: &State, cmds: &Cmds, group: String, name: String, window: &mut Window, cx: &mut App) {
        Self::finish_previous(&caches.borrow().group_rename_input, window, cx);
        let gm = caches.borrow().group_rename_members.clone();
        let input = crate::inline_input::InlineInput::build(cx, 24.0, 0.0, name, {
            let q = st.clone();
            let c = cmds.clone();
            move |text: &str, _, _| {
                let name = text.trim().to_string();
                let old = q.borrow_mut().rename_group.take();
                if let Some(old) = old {
                    let members = gm.borrow().clone();
                    if let Some((renamed, moves)) = group_renames(&members, &old, &name) {
                        // 整组改名是**一个手势**：合成一步撤销（`command.batch`）。
                        // `old` 是完整组路径（如 `Brand/Buttons`），编辑器里只有短名；
                        // `group_renames` 保留父路径，`/` 则把组嵌进新父组。
                        let steps: Vec<Value> = moves
                            .into_iter()
                            .map(|(id, name)| json!({ "command": "variable.update", "params": { "variable": id, "name": name } }))
                            .collect();
                        if !steps.is_empty() {
                            c.push(Cmd { id: "command.batch", params: Some(json!({ "label": tr("Rename group"), "commands": steps })) });
                            let mut s = q.borrow_mut();
                            if let Some(sel) = s.group.clone() {
                                if sel == old || sel.starts_with(&format!("{old}/")) {
                                    s.group = Some(format!("{renamed}{}", &sel[old.len()..]));
                                }
                            }
                        }
                    }
                }
            }
        });
        caches.borrow_mut().group_rename_input = Some(input);
        st.borrow_mut().rename_group = Some(group);
    }

    /// 换目标前先把旧编辑器提交掉（旧 `rename` 目标还没被覆盖）。
    fn finish_previous(old: &Option<Entity<crate::inline_input::InlineInput>>, window: &mut Window, cx: &mut App) {
        if let Some(old) = old.clone() {
            old.update(cx, |f, cx| {
                f.finish_edit(window, cx);
            });
        }
    }

    pub fn is_full(&self) -> bool {
        self.state.borrow().full
    }

    /// 由根视图写回「记住的 Full/Floating」。变量编辑器实体跨开关保留在视图上，
    /// 呈现模式是会话级状态，不该每次重新打开都回到 Floating。
    pub fn set_full(&self, full: bool) {
        self.state.borrow_mut().full = full;
    }

    /// 重新打开时刷新文档变量快照（交互状态仍保留）。
    pub fn set_data(&mut self, data: VariablesData) {
        // Drop selection ids that no longer exist (e.g. after a bulk delete) and clear
        // the range anchor if its endpoints vanished, so copy never targets a deleted row.
        {
            let mut st = self.state.borrow_mut();
            let live: std::collections::HashSet<String> = data.collections.iter()
                .chain(data.libraries.iter().flat_map(|l| l.collections.iter()))
                .flat_map(|c| c.rows.iter().map(|r| r.id.clone())).collect();
            st.selected.retain(|id| live.contains(id));
            if st.anchor.as_ref().is_some_and(|a| !live.contains(a)) { st.anchor = None; }
        }
        self.data = data;
    }

    /// 8 个边缘/角 handle 是否正被拖动（窗口级拖拽路由用）。
    pub fn resizing(&self) -> bool {
        self.state.borrow().resize.is_some()
    }

    /// 是否有「内置拖拽」正在进行（列表行拖动 / 列宽 / 侧栏 / 分栏 / 模式列宽）。
    /// 这些由编辑器内部 `on_move` 处理，整卡移动必须让位——否则在表格行上拖动会同时把整张卡也拖走。
    pub fn internal_drag(&self) -> bool {
        let s = self.state.borrow();
        s.drag.is_some()
            || s.mode_drag.is_some()
            || s.col_resize.is_some()
            || s.sidebar_resize.is_some()
            || s.split_resize.is_some()
            || s.mode_resize.is_some()
            || s.hscroll_drag.is_some()
            || s.details_drag.is_some()
    }

    /// 拖拽中：由**根视图**用窗口坐标推进（GPUI 的元素级 `on_mouse_move` 只在指针悬停其
    /// hitbox 时触发，指针移出卡片就断；与 `motion/timeline.rs` 的 `route_global_move` 同构）。
    /// 这里只改状态；重绘由调用方 `cx.notify()` 负责。
    pub fn route_move(&self, at: Point<Pixels>) {
        Self::on_move(&self.state, at);
    }

    /// 抬起 / Escape：清掉所有进行中的拖动状态。
    ///
    /// ⚠️ 提交必须在这里做：窗口级 mouse-up 监听先于元素级 `on_mouse_up` 派发，
    /// 元素级那层看到 `drag` / `mode_drag` 已经被这里清空过，于是行排序、mode 列重排
    /// 永远不落地（用户 2026-11-17：「拖不动」「列没有拖动行为」）。
    pub fn end_resize(&self) {
        let cid = self.coll().map(|c| c.id.clone()).unwrap_or_default();
        let mode_ids: Vec<String> = self.coll().map(|c| c.modes.iter().map(|m| m.id.clone()).collect()).unwrap_or_default();
        let (d, md, mode_ws) = {
            let mut s = self.state.borrow_mut();
            s.resize = None;
            s.col_resize = None;
            s.mode_resize = None;
            s.sidebar_resize = None;
            s.split_resize = None;
            s.hscroll_drag = None;
            s.details_drag = None;
            (s.drag.take(), s.mode_drag.take(), s.mode_ws.clone())
        };
        if let Some(d) = d {
            Self::commit_drag(&self.state, &d, &self.cmds);
        }
        // mode 列拖动：按**实际列宽**累加，越过下一列宽度就前移/后移一位。
        if let Some((from, x0, x1)) = md {
            let dx = x1 - x0;
            if !mode_ids.is_empty() && dx.abs() > 8.0 {
                let mut to = from;
                if dx > 0.0 {
                    let mut acc = 0.0;
                    for k in from..mode_ids.len().saturating_sub(1) {
                        acc += mode_ws.get(k).copied().unwrap_or(MODE_W);
                        if dx >= acc {
                            to = k + 1;
                        } else {
                            break;
                        }
                    }
                } else {
                    let mut acc = 0.0;
                    for k in (0..from).rev() {
                        acc += mode_ws.get(k).copied().unwrap_or(MODE_W);
                        if -dx >= acc {
                            to = k;
                        } else {
                            break;
                        }
                    }
                }
                if to != from {
                    if let Some(mid) = mode_ids.get(from) {
                        self.cmds.push(Cmd { id: "variable.moveMode", params: Some(json!({ "collection": cid, "mode": mid, "to": to })) });
                    }
                }
            }
        }
    }

    /// 用户拖出来的窗口尺寸（`WindowState::render` 用它覆盖默认宽高）。
    pub fn size(&self) -> Option<(f32, f32)> {
        self.state.borrow().size
    }

    /// 当前集合的变量总数（标题栏计数 chip）。
    pub fn total(&self) -> usize {
        self.coll().map(|c| c.rows.len()).unwrap_or(0)
    }

    /// 根视图在 `open_window` 时注入 Floating/Full 切换与关闭回调。
    pub fn set_callbacks(&mut self, on_full: Rc<dyn Fn(bool, &mut App)>, on_close: Rc<dyn Fn(&mut App)>) {
        self.on_full = on_full;
        self.on_close = on_close;
    }

    /// 键盘：Escape 先收掉内部菜单 / 改名 / 拖拽（没有内部状态就返回 false，让通用 Escape 关窗口）；
    /// Ctrl/Cmd+C / V 复制粘贴整行。返回 true = 这次按键已被变量面板消费。
    pub fn key(&self, key: &str, accel: bool) -> bool {
        let busy = {
            let s = self.state.borrow();
            s.menu.is_some()
                || s.rename.is_some()
                || s.rename_mode.is_some()
                || s.drag.is_some()
                || s.mode_drag.is_some()
                || s.col_resize.is_some()
                || s.mode_resize.is_some()
                || s.resize.is_some()
        };
        if key == "escape" {
            if busy {
                let mut s = self.state.borrow_mut();
                s.menu = None;
                s.rename = None;
                s.rename_mode = None;
                s.rename_collection = None;
                s.rename_group = None;
                s.drag = None;
                s.mode_drag = None;
                s.col_resize = None;
                s.mode_resize = None;
                s.sidebar_resize = None;
                s.split_resize = None;
                s.resize = None;
                return true;
            }
            // 没有内部状态时：先释放选中 / 关详情，再一次 Escape 才交给外层关窗口。
            let had = {
                let s = self.state.borrow();
                !s.selected.is_empty() || s.selected_mode.is_some() || s.details.is_some() || s.bulk_details.is_some()
            };
            if had {
                let mut s = self.state.borrow_mut();
                s.selected.clear();
                s.anchor = None;
                s.selected_mode = None;
                s.details = None;
                s.bulk_details = None;
                return true;
            }
            return false;
        }
        if accel && (key.eq_ignore_ascii_case("c") || key.eq_ignore_ascii_case("v")) {
            // 有选中的 mode 列就整列复制 / 粘贴，否则按整行（用户 2026-11-17）。
            let mode = self.state.borrow().selected_mode.clone();
            if key.eq_ignore_ascii_case("c") {
                if let Some(m) = mode {
                    Self::copy_column(&self.state, &self.data, &m);
                } else {
                    Self::copy_row(&self.state, &self.data);
                }
            } else if let Some(m) = mode {
                Self::paste_column(&self.state, &self.cmds, &self.data, &m);
            } else {
                Self::paste_row(&self.state, &self.data, &self.cmds);
            }
            return true;
        }
        false
    }

    /// 导出：系统「另存为」对话框 → `variable.export {format, path}`。
    fn export_dialog(cmds: &Cmds, format: &str) {
        let Some(path) =
            rfd::FileDialog::new().set_file_name(&format!("variables.{format}")).add_filter(format.to_uppercase(), &[format]).save_file()
        else {
            return;
        };
        cmds.push(Cmd { id: "variable.export", params: Some(json!({ "format": format, "path": path.to_string_lossy().to_string() })) });
    }

    /// 导入：系统「打开」对话框 → `variable.import {variables: path}`。
    fn import_dialog(cmds: &Cmds) {
        let Some(path) = rfd::FileDialog::new().add_filter("Variables JSON", &["json"]).pick_file() else {
            return;
        };
        cmds.push(Cmd { id: "variable.import", params: Some(json!({ "variables": path.to_string_lossy().to_string() })) });
    }

    /// 当前来源（本文件 / 库）下的集合切片。
    fn collections<'a>(st: &State, data: &'a VariablesData) -> &'a [CollectionSnap] {
        let lib = st.borrow().library.clone();
        match lib {
            Some(id) => data.libraries.iter().find(|l| l.id == id).map(|l| l.collections.as_slice()).unwrap_or(&[]),
            None => data.collections.as_slice(),
        }
    }

    /// 复制选中行：按 **mode 名** 存值，所以粘到 mode 名不同的集合也不会串列。
    fn copy_row(st: &State, data: &VariablesData) {
        // A bulk delete leaves stale ids in `selected`; resolve the first *live* selected
        // row instead of blindly taking selected[0].
        let selected = st.borrow().selected.clone();
        let sel = st.borrow().selected.first().cloned();
        let Some(sel) = sel else { return };
        let list = Self::collections(st, data);
        let Some((c, r)) = selected.iter().find_map(|sel| {
            list.iter().find_map(|c| c.rows.iter().find(|r| r.id == *sel).map(|r| (c, r)))
        }) else {
            return;
        };
        let values = c.modes.iter().zip(r.values.iter()).map(|(m, v)| (m.name.clone(), v.clone())).collect();
        st.borrow_mut().clipboard = Some(ClipRow { name: r.full_name.clone(), kind: r.kind, values });
    }

    /// 粘贴 = 在当前集合里**新建**一个变量（`variable.create` 带 `values {modeName: value}`），
    /// 而不是覆盖选中行的值 —— 与 egui `paste()` 一致。
    fn paste_row(st: &State, data: &VariablesData, cmds: &Cmds) {
        let clip = st.borrow().clipboard.clone();
        let Some(clip) = clip else { return };
        let list = Self::collections(st, data);
        let i = st.borrow().collection.min(list.len().saturating_sub(1));
        let Some(c) = list.get(i) else { return };
        if !c.editable {
            return;
        }
        let base = format!("{} copy", clip.name);
        let name = unique(&base, |n| c.rows.iter().any(|r| r.full_name == n));
        let mut values = serde_json::Map::new();
        for (mode, cell) in &clip.values {
            if let Some(v) = cell_value(cell) {
                values.insert(mode.clone(), v);
            }
        }
        cmds.push(Cmd {
            id: "variable.create",
            params: Some(json!({ "collection": c.id, "name": name, "type": clip.kind.id(), "values": Value::Object(values) })),
        });
    }

    /// 复制整列：记下当前可见行在该 mode 下的值。
    fn copy_column(st: &State, data: &VariablesData, mode: &str) {
        let list = Self::collections(st, data);
        let i = st.borrow().collection.min(list.len().saturating_sub(1));
        let Some(c) = list.get(i) else { return };
        let Some(mi) = c.modes.iter().position(|m| m.id == mode) else { return };
        let vals = c.rows.iter().filter_map(|r| r.values.get(mi).map(|v| (r.id.clone(), v.clone()))).collect();
        st.borrow_mut().clipboard_column = Some(vals);
    }

    /// 粘贴整列到目标 mode。一次粘贴是**一个手势**，走 `command.batch` 合成一步撤销
    /// （用户 2026-11-17：所有行为都要可逆、进撤销重做栈）。
    fn paste_column(st: &State, cmds: &Cmds, data: &VariablesData, mode: &str) {
        let Some(vals) = st.borrow().clipboard_column.clone() else { return };
        // 剪贴板里可能还留着已删除的变量：variable.set 会拒绝它，整批 command.batch
        // 因此回滚，连有效的那几格也贴不进去（CodeRabbit PR#29）。先按当前集合过滤。
        let list = Self::collections(st, data);
        let ci = st.borrow().collection.min(list.len().saturating_sub(1));
        let live: std::collections::HashSet<&str> = list.get(ci).map(|c| c.rows.iter().map(|r| r.id.as_str()).collect()).unwrap_or_default();
        let steps: Vec<Value> = vals
            .iter()
            .filter(|(vid, _)| live.contains(vid.as_str()))
            .filter_map(|(vid, cell)| {
                cell_value(cell).map(|v| json!({ "command": "variable.set", "params": { "variable": vid, "mode": mode, "value": v } }))
            })
            .collect();
        if steps.is_empty() {
            return;
        }
        cmds.push(Cmd { id: "command.batch", params: Some(json!({ "label": tr("Paste column"), "commands": steps })) });
    }

    fn push(&self, id: &'static str, params: Value) {
        self.cmds.push(Cmd { id, params: Some(params) });
    }

    /// 当前集合（越界时收敛到 0）。
    fn coll(&self) -> Option<&CollectionSnap> {
        let s = self.state.borrow();
        let list = match &s.library {
            Some(lib) => self.data.libraries.iter().find(|l| &l.id == lib).map(|l| l.collections.as_slice()).unwrap_or(&[]),
            None => self.data.collections.as_slice(),
        };
        list.get(s.collection.min(list.len().saturating_sub(1)))
    }

    fn coll_mut(&mut self) -> Option<&mut CollectionSnap> {
        let (lib, i) = {
            let s = self.state.borrow();
            (s.library.clone(), s.collection)
        };
        let list = match lib {
            Some(l) => self.data.libraries.iter_mut().find(|x| x.id == l).map(|x| &mut x.collections)?,
            None => &mut self.data.collections,
        };
        let i = i.min(list.len().saturating_sub(1));
        list.get_mut(i)
    }

    /// 过滤 + 分组排序后的可见变量行（egui `table()` 的 list）。
    fn visible_rows(&self, c: &CollectionSnap) -> Vec<VarRow> {
        let st = self.state.borrow();
        let q = st.query.as_str();
        let mut list: Vec<VarRow> = c
            .rows
            .iter()
            .filter(|v| {
                // 选中父组（`Brand`）时，子组（`Brand/Dark`）里的变量也要显示出来，
                // 只过滤掉真正不属于这棵子树的（用户 2026-11-17）。
                let in_group = st.group.as_ref().is_none_or(|g| v.group == *g || v.group.starts_with(&format!("{g}/")));
                let full_name = if v.group.is_empty() { v.name.clone() } else { format!("{}/{}", v.group, v.name) };
                in_group && !st.hidden[v.kind.index()] && (q.is_empty() || full_name.to_lowercase().contains(q))
            })
            .cloned()
            .collect();
        // 顺序：**无组变量在前**（按文档顺序），组在它们后面；组内按「树序」排：
        // 一级组在前，二级组紧跟其父组，三级组再跟在二级组后面（用户 2026-11-17）。
        // 新建变量没有组、引擎追加在集合末尾，以前会落到某个组后面，看起来像「进了组」。
        list.sort_by(|a, b| {
            let key =
                |r: &VarRow| -> (u8, Vec<usize>) { if r.group.is_empty() { (0, Vec::new()) } else { (1, group_rank(&r.group, &st.group_order)) } };
            // `sort_by` 是稳定排序：同 key 保持引擎里的原始顺序。
            key(a).cmp(&key(b))
        });
        list
    }

    /// 主体：`body_h` 是通用窗口留给 body 的高度（Full 时是整个视口减去 header）；
    /// `viewport` 供详情浮层用通用 `Popover` 做边界夹取。
    pub fn render(
        &mut self,
        cx: &mut App,
        window: &mut Window,
        body_h: f32,
        viewport: Size<Pixels>,
        origin: Point<Pixels>,
        drag_start: Option<DragStart>,
        card_w: f32,
    ) -> AnyElement {
        let t = tokens();
        let st = self.state.clone();
        let cmds = self.cmds.clone();
        let data = self.data.clone();
        let coll = self.coll().cloned();
        let full = st.borrow().full;
        let (sidebar, sidebar_w) = {
            let s = st.borrow();
            (s.sidebar, s.sidebar_w)
        };
        // 表格可用宽度 = **卡片宽度**（不是窗口宽度）减去侧栏与其 1pt 分隔线。
        // 以前误用 `viewport.width`，导致表格/滚动条按整窗宽算，列宽和滚动条全错。
        let table_w = (card_w - if sidebar { sidebar_w + 1.0 } else { 0.0 }).max(120.0);

        // 改名焦点由 InlineInput 自己管：每帧 poll 一次「正在改名」的那个，Ended（失焦/提交）
        // 就把对应状态收掉 —— 这一步同时负责首帧抢焦点+全选、以及点空白取消。
        {
            let mut s = st.borrow_mut();
            if let Some(input) = self.row_caches.borrow().rename_input.clone() {
                if s.rename.is_some() && input.update(cx, |f, cx| f.poll_active(window, cx)) == crate::inline_input::EditState::Ended {
                    s.rename = None;
                }
            }
            if let Some(input) = self.row_caches.borrow().mode_rename_input.clone() {
                if s.rename_mode.is_some() && input.update(cx, |f, cx| f.poll_active(window, cx)) == crate::inline_input::EditState::Ended {
                    s.rename_mode = None;
                }
            }
            if let Some(input) = self.row_caches.borrow().collection_rename_input.clone() {
                if s.rename_collection.is_some() && input.update(cx, |f, cx| f.poll_active(window, cx)) == crate::inline_input::EditState::Ended {
                    s.rename_collection = None;
                }
            }
            if let Some(input) = self.row_caches.borrow().group_rename_input.clone() {
                if s.rename_group.is_some() && input.update(cx, |f, cx| f.poll_active(window, cx)) == crate::inline_input::EditState::Ended {
                    s.rename_group = None;
                }
            }
        }

        // 根：按下任意处关掉内联菜单；松开时提交拖拽。
        // ⛔ 这里**不要**铺一块矩形 `bg(t.panel)`：`variables_table` 的卡片 `pad = 0`，这块矩形会
        // 正好铺满整张卡片、把卡片自己的圆角（`Popover` 的 `rounded(RADIUS)`）整个盖掉 —— 渲染出来
        // 就是直角。egui 的 `Frame::window` 圆角之所以露得出来，正是因为内容层不铺满底色。
        // 底色由外层卡片提供（同为 `t.panel`），这里保持透明即可。
        let mut root = div()
            .relative()
            .w_full()
            .h(pt(body_h))
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, {
                let st = st.clone();
                let caches = self.row_caches.clone();
                move |_e: &MouseDownEvent, window, cx| {
                    // 点面板内任意非输入框位置 = 失焦提交（与根视图 `commit_edits` 同款）。
                    // 普通 div 不会夺走 TextEdit 的焦点，所以必须显式 finish_edit；输入框自己
                    // 的 on_mouse_down 已 stop_propagation，不会走到这里。
                    let inputs = {
                        let c = caches.borrow();
                        [c.rename_input.clone(), c.mode_rename_input.clone(), c.collection_rename_input.clone(), c.group_rename_input.clone()]
                    };
                    for input in inputs.into_iter().flatten() {
                        input.update(cx, |f, cx| f.finish_edit(window, cx));
                    }
                    // 点面板内任意非输入框位置都释放键盘焦点：否则焦点一直留在搜索框/改名框里，
                    // 后续 Ctrl+C/V/Z 会被那个输入框吞掉，表格的行列复制粘贴和撤销重做就成了
                    // 死代码（用户 2026-11-17）。
                    window.blur();
                    st.borrow_mut().menu = None;
                }
            })
            .on_mouse_move({
                let st = st.clone();
                move |e: &MouseMoveEvent, _, _| Self::on_move(&st, e.position)
            })
            .on_mouse_up(MouseButton::Left, {
                let st = st.clone();
                let cmds = cmds.clone();
                let cid = coll.as_ref().map(|c| c.id.clone()).unwrap_or_default();
                let mode_ids: Vec<String> = coll.as_ref().map(|c| c.modes.iter().map(|m| m.id.clone()).collect()).unwrap_or_default();
                move |_e: &MouseUpEvent, _, _| {
                    {
                        let mut s = st.borrow_mut();
                        s.col_resize = None;
                        s.mode_resize = None;
                        s.resize = None;
                        s.sidebar_resize = None;
                        s.split_resize = None;
                        s.hscroll_drag = None;
                        s.details_drag = None;
                    }
                    let d = st.borrow_mut().drag.take();
                    if let Some(d) = d {
                        Self::commit_drag(&st, &d, &cmds);
                    }
                    // mode 列拖动：按**实际列宽**累加，越过下一列宽度就前移/后移一位。
                    let (md, mode_ws) = {
                        let mut s = st.borrow_mut();
                        (s.mode_drag.take(), s.mode_ws.clone())
                    };
                    if let Some((from, x0, x1)) = md {
                        let dx = x1 - x0;
                        if !mode_ids.is_empty() && dx.abs() > 8.0 {
                            let mut to = from;
                            if dx > 0.0 {
                                let mut acc = 0.0;
                                for k in from..mode_ids.len().saturating_sub(1) {
                                    acc += mode_ws.get(k).copied().unwrap_or(MODE_W);
                                    if dx >= acc {
                                        to = k + 1;
                                    } else {
                                        break;
                                    }
                                }
                            } else {
                                let mut acc = 0.0;
                                for k in (0..from).rev() {
                                    acc += mode_ws.get(k).copied().unwrap_or(MODE_W);
                                    if -dx >= acc {
                                        to = k;
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if to != from {
                                if let Some(mid) = mode_ids.get(from) {
                                    cmds.push(Cmd { id: "variable.moveMode", params: Some(json!({ "collection": cid, "mode": mid, "to": to })) });
                                }
                            }
                        }
                    }
                }
            });

        root = root.child(self.header(&st, &data, full, drag_start));
        let body = div().flex().flex_row().flex_1().min_h(pt(0.0)).w_full();
        let body = if sidebar {
            body.child(self.sidebar(&st, &data, cx, window, body_h)).child(div().flex_none().w(pt(1.0)).h_full().bg(rgba(t.border)))
        } else {
            body
        };
        // 表格在窗口里的左缘：卡片原点 + 侧栏宽度（+1pt 分隔线）。
        let table_left = f32::from(origin.x) + if sidebar { sidebar_w + 1.0 } else { 0.0 };
        // 表格和底栏是**同一列的上下两块**（用户 2026-11-17）：左边侧栏整列延伸到底。
        let table_col = div()
            .flex_1()
            .min_w(pt(0.0))
            .h_full()
            .flex()
            .flex_col()
            .child(self.table(&st, &coll, cx, window, body_h, table_w, table_left))
            .child(self.bottom_bar(&st, &coll));
        root = root.child(body.child(table_col));
        root = root.child(self.menu_overlay(&st, origin));
        root = root.child(self.details_overlay(&st, cx, viewport));
        // 记录 root 在窗口里的真实矩形：行拖动要画的插入线，落点 y 是窗口坐标，
        // 而插入线挂在 root 这个 relative 容器里，必须换算成 root 局部坐标才画得对。
        // 跨帧保留的 root 矩形（prepaint 写入，下一帧读；每帧新建会永远读到 None）。
        let root_bounds = st.borrow().root_bounds.clone();
        let rb = root_bounds.clone();
        root = root.child(gpui::canvas(move |bounds, _, _| rb.set(Some(bounds)), |_, _, _, _| {}).absolute().top_0().left_0().size_full());
        if let Some(el) = self.drag_line(&st, root_bounds.get()) {
            root = root.child(el);
        }
        // 8 个 resize handle（四边 + 四角）。方向位：E=1 W=2 S=4 N=8。
        let (sw, sh) = st.borrow().size.unwrap_or((WINDOW_W, WINDOW_H));
        let handles: [(u8, Div); 8] = [
            (1, div().absolute().right(pt(0.0)).top(pt(0.0)).w(pt(6.0)).h_full()),
            (2, div().absolute().left(pt(0.0)).top(pt(0.0)).w(pt(6.0)).h_full()),
            (4, div().absolute().bottom(pt(0.0)).left(pt(0.0)).h(pt(6.0)).w_full()),
            (8, div().absolute().top(pt(0.0)).left(pt(0.0)).h(pt(6.0)).w_full()),
            (5, div().absolute().right(pt(0.0)).bottom(pt(0.0)).size(pt(12.0))),
            (6, div().absolute().left(pt(0.0)).bottom(pt(0.0)).size(pt(12.0))),
            (9, div().absolute().right(pt(0.0)).top(pt(0.0)).size(pt(12.0))),
            (10, div().absolute().left(pt(0.0)).top(pt(0.0)).size(pt(12.0))),
        ];
        for (dir, el) in handles {
            let st = st.clone();
            // 悬停边/角时显示对应方向的缩放光标，而不是默认箭头。
            let cursor = match dir {
                1 | 2 => CursorStyle::ResizeLeftRight,
                4 | 8 => CursorStyle::ResizeUpDown,
                5 | 10 => CursorStyle::ResizeUpLeftDownRight,
                6 | 9 => CursorStyle::ResizeUpRightDownLeft,
                _ => CursorStyle::Arrow,
            };
            root = root.child(el.cursor(cursor).on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _, _| {
                st.borrow_mut().resize = Some((dir, f32::from(e.position.x), f32::from(e.position.y), sw, sh));
            }));
        }
        let _ = full;
        root.into_any_element()
    }

    // ── header ────────────────────────────────────────────────────────────────────────────
    /// 第二层工具条。**标题与关闭按钮由外层通用 `Popover` 提供**（"Variables" + 计数 chip + X），
    /// 这里不再重复画一个 "Local variables" 标题或第二个 X —— 那会叠成 80pt 的双层 header。
    fn header(&mut self, st: &State, _data: &VariablesData, full: bool, drag_start: Option<DragStart>) -> Div {
        let t = tokens();
        let filtered = st.borrow().hidden.iter().any(|&h| h);
        let mut h = div()
            .flex_none()
            .w_full()
            .h(pt(HEADER_H))
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .pl(pt(16.0))
            // 右侧给浮在右上角的 X 留出 32pt，最后一个工具按钮不会被盖住。
            .pr(pt(48.0))
            .border_b_1()
            .border_color(rgba(t.border));
        // 侧栏显示 / 隐藏（egui `vars_sidebar`）。
        // 标题 + 计数 chip 画进工具条（外层通用 Popover 已改成"整窗一个插槽"，
        // 不再单独占一行标题栏）。
        let title_group = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(tr("Variables")))
            .child(
                div()
                    .px(pt(5.0))
                    .h(pt(16.0))
                    .rounded(pt(4.0))
                    .bg(rgba(t.hover))
                    .flex()
                    .items_center()
                    .text_size(pt(9.0))
                    .text_color(rgba(t.text_secondary))
                    .child(format!("{}", self.total())),
            );
        // ⛔ 整卡拖动只绑在**头部左侧**（egui `vars_drag`：标题到工具按钮之前那块），
        // 不是整张卡片。绑整卡会让「点表格行 / 点按钮」也把窗口拖走（用户 2026-11-07）。
        // 标题 + 侧栏开关 + 之后的空白一起放进 `left`（flex_1），整块都能拖。
        let mut left = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .flex_1()
            .min_w(pt(0.0))
            .child(title_group)
            // 注意：`Icon::Panel` 在 icon.rs 里仍是圆点占位（未实现），用已实现的 `Icon::Panels`。
            .child(IconButton::new(Icon::Panels).size(24.0).on_click({
                let st = st.clone();
                move |_, _, _| {
                    let mut s = st.borrow_mut();
                    s.sidebar = !s.sidebar;
                }
            }));
        if let Some(d) = drag_start {
            left = left.cursor_grab().on_mouse_down(MouseButton::Left, move |e, w, cx| d(e, w, cx));
        }
        h = h.child(left);

        // Import… / Export… 是两个独立按钮（egui header 的 right_to_left 条），不是一个混合菜单。
        let import_cmds = self.cmds.clone();
        h = h.child(
            div()
                .flex_none()
                .h(pt(24.0))
                .px(pt(6.0))
                .flex()
                .items_center()
                .rounded(pt(4.0))
                .cursor_pointer()
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .hover(move |s| s.bg(rgba(t.hover)))
                .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, _| Self::import_dialog(&import_cmds))
                .child(tr("Import…")),
        );
        h = h.child(
            div()
                .flex_none()
                .h(pt(24.0))
                .px(pt(6.0))
                .flex()
                .items_center()
                .rounded(pt(4.0))
                .cursor_pointer()
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .hover(move |s| s.bg(rgba(t.hover)))
                .on_mouse_down(MouseButton::Left, {
                    let st = st.clone();
                    move |e: &MouseDownEvent, _, _| {
                        st.borrow_mut().menu = Some(Menu::at(
                            f32::from(e.position.x),
                            f32::from(e.position.y),
                            vec![
                                MenuItem::new(tr("Variables JSON…"), "ui.export", json!({ "format": "json" })),
                                MenuItem::new(tr("CSS custom properties…"), "ui.export", json!({ "format": "css" })),
                                MenuItem::new(tr("Dart constants for Flutter…"), "ui.export", json!({ "format": "dart" })),
                            ],
                        ));
                    }
                })
                .child(tr("Export…")),
        );

        // 搜索框 174 + 过滤钮 25 拼成 egui 的 199 宽一体控件。
        h = h.child(
            div()
                .flex_none()
                .w(pt(SEARCH_W + FILTER_W))
                .h(pt(24.0))
                .flex()
                .flex_row()
                .child(div().flex_none().w(pt(SEARCH_W)).child(self.search.clone()))
                .child({
                    // 过滤菜单要贴着眼睛按钮的右缘：先用 canvas 记下按钮真实矩形。
                    let st_cap = st.clone();
                    div()
                        .relative()
                        .flex_none()
                        .w(pt(FILTER_W))
                        .h_full()
                        .child(
                            gpui::canvas(
                                move |bounds, _, _| {
                                    st_cap.borrow_mut().filter_anchor = Some(bounds);
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                        )
                        .child(IconButton::new(Icon::Eye).size(24.0).active(filtered).on_click({
                            let st = st.clone();
                            move |_, window, _| {
                                let mut s = st.borrow_mut();
                                let open = s.menu.take().is_none();
                                if open {
                                    let mut items: Vec<MenuItem> = TYPES
                                        .iter()
                                        .enumerate()
                                        .map(|(i, (name, _))| {
                                            MenuItem::new(
                                                format!("{} {}", if s.hidden[i] { "○" } else { "✓" }, name),
                                                "ui.filter",
                                                json!({ "kind": i }),
                                            )
                                        })
                                        .collect();
                                    if s.hidden.iter().any(|&h| h) {
                                        items.push(MenuItem::new("Show every type", "ui.showAll", json!({})));
                                    }
                                    let anchor = s.filter_anchor;
                                    // 眼睛按钮**下方 4pt**，弹窗**右缘**与按钮右缘对齐，宽度随内容。
                                    // 通用 `Menu` 只按左缘定位，所以把锚点左缘挪到「右缘 - 预计宽」。
                                    let w = menu_fit_width(window, &items);
                                    let anchor = anchor.map(|b| Bounds {
                                        origin: point(b.origin.x + b.size.width - px(w), b.origin.y),
                                        size: gpui::size(px(0.0), b.size.height),
                                    });
                                    s.menu = Some(Menu::on(anchor, crate::widgets::RectAlign::BottomStart, 4.0, items));
                                }
                            }
                        }))
                }),
        );
        // Floating / Full
        h = h.child(IconButton::new(Icon::Frame).size(24.0).active(full).on_click({
            let st = st.clone();
            let on_full = self.on_full.clone();
            move |_, _, cx: &mut App| {
                let v = {
                    let mut s = st.borrow_mut();
                    s.full = !s.full;
                    s.full
                };
                on_full(v, cx);
            }
        }));
        h
    }

    fn sidebar(&self, st: &State, data: &VariablesData, cx: &mut App, window: &mut Window, body_h: f32) -> Stateful<Div> {
        let t = tokens();
        let (cur, group_sel, collapsed, library_sel, sidebar_w, split, renaming_coll, renaming_group, selected_all) = {
            let s = st.borrow();
            (
                s.collection.min(data.collections.len().saturating_sub(1)),
                s.group.clone(),
                s.collapsed.clone(),
                s.library.clone(),
                s.sidebar_w,
                s.sidebar_split,
                s.rename_collection.clone(),
                s.rename_group.clone(),
                s.selected_all,
            )
        };
        // egui `sidebar()`：Collections 标题 34 + 独立滚动列表 + 7pt 可拖分隔条 +
        // Groups 标题 34 + 独立滚动列表。两个区各自滚动，Groups 不会被库列表挤到屏幕外。
        // 侧栏一直伸到卡片底部；「+ Create variable」收进表格那一列，和表格上下排列，
        // 左边仍是侧栏（用户 2026-11-17）。所以 body 里只多出侧栏那一列的高度，
        // 底栏的高度算在表格列内部，`avail` 不再扣掉它。
        let avail = (body_h - HEADER_H).max(160.0);
        let split_total = (avail - 34.0 - 7.0 - 34.0).max(80.0);
        let collections_h = (split * split_total).clamp(40.0, (split_total - 40.0).max(40.0));
        let groups_h = (split_total - collections_h).max(40.0);
        // 用户 2026-11-17：侧栏不再自带底色，跟随卡片背景。
        let mut col = div().id("vars-sidebar").flex_none().w(pt(sidebar_w)).h_full().flex().flex_col();

        // ① Collections 标题 + 新建（unique 命名，不再用时间戳）。
        col = col.child(
            div()
                .flex_none()
                .w_full()
                .h(pt(34.0))
                .flex()
                .flex_row()
                .items_center()
                .pl(pt(16.0))
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .child(tr("Collections"))
                .child(div().flex_1().min_w(pt(0.0)))
                .child(IconButton::new(Icon::Plus).id("vars-add-collection").size(24.0).on_click({
                    let cmds = self.cmds.clone();
                    let taken: Vec<String> = data.collections.iter().map(|c| c.name.clone()).collect();
                    move |_, _, _| {
                        let name = unique(tr("Collection"), |n| taken.iter().any(|x| x == n));
                        cmds.push(Cmd { id: "variable.createCollection", params: Some(json!({ "name": name, "modes": ["Mode 1"] })) });
                    }
                })),
        );
        // ② Collections 列表（独立滚动）。
        let coll_ids: Vec<String> = data.collections.iter().map(|c| c.id.clone()).collect();
        let mut cl = div().id("var-collections").flex_none().w_full().h(pt(collections_h)).flex().flex_col().overflow_y_scroll();
        for (i, c) in data.collections.iter().enumerate() {
            let active = library_sel.is_none() && i == cur;
            let mut inner = div().w_full().h(pt(24.0)).flex().flex_row().items_center().pl(pt(8.0)).pr(pt(8.0)).rounded(pt(5.0));
            // 高亮块内缩 4pt（egui `hl = r.shrink2(vec2(0,4))`）；选中时不再叠 hover，避免选中色被覆盖。
            if active {
                inner = inner.bg(rgba(t.selected_row));
            } else {
                let hover = t.hover;
                inner = inner.hover(move |s| s.bg(rgba(hover)));
            }
            inner = if renaming_coll.as_deref() == Some(c.id.as_str()) {
                if let Some(input) = self.row_caches.borrow().collection_rename_input.clone() {
                    inner.child(div().flex_1().min_w(pt(0.0)).h_full().child(input))
                } else {
                    inner.child(div().flex_1().min_w(pt(0.0)).child(c.name.clone()))
                }
            } else {
                inner.child(div().flex_1().min_w(pt(0.0)).overflow_hidden().child(crate::inline_input::static_text(
                    c.name.clone().into(),
                    gpui::Hsla::from(rgba(t.text)),
                    24.0,
                )))
            };
            inner = inner.child(div().text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(format!("{}", c.rows.len())));
            let coll_bounds = self.row_caches.borrow().drag_bounds.clone();
            let coll_bounds_key = c.id.clone();
            cl = cl.child(
                div()
                    .id(SharedString::from(format!("vcoll-{i}")))
                    .relative()
                    .flex_none()
                    .w_full()
                    .h(pt(SIDE_ROW_H))
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(pt(8.0))
                    .on_mouse_down(MouseButton::Left, {
                        let st = st.clone();
                        let ids = coll_ids.clone();
                        let cid = c.id.clone();
                        let caches = self.row_caches.clone();
                        let cmds = self.cmds.clone();
                        let coll_name = c.name.clone();
                        move |e: &MouseDownEvent, window, cx| {
                            let mut s = st.borrow_mut();
                            s.library = None;
                            s.collection = i;
                            s.group = None;
                            s.selected_all = true;
                            s.menu = None;
                            // 双击判定用 GPUI 的原生 `click_count`（与左栏图层 / 页面行同一套），
                            // 不再自己按 400ms 猜双击；并且这一下必须拦冒泡：根视图的
                            // 「点别处就提交」会把刚打开的改名框收掉。
                            if e.click_count >= 2 {
                                cx.stop_propagation();
                                drop(s);
                                Self::start_collection_rename(&caches, &st, &cmds, cid.clone(), coll_name.clone(), window, cx);
                            } else {
                                let rect = caches.borrow().drag_bounds.borrow().get(&cid).copied();
                                s.drag = Some(Self::begin_drag(DragList::Collections, i, &ids, e.position, &[], rect));
                            }
                        }
                    })
                    .on_mouse_down(MouseButton::Right, {
                        let st = st.clone();
                        let cid = c.id.clone();
                        let cname = c.name.clone();
                        move |e: &MouseDownEvent, _, _| {
                            st.borrow_mut().menu = Some(Menu::at(
                                f32::from(e.position.x),
                                f32::from(e.position.y),
                                vec![
                                    MenuItem::new("Rename collection", "ui.renameCollection", json!({ "collection": cid, "name": cname })),
                                    MenuItem::new(tr("Delete collection"), "variable.deleteCollection", json!({ "collection": cid })),
                                ],
                            ));
                        }
                    })
                    .child(
                        gpui::canvas(
                            move |bounds, _, _| {
                                coll_bounds.borrow_mut().insert(coll_bounds_key.clone(), bounds);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
                    .child(inner),
            );
        }
        // 已启用库：库标题带图标，集合只读、右键 Extend collection。
        for lib in &data.libraries {
            cl = cl.child(
                div()
                    .flex_none()
                    .w_full()
                    .h(pt(26.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(6.0))
                    .pl(pt(22.0))
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text_secondary))
                    .child(paint_icon(Icon::Component, rgba(t.text_secondary), 12.0))
                    .child(lib.name.clone()),
            );
            for (i, lc) in lib.collections.iter().enumerate() {
                let active = library_sel.as_deref() == Some(lib.id.as_str()) && i == cur;
                let lid = lib.id.clone();
                let cid = lc.id.clone();
                let count = lc.rows.len();
                let name = lc.name.clone();
                let mut inner = div().w_full().h(pt(24.0)).flex().flex_row().items_center().pl(pt(26.0)).pr(pt(8.0)).rounded(pt(5.0));
                if active {
                    inner = inner.bg(rgba(t.selected_row));
                } else {
                    let hover = t.hover;
                    inner = inner.hover(move |s| s.bg(rgba(hover)));
                }
                inner = inner
                    .child(div().flex_1().min_w(pt(0.0)).overflow_hidden().text_size(pt(11.0)).text_color(rgba(t.text)).child(name))
                    .child(div().text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(format!("{count}")));
                cl = cl.child(
                    div()
                        .flex_none()
                        .w_full()
                        .h(pt(SIDE_ROW_H))
                        .flex()
                        .flex_row()
                        .items_center()
                        .px(pt(8.0))
                        .on_mouse_down(MouseButton::Left, {
                            let st = st.clone();
                            move |_: &MouseDownEvent, _, _| {
                                let mut s = st.borrow_mut();
                                s.library = Some(lid.clone());
                                s.collection = i;
                                s.group = None;
                                s.selected_all = true;
                                s.query.clear();
                                s.menu = None;
                            }
                        })
                        .on_mouse_down(MouseButton::Right, {
                            let st = st.clone();
                            let lid_r = lib.id.clone();
                            move |e: &MouseDownEvent, _, _| {
                                st.borrow_mut().menu = Some(Menu::at(
                                    f32::from(e.position.x),
                                    f32::from(e.position.y),
                                    vec![MenuItem::new(
                                        tr("Extend collection"),
                                        "library.extendCollection",
                                        json!({ "library": lid_r, "collection": cid }),
                                    )],
                                ));
                            }
                        })
                        .child(inner),
                );
            }
        }
        col = col.child(cl);

        // ③ 7pt 可拖分隔条（egui `vars_split`）。
        col = col.child(
            div()
                .flex_none()
                .w_full()
                .h(pt(7.0))
                .flex()
                .items_center()
                .cursor(CursorStyle::ResizeUpDown)
                .child(div().w_full().h(pt(1.0)).bg(rgba(t.border)))
                .on_mouse_down(MouseButton::Left, {
                    let st = st.clone();
                    move |e: &MouseDownEvent, _, _| {
                        let mut s = st.borrow_mut();
                        s.split_resize = Some((f32::from(e.position.y), s.sidebar_split));
                    }
                }),
        );
        // ④ Groups 标题。
        col = col.child(
            div()
                .flex_none()
                .w_full()
                .h(pt(34.0))
                .flex()
                .items_center()
                .pl(pt(16.0))
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .child(tr("Groups")),
        );
        let coll_snap = match &library_sel {
            Some(l) => data.libraries.iter().find(|x| &x.id == l).and_then(|x| x.collections.get(cur)).cloned(),
            None => data.collections.get(cur).cloned(),
        };
        // 显示顺序必须与拖动落点使用同一份列表：`group_order` 以前只用于表格排序，
        // 侧栏仍按名字排序 —— 拖动线画到了新位置，松手后侧栏纹丝不动（用户 2026-11-17：
        // 「拖动和实际位移不匹配」）。这里先按 `group_order` 排，再过滤掉祖先折叠的子组。
        let mut groups = coll_snap.as_ref().map(|c| c.groups.clone()).unwrap_or_default();
        {
            let ord = st.borrow().group_order.clone();
            groups.sort_by_key(|g| group_rank(g, &ord));
        }
        let visible_groups: Vec<String> = groups.iter().filter(|g| !collapsed.iter().any(|c| g.starts_with(&format!("{c}/")))).cloned().collect();
        let group_ids: Vec<String> = visible_groups.clone();
        // 行快照：虚拟列表的渲染闭包必须是 'static，只能拿 owned / Rc 数据。
        #[derive(Clone)]
        struct GroupRow {
            full: String,
            short: String,
            level: usize,
            count: usize,
            has_children: bool,
            is_collapsed: bool,
            vars: Vec<String>,
        }
        let group_rows: Vec<GroupRow> = visible_groups
            .iter()
            .map(|g| {
                // 组计数与「Delete group」成员都含子组：与选中父组后表格里显示的行一致。
                let prefix = format!("{g}/");
                let members: Vec<String> = coll_snap
                    .as_ref()
                    .map(|c| c.rows.iter().filter(|r| r.group == *g || r.group.starts_with(&prefix)).map(|r| r.id.clone()).collect())
                    .unwrap_or_default();
                GroupRow {
                    full: g.clone(),
                    short: g.rsplit('/').next().unwrap_or(g).to_string(),
                    level: g.matches('/').count() + 1,
                    count: members.len(),
                    has_children: groups.iter().any(|other| other.starts_with(&prefix)),
                    is_collapsed: collapsed.contains(g),
                    vars: members,
                }
            })
            .collect();
        let row_count = group_rows.len() + 1;
        // 选中父组时，它和它下面的子组连成**一个整块**（左栏图层 / 右栏样式列表的
        // 「父行深色 + 后代浅色」表现，用户 2026-11-17）。树序保证后代行紧跟父组行，
        // 这里先求出这一段的起止下标，渲染时只做圆角/上下留白的收边。
        let sel_run: Option<(usize, usize)> = (!selected_all)
            .then(|| {
                group_sel.as_ref().and_then(|sel| {
                    let start = group_rows.iter().position(|r| r.full == *sel)?;
                    let lvl = group_rows[start].level;
                    let mut end = start;
                    while group_rows.get(end + 1).is_some_and(|next| next.level > lvl) {
                        end += 1;
                    }
                    Some((start, end))
                })
            })
            .flatten();
        let sel_run = Rc::new(sel_run);
        let item_sizes: Rc<Vec<Size<Pixels>>> = Rc::new((0..row_count).map(|_| Size::new(px(sidebar_w), px(SIDE_ROW_H))).collect());
        let group_rows = Rc::new(group_rows);
        let group_ids = Rc::new(group_ids);
        let caches = self.row_caches.clone();
        let cmds = self.cmds.clone();
        let st2 = st.clone();
        let scroll = st.borrow().groups_scroll.clone();
        let coll_snap_r = Rc::new(coll_snap.clone());
        let renaming_r = Rc::new(renaming_group.clone());
        let gsel_r = Rc::new(group_sel.clone());
        let sel_run_r = sel_run.clone();
        let all_sel = selected_all;
        // Groups 列表与左栏图层 / 右栏样式同一套虚拟化：只渲染可见行，
        // 拖动行矩形来自每行 canvas prepaint 的真实 bounds（不再按固定行高硬算）。
        let gl = div().id("var-groups").flex_none().w_full().h(pt(groups_h)).child(v_virtual_list(
            "var-groups-vlist",
            &scroll,
            item_sizes,
            move |range, _window, cx| {
                let t = tokens();
                let mut out = Vec::with_capacity(range.end - range.start);
                for ix in range {
                    if ix == 0 {
                        let mut all_inner = div().w_full().h(pt(24.0)).flex().items_center().pl(pt(8.0)).rounded(pt(5.0));
                        if all_sel {
                            all_inner = all_inner.bg(rgba(t.selected_row));
                        } else {
                            let hover = t.hover;
                            all_inner = all_inner.hover(move |s| s.bg(rgba(hover)));
                        }
                        all_inner = all_inner.child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("All variables")));
                        out.push(
                            div()
                                .flex_none()
                                .w_full()
                                .h(pt(SIDE_ROW_H))
                                .flex()
                                .items_center()
                                .px(pt(8.0))
                                .on_mouse_down(MouseButton::Left, {
                                    let st = st2.clone();
                                    move |_: &MouseDownEvent, _, _| {
                                        let mut s = st.borrow_mut();
                                        s.group = None;
                                        s.selected_all = true;
                                        s.menu = None;
                                    }
                                })
                                .child(all_inner)
                                .into_any_element(),
                        );
                        continue;
                    }
                    let Some(gr) = group_rows.get(ix - 1) else { continue };
                    let g = gr.full.clone();
                    let active = gsel_r.as_deref() == Some(g.as_str()) && !all_sel;
                    // 选中父组时，它的子组也算「选中块」的一部分（与右栏样式列表同一套：
                    // 父行 `selected_row`、子行 `selected_row_child`，用户 2026-11-17）。
                    let under = !all_sel && gsel_r.as_deref().is_some_and(|sel| g.starts_with(&format!("{sel}/")));
                    let mut inner = div().w_full().flex().flex_row().items_center();
                    // 选中父组 = 父行深色 + 后代浅色，连成**一整块**（左栏图层 / 右栏样式
                    // 列表同一表现）：块内行上下不留白，只有整块的顶/底收 4pt 并加圆角。
                    let in_run = active || under;
                    let run_top = in_run && sel_run_r.as_ref().is_some_and(|(s, _)| ix - 1 == s);
                    let run_bottom = in_run && sel_run_r.as_ref().is_some_and(|(_, e)| ix - 1 == e);
                    if in_run {
                        let (top_gap, bottom_gap) = (if run_top { 4.0 } else { 0.0 }, if run_bottom { 4.0 } else { 0.0 });
                        inner = inner.h(pt(SIDE_ROW_H - top_gap - bottom_gap)).mt(pt(top_gap)).mb(pt(bottom_gap));
                        if run_top {
                            inner = inner.rounded_t(pt(5.0));
                        }
                        if run_bottom {
                            inner = inner.rounded_b(pt(5.0));
                        }
                    } else {
                        inner = inner.h(pt(24.0)).rounded(pt(5.0));
                    }
                    // 高亮块**不跟缩进走**：整行满宽，缩进只作用在内容上。
                    inner = inner.pl(pt((gr.level - 1) as f32 * 24.0));
                    if active {
                        inner = inner.bg(rgba(t.selected_row));
                    } else if under {
                        inner = inner.bg(rgba(t.selected_row_child));
                    } else {
                        let hover = t.hover;
                        inner = inner.hover(move |s| s.bg(rgba(hover)));
                    }
                    // Chevron 自己吞掉按下并切换折叠；否则这次按下会落到整行的 handler
                    // 上变成「选中 + 开始拖动」，用户根本无法折叠/展开嵌套组
                    // （CodeRabbit PR#29）。wake 下一帧用新的 collapsed 重绘图标。
                    let st_chev = st2.clone();
                    let g_chev = g.clone();
                    let has_children = gr.has_children;
                    let chev_wake = cmds.clone();
                    inner = inner.child(
                        div()
                            .flex_none()
                            .w(pt(24.0))
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
                                if has_children {
                                    let mut s = st_chev.borrow_mut();
                                    if s.collapsed.contains(&g_chev) {
                                        s.collapsed.remove(&g_chev);
                                    } else {
                                        s.collapsed.insert(g_chev.clone());
                                    }
                                }
                                cx.stop_propagation();
                                chev_wake.wake();
                            })
                            .child(if gr.has_children {
                                paint_icon(if gr.is_collapsed { Icon::ChevronRight } else { Icon::ChevronDown }, rgba(t.text_secondary), 12.0)
                                    .into_any_element()
                            } else {
                                div().into_any_element()
                            }),
                    );
                    inner = if renaming_r.as_deref() == Some(g.as_str()) {
                        if let Some(input) = caches.borrow().group_rename_input.clone() {
                            // 提交时要用组内成员：这里先把 (变量 id, 组内剩余名) 填进缓存。
                            if let Some(cs) = coll_snap_r.as_ref() {
                                let prefix = format!("{g}/");
                                *caches.borrow().group_rename_members.borrow_mut() = cs
                                    .rows
                                    .iter()
                                    // 继承来的变量引擎拒绝改名，一条失败会让整批回滚、本地
                                    // 成员也改不掉（CodeRabbit #30/#31/#32）。组删除已这样过滤。
                                    .filter(|r| r.full_name.starts_with(&prefix) && !r.inherited)
                                    .map(|r| (r.id.clone(), r.full_name[prefix.len()..].to_string()))
                                    .collect();
                            }
                            inner.child(div().flex_1().min_w(pt(0.0)).h_full().child(input))
                        } else {
                            inner.child(gr.short.clone())
                        }
                    } else {
                        inner.child(div().flex_1().min_w(pt(0.0)).overflow_hidden().child(crate::inline_input::static_text(
                            gr.short.clone().into(),
                            gpui::Hsla::from(rgba(t.text)),
                            24.0,
                        )))
                    };
                    inner = inner.child(div().pr(pt(8.0)).text_size(pt(10.0)).text_color(rgba(t.text_tertiary)).child(format!("{}", gr.count)));
                    let g_toggle = g.clone();
                    let g_sel = g.clone();
                    let g_dbl = g.clone();
                    let g_bounds_key = g.clone();
                    let short_for_rename = gr.short.clone();
                    let group_bounds = caches.borrow().drag_bounds.clone();
                    let vars = gr.vars.clone();
                    let idx = ix - 1;
                    let ids = group_ids.clone();
                    let caches_row = caches.clone();
                    let cmds_row = cmds.clone();
                    let st_row = st2.clone();
                    out.push(
                        div()
                            .relative()
                            .flex_none()
                            .w_full()
                            .h(pt(SIDE_ROW_H))
                            .flex()
                            .items_center()
                            .px(pt(8.0))
                            .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
                                let mut s = st_row.borrow_mut();
                                s.menu = None;
                                // 同集合行：原生 `click_count` 判双击，并拦住这次按下不让根视图提交。
                                if e.click_count >= 2 {
                                    cx.stop_propagation();
                                    drop(s);
                                    Self::start_group_rename(&caches_row, &st_row, &cmds_row, g_dbl.clone(), short_for_rename.clone(), window, cx);
                                } else {
                                    s.group = Some(g_sel.clone());
                                    s.selected_all = false;
                                    let rect = caches_row.borrow().drag_bounds.borrow().get(&g_dbl).copied();
                                    s.drag = Some(Self::begin_drag(DragList::Groups, idx, &ids, e.position, &[], rect));
                                }
                            })
                            .on_mouse_down(MouseButton::Right, {
                                let st = st2.clone();
                                let g_short = gr.short.clone();
                                move |e: &MouseDownEvent, _, _| {
                                    st.borrow_mut().menu = Some(Menu::at(
                                        f32::from(e.position.x),
                                        f32::from(e.position.y),
                                        vec![
                                            MenuItem::new(tr("Rename group"), "ui.renameGroup", json!({ "group": g_toggle.clone(), "name": g_short })),
                                            MenuItem::new(tr("Duplicate group"), "ui.duplicateGroup", json!({ "group": g_toggle.clone() })),
                                            MenuItem::new(tr("Delete group"), "ui.deleteGroup", json!({ "variables": vars })),
                                        ],
                                    ));
                                }
                            })
                            .child(
                                gpui::canvas(
                                    move |bounds, _, _| {
                                        group_bounds.borrow_mut().insert(g_bounds_key.clone(), bounds);
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full(),
                            )
                            .child(inner)
                            .into_any_element(),
                    );
                }
                out
            },
        ));
        col = col.child(gl);
        // ⑥ 侧栏右缘拖动柄（egui `vars_sidebar_edge`，180..420）。
        // 热区必须**以分隔线为中心左右各 3pt**：原来 `right(0)` 让 6pt 全落在线的左侧，
        // 指针在线的右边（表格一侧）拖不动（用户 2026-11-17）。
        col.child(div().absolute().right(pt(-3.0)).top(pt(0.0)).w(pt(6.0)).h_full().cursor(CursorStyle::ResizeLeftRight).on_mouse_down(
            MouseButton::Left,
            {
                let st = st.clone();
                move |e: &MouseDownEvent, _, _| {
                    let mut s = st.borrow_mut();
                    s.sidebar_resize = Some((f32::from(e.position.x), s.sidebar_w));
                }
            },
        ))
    }

    // ── table ─────────────────────────────────────────────────────────────────────────────
    fn table(
        &mut self,
        st: &State,
        coll: &Option<CollectionSnap>,
        cx: &mut App,
        window: &mut Window,
        body_h: f32,
        table_w: f32,
        table_left: f32,
    ) -> Stateful<Div> {
        let t = tokens();
        let Some(c) = coll else {
            return div().id("vars-empty").flex_1().min_w(pt(0.0)).h_full().flex().flex_col().child(
                div()
                    .flex_1()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(pt(12.0))
                    .child(
                        div()
                            .text_size(pt(11.0))
                            .text_color(rgba(t.text_secondary))
                            .child(tr("Variables store reusable values (colours, numbers, strings, booleans) with modes such as Light and Dark.")),
                    )
                    .child(IconButton::new(Icon::Plus).id("vars-add-collection-empty").size(24.0).on_click({
                        let cmds = self.cmds.clone();
                        move |_, _, _| {
                            cmds.push(Cmd { id: "variable.createCollection", params: Some(json!({ "name": "Collection 1", "modes": ["Mode 1"] })) })
                        }
                    })),
            );
        };
        // 列宽数组与集合的 mode 数对齐，算出一个确定的表格总宽：横向滚动靠它。
        {
            let mut s = st.borrow_mut();
            while s.mode_ws.len() < c.modes.len() {
                s.mode_ws.push(200.0);
            }
            s.mode_ws.truncate(c.modes.len());
        }
        let (name_w, mode_ws) = {
            let s = st.borrow();
            (s.name_w, s.mode_ws.clone())
        };
        // egui `Columns::room`：Name 列先让位，保证 name + add + 一段 mode 区一定放进表格宽度。
        // 以前直接用 `name_w` 并且拿**窗口宽**当表格宽，表格会整块超出卡片。
        let name_eff = name_w.clamp(NAME_MIN, NAME_MAX).min((table_w - ADD_W - 2.0).max(0.0));
        // 只有 mode 列横向滚动：Name / add 列固定。`modes_full` 含每列 1pt 网格线。
        let modes_room = (table_w - name_eff - ADD_W - 2.0).max(0.0);
        // 每列的 1pt 网格线是**画在列内部右缘**（`cell` 的 absolute 子节点），不额外占宽；
        // 所以内容总宽就是各列宽之和。以前多加了 1pt/列，插入线和分隔线因此差 n 个像素。
        let modes_full: f32 = mode_ws.iter().sum();
        let max_sx = (modes_full - (modes_room + 1.0)).max(0.0);
        let sx = (-f32::from(st.borrow().hscroll.offset().x)).clamp(0.0, max_sx);
        let mut col = div().flex_1().min_h(pt(0.0)).w_full().flex().flex_col().relative();
        // 表头（底部分隔线 + 每列右缘 1px 网格线）
        let (selected_mode, has_clip_col) = {
            let s = st.borrow();
            (s.selected_mode.clone(), s.clipboard_column.is_some())
        };
        let mut head = div().relative().flex_none().w_full().h(pt(HEAD_ROW_H)).flex().flex_row().items_center();
        let name_w = name_eff;
        // Name 列：双击拖动柄 = 自动适配到最长完整变量名。
        let auto_name_w = {
            let longest = c.rows.iter().map(|r| r.full_name.chars().count()).max().unwrap_or(0) as f32;
            (longest * 7.0 + 40.0 + 24.0).clamp(NAME_MIN, NAME_MAX)
        };
        head = head.child(
            div()
                .relative()
                .flex_none()
                .w(pt(name_w))
                .h_full()
                .flex()
                .items_center()
                .pl(pt(16.0))
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .child(tr("Name"))
                .child(div().absolute().right_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border)))
                // 右缘 6pt 拖动柄；双击自动适配。悬停显示左右拖动光标。
                .child(div().absolute().right(pt(0.0)).top(pt(0.0)).w(pt(6.0)).h_full().cursor(CursorStyle::ResizeLeftRight).on_mouse_down(
                    MouseButton::Left,
                    {
                        let st = st.clone();
                        move |e: &MouseDownEvent, _, cx: &mut App| {
                            // 列宽柄是单元格的子节点：不拦冒泡的话，父级 mode 单元格的
                            // on_mouse_down 会把 `last_mode_down` 覆盖成 mode id，双击判定
                            // 永远等不到同一个键 → 双击列边永远不生效（用户 2026-11-07）。
                            cx.stop_propagation();
                            let mut s = st.borrow_mut();
                            let dbl = s.last_mode_down.as_ref().is_some_and(|(at, id)| id == "__name" && at.elapsed().as_millis() < 400);
                            s.last_mode_down = Some((Instant::now(), "__name".into()));
                            if dbl {
                                s.name_w = auto_name_w;
                            } else {
                                s.col_resize = Some((f32::from(e.position.x), s.name_w));
                            }
                        }
                    },
                )),
        );
        let n_modes = c.modes.len();
        // mode 列区：固定宽度 + 裁切，内容按 `sx` 左移（egui `modes_rect` + `sx`）。
        // Name / add 列在它两侧固定不动，所以横向滚动不会把 Name 列一起滚走。
        let mut modes_head = div().flex_none().w(pt(modes_room)).h_full().relative().overflow_hidden();
        let mut modes_inner = div().absolute().left(pt(-sx)).top_0().h_full().flex().flex_row();
        for (mi, m) in c.modes.iter().enumerate() {
            let renaming = st.borrow().rename_mode.as_deref() == Some(m.id.as_str());
            let mw = mode_ws.get(mi).copied().unwrap_or(MODE_W);
            let is_sel = selected_mode.as_deref() == Some(m.id.as_str());
            // 双击列边自动适配：该列最长值文本宽。
            let auto_mw = {
                let longest = c.rows.iter().map(|r| r.values.get(mi).map(|v| v.text().chars().count()).unwrap_or(0)).max().unwrap_or(0) as f32;
                (longest * 7.0 + 24.0).clamp(MODE_MIN, MODE_MAX)
            };
            let mut cell = div()
                .relative()
                .flex_none()
                .w(pt(mw))
                .h_full()
                .flex()
                .items_center()
                .pl(pt(4.0))
                .overflow_hidden()
                .text_size(pt(11.0))
                .text_color(rgba(if is_sel { t.text } else { t.text_secondary }))
                // 列选中与行选中同一底色（原来是 t.hover，比行选中浅）。
                .bg(rgba(if is_sel { t.selected_row } else { t.panel }));
            cell = if renaming {
                if let Some(input) = self.row_caches.borrow().mode_rename_input.clone() {
                    cell.child(div().flex_1().min_w(pt(0.0)).h_full().child(input))
                } else {
                    cell.child(div().flex_1().min_w(pt(0.0)).h_full().child(crate::inline_input::static_text(
                        m.name.clone().into(),
                        gpui::Hsla::from(rgba(if is_sel { t.text } else { t.text_secondary })),
                        HEAD_ROW_H,
                    )))
                }
            } else {
                cell.child(div().flex_1().min_w(pt(0.0)).h_full().child(crate::inline_input::static_text(
                    m.name.clone().into(),
                    gpui::Hsla::from(rgba(if is_sel { t.text } else { t.text_secondary })),
                    HEAD_ROW_H,
                )))
            };
            let mid = m.id.clone();
            let mname = m.name.clone();
            let cid = c.id.clone();
            cell = cell
                .on_mouse_down(MouseButton::Left, {
                    let st = st.clone();
                    let mid = mid.clone();
                    let mname = mname.clone();
                    let cid = cid.clone();
                    let caches = self.row_caches.clone();
                    let cmds = self.cmds.clone();
                    let editable = c.editable;
                    move |e: &MouseDownEvent, window, cx| {
                        let mut s = st.borrow_mut();
                        s.menu = None;
                        s.selected.clear();
                        s.anchor = None;
                        s.selected_mode = Some(mid.clone());
                        if !editable {
                            return;
                        }
                        // 同左栏：原生 `click_count` 判双击；这一下拦住冒泡，否则根视图
                        // 的「点别处就提交」会把刚打开的改名框收掉（要点三四次才进输入态）。
                        if e.click_count >= 2 {
                            cx.stop_propagation();
                            drop(s);
                            Self::start_mode_rename(&caches, &st, &cmds, cid.clone(), mid.clone(), mname.clone(), window, cx);
                        } else {
                            // 同上：交给根节点统一提交，别提前清掉改名目标。
                            let x = f32::from(e.position.x);
                            s.mode_drag = Some((mi, x, x));
                        }
                    }
                })
                .on_mouse_down(MouseButton::Right, {
                    let st = st.clone();
                    let mid = mid.clone();
                    let cid = cid.clone();
                    let mname = mname.clone();
                    let editable = c.editable;
                    move |e: &MouseDownEvent, _, _| {
                        if !editable {
                            return;
                        }
                        let mut items = vec![
                            MenuItem::new(tr("Rename mode"), "ui.renameMode", json!({ "mode": mid.clone(), "name": mname.clone() })),
                            MenuItem::new(tr("Add mode"), "variable.addMode", json!({ "collection": cid.clone(), "name": tr("Mode") })),
                            MenuItem::new(
                                "Duplicate mode",
                                "variable.addMode",
                                json!({ "collection": cid.clone(), "name": format!("{} copy", mname), "from": mid.clone() }),
                            ),
                        ];
                        if mi > 0 {
                            items.push(MenuItem::new(
                                tr("Set as default"),
                                "variable.moveMode",
                                json!({ "collection": cid.clone(), "mode": mid.clone(), "to": 0 }),
                            ));
                            items.push(MenuItem::new(
                                "Move left",
                                "variable.moveMode",
                                json!({ "collection": cid.clone(), "mode": mid.clone(), "to": mi - 1 }),
                            ));
                        }
                        if mi + 1 < n_modes {
                            items.push(MenuItem::new(
                                "Move right",
                                "variable.moveMode",
                                json!({ "collection": cid.clone(), "mode": mid.clone(), "to": mi + 1 }),
                            ));
                        }
                        items.push(MenuItem::new("Copy column", "ui.copyColumn", json!({ "mode": mid.clone() })));
                        if has_clip_col {
                            items.push(MenuItem::new("Paste column", "ui.pasteColumn", json!({ "mode": mid.clone() })));
                        }
                        if n_modes > 1 {
                            items.push(MenuItem::new(
                                tr("Delete mode"),
                                "variable.removeMode",
                                json!({ "collection": cid.clone(), "mode": mid.clone() }),
                            ));
                        }
                        st.borrow_mut().menu = Some(Menu::at(f32::from(e.position.x), f32::from(e.position.y), items));
                    }
                });
            cell = cell.child(div().absolute().right_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border)));
            cell = cell.child(div().absolute().right(pt(0.0)).top(pt(0.0)).w(pt(6.0)).h_full().cursor(CursorStyle::ResizeLeftRight).on_mouse_down(
                MouseButton::Left,
                {
                    let st = st.clone();
                    let edge_key = format!("{mid}-edge");
                    move |e: &MouseDownEvent, _, cx: &mut App| {
                        // 同 Name 列：先拦冒泡，父级单元格不会再把 `last_mode_down`
                        // 覆盖成 mode id（否则双击列边自动宽度永远不生效）。
                        cx.stop_propagation();
                        let mut s = st.borrow_mut();
                        let dbl = s.last_mode_down.as_ref().is_some_and(|(at, id)| id == &edge_key && at.elapsed().as_millis() < 400);
                        s.last_mode_down = Some((Instant::now(), edge_key.clone()));
                        if dbl {
                            if let Some(w) = s.mode_ws.get_mut(mi) {
                                *w = auto_mw;
                            }
                        } else {
                            s.mode_resize = Some((mi, f32::from(e.position.x), mw));
                        }
                    }
                },
            ));
            modes_inner = modes_inner.child(cell);
        }
        modes_head = modes_head.child(modes_inner);
        head = head.child(modes_head);
        let mut add_cell = div().relative().flex_none().w(pt(ADD_W)).h_full().flex().items_center().justify_center();
        if c.editable {
            add_cell = add_cell.child(IconButton::new(Icon::Plus).id("vars-add-mode").size(24.0).on_click({
                let cmds = self.cmds.clone();
                let cid = c.id.clone();
                move |_, _, _| cmds.push(Cmd { id: "variable.addMode", params: Some(json!({ "collection": cid, "name": tr("Mode") })) })
            }));
        }
        add_cell = add_cell.child(div().absolute().left_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border)));
        head = head.child(add_cell);
        head = head.child(div().absolute().bottom_0().left_0().w_full().h(pt(1.0)).bg(rgba(t.border)));
        col = col.child(head);

        // mode 列拖动：竖插入线。判定与 `end_resize` 完全一致（越过下一列的实际列宽就换位），
        // 只是换成「落到第几个缝」再换算成表格内的 x。行拖动画横线，列拖动画竖线。
        let mode_line_x = {
            let md = st.borrow().mode_drag;
            md.and_then(|(from, x0, x1)| {
                let dx = x1 - x0;
                if dx.abs() <= 4.0 {
                    return None;
                }
                let mut to = from;
                if dx > 0.0 {
                    let mut acc = 0.0;
                    for k in from..n_modes.saturating_sub(1) {
                        acc += mode_ws.get(k).copied().unwrap_or(MODE_W);
                        if dx >= acc {
                            to = k + 1;
                        } else {
                            break;
                        }
                    }
                } else {
                    let mut acc = 0.0;
                    for k in (0..from).rev() {
                        acc += mode_ws.get(k).copied().unwrap_or(MODE_W);
                        if -dx >= acc {
                            to = k;
                        } else {
                            break;
                        }
                    }
                }
                if to == from {
                    return None;
                }
                let insert = if to > from { to + 1 } else { to };
                // 分隔线左缘 = 前 insert 列宽度之和 − 1（1pt 线画在该列内部右缘）。
                let acc: f32 = (0..insert).map(|k| mode_ws.get(k).copied().unwrap_or(MODE_W)).sum();
                let pos = acc - sx;
                if pos < 0.0 || pos > modes_room {
                    return None;
                }
                Some(name_w + pos)
            })
        };
        // mode 列插入线在函数末尾画：必须盖在行区与其 1pt 竖分隔线之上（用户 2026-11-17）。

        // notice_row：库集合只读 / 扩展集合提示（egui `notice_row`）。
        if !c.editable || c.extends.is_some() {
            let msg = if !c.editable { "Library collection · read-only" } else { "Extended collection" };
            let mut n = div()
                .flex_none()
                .w_full()
                .h(pt(32.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(8.0))
                .pl(pt(16.0))
                .pr(pt(8.0))
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .child(msg);
            if !c.editable {
                let cmds = self.cmds.clone();
                let lib = c.library.clone().unwrap_or_default();
                let cid = c.id.clone();
                n = n.child(div().flex_1().min_w(pt(0.0))).child(
                    div()
                        .h(pt(22.0))
                        .px(pt(8.0))
                        .rounded(pt(4.0))
                        .border_1()
                        .border_color(rgba(t.border_strong))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .child(tr("Extend collection"))
                        .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, _| {
                            cmds.push(Cmd { id: "library.extendCollection", params: Some(json!({ "library": lib, "collection": cid })) });
                        }),
                );
            }
            col = col.child(n);
        }

        // 行区（虚拟列表：只渲染可视区间内的条目，几千行也不卡）。
        // 底部常驻 12pt 水平滚动条带（egui `HSCROLL_H`），所以行区高度要扣掉它。
        let rows_h = (body_h - HEADER_H - HEAD_ROW_H - BOTTOM_H - HSCROLL_H).max(96.0);
        let visible = self.visible_rows(c);

        // 把变量行（含分组间隔 / 分组标题）摊平成一个固定顺序的「条目」序列，
        // 每项高度已知，虚拟列表据此做前缀和 + 二分求可视区间。
        let collapsed = st.borrow().collapsed.clone();
        let mut items: Vec<RowItem> = Vec::new();
        let mut prev_group: Option<String> = None;
        let mut skip = false;
        // 拖拽行下标（分组标题与变量行共用一条序列）。
        let mut drag_index = 0usize;
        for r in &visible {
            if prev_group.as_deref() != Some(&r.group) {
                prev_group = Some(r.group.clone());
                // 折叠父组要连子孙组一起隐藏：只看精确匹配会让 `Brand` 折叠后
                // `Brand/Dark` 的变量仍在表里（CodeRabbit #30/#31）。按 `/` 边界匹配，
                // 不误伤同前缀的兄弟组（如 `Branding`）。
                skip = group_or_ancestor_collapsed(&r.group, &collapsed);
                if !r.group.is_empty() {
                    items.push(RowItem::Gap);
                    // 分组标题也要进拖拽序列：否则「组上方」没有任何缝，行只能落进组里。
                    items.push(RowItem::GroupHeader { name: r.group.clone(), first: r.id.clone() });
                    drag_index += 1;
                }
            }
            if skip {
                continue;
            }
            items.push(RowItem::Row { index: drag_index, row: r.clone() });
            drag_index += 1;
        }

        // 已渲染行（含分组标题）的拖拽键与「内容坐标」顶边，两者下标一致、且与 `items`
        // 的实际高度推进完全同步。拖动重建矩形必须用这份**过滤后**的列表：把折叠分组里没画
        // 出来的行也塞进去，它们会全落在 y=0，`resolve` 的插入缝就会跳到最上面（用户 2026-11-17）。
        let mut drag_ids: Vec<String> = Vec::with_capacity(drag_index);
        let mut drag_tops: Vec<f32> = Vec::with_capacity(drag_index);
        {
            let mut y = 0.0f32;
            for it in &items {
                match it {
                    RowItem::GroupHeader { name, first, .. } => {
                        drag_ids.push(drag_group_key(name, first));
                        drag_tops.push(y);
                    }
                    RowItem::Row { row, .. } => {
                        drag_ids.push(row.id.clone());
                        drag_tops.push(y);
                    }
                    RowItem::Gap => {}
                }
                y += it.height();
            }
        }
        let drag_ids = Rc::new(drag_ids);
        let row_tops: Rc<Vec<f32>> = Rc::new(drag_tops);

        if visible.is_empty() {
            col = col.child(
                div()
                    .flex_none()
                    .w_full()
                    .h(pt(ROW_H))
                    .flex()
                    .items_center()
                    .pl(pt(16.0))
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text_secondary))
                    .child(tr("No matching variables")),
            );
        } else {
            // 每项尺寸：宽 = 表格可用宽度（行内 Name/add 固定、mode 区自己裁切）；
            // 高 = 该条目类型固定高度。
            let item_sizes: Rc<Vec<Size<Pixels>>> = Rc::new(items.iter().map(|it| size(px(table_w), px(it.height()))).collect());
            let scroll = st.borrow().scroll.clone();
            let st2 = st.clone();
            let cmds2 = self.cmds.clone();
            let caches2 = self.row_caches.clone();
            let c2 = c.clone();
            let ids2 = drag_ids.clone();
            let bulk_rows = std::rc::Rc::new(c.rows.iter().filter(|x| !x.inherited)
                .map(|x| (x.id.clone(), x.full_name.clone(), x.kind)).collect::<Vec<_>>());
            let bulk_groups = std::rc::Rc::new(c.groups.clone());
            let list = v_virtual_list("vars-rows", &scroll, item_sizes, move |range, _window, cx| {
                let mut out: Vec<AnyElement> = Vec::new();
                for ix in range {
                    let Some(item) = items.get(ix) else { continue };
                    let el: AnyElement = match item {
                        RowItem::Gap => div().flex_none().w(px(table_w)).h(px(GROUP_GAP)).into_any_element(),
                        RowItem::GroupHeader { name, .. } => group_header_div(name).w(px(table_w)).into_any_element(),
                        RowItem::Row { index, row } => {
                            VariablesEditor::var_row(&st2, &cmds2, &caches2, &c2, row, *index, &ids2, row_tops.clone(), name_eff, modes_room, sx, bulk_rows.clone(), bulk_groups.clone(), cx)
                                .w(px(table_w))
                                .into_any_element()
                        }
                    };
                    out.push(el);
                }
                out
            })
            .h(pt(rows_h));
            col = col.child(list);
        }
        // 12pt 水平滚动条带（egui `hscroll`）：只覆盖 mode 区（Name 与 add 之间）。
        let hscroll = st.borrow().hscroll.clone();
        let track_start = name_eff + 9.0;
        let track_w = (table_w - ADD_W - 9.0 - track_start).max(1.0);
        let thumb_w = (track_w * (modes_room / modes_full.max(1.0)).clamp(0.12, 1.0)).min(track_w);
        let max_scroll = max_sx.max(1.0);
        let off = sx.clamp(0.0, max_sx);
        let thumb_x = off / max_scroll * (track_w - thumb_w);
        let bar = div()
            .flex_none()
            .w_full()
            .h(pt(HSCROLL_H))
            .relative()
            .on_mouse_down(MouseButton::Left, {
                let st = st.clone();
                let hscroll = hscroll.clone();
                move |e: &MouseDownEvent, _, _| {
                    let rel = f32::from(e.position.x) - table_left - track_start;
                    let on_thumb = rel >= thumb_x && rel <= thumb_x + thumb_w;
                    let mut s = st.borrow_mut();
                    let start = if on_thumb {
                        sx
                    } else {
                        // 点轨道空白：先跳到指针处，再从那里接着拖。
                        let x = rel.clamp(0.0, track_w);
                        let target = (x / (track_w - thumb_w).max(1.0) * max_sx).clamp(0.0, max_sx);
                        hscroll.set_offset(point(px(-target), px(0.0)));
                        target
                    };
                    s.hscroll_drag = Some((f32::from(e.position.x), start, (track_w - thumb_w).max(1.0), max_sx));
                }
            })
            .child(div().absolute().left(pt(track_start)).top(pt(5.0)).w(pt(track_w)).h(pt(2.0)).bg(rgba(t.border)))
            .child(
                div().absolute().left(pt(track_start + thumb_x)).top(pt(4.0)).w(pt(thumb_w)).h(pt(4.0)).rounded(pt(2.0)).bg(rgba(t.border_strong)),
            );
        // mode 列拖动的竖插入线：最后画，盖在行区与每格右缘的 1pt 竖分隔线之上。
        if let Some(x) = mode_line_x {
            col = col.child(div().absolute().left(pt(x - 1.0)).top_0().bottom_0().w(pt(2.0)).bg(rgba(t.accent)));
        }
        div().id("vars-table").flex_1().min_w(pt(0.0)).min_h(pt(0.0)).flex().flex_col().child(col).child(bar)
    }

    #[allow(clippy::too_many_arguments)]
    fn var_row(
        st: &State,
        cmds: &Cmds,
        caches: &Rc<RefCell<RowCaches>>,
        c: &CollectionSnap,
        r: &VarRow,
        index: usize,
        ids: &[String],
        tops: Rc<Vec<f32>>,
        name_eff: f32,
        modes_room: f32,
        sx: f32,
        bulk_rows: std::rc::Rc<Vec<(String, String, VarKind)>>,
        bulk_groups: std::rc::Rc<Vec<String>>,
        cx: &mut App,
    ) -> Div {
        let t = tokens();
        let (selected, renaming, mode_ws, has_clip, selected_mode) = {
            let s = st.borrow();
            (
                c.editable && s.selected.iter().any(|id| id == &r.id),
                s.rename.as_deref() == Some(r.id.as_str()),
                s.mode_ws.clone(),
                s.clipboard.is_some(),
                s.selected_mode.clone(),
            )
        };
        // 库变量 / 扩展集合里继承来的变量：不可拖拽、改名、删除。
        let movable = c.editable && !r.inherited;
        let has_override = r.overrides.iter().any(|&o| o);
        // Bulk-menu snapshot is built once per table rebuild and shared by every row
        // (see `table`); cloning it per row made large collections allocate quadratically.
        // Bulk-menu snapshot: the rows this collection can act on (inherited ones are changed in
        // their own collection, as in egui `bulk_ids`).
        let bulk_rows: Vec<(String, String, VarKind)> = c
            .rows
            .iter()
            .filter(|x| !x.inherited)
            .map(|x| (x.id.clone(), x.full_name.clone(), x.kind))
            .collect();
        let bulk_groups = c.groups.clone();
        let single_items = if movable {
            let mut v = vec![
                MenuItem::new(tr("Edit variable"), "ui.details", json!({ "variable": r.id })),
                MenuItem::new(tr("Rename"), "ui.rename", json!({ "variable": r.id, "name": r.full_name })),
                MenuItem::new(tr("Copy"), "ui.copy", json!({})),
            ];
            if has_clip {
                v.push(MenuItem::new("Paste", "ui.paste", json!({})));
            }
            v.push(MenuItem::new(tr("Duplicate"), "variable.duplicate", json!({ "variable": r.id })));
            if has_override {
                v.push(MenuItem::new(tr("Reset overrides"), "variable.resetOverride", json!({ "variable": r.id })));
            }
            v.push(MenuItem::new(tr("Delete"), "variable.delete", json!({ "variable": r.id })));
            v
        } else {
            vec![MenuItem::new(tr("Copy"), "ui.copy", json!({}))]
        };
        let mut row = div().relative().flex_none().w_full().h(pt(ROW_H)).flex().flex_row().items_center();
        row = if selected { row.bg(rgba(t.selected_row)) } else { row.hover(move |s| s.bg(rgba(t.hover))) };

        // 行选择 / 双击改名 / 右键菜单 / 拖拽
        let rid = r.id.clone();
        // 改名输入框里必须是**完整名（含组前缀）**：只给叶子名时，再打一个斜杠会把原有组
        // 替换掉而不是挂到它下面；输入与原名相同的「组/叶」还会被引擎当成无变化跳过。
        // 用户 2026-11-17：打斜杠要能加入已有组 / 在组下再加一级。
        let rname = r.full_name.clone();
        row = row
            .on_mouse_down(MouseButton::Left, {
                let st = st.clone();
                let rid = rid.clone();
                let rname = rname.clone();
                let ids = ids.to_vec();
                let caches = caches.clone();
                let cmds = (*cmds).clone();
                move |e: &MouseDownEvent, window, cx| {
                    let mut s = st.borrow_mut();
                    s.menu = None;
                    s.selected_mode = None;
                    select_var(&mut s, &rid, &ids, &e.modifiers);
                    // 与左栏 Layers 行一致：原生 `click_count` 判双击，并在这一下拦住冒泡，
                    // 否则根视图「点别处就提交」会立刻把刚打开的改名框收掉。
                    if e.click_count >= 2 && movable {
                        cx.stop_propagation();
                        drop(s);
                        Self::start_variable_rename(&caches, &st, &cmds, rid.clone(), rname.clone(), window, cx);
                    } else if s.rename.as_deref() != Some(rid.as_str()) {
                        // 不在这里清 `rename`：根节点 on_mouse_down 会先 finish_edit 提交
                        // 正在改名的输入框（提交回调自己 take 掉），这里清掉会让那一下丢提交。
                        if movable {
                            let rect = caches.borrow().drag_bounds.borrow().get(&rid).copied();
                            s.drag = Some(Self::begin_drag(DragList::Variables, index, &ids, e.position, tops.as_slice(), rect));
                        }
                    }
                }
            })
            .on_mouse_down(MouseButton::Right, {
                let st = st.clone();
                let rid = rid.clone();
                let single = single_items.clone();
                let bulk_rows = bulk_rows.clone();
                let bulk_groups = bulk_groups.clone();
                let editable = c.editable;
                move |e: &MouseDownEvent, _, _| {
                    let selected = {
                        let s = st.borrow();
                        editable.then(|| s.selected.clone()).filter(|sel| sel.len() > 1 && sel.iter().any(|x| x == &rid))
                    };
                    let items = if let Some(sel) = selected {
                        let rows: Vec<(String, String, VarKind)> = bulk_rows.iter().filter(|(id, ..)| sel.contains(id)).cloned().collect();
                        if rows.len() > 1 {
                            bulk_menu_items(&rows, &bulk_groups)
                        } else {
                            single.clone()
                        }
                    } else {
                        single.clone()
                    };
                    st.borrow_mut().menu = Some(Menu::at(f32::from(e.position.x), f32::from(e.position.y), items));
                }
            });

        // Name 列（右缘 1px 网格线）
        let mut name_cell = div()
            .relative()
            .flex_none()
            .w(pt(name_eff))
            .h_full()
            .flex()
            .flex_row()
            .items_center()
            .child(div().flex_none().w(pt(40.0)).h_full().flex().items_center().pl(pt(17.0)).child(kind_glyph(r)));
        if renaming {
            // 复用左栏同款 InlineInput：焦点与全选由本帧 poll_active 完成，这里只负责显示。
            if let Some(input) = caches.borrow().rename_input.clone() {
                name_cell = name_cell.child(div().flex_1().min_w(pt(0.0)).h_full().child(input));
            }
        } else {
            // 与编辑态同源绘制（inline_input::static_text），否则进入/退出编辑时文字会上下跳。
            name_cell = name_cell.child(div().flex_1().min_w(pt(0.0)).overflow_hidden().child(crate::inline_input::static_text(
                rname.clone().into(),
                gpui::Hsla::from(rgba(t.text)),
                ROW_H,
            )));
        }
        name_cell = name_cell.child(div().absolute().right_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border)));
        row = row.child(name_cell);

        // 值格（右缘 1px 网格线；override 竖条 + 重置按钮）
        // mode 区同样固定宽度 + 裁切 + 按 `sx` 左移，Name / add 列固定不动。
        let mut modes_strip = div().flex_none().w(pt(modes_room)).h_full().relative().overflow_hidden();
        let mut modes_inner = div().absolute().left(pt(-sx)).top_0().h_full().flex().flex_row();
        for (mi, m) in c.modes.iter().enumerate() {
            let cell = r.values.get(mi).cloned().unwrap_or(VarCell::Empty);
            let overridden = r.overrides.get(mi).copied().unwrap_or(false);
            let cw = mode_ws.get(mi).copied().unwrap_or(MODE_W);
            let mut cellbox = div().relative().flex_none().w(pt(cw)).h_full().flex().flex_row().items_center().gap(pt(4.0)).pl(pt(4.0)).pr(pt(8.0));
            // 用户：列选中样式和行一样 —— 选中整列时，每行的值格都用 selected_row 底色。
            if selected_mode.as_deref() == Some(m.id.as_str()) {
                cellbox = cellbox.bg(rgba(t.selected_row));
            }
            if overridden {
                cellbox = cellbox.child(div().absolute().left(pt(4.0)).top(pt(8.0)).w(pt(2.0)).h(pt(24.0)).bg(rgba(t.accent)));
            }
            cellbox = cellbox.child(Self::value_cell(caches, cmds, st, c, r, m, &cell, cw, cx));
            if overridden && c.editable {
                let vid = r.id.clone();
                let mid = m.id.clone();
                let reset_cmds = cmds.clone();
                cellbox = cellbox.child(IconButton::new(Icon::Detach).size(16.0).on_click(move |_, _, _| {
                    reset_cmds.push(Cmd { id: "variable.resetOverride", params: Some(json!({ "variable": vid, "mode": mid })) });
                }));
            }
            cellbox = cellbox.child(div().absolute().right_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border)));
            modes_inner = modes_inner.child(cellbox);
        }
        modes_strip = modes_strip.child(modes_inner);
        row = row.child(modes_strip);
        // 表尾 40pt 列：Edit variable（打开详情，egui 的 `Icon::Adjust` 在 add 列中心）。
        row = row.child(
            div()
                .relative()
                .flex_none()
                .w(pt(ADD_W))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(IconButton::new(Icon::Pencil).size(20.0).on_click({
                    let st = st.clone();
                    let rid = rid.clone();
                    move |_, _, _| st.borrow_mut().details = Some(rid.clone())
                }))
                .child(div().absolute().left_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border))),
        );
        // 记录行在窗口里的真实矩形：插入线与落点判定用它校准（按下点不等于行顶）。
        {
            let bounds_map = caches.borrow().drag_bounds.clone();
            let bounds_key = rid.clone();
            row = row.child(
                gpui::canvas(
                    move |bounds, _, _| {
                        bounds_map.borrow_mut().insert(bounds_key.clone(), bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        }
        row.child(div().absolute().bottom_0().left_0().w_full().h(pt(1.0)).bg(rgba(t.border)))
    }

    /// 值格的统一底框（24 高 / 圆角 5 / `t.field` 底 / 6pt 内边距）。
    fn value_shell(t: &crate::theme::Tokens) -> Div {
        div()
            .flex_1()
            .min_w(pt(0.0))
            .h(pt(24.0))
            .rounded(pt(5.0))
            .bg(rgba(t.field))
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(6.0))
            .px(pt(6.0))
            .overflow_hidden()
            .text_size(pt(11.0))
            .text_color(rgba(t.text))
    }

    /// 颜色 swatch（16×16 / 圆角 3 / 1px 边框）。
    fn value_swatch(c: Color32, t: &crate::theme::Tokens) -> Div {
        div().flex_none().size(pt(16.0)).rounded(pt(3.0)).bg(rgba(c)).border_1().border_color(rgba(t.border))
    }

    /// 文本型值域（Color 的 hex / String）。
    fn text_value_field(
        caches: &Rc<RefCell<RowCaches>>,
        cmds: &Cmds,
        cx: &mut App,
        key: &str,
        initial: &str,
        vid: &str,
        mid: &str,
        kind: VarKind,
        width: f32,
    ) -> Entity<TextField> {
        if let Some(e) = caches.borrow().value_editors.get(key) {
            return e.clone();
        }
        let f = text_field(cx, width, initial, "", Slot::None, Slot::None);
        let cmds = cmds.clone();
        let vid = vid.to_string();
        let mid = mid.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let value = encode_value(kind, text);
                cmds.push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mid, "value": value })) });
            });
        });
        caches.borrow_mut().value_editors.insert(key.to_string(), f.clone());
        f
    }

    /// 数字型值域：用通用 `number_field`（箭头步进 / 拖动 scrub / 相对表达式）。
    fn number_value_field(
        caches: &Rc<RefCell<RowCaches>>,
        cmds: &Cmds,
        cx: &mut App,
        key: &str,
        value: Option<f64>,
        vid: &str,
        mid: &str,
    ) -> Entity<TextField> {
        if let Some(e) = caches.borrow().value_editors.get(key) {
            return e.clone();
        }
        let f = number_field(cx, VALUE_FIELD_W, value, "", 1.0, Slot::None);
        let cmds = cmds.clone();
        let vid = vid.to_string();
        let mid = mid.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                if let Ok(n) = text.trim().parse::<f64>() {
                    cmds.push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mid, "value": n })) });
                }
            });
        });
        caches.borrow_mut().value_editors.insert(key.to_string(), f.clone());
        f
    }

    fn alpha_value_field(
        caches: &Rc<RefCell<RowCaches>>,
        cmds: &Cmds,
        cx: &mut App,
        key: &str,
        alpha: u8,
        color: Color32,
        vid: &str,
        mid: &str,
    ) -> Entity<TextField> {
        if let Some(e) = caches.borrow().value_editors.get(key) {
            return e.clone();
        }
        let f = number_field(cx, 64.0, Some(alpha as f64), "%", 1.0, Slot::None);
        let cmds = cmds.clone();
        let vid = vid.to_string();
        let mid = mid.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let a = text.trim().trim_end_matches('%').trim().parse::<f64>().unwrap_or(alpha as f64).clamp(0.0, 100.0);
                let a8 = (a / 100.0 * 255.0).round() as u8;
                let hex = format!("#{:02X}{:02X}{:02X}{:02X}", color.0[0], color.0[1], color.0[2], a8);
                cmds.push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mid, "value": hex })) });
            });
        });
        caches.borrow_mut().value_editors.insert(key.to_string(), f.clone());
        f
    }

    fn value_cell(
        caches: &Rc<RefCell<RowCaches>>,
        cmds: &Cmds,
        st: &State,
        c: &CollectionSnap,
        r: &VarRow,
        m: &ModeSnap,
        cell: &VarCell,
        width: f32,
        cx: &mut App,
    ) -> AnyElement {
        // 'static 回调要持有命令队列的所有权，不能用本帧的引用。
        let cmds = cmds.clone();
        let t = tokens();
        // 只读（库集合 / 继承变量）统一走静态样式 —— 所有类型都在这里收口，
        // 不会再被 Color / Bool 分支截胡成"看起来可编辑"。
        if !c.editable {
            let mut d = Self::value_shell(&t);
            match cell {
                VarCell::Color { color, hex, alpha } => {
                    d = d.child(Self::value_swatch(*color, &t));
                    d = d.child(hex.clone());
                    d = d.child(div().text_color(rgba(t.text_secondary)).child(format!("{alpha}%")));
                }
                VarCell::Bool(b) => d = d.child(if *b { tr("True") } else { tr("False") }),
                VarCell::Alias(name) => {
                    d = d.child(paint_icon(Icon::Arrow, rgba(t.text_secondary), 12.0)).child(name.clone());
                }
                VarCell::Empty => {}
                _ => d = d.child(cell.text()),
            }
            return d.into_any_element();
        }
        match cell {
            VarCell::Color { color, hex, alpha } => {
                // 复用通用 `ColorField`（与右栏 Fill 行同源）：swatch / hex / alpha 全由控件画，
                // 不再手写绝对定位的几个输入框。
                let ckey = format!("{}/{}:color", r.id, m.id);
                // 最新 RGB 放在共享 Cell 里：每帧刷新，opacity 回调读它而不是创建时
                // 捕获的旧值（取色器改色后再改透明度不能把旧色写回，CodeRabbit PR#29）。
                let rgb_cell = {
                    let mut c = caches.borrow_mut();
                    c.color_rgb.entry(ckey.clone()).or_insert_with(|| Rc::new(Cell::new([color.0[0], color.0[1], color.0[2], color.0[3]]))).clone()
                };
                rgb_cell.set([color.0[0], color.0[1], color.0[2], color.0[3]]);
                let (hex_ed, op_ed) = if let Some(e) = caches.borrow().color_editors.get(&ckey) {
                    e.clone()
                } else {
                    let vid = r.id.clone();
                    let mid = m.id.clone();
                    let cmds_h = cmds.clone();
                    let cmds_a = cmds.clone();
                    let rgb_for_alpha = rgb_cell.clone();
                    let (h, o) = crate::widgets::ColorField::color_row_editors(
                        cx,
                        hex,
                        *alpha as i64,
                        move |text, _, _| {
                            let v = json!(format!("#{}", text.trim().trim_start_matches('#')));
                            cmds_h.push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mid, "value": v })) });
                        },
                        {
                            let vid = r.id.clone();
                            let mid = m.id.clone();
                            move |pct, _, _| {
                                let a8 = (pct.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8;
                                let rgb = rgb_for_alpha.get();
                                let hex = format!("#{:02X}{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2], a8);
                                cmds_a.push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mid, "value": json!(hex) })) });
                            }
                        },
                    );
                    caches.borrow_mut().color_editors.insert(ckey, (h.clone(), o.clone()));
                    (h, o)
                };
                let mut cf = crate::widgets::ColorField::new(
                    cx,
                    figcraft_color::Paint::solid(figcraft_color::Color::from_rgba8(color.0[0], color.0[1], color.0[2], color.0[3])),
                    crate::widgets::ColorText::Hex(SharedString::from(hex.clone())),
                    width.max(120.0),
                    hex_ed,
                    op_ed,
                );
                // 点 swatch 打开通用取色器，选色直接写回变量（用户 2026-11-17：颜色值格
                // 以前只是静态色块，点了没反应）。取色器走 `WindowState::color`，与右栏 Fill 同源。
                if let Some(popups) = caches.borrow().popups.clone() {
                    let vid = r.id.clone();
                    let mid = m.id.clone();
                    let cmds_pick = cmds.clone();
                    let current = *color;
                    cf = cf.on_swatch(move |pos: Point<Pixels>, _w, cx: &mut App| {
                        let on_pick: Rc<dyn Fn(Color32, &mut Window, &mut App)> = Rc::new({
                            let cmds_pick = cmds_pick.clone();
                            let vid = vid.clone();
                            let mid = mid.clone();
                            move |c: Color32, _w, _cx| {
                                // `Color32::to_hex()` 自带 `#`，再拼一个就是 `##RRGGBB`，
                                // 引擎解析失败 → 颜色永远写不回去（用户 2026-11-17）。
                                // `to_hex_alpha()` 带上透明度，色板里的 alpha 才不会丢。
                                let hex = c.to_hex_alpha();
                                cmds_pick
                                    .push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mid, "value": json!(hex) })) });
                            }
                        });
                        let w = crate::popovers::WindowState::color(cx, pos, current, None, Default::default(), on_pick, cmds_pick.clone());
                        popups.request(w);
                    });
                }
                cf.into_any_element()
            }
            VarCell::Bool(b) => {
                // 布尔值格直接用通用 `Checkbox`（与右栏 Clip content 同一个控件），
                // 不再手画 14pt 方框 + True/False 文本。
                let on = *b;
                let vid = r.id.clone();
                let mid = m.id.clone();
                let cmds = cmds.clone();
                Checkbox::new(gpui::ElementId::Name(format!("{}/{}:bool", r.id, m.id).into()))
                    .label(if on { tr("True") } else { tr("False") })
                    .checked(on)
                    .on_change(move |next: &bool, _w, _cx| {
                        cmds.push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mid, "value": *next })) });
                    })
                    .into_any_element()
            }
            VarCell::Alias(name) => {
                // 别名 chip：点击弹出同类型变量列表（+ Detach）。
                let vid = r.id.clone();
                let mid = m.id.clone();
                let kind = r.kind;
                let candidates: Vec<(String, String)> =
                    c.rows.iter().filter(|x| x.kind == kind && x.id != vid).map(|x| (x.id.clone(), x.full_name.clone())).collect();
                let mut d = Self::value_shell(&t).cursor_pointer();
                d = d.child(
                    div()
                        .flex_1()
                        .min_w(pt(0.0))
                        .h(pt(18.0))
                        .rounded(pt(4.0))
                        .border_1()
                        .border_color(rgba(t.border_strong))
                        .flex()
                        .items_center()
                        .gap(pt(4.0))
                        .px(pt(4.0))
                        .child(paint_icon(Icon::Arrow, rgba(t.text_secondary), 12.0))
                        .child(div().flex_1().min_w(pt(0.0)).overflow_hidden().child(name.clone())),
                );
                d.on_mouse_down(MouseButton::Left, {
                    let st = st.clone();
                    move |e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let items: Vec<MenuItem> = candidates
                            .iter()
                            .map(|(id, name)| MenuItem::new(name.clone(), "ui.setAlias", json!({ "variable": vid, "mode": mid, "alias": id })))
                            .collect();
                        st.borrow_mut().menu = Some(Menu::at(f32::from(e.position.x), f32::from(e.position.y), items));
                    }
                })
                .into_any_element()
            }
            VarCell::Number(s) => {
                let key = format!("{}/{}:num", r.id, m.id);
                let v = s.trim().parse::<f64>().ok();
                // 共用的 `number_field` 自己带边框 / 背景，外面再套 `value_shell` 就是
                // 双层外框、颜色也对不上（用户 2026-11-17）。
                Self::number_value_field(caches, &cmds, cx, &key, v, &r.id, &m.id).into_any_element()
            }
            VarCell::Text(s) => {
                let key = format!("{}/{}:txt", r.id, m.id);
                Self::text_value_field(caches, &cmds, cx, &key, s, &r.id, &m.id, VarKind::String, VALUE_FIELD_W).into_any_element()
            }
            VarCell::Empty => Self::value_shell(&t).into_any_element(),
        }
    }

    // ── bottom bar ────────────────────────────────────────────────────────────────────────
    fn bottom_bar(&self, st: &State, coll: &Option<CollectionSnap>) -> Div {
        let t = tokens();
        let Some(c) = coll else { return div().flex_none().w_full().h(pt(BOTTOM_H)).border_t_1().border_color(rgba(t.border)) };
        if !c.editable {
            return div()
                .flex_none()
                .w_full()
                .h(pt(BOTTOM_H))
                .flex()
                .items_center()
                .pl(pt(16.0))
                .border_t_1()
                .border_color(rgba(t.border))
                .text_size(pt(10.0))
                .text_color(rgba(t.text_secondary))
                .child(tr("Library collection (read-only)"));
        }
        // "+ Create variable" 是**整块**可点的按钮（图标 + 文本一起响应），且用 `unique()` 命名，
        // 不会每次都建出同名 "Variable"。
        let taken: Vec<String> = c.rows.iter().map(|r| r.full_name.clone()).collect();
        let name = unique(tr("Variable"), |n| taken.iter().any(|x| x == n));
        let cid = c.id.clone();
        let items: Vec<MenuItem> = TYPES
            .iter()
            // 不传 `value`：引擎按类型取 `VariableValue::default_for(kind)`。
            // 以前统一传字符串 "0"，Boolean 走 `as_bool()` 解析失败 → "invalid value"，
            // 布尔变量根本建不出来（用户 2026-11-17）。
            .map(|(label, k)| MenuItem::new(*label, "variable.create", json!({ "collection": cid, "name": name, "type": k.id() })))
            .collect();
        div().flex_none().w_full().h(pt(BOTTOM_H)).flex().flex_row().items_center().px(pt(16.0)).border_t_1().border_color(rgba(t.border)).child(
            div()
                .relative()
                .flex_none()
                .child(
                    gpui::canvas(
                        {
                            let st_cap = st.clone();
                            move |bounds, _, _| {
                                st_cap.borrow_mut().add_anchor = Some(bounds);
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(pt(4.0))
                        .h(pt(24.0))
                        .px(pt(6.0))
                        .rounded(pt(4.0))
                        .cursor_pointer()
                        .text_size(pt(11.0))
                        .text_color(rgba(t.text_secondary))
                        .hover(move |s| s.bg(rgba(t.hover)))
                        .on_mouse_down(MouseButton::Left, {
                            let st = st.clone();
                            move |_, _, cx| {
                                cx.stop_propagation();
                                let anchor = st.borrow().add_anchor;
                                // 按钮**上方 4pt、左对齐**，宽度随内容。通用 `Menu` 的
                                // `TopStart` 是「卡片顶边 = 锚点顶边 - gap」，所以 gap 要加上
                                // 菜单自身高度，卡片**底边**才落在按钮上方 4pt（用户 2026-11-17）。
                                let h = items.len() as f32 * 28.0 + 2.0 * crate::menu_metrics::MENU_PAD;
                                st.borrow_mut().menu = Some(Menu::on(anchor, crate::widgets::RectAlign::TopStart, h + 4.0, items.clone()));
                            }
                        })
                        .child(paint_icon(Icon::Plus, rgba(t.text_secondary), 12.0))
                        .child(tr("Create variable")),
                ),
        )
    }

    // ── 内联菜单 ──────────────────────────────────────────────────────────────────────────
    /// 内联菜单。外壳走**通用 `Menu` 控件**（`menu_surface()`：13 圆角 / 8 内边距 / 无边框 /
    /// 标准阴影 / occlude），行高与 `MenuRow` 一致 28pt；只有行的业务回调是变量面板自己的。
    fn menu_overlay(&self, st: &State, origin: Point<Pixels>) -> AnyElement {
        let t = tokens();
        let Some(menu) = st.borrow().menu.clone() else { return div().into_any_element() };
        // `menu.x/y` 是 MouseDown 的**窗口坐标**；这个菜单画在卡片（`relative`）内部，
        // `menu()` 生成的 `.absolute().left/top` 以卡片左上角为原点，故需减掉卡片原点。
        let local_x = menu.x - f32::from(origin.x);
        let local_y = menu.y - f32::from(origin.y);
        // 锚点：按钮类菜单用控件的真实矩形（贴边），右键菜单仍以按下点为锚。
        let at = match menu.anchor {
            Some(b) => Bounds { origin: point(b.origin.x - origin.x, b.origin.y - origin.y), size: b.size },
            None => crate::widgets::at_point(local_x, local_y.max(4.0)),
        };
        let align = menu.align;
        let gap = menu.gap;
        let width = menu.width;
        let cmds = self.cmds.clone();
        let state = st.clone();
        let data = self.data.clone();
        let mut col = div().flex().flex_col();
        for item in menu.items {
            let label = item.label.clone();
            let cmd = item.cmd;
            let params = item.params.clone();
            let cmds = cmds.clone();
            let state = state.clone();
            let data = data.clone();
            let caches = self.row_caches.clone();
            let item_name = params["name"].as_str().map(str::to_owned);
            col = col.child(
                div()
                    .flex_none()
                    .w_full()
                    .h(pt(28.0))
                    .flex()
                    .items_center()
                    .px(pt(8.0))
                    .rounded(pt(5.0))
                    .cursor_pointer()
                    .text_size(pt(11.0))
                    .text_color(rgba(t.menu_text))
                    .hover(move |s| s.bg(rgba(t.menu_hover)))
                    .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
                        state.borrow_mut().menu = None;
                        match cmd {
                            "ui.filter" => {
                                if let Some(i) = params["kind"].as_u64() {
                                    let k = (i as usize).min(3);
                                    let mut s = state.borrow_mut();
                                    let on = s.hidden[k];
                                    s.hidden[k] = !on;
                                }
                            }
                            "ui.showAll" => state.borrow_mut().hidden = [false; 4],
                            "ui.rename" => {
                                if let Some(v) = params["variable"].as_str() {
                                    let name = item_name.clone().unwrap_or_default();
                                    Self::start_variable_rename(&caches, &state, &cmds, v.to_string(), name, window, cx);
                                }
                            }
                            "ui.renameMode" => {
                                if let Some(v) = params["mode"].as_str() {
                                    let name = item_name.clone().unwrap_or_default();
                                    let coll =
                                        data.collections.iter().find(|c| c.modes.iter().any(|m| m.id == v)).map(|c| c.id.clone()).unwrap_or_default();
                                    Self::start_mode_rename(&caches, &state, &cmds, coll, v.to_string(), name, window, cx);
                                }
                            }
                            "ui.renameCollection" => {
                                if let Some(v) = params["collection"].as_str() {
                                    let name = item_name.clone().unwrap_or_default();
                                    Self::start_collection_rename(&caches, &state, &cmds, v.to_string(), name, window, cx);
                                }
                            }
                            "ui.renameGroup" => {
                                if let Some(v) = params["group"].as_str() {
                                    let name = item_name.clone().unwrap_or_default();
                                    Self::start_group_rename(&caches, &state, &cmds, v.to_string(), name, window, cx);
                                }
                            }
                            "ui.details" => {
                                let ids: Vec<String> =
                                    params["variables"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
                                let mut s = state.borrow_mut();
                                s.details_drag = None;
                                if ids.len() > 1 {
                                    s.bulk_details = Some(ids);
                                    s.bulk_details_tab = 0;
                                    s.details = None;
                                } else if let Some(v) = ids.first().cloned().or_else(|| params["variable"].as_str().map(str::to_string)) {
                                    s.details = Some(v);
                                    s.bulk_details = None;
                                }
                            }
                            "ui.export" => Self::export_dialog(&cmds, params["format"].as_str().unwrap_or("json")),
                            "ui.import" => Self::import_dialog(&cmds),
                            "ui.setAlias" => {
                                if let (Some(v), Some(m), Some(a)) = (params["variable"].as_str(), params["mode"].as_str(), params["alias"].as_str())
                                {
                                    cmds.push(Cmd { id: "variable.set", params: Some(json!({ "variable": v, "mode": m, "value": { "alias": a } })) });
                                }
                            }
                            "ui.copy" => Self::copy_row(&state, &data),
                            "ui.paste" => Self::paste_row(&state, &data, &cmds),
                            "ui.copyColumn" => {
                                if let Some(m) = params["mode"].as_str() {
                                    Self::copy_column(&state, &data, m);
                                }
                            }
                            "ui.pasteColumn" => {
                                if let Some(m) = params["mode"].as_str() {
                                    Self::paste_column(&state, &cmds, &data, m);
                                }
                            }
                            "ui.duplicateGroup" => {
                                let Some(g) = params["group"].as_str() else { return };
                                let colls = Self::collections(&state, &data);
                                let ci = state.borrow().collection.min(colls.len().saturating_sub(1));
                                let Some(c) = colls.get(ci) else { return };
                                if let Some(batch) = duplicate_group_batch(&c.rows, &c.groups, g) {
                                    cmds.push(Cmd { id: "command.batch", params: Some(batch) });
                                }
                            }
                            "ui.deleteGroup" => {
                                let ids: Vec<String> = params["variables"]
                                    .as_array()
                                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                                    .unwrap_or_default();
                                // 扩展集合里组可能同时有 inherited 与本地成员。引擎拒绝删
                                // inherited，整批 command.batch 会因此回滚、本地成员也删不掉
                                // （CodeRabbit PR#29）：只把本地（非继承）成员放进批次。
                                let colls = Self::collections(&state, &data);
                                let ci = state.borrow().collection.min(colls.len().saturating_sub(1));
                                let deletable: std::collections::HashSet<&str> = colls
                                    .get(ci)
                                    .map(|c| c.rows.iter().filter(|r| !r.inherited).map(|r| r.id.as_str()).collect())
                                    .unwrap_or_default();
                                // 删整组 = 一个手势 = 一步撤销，而不是每个变量一步。
                                let steps: Vec<Value> = ids
                                    .iter()
                                    .filter(|id| deletable.contains(id.as_str()))
                                    .map(|id| json!({ "command": "variable.delete", "params": { "variable": id } }))
                                    .collect();
                                if !steps.is_empty() {
                                    cmds.push(Cmd { id: "command.batch", params: Some(json!({ "label": tr("Delete group"), "commands": steps })) });
                                }
                            }
                            _ => cmds.push(Cmd { id: cmd, params: Some(params.clone()) }),
                        }
                    })
                    .child(label),
            );
        }
        crate::widgets::menu(at).align(align).gap(gap).width(width).show(|| col.into_any_element()).unwrap_or_else(|| div().into_any_element())
    }

    // ── 拖拽 ──────────────────────────────────────────────────────────────────────────────

    /// 用按下行的**真实窗口矩形**做参考，重建整列的行矩形。
    /// 不能用按下点当行顶：指针常落在行中间，纵向偏移会被算成整列偏移，
    /// 插入线因此错位、并且要拖出大半行才动一格（用户 2026-11-17）。
    /// 平面列表 `can_into = false`，所以 `resolve` 只会在 Above / Below 之间选，`gap` 即插入缝。
    fn begin_drag(kind: DragList, from: usize, ids: &[String], hit: Point<Pixels>, tops: &[f32], hit_rect: Option<Bounds<Pixels>>) -> ListDragState {
        let h = match kind {
            DragList::Variables => ROW_H,
            _ => SIDE_ROW_H,
        };
        // 变量表带分组间隔 / 分组标题，行不是等距的：用 `tops`（内容坐标里的真实顶边）
        // 反推整列矩形。按下行的真实窗口顶边减去它的内容 y，就得到内容原点的窗口 y；
        // 纵向滚动也因此自动抵消。侧栏两个列表是等距平铺，`tops` 为空，走原公式。
        let from_top = tops.get(from).copied().unwrap_or(from as f32 * h);
        let base_y = hit_rect.map(|r| f32::from(r.origin.y)).unwrap_or(f32::from(hit.y)) - from_top;
        let (x, w) = hit_rect.map(|r| (f32::from(r.origin.x), f32::from(r.size.width))).unwrap_or((0.0, 10000.0));
        let mut rows: Vec<DragRow> = ids
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let y = match tops.get(i) {
                    Some(t) => base_y + *t,
                    None => base_y + i as f32 * h,
                };
                DragRow {
                    id: i as u64,
                    path: i.to_string(),
                    can_into: false,
                    collapsed: false,
                    rect: Bounds { origin: Point { x: px(x), y: px(y) }, size: size(px(w), px(h)) },
                }
            })
            .collect();
        // 侧栏两个列表是等距平铺：直接复用通用拖拽件的 `calibrate`，用命中行的真实矩形
        // 反推整列（与左栏图层同源）。变量表有分组间隔，行不等距，仍用上面的 `tops`。
        if kind != DragList::Variables
            && let Some(rect) = hit_rect
        {
            drag::calibrate(&mut rows, from, rect);
        }
        ListDragState { kind, from, ids: ids.to_vec(), rows, pointer: hit, target: None, armed: false }
    }

    fn on_move(st: &State, pointer: Point<Pixels>) {
        let mut s = st.borrow_mut();
        // 「Edit variable」浮层标题栏拖动：位置是窗口坐标，按按下的抓取偏移平移。
        if let Some(grab) = s.details_drag {
            s.details_pos = Some(point(px(f32::from(pointer.x) - f32::from(grab.x)), px(f32::from(pointer.y) - f32::from(grab.y))));
        }
        // 横向滚动条：按住滑块（或轨道）拖动，按行程把指针位移换算成滚动量。
        if let Some((x0, off0, range, max_scroll)) = s.hscroll_drag {
            let target = (off0 + (f32::from(pointer.x) - x0) / range.max(1.0) * max_scroll).clamp(0.0, max_scroll);
            s.hscroll.set_offset(point(px(-target), px(0.0)));
        }
        if let Some((_, _, cur)) = s.mode_drag.as_mut() {
            *cur = f32::from(pointer.x);
        }
        if let Some((x0, w0)) = s.col_resize {
            s.name_w = (w0 + f32::from(pointer.x) - x0).clamp(NAME_MIN, NAME_MAX);
        }
        if let Some((x0, w0)) = s.sidebar_resize {
            s.sidebar_w = (w0 + f32::from(pointer.x) - x0).clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        }
        if let Some((y0, split0)) = s.split_resize {
            // 上下分区的比例：以 ~400pt 的可用高度折算，夹在 5%..95%。
            s.sidebar_split = (split0 + (f32::from(pointer.y) - y0) / 400.0).clamp(0.05, 0.95);
        }
        if let Some((dir, x0, y0, w0, h0)) = s.resize {
            let dx = f32::from(pointer.x) - x0;
            let dy = f32::from(pointer.y) - y0;
            let mut w = w0;
            let mut h = h0;
            if dir & 1 != 0 {
                w = w0 + dx;
            }
            if dir & 2 != 0 {
                w = w0 - dx;
            }
            if dir & 4 != 0 {
                h = h0 + dy;
            }
            if dir & 8 != 0 {
                h = h0 - dy;
            }
            s.size = Some((w.clamp(480.0, 1600.0), h.clamp(300.0, 1200.0)));
        }
        if let Some((i, x0, w0)) = s.mode_resize {
            if let Some(w) = s.mode_ws.get_mut(i) {
                *w = (w0 + f32::from(pointer.x) - x0).clamp(MODE_MIN, MODE_MAX);
            }
        }
        let Some(d) = s.drag.as_mut() else { return };
        let dx = (f32::from(pointer.x) - f32::from(d.pointer.x)).abs();
        let dy = (f32::from(pointer.y) - f32::from(d.pointer.y)).abs();
        if !d.armed && dx + dy < 4.0 {
            return;
        }
        d.armed = true;
        d.pointer = pointer;
        // 层级基准用行矩形真实左缘（窗口坐标），否则缩进判定把指针当成从 0 开始。
        let base = d.rows.first().map(|r| f32::from(r.rect.origin.x)).unwrap_or(0.0);
        d.target = drag::resolve(&d.rows, d.from, pointer, DragCfg { base, ..DragCfg::default() });
    }

    fn commit_drag(st: &State, d: &ListDragState, cmds: &Cmds) {
        let Some(t) = d.target.as_ref() else { return };
        if t.refused || t.noop || !d.armed {
            return;
        }
        let gap = t.gap.min(d.ids.len());
        let before = d.ids.get(gap).cloned();
        // 单击时的轻微移动也会让拖拽「armed」，但落点仍是自己这一行（`gap == from`，
        // `before` 就是被拖项本身）。若当成「移到自己前面」提交，列表会把这一项挪到末尾——
        // 表现就是「点一下选中，布局跳一次」（用户 2026-11-17）。原地落点直接忽略。
        if before.as_deref() == d.ids.get(d.from).map(String::as_str) {
            return;
        }
        match d.kind {
            DragList::Collections => {
                let from = d.ids.get(d.from).cloned().unwrap_or_default();
                let mut p = json!({ "collection": from });
                if let Some(b) = before {
                    p["before"] = json!(b);
                }
                cmds.push(Cmd { id: "variable.moveCollection", params: Some(p) });
            }
            DragList::Variables => {
                let from = d.ids.get(d.from).cloned().unwrap_or_default();
                let mut p = json!({ "variable": from });
                // 落点是分组标题锚点 = 插到该组**上方**（组外）：`before` 用组首变量，
                // 并显式 `group: ""`，否则引擎会按 `before` 所在文件夹把行放进组里。
                let mut out_of_group = false;
                let before_id = before.as_deref().map(|b| match drag_group_first(b) {
                    Some(first) => {
                        out_of_group = true;
                        first.to_string()
                    }
                    None => b.to_string(),
                });
                if let Some(b) = before_id {
                    p["before"] = json!(b);
                }
                if out_of_group {
                    p["group"] = json!("");
                }
                cmds.push(Cmd { id: "variable.move", params: Some(p) });
            }
            DragList::Groups => {
                // egui 的 group_order 是会话视图偏好，不进文档：只重排本地顺序。
                // `d.ids` 就是**当前显示顺序**（与插入线同一个列表），把被拖名插到 `before`
                // 之前并整体写回，侧栏下一帧就出现在线画的位置——不会再「线动了、行没动」。
                let mut s = st.borrow_mut();
                if let Some(name) = d.ids.get(d.from).cloned() {
                    let mut order: Vec<String> = d.ids.clone();
                    order.retain(|g| g != &name);
                    let at = before.and_then(|b| order.iter().position(|g| g == &b)).unwrap_or(order.len());
                    order.insert(at.min(order.len()), name);
                    s.group_order = order;
                }
            }
        }
    }

    // ── Edit variable 详情面板 ───────────────────────────────────────────────────────────
    /// 一个详情字段（Name / Description）：commit 时推 `variable.update {field: text}`。
    fn details_field(&mut self, cx: &mut App, key: &str, initial: &str, vid: &str, field: &'static str) -> Entity<TextField> {
        if let Some(e) = self.details_fields.get(key) {
            return e.clone();
        }
        let f = text_field(cx, 260.0, initial, "", Slot::None, Slot::None);
        let cmds = self.cmds.clone();
        let vid = vid.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let mut p = json!({ "variable": vid });
                p[field] = json!(text);
                cmds.push(Cmd { id: "variable.update", params: Some(p) });
            });
        });
        self.details_fields.insert(key.to_string(), f.clone());
        f
    }

    /// 代码名（WEB / iOS / ANDROID）：`variable.update {codeSyntax: {platform: text}}`。
    fn details_code_field(&mut self, cx: &mut App, key: &str, initial: &str, vid: &str, platform: &str) -> Entity<TextField> {
        if let Some(e) = self.details_fields.get(key) {
            return e.clone();
        }
        let f = text_field(cx, 180.0, initial, "", Slot::None, Slot::None);
        let cmds = self.cmds.clone();
        let vid = vid.to_string();
        let platform = platform.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let mut code = serde_json::Map::new();
                code.insert(platform.clone(), json!(text));
                let mut p = json!({ "variable": vid });
                p["codeSyntax"] = Value::Object(code);
                cmds.push(Cmd { id: "variable.update", params: Some(p) });
            });
        });
        self.details_fields.insert(key.to_string(), f.clone());
        f
    }

    /// 变量详情（egui `details()`）。外壳用**通用 `Popover`**（40pt 标题栏 / 13 圆角 / 标准阴影 /
    /// 统一 X），不再手搓 absolute 卡片。Name 用完整名，改名才不会丢掉分组前缀。
    /// 多选「Edit variables」：egui `bulk_details` 的 GPUI 版。Details 页签只放
    /// 「Hide from publishing」三态；Scope 页签只列出所有选中类型共有的范围。每次点击
    /// 对每个选中变量发一条 `variable.update`，包进一个 `command.batch` 作为一步撤销。
    fn bulk_details_overlay(&mut self, st: &State, cx: &mut App, viewport: Size<Pixels>) -> AnyElement {
        let t = tokens();
        if st.borrow().library.is_some() {
            st.borrow_mut().bulk_details = None;
            return div().into_any_element();
        }
        let ids = st.borrow().bulk_details.clone().unwrap_or_default();
        let rows: Vec<VarRow> = self
            .data
            .collections
            .iter()
            .flat_map(|c| c.rows.iter())
            .filter(|r| ids.iter().any(|id| id == &r.id))
            .cloned()
            .collect();
        if rows.len() < 2 {
            let mut s = st.borrow_mut();
            s.bulk_details = None;
            s.details_drag = None;
            if let Some(row) = rows.first() {
                s.details = Some(row.id.clone());
            }
            return div().into_any_element();
        }
        let tab = st.borrow().bulk_details_tab.min(1);
        let mut body = crate::widgets::popover::popover_body();
        let tab_st = st.clone();
        body = body.child(
            crate::widgets::segmented::Segmented::new(vec![
                crate::widgets::segmented::SegItem::text(tr("Details")),
                crate::widgets::segmented::SegItem::text(tr("Scope")),
            ])
            .active(Some(tab))
            .on_pick(move |i, _, _| tab_st.borrow_mut().bulk_details_tab = i)
            .build(),
        );
        if tab == 0 {
            let all_hidden = rows.iter().all(|r| r.hidden);
            let cmds = self.cmds.clone();
            let row_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
            body = body.child(
                Checkbox::new(gpui::ElementId::Name("bulk-details-hidden".into()))
                    .label(tr("Hide from publishing"))
                    .text_xs()
                    .checked(all_hidden)
                    .on_change(move |next: &bool, _, _| {
                        cmds.push(Cmd {
                            id: "command.batch",
                            params: Some(bulk_hidden_batch(&row_ids, *next)),
                        });
                    }),
            );
        } else {
            let mut common: Vec<(&'static str, &'static str)> =
                rows.first().map(|r| figcraft_doc::styles::scopes_for(r.kind.doc_type()).to_vec()).unwrap_or_default();
            for row in rows.iter().skip(1) {
                let kind_scopes = figcraft_doc::styles::scopes_for(row.kind.doc_type());
                common.retain(|(id, _)| kind_scopes.iter().any(|(sid, _)| sid == id));
            }
            let all_scopes = rows.iter().all(|r| r.scopes.is_none());
            let cmds = self.cmds.clone();
            let row_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
            body = body.child(
                Checkbox::new(gpui::ElementId::Name("bulk-all-scopes".into()))
                    .label(tr("All supported properties"))
                    .text_xs()
                    .checked(all_scopes)
                    .on_change(move |next: &bool, _, _| {
                        let steps: Vec<Value> = row_ids
                            .iter()
                            .map(|id| {
                                json!({"command": "variable.update", "params": {"variable": id, "scopes": if *next { Value::Null } else { json!([]) }}})
                            })
                            .collect();
                        cmds.push(Cmd {
                            id: "command.batch",
                            params: Some(json!({"label": "Edit variables", "commands": steps})),
                        });
                    }),
            );
            for &(sid, label) in &common {
                let checked = rows.iter().all(|r| r.scopes.as_ref().is_none_or(|s| s.iter().any(|x| x == sid)));
                let cmds = self.cmds.clone();
                let per_row: Vec<(String, VarKind, Option<Vec<String>>)> =
                    rows.iter().map(|r| (r.id.clone(), r.kind, r.scopes.clone())).collect();
                let sid_owned = sid.to_string();
                body = body.child(div().pl(pt(16.0)).child(
                    Checkbox::new(gpui::ElementId::Name(format!("bulk-scope-{sid}").into()))
                        .label(tr(label))
                        .text_xs()
                        .checked(checked)
                        .on_change(move |next: &bool, _, _| {
                            cmds.push(Cmd {
                                id: "command.batch",
                                params: Some(bulk_scope_batch(&per_row, &sid_owned, *next)),
                            });
                        }),
                ));
            }
            if common.is_empty() {
                body = body.child(
                    div()
                        .text_size(pt(11.0))
                        .text_color(rgba(t.text_secondary))
                        .child(tr("No scope is shared by all the selected variables")),
                );
            }
        }
        let close_st = st.clone();
        let dismiss: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |_, _| {
            let mut s = close_st.borrow_mut();
            s.bulk_details = None;
            s.details_drag = None;
        });
        let at = st.borrow().details_pos.unwrap_or(point(px(300.0), px(60.0)));
        let est_h = st.borrow().details_h.unwrap_or_else(|| {
            if tab == 0 {
                88.0
            } else {
                let scope_rows = rows
                    .first()
                    .map(|r| figcraft_doc::styles::scopes_for(r.kind.doc_type()).len() as f32 * 22.0)
                    .unwrap_or(0.0);
                40.0 + 22.0 + 32.0 + scope_rows
            }
        });
        let measure_st = st.clone();
        let pop = popover_window(PopoverOpen::At(at))
            .width(280.0)
            .height(est_h)
            .on_measure(move |h: Pixels| measure_st.borrow_mut().details_h = Some(f32::from(h)))
            .pad(16.0)
            .surface(Surface::Panel)
            .title(title_text(tr("Edit variables")));
        let origin = pop.origin(viewport);
        let drag_st = st.clone();
        pop.on_drag_start(move |e: &MouseDownEvent, _, _| {
            let mut s = drag_st.borrow_mut();
            s.details_drag = Some(point(e.position.x - origin.x, e.position.y - origin.y));
        })
        .body(body)
        .show(viewport, dismiss)
        .1
    }

    fn details_overlay(&mut self, st: &State, cx: &mut App, viewport: Size<Pixels>) -> AnyElement {
        if st.borrow().bulk_details.is_some() {
            return self.bulk_details_overlay(st, cx, viewport);
        }
        let t = tokens();
        // egui：库变量（只读）不显示详情。
        if st.borrow().library.is_some() {
            st.borrow_mut().details = None;
            return div().into_any_element();
        }
        let Some(id) = st.borrow().details.clone() else { return div().into_any_element() };
        let Some(row) = self.data.collections.iter().flat_map(|c| c.rows.iter()).find(|r| r.id == id).cloned() else {
            st.borrow_mut().details = None;
            return div().into_any_element();
        };
        // 与其它弹窗共用同一份 body 容器（项间距 8pt），不再自己写 6。
        let mut body = crate::widgets::popover::popover_body();
        body = body.child(detail_label(tr("Name")));
        body = body.child(self.details_field(cx, &format!("d-name-{}", row.id), &row.full_name, &row.id, "name"));
        body = body.child(detail_label(tr("Description")));
        body = body.child(self.details_field(cx, &format!("d-desc-{}", row.id), &row.description, &row.id, "description"));
        // Hide from publishing
        let hidden = row.hidden;
        let hid_cmds = self.cmds.clone();
        let hid_vid = row.id.clone();
        body = body.child(
            Checkbox::new(gpui::ElementId::Name(format!("d-hidden-{}", row.id).into()))
                .label(tr("Hide from publishing"))
                .text_xs()
                .checked(hidden)
                .on_change(move |next: &bool, _, _| {
                    hid_cmds.push(Cmd { id: "variable.update", params: Some(json!({ "variable": hid_vid, "hiddenFromPublishing": *next })) });
                }),
        );
        body = body.child(detail_label(tr("Code syntax")));
        for (k, v) in row.code.clone() {
            let key = format!("d-code-{}-{}", row.id, k);
            body = body.child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(6.0))
                    .child(div().flex_none().w(pt(60.0)).text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(k.clone()))
                    .child(self.details_code_field(cx, &key, &v, &row.id, &k)),
            );
        }
        // Scoping：egui `details()` 的 "All supported properties" + 逐项范围。
        body = body.child(detail_label(tr("Scoping: where pickers offer it")));
        let all_scopes = row.scopes.is_none();
        let sc_cmds = self.cmds.clone();
        let sc_vid = row.id.clone();
        body = body.child(
            Checkbox::new(gpui::ElementId::Name(format!("d-scopes-{}", row.id).into()))
                .label(tr("All supported properties"))
                .text_xs()
                .checked(all_scopes)
                .on_change(move |next: &bool, _, _| {
                    // 勾上 = 不限范围（`null`）；取消 = 空数组（与 egui 一致）。
                    sc_cmds.push(Cmd {
                        id: "variable.update",
                        params: Some(json!({ "variable": sc_vid, "scopes": if *next { Value::Null } else { json!([]) } })),
                    });
                }),
        );
        for (id, label) in figcraft_doc::styles::scopes_for(row.kind.doc_type()) {
            let checked = row.scopes.as_ref().is_none_or(|s| s.iter().any(|x| x == id));
            let current = row.scopes.clone();
            let cmds = self.cmds.clone();
            let vid = row.id.clone();
            let kind = row.kind;
            let sid = id.to_string();
            body = body.child(div().pl(pt(16.0)).child(
                Checkbox::new(gpui::ElementId::Name(format!("d-scope-{}-{}", row.id, id).into()))
                    .label(tr(label))
                    .text_xs()
                    .checked(checked)
                    .on_change(move |next: &bool, _, _| {
                        let scopes = toggle_scope(kind, current.as_deref(), &sid, *next);
                        cmds.push(Cmd { id: "variable.update", params: Some(json!({ "variable": vid, "scopes": scopes })) });
                    }),
            ));
        }
        let close_st = st.clone();
        let dismiss: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |_, _| {
            let mut s = close_st.borrow_mut();
            s.details = None;
            s.details_drag = None;
        });
        // 与其它弹窗一样由标题栏拖动：按下时记录抓取偏移，后续移动经根视图的窗口级
        // 路由推进（`internal_drag()` 已包含 `details_drag`），指针移出卡片也不会断。
        let at = st.borrow().details_pos.unwrap_or(point(px(300.0), px(60.0)));
        // clamp / origin 要用这张卡片的真实高度，默认 72pt 会让详情底部控件出屏。
        // 首帧按内容给个估算，之后用 on_measure 回写的实测值（CodeRabbit PR#29）。
        let est_h = st.borrow().details_h.unwrap_or_else(|| {
            let code_rows = row.code.len() as f32 * 40.0;
            let scope_rows = figcraft_doc::styles::scopes_for(row.kind.doc_type()).len() as f32 * 22.0;
            40.0 + 22.0 + 40.0 + 22.0 + 40.0 + 32.0 + 22.0 + code_rows + 22.0 + 32.0 + scope_rows + 32.0
        });
        let measure_st = st.clone();
        let pop = popover_window(PopoverOpen::At(at))
            .width(280.0)
            .height(est_h)
            .on_measure(move |h: Pixels| measure_st.borrow_mut().details_h = Some(f32::from(h)))
            .pad(16.0)
            .surface(Surface::Panel)
            .title(title_text(tr("Edit variable")));
        let origin = pop.origin(viewport);
        let drag_st = st.clone();
        pop.on_drag_start(move |e: &MouseDownEvent, _, _| {
            let mut s = drag_st.borrow_mut();
            s.details_drag = Some(point(e.position.x - origin.x, e.position.y - origin.y));
        })
        .body(body)
        .show(viewport, dismiss)
        .1
    }

    fn drag_line(&self, st: &State, root_bounds: Option<Bounds<Pixels>>) -> Option<AnyElement> {
        let t = tokens();
        let s = st.borrow();
        let d = s.drag.as_ref()?;
        let tg = d.target.as_ref()?;
        if tg.refused || tg.noop || !d.armed {
            return None;
        }
        // 落点几何是**窗口坐标**（行矩形来自各行 prepaint 的真实矩形），插入线挂在 root 里：
        // 统一减去 root 的窗口原点换成局部坐标。行矩形缺失（没记录到）时才退回旧的
        // 「从表格左缘铺满」算法。
        let rb = root_bounds?;
        let ox = f32::from(rb.origin.x);
        let oy = f32::from(rb.origin.y);
        let w = f32::from(rb.size.width);
        let h = f32::from(rb.size.height);
        let table_left = if s.sidebar { s.sidebar_w + 1.0 } else { 0.0 };
        let (x0, x1) = if tg.line.x0 > 0.0 { (tg.line.x0 - ox, (tg.line.x1 - ox).min(w)) } else { (table_left, (table_left + tg.line.x1).min(w)) };
        let y = tg.line.y - oy;
        if y < 0.0 || y > h || x1 <= x0 {
            return None;
        }
        Some(div().absolute().left(pt(x0)).top(pt(y)).w(pt((x1 - x0).max(40.0))).h(pt(2.0)).bg(rgba(t.accent)).into_any_element())
    }
}

/// 分组的「树序」权重：按 `/` 路径逐段取它在 `group_order` 里的位置。
/// 父组永远排在自己的子组之前（`"A"` 是 `[i]`，`"A/B"` 是 `[i, j]`，前缀相同则短的在前），
/// 因此表格与侧栏都是「一级组 → 它的二级组 → 三级组 → 下一个一级组」。
///
/// 未列进 `group_order` 的段不能统一给 `usize::MAX`：那样 `A/one, B/two, A/three` 的
/// 两个 `A` 之间夹着 `B`，稳定排序也救不回来，表格渲染出两段 `A`（CodeRabbit PR#29
/// 折叠项）。这里给未列出段一个**由段名确定性算出的 rank**，排在列出项之后：同名段
/// 同 rank → 连续；字典序仍保证父在子前；组内文档顺序由稳定排序保留。
fn group_rank(group: &str, order: &[String]) -> Vec<usize> {
    const UNLISTED_BASE: usize = 1 << 20;
    // FNV-1a：确定性、同名同值，映射进 [0, 1<<20) 避免和列出的 position 撞。
    let segment_rank = |seg: &str| -> usize {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in seg.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        UNLISTED_BASE + (h as usize % UNLISTED_BASE)
    };
    let mut out = Vec::new();
    let mut acc = String::new();
    for seg in group.split('/') {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(seg);
        out.push(order.iter().position(|g| g == &acc).unwrap_or_else(|| segment_rank(&acc)));
    }
    out
}

/// egui `unique()`：重名时加 ` 2`、` 3`…（新建集合 / 变量 / mode 都用它）。
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    for i in 2..10000 {
        let s = format!("{base} {i}");
        if !taken(&s) {
            return s;
        }
    }
    base.to_string()
}

/// 详情面板的小标题。
fn detail_label(text: &'static str) -> Div {
    let t = tokens();
    div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(text)
}

/// 剪贴板里的值 → `variable.set` 的 value（egui `value_json` 的等价）。
/// `group`（如 `Brand/Dark`）自身或其任一父组在 `collapsed` 里就隐藏表格行。
/// 只按 `/` 边界取祖先，`Brand` 不会把 `Branding` 一起折叠。
fn group_or_ancestor_collapsed(group: &str, collapsed: &std::collections::BTreeSet<String>) -> bool {
    if group.is_empty() {
        return false;
    }
    if collapsed.contains(group) {
        return true;
    }
    let mut rest = group;
    while let Some((parent, _)) = rest.rsplit_once('/') {
        if collapsed.contains(parent) {
            return true;
        }
        rest = parent;
    }
    false
}

fn cell_value(c: &VarCell) -> Option<Value> {
    match c {
        // 复制色值时保留 alpha：`encode_value` 接受 #RRGGBBAA，粘贴后不能把半透明色变全
        // 不透明（CodeRabbit #31）。`alpha` 是 0..100 的百分数。
        VarCell::Color { hex, alpha, .. } => {
            if *alpha >= 100 {
                Some(json!(format!("#{hex}")))
            } else {
                let a = ((*alpha as f32 / 100.0) * 255.0).round() as u8;
                Some(json!(format!("#{hex}{a:02X}")))
            }
        }
        VarCell::Number(s) => s.trim().parse::<f64>().ok().map(|n| json!(n)),
        VarCell::Text(s) => Some(json!(s)),
        VarCell::Bool(b) => Some(json!(b)),
        VarCell::Alias(name) => Some(json!({ "alias": name })),
        VarCell::Empty => None,
    }
}

/// 输入框文本 → `variable.set` 的 value（egui `value_json` / `var_value` 的等价）。
fn encode_value(kind: VarKind, text: &str) -> Value {
    match kind {
        VarKind::Number => text.trim().parse::<f64>().map(|n| json!(n)).unwrap_or_else(|_| json!(0.0)),
        VarKind::Boolean => json!(text.trim().eq_ignore_ascii_case("true")),
        // hex 统一成 `#RRGGBB` / `#RRGGBBAA`（用户可能带或不带 `#`）。
        VarKind::Color => json!(format!("#{}", text.trim().trim_start_matches('#'))),
        VarKind::String => json!(text.trim()),
    }
}

/// egui `kind_glyph`：Color = 色块、Number = `#`、String = Text 图标、Boolean = 小开关。
fn kind_glyph(r: &VarRow) -> AnyElement {
    let t = tokens();
    match r.kind {
        VarKind::Color => {
            let c = r
                .values
                .iter()
                .find_map(|v| match v {
                    VarCell::Color { color, .. } => Some(*color),
                    _ => None,
                })
                .unwrap_or(Color32::GRAY);
            // 用户要求：变量名前的颜色图标用通用色块控件（圆形 `Swatch`），不要再手写
            // div + rounded_full（用户 2026-11-17：那不是通用控件）。
            let paint = figcraft_color::Paint::solid(figcraft_color::Color::from_rgba8(c.0[0], c.0[1], c.0[2], c.0[3]));
            crate::widgets::swatch::swatch(paint, 14.0).circle().into_any_element()
        }
        VarKind::Number => paint_icon(Icon::Number, rgba(t.text_secondary), 14.0).into_any_element(),
        VarKind::String => paint_icon(Icon::Text, rgba(t.text_secondary), 14.0).into_any_element(),
        VarKind::Boolean => paint_icon(Icon::Boolean, rgba(t.text_secondary), 14.0).into_any_element(),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Styled;

    fn order() -> Vec<String> {
        ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ctrl_click_toggles_a_variable() {
        let mut st = EditorState::new();
        st.selected = vec!["a".into()];
        select_var(&mut st, "c", &order(), &Modifiers { control: true, ..Default::default() });
        assert_eq!(st.selected, ["a", "c"]);
        select_var(&mut st, "a", &order(), &Modifiers { control: true, ..Default::default() });
        assert_eq!(st.selected, ["c"]);
    }

    #[test]
    fn shift_click_selects_the_visible_range() {
        let mut st = EditorState::new();
        st.selected = vec!["a".into()];
        st.anchor = Some("a".into());
        select_var(&mut st, "c", &order(), &Modifiers { shift: true, ..Default::default() });
        assert_eq!(st.selected, ["a", "b", "c"]);
    }

    #[test]
    fn shift_ctrl_click_merges_the_range() {
        let mut st = EditorState::new();
        st.selected = vec!["a".into(), "d".into()];
        st.anchor = Some("a".into());
        select_var(&mut st, "c", &order(), &Modifiers { shift: true, control: true, ..Default::default() });
        assert_eq!(st.selected, ["a", "d", "b", "c"]);
    }

    #[test]
    fn plain_click_replaces_the_selection() {
        let mut st = EditorState::new();
        st.selected = vec!["a".into(), "d".into()];
        st.anchor = Some("a".into());
        select_var(&mut st, "b", &order(), &Modifiers::default());
        assert_eq!(st.selected, ["b"]);
        assert_eq!(st.anchor.as_deref(), Some("b"));
    }

    #[test]
    fn shift_click_without_a_valid_anchor_toggles() {
        // The anchor must still be selected; a stale one makes Shift behave like Cmd/Ctrl.
        let mut st = EditorState::new();
        st.selected = vec!["d".into()];
        st.anchor = Some("a".into());
        select_var(&mut st, "c", &order(), &Modifiers { shift: true, ..Default::default() });
        assert_eq!(st.selected, ["d", "c"]);
        assert_eq!(st.anchor.as_deref(), Some("c"));
    }

    #[gpui::test]
    async fn escape_clears_the_selection(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let wake: Rc<dyn Fn()> = Rc::new(|| {});
            let mut st = VariablesEditor::new(
                Cmds::new(&wake),
                VariablesData::default(),
                cx,
                None,
                Rc::new(|_, _| {}),
                Rc::new(|_| {}),
            );
            {
                let mut s = st.state.borrow_mut();
                s.selected = vec!["a".into(), "b".into()];
                s.anchor = Some("a".into());
            }
            assert!(st.key("escape", false));
            assert!(st.state.borrow().selected.is_empty());
            assert!(st.state.borrow().anchor.is_none());
            // The next Escape has nothing left to release and must reach the panel/window.
            assert!(!st.key("escape", false));
        });
    }

    #[test]
    fn bulk_menu_omits_a_move_that_changes_nothing() {
        let rows = vec![("1".to_string(), "Brand/Primary".to_string(), VarKind::Color)];
        let items = bulk_menu_items(&rows, &["Brand".into(), "Other".into()]);
        // Already in Brand: `Brand/Primary` would be the same name, so no step and no item.
        assert!(!items.iter().any(|i| i.label.contains("Move to group Brand")), "{:?}", items.iter().map(|i| &i.label).collect::<Vec<_>>());
        assert!(items.iter().any(|i| i.label.contains("Move out of any group")));
        assert!(items.iter().any(|i| i.label.contains("Move to group Other")));
    }

    #[test]
    fn bulk_menu_scopes_only_what_every_type_supports() {
        let rows = vec![
            ("1".to_string(), "Brand/Primary".to_string(), VarKind::Color),
            ("2".to_string(), "Flag/On".to_string(), VarKind::Boolean),
        ];
        let items = bulk_menu_items(&rows, &[]);
        // Colour scopes and boolean scopes do not overlap, so only the generic entries stay.
        assert!(!items.iter().any(|i| i.label.starts_with("Scope: only")), "{:?}", items.iter().map(|i| &i.label).collect::<Vec<_>>());
        assert!(items.iter().any(|i| i.label == "Scope: hidden from pickers"));
        assert!(items.iter().any(|i| i.label == "Scope: all supported properties"));
    }

    #[test]
    fn scope_checkboxes_narrow_and_widen_the_scope_list() {
        // "All supported properties" starts with every colour scope checked; turning Frame fill
        // off keeps the rest instead of collapsing to a single scope.
        let all_off = toggle_scope(VarKind::Color, None, "FRAME_FILL", false);
        assert_eq!(all_off, ["SHAPE_FILL", "TEXT_FILL", "STROKE_COLOR", "EFFECT_COLOR"]);
        // From an explicit single scope, checking another appends it.
        let start = vec!["FRAME_FILL".to_string()];
        let wider = toggle_scope(VarKind::Color, Some(&start), "STROKE_COLOR", true);
        assert_eq!(wider, ["FRAME_FILL", "STROKE_COLOR"]);
        // Unchecking the last scope means hidden from pickers (empty list, not `None`).
        let last = vec!["FRAME_FILL".to_string()];
        assert!(toggle_scope(VarKind::Color, Some(&last), "FRAME_FILL", false).is_empty());
        // Booleans have no scopes at all (egui `booleans_share_no_scope`).
        assert!(figcraft_doc::styles::scopes_for(figcraft_doc::VariableType::Boolean).is_empty());
    }

    #[test]
    fn bulk_menu_lists_every_group_so_the_popup_can_scroll() {
        let groups: Vec<String> = (0..40).map(|i| format!("Group {i}")).collect();
        let rows = vec![("1".to_string(), "Brand/Primary".to_string(), VarKind::Color)];
        let items = bulk_menu_items(&rows, &groups);
        assert_eq!(items.iter().filter(|i| i.label.starts_with("Move to group ")).count(), 40);
    }

    #[test]
    fn group_rename_keeps_the_parent_and_nests_on_a_slash() {
        let members = vec![("1".to_string(), "Primary".to_string()), ("2".to_string(), "Disabled".to_string())];
        let (renamed, moves) = group_renames(&members, "Brand/Buttons", "Controls").expect("a rename");
        assert_eq!(renamed, "Brand/Controls");
        assert!(moves.iter().any(|(_, n)| n == "Brand/Controls/Primary"));
        assert!(moves.iter().any(|(_, n)| n == "Brand/Controls/Disabled"));

        let (renamed, moves) = group_renames(&members, "Brand/Buttons", "Palette/Orange").expect("a nested rename");
        assert_eq!(renamed, "Brand/Palette/Orange");
        assert!(moves.iter().any(|(_, n)| n == "Brand/Palette/Orange/Primary"));
    }

    #[test]
    fn group_rename_does_nothing_when_the_path_is_unchanged() {
        let members = vec![("1".to_string(), "Primary".to_string())];
        assert!(group_renames(&members, "Brand", "Brand").is_none());
        assert!(group_renames(&members, "Brand", " / ").is_none());
    }

    fn var_row(id: &str, full_name: &str, inherited: bool) -> VarRow {
        let (group, name) = full_name.rsplit_once('/').map_or(("", full_name), |(g, n)| (g, n));
        VarRow {
            id: id.to_string(),
            full_name: full_name.to_string(),
            name: name.to_string(),
            group: group.to_string(),
            kind: VarKind::Color,
            values: Vec::new(),
            overrides: Vec::new(),
            description: String::new(),
            hidden: false,
            code: Vec::new(),
            scopes: None,
            inherited,
        }
    }

    #[test]
    fn bulk_menu_moves_every_selected_variable_in_one_batch() {
        let rows = vec![
            ("1".to_string(), "Brand/Primary".to_string(), VarKind::Color),
            ("2".to_string(), "Brand/Secondary".to_string(), VarKind::Color),
        ];
        let items = bulk_menu_items(&rows, &["Space".into()]);
        let moving = items.iter().find(|i| i.label.contains("Move to group Space")).expect("the move item");
        let steps = moving.params["commands"].as_array().expect("batch steps");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0]["params"]["name"], "Space/Primary");
        assert_eq!(steps[1]["params"]["name"], "Space/Secondary");
    }

    #[test]
    fn bulk_menu_scope_change_sets_the_same_scope_on_every_selected_variable() {
        let rows = vec![
            ("1".to_string(), "Brand/Primary".to_string(), VarKind::Color),
            ("2".to_string(), "Brand/Secondary".to_string(), VarKind::Color),
        ];
        let items = bulk_menu_items(&rows, &[]);
        let scoped = items.iter().find(|i| i.label == "Scope: only Frame fill").expect("the fill scope item");
        let steps = scoped.params["commands"].as_array().expect("batch steps");
        assert_eq!(steps.len(), 2);
        for step in steps {
            assert_eq!(step["params"]["scopes"], json!(["FRAME_FILL"]));
        }
    }

    #[test]
    fn edit_variables_sets_stroke_on_all_selected_in_one_batch() {
        let rows = vec![
            ("1".to_string(), VarKind::Color, Some(vec!["FRAME_FILL".to_string(), "STROKE_COLOR".to_string()])),
            ("2".to_string(), VarKind::Color, Some(vec!["FRAME_FILL".to_string(), "STROKE_COLOR".to_string()])),
            ("3".to_string(), VarKind::Color, Some(vec!["FRAME_FILL".to_string()])),
        ];
        let batch = bulk_scope_batch(&rows, "STROKE_COLOR", true);
        assert_eq!(batch["label"], "Edit variables");
        let steps = batch["commands"].as_array().expect("batch steps");
        assert_eq!(steps.len(), 3, "every selected variable, one undoable batch");
        // Already-scoped rows keep their existing list; the unscoped row gains Stroke.
        assert_eq!(steps[0]["params"]["scopes"], json!(["FRAME_FILL", "STROKE_COLOR"]));
        assert_eq!(steps[2]["params"]["scopes"], json!(["FRAME_FILL", "STROKE_COLOR"]));
    }

    #[test]
    fn edit_variables_hides_all_selected_in_one_batch() {
        let ids = vec!["1".to_string(), "2".to_string()];
        let batch = bulk_hidden_batch(&ids, true);
        assert_eq!(batch["label"], "Edit variables");
        let steps = batch["commands"].as_array().expect("batch steps");
        assert_eq!(steps.len(), 2);
        assert!(steps.iter().all(|s| s["params"]["hiddenFromPublishing"] == true));
    }

    #[test]
    fn duplicate_group_copies_every_local_variable_under_the_next_free_name() {
        let rows = vec![
            var_row("1", "Brand/Primary", false),
            var_row("2", "Brand/Secondary", false),
            var_row("3", "Other/Tertiary", false),
        ];
        let groups = vec!["Brand".into(), "Brand 2".into(), "Other".into()];
        let batch = duplicate_group_batch(&rows, &groups, "Brand").expect("a batch");
        assert_eq!(batch["label"], "Duplicate group");
        let steps = batch["commands"].as_array().expect("steps");
        assert_eq!(steps.len(), 4, "two copies, each duplicate + rename");
        assert_eq!(steps[0]["as"], "copy0");
        assert_eq!(steps[1]["params"]["variable"]["$ref"], "copy0");
        assert_eq!(steps[1]["params"]["name"], "Brand 3/Primary");
        assert_eq!(steps[3]["params"]["name"], "Brand 3/Secondary");
    }

    #[test]
    fn duplicate_group_skips_inherited_members_and_empty_groups() {
        let rows = vec![var_row("1", "Brand/Primary", false), var_row("2", "Brand/Library", true)];
        let groups = vec!["Brand".into()];
        let batch = duplicate_group_batch(&rows, &groups, "Brand").expect("a batch");
        assert_eq!(batch["commands"].as_array().map(Vec::len), Some(2), "only the local member");
        assert_eq!(duplicate_group_batch(&[var_row("3", "Other/x", false)], &["Other".into()], "Empty"), None);
    }

    #[test]
    fn bulk_menu_acts_on_every_selected_variable() {
        let rows = vec![
            ("1".to_string(), "Brand/Primary".to_string(), VarKind::Color),
            ("2".to_string(), "Brand/Secondary".to_string(), VarKind::Color),
        ];
        let items = bulk_menu_items(&rows, &["Space".into()]);
        assert_eq!(items[1].cmd, "command.batch", "delete is one undo step");
        assert_eq!(items[1].params["commands"].as_array().map(Vec::len), Some(2));
        assert!(items.iter().any(|i| i.label.contains("Move to group Space")));
        assert!(items.iter().any(|i| i.label == "Scope: hidden from pickers"));
    }

    fn style_row(id: &str, name: &str) -> VarRow {
        VarRow {
            id: id.to_string(),
            full_name: name.to_string(),
            name: name.to_string(),
            group: String::new(),
            kind: VarKind::Color,
            values: Vec::new(),
            overrides: Vec::new(),
            description: String::new(),
            hidden: false,
            code: Vec::new(),
            scopes: None,
            inherited: false,
        }
    }

    /// A selected row uses the theme's selected-row fill; an unselected one has no background
    /// until hover (`var_row`). Mirrors egui `selected_rows_are_painted_with_the_selected_row_colour`.
    #[gpui::test]
    async fn selected_rows_are_painted_with_the_selected_row_colour(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let wake: Rc<dyn Fn()> = Rc::new(|| {});
            let cmds = Cmds::new(&wake);
            let caches = Rc::new(RefCell::new(RowCaches::default()));
            let row = style_row("a", "Alpha");
            let collection = CollectionSnap {
                id: "c".into(),
                name: "Collection".into(),
                modes: Vec::new(),
                rows: vec![row.clone()],
                groups: Vec::new(),
                library: None,
                extends: None,
                editable: true,
            };
            let ids = vec!["a".into()];
            let tops = Rc::new(vec![0.0]);
            let bulk_rows = Rc::new(vec![("a".into(), "Alpha".into(), VarKind::Color)]);
            let bulk_groups = Rc::new(Vec::new());
            let state = Rc::new(RefCell::new(EditorState::new()));

            state.borrow_mut().selected = vec!["a".into()];
            let mut selected = VariablesEditor::var_row(
                &state,
                &cmds,
                &caches,
                &collection,
                &row,
                0,
                &ids,
                tops.clone(),
                200.0,
                100.0,
                0.0,
                bulk_rows.clone(),
                bulk_groups.clone(),
                cx,
            );
            assert_eq!(
                selected.style().background.as_ref().and_then(|f| f.color()),
                Some(rgba(tokens().selected_row).into()),
                "selected row background"
            );

            state.borrow_mut().selected.clear();
            let mut unselected = VariablesEditor::var_row(
                &state,
                &cmds,
                &caches,
                &collection,
                &row,
                0,
                &ids,
                tops,
                200.0,
                100.0,
                0.0,
                bulk_rows,
                bulk_groups,
                cx,
            );
            assert_eq!(
                unselected.style().background.as_ref().and_then(|f| f.color()),
                None,
                "unselected row has no fill until hover"
            );
        });
    }
}
