# 变量绑定：上游 FigCraft ↔ Monet 差异与补全计划

> 审计日期：2026-11-26
> 本地基线：`main` `ff8e5e617`；工作树 HEAD `5cfe98448`（`feat/crash-report`）。抽查的变量相关文件在两者间无差异，审计以 HEAD 为准；工作树的未提交改动（文本输入 / 矢量 / 剪贴板）不涉及变量绑定。
> 上游基线：`storytold/figcraft` `main` `e1361cb15`（本次 fetch）
> 方法：树级内容 diff（两仓无共同祖先，不能用 merge-base）+ 全量 `#[test]` 名差集 + `spec!` / 字段表 / 绑定入口逐条核对。只读审计，未改代码。

---

## 0. 结论摘要

**绑定模型同构，执行链路同构；缺的是 5 类可独立合入的上游修正（#2174 文本字重 / #2441 REST 分边 / #2416 库更新 / duplicate 精确 id / Dev Mode 读取）+ 1 个跨 crate 的缩放语义（#2090）。本地在行排序、`.fig` 变量映射、GPUI 变量面板上领先，不能整文件覆盖。**

| 维度 | 结论 |
|---|---|
| 变量模型（`Variables` / `Variable` / `Collection` / `VariableValue` / 颜色计算 / resolve / scope） | `crates/doc/src/styles.rs` 与上游 **无差异**（`git diff` 为空） |
| 绑定存储 | 同为 `Node.bound: BTreeMap<String, VariableId>` + `Node.var_modes`；仅上游多 `scale_factor` 字段（#2090） |
| 绑定执行 | `variable.bind`、`apply_bindings_*`、paint/effect/grid 重绑、`bound_alpha`、binding lists 与上游同名同测 |
| 命令面 | 本地 = 上游 + `variable.move` + `variable.moveCollection`（上游没有）；其余 `variable.*` 完全一致 |
| 上游独有绑定行为测试 | **27 条**：14 缩放×绑定、5 文本字重/行高、2 REST 分边描边、1 库更新、1 CSS 导出、1 duplicate、2 存储 id、1 Dev Mode 读取 |
| 本地独有 | `variable.move`/`moveCollection`（含 3 条测试 + GPUI 拖拽）、`.fig` 的 `variableConsumptionMap` 映射、GPUI 变量编辑器（4,014 行） |
| 15 个变量/绑定测试文件合计 | 本地 **143** / 上游 **164**；补全后目标 **167**（多出的 3 条是本地 move 测试） |

三类缺口：

- **A 类（上游有、本地缺，确定要补）**：9 项，见 §3。
- **B 类（本地领先，补的时候不要覆盖）**：5 项，见 §4。
- **C 类（语义分歧，要先决策）**：4 项，见 §5。

---

## 1. 方法说明

1. 两仓历史被重写、无共同祖先，`git merge` 不可用；全部按**文件内容**比对，不看提交数。
2. 行为覆盖用 `#[test]` 函数名差集做硬指标，再逐条判断「真缺 / 只是改名 / 本地已手工移植」。
3. 命令与字段用 `spec!` 清单、`field_type` / `field_takes` / `NODE_KEYS` 表逐条核对。
4. 本轮**未运行**测试；所有「本地现状」均为代码路径确认，补全时以新增测试转绿为准。

---

## 2. 分层对照

| 层 | 文件 | 状态 | 说明 |
|---|---|---|---|
| 文档模型 | `crates/doc/src/styles.rs` | **一致** | 变量、集合、模式、别名、颜色计算、scope、`resolve`、`flattened` 全部相同 |
| 节点存储 | `crates/doc/src/node.rs` | 本地少 1 字段 | `bound` / `var_modes` 相同；上游有 `scale_factor`（525 行），本地没有 |
| 变量命令 | `crates/engine/src/cmd/variables.rs` | 本地多 2 命令、少 1 修复 | 本地多 `variable.move` / `moveCollection`；`duplicate` 少了「精确 id 优先」 |
| 绑定执行 | `crates/engine/src/cmd/styles.rs` | 主体一致 | 本地少 `LENGTH_FIELDS`、`scale_of`、`bake_scale`、`bake_stale_scales`、`fontWeight` 样式绑定；本地多 `style.move {name?, type?}` |
| 样式变量 | `crates/engine/src/assets/style_vars.rs` | 本地少字重 | `fontSize/lineHeight/letterSpacing` 相同；本地缺 `fontWeight` 的字段表、scope 与应用 |
| 库变量 | `crates/engine/src/assets/library.rs` | 本地少 1 行种子 | `prune` 没有把 `styles.bound` 计入存活变量（#2416） |
| REST 导入 | `crates/format/src/rest/variables.rs` | 本地少 2 块 | 缺 `individualStrokeWeights` 分边映射；缺 `bound_by_items` 列表去重（#2441） |
| `.fig` 导入 | `crates/format/src/fig/map*` | **本地领先** | 本地拆成 `map/` 模块并新增 `variables.rs`；上游 `fig/map.rs` 检索不到 `variableConsumptionMap` |
| egui 参照 | `crates/ui-egui/src/panels/variable_ui.rs`、`widgets/style_picker.rs` | **与上游一致** | 已带字重入口，但本地引擎不认 `fontWeight` |
| GPUI | `crates/ui-gpui/src/panels/right/variables.rs` 等 | 本地独有 | 面板 4,014 行；`style.bind` 零引用；行内绑定只覆盖 width/height/gap/opacity/fontSize/fill/visible/characters |

---

## 3. A 类缺口总表（上游已合并、本地缺）

| # | 缺口 | 层 | 上游依据 | 本地证据 | 优先级 | 预估 |
|---|---|---|---|---|---|---|
| A1 | Scale 工具 × 绑定长度：实例按因子绘制并保留绑定，普通 frame 烘值并解绑 | doc + engine + collab + format | `3053b7711`（#2090）、`0636a3e6a`（#2134 review） | `doc/node.rs` 无 `scale_factor`；`cmd/styles.rs` 无 `LENGTH_FIELDS`/`scale_of`/`bake_scale`/`bake_stale_scales`；`collab/change.rs` 无 `ScaleFactor` | **P1**（错误可见，改动最大） | 2–3 d |
| A2 | 文本样式绑定数字字重：`style.bind fontWeight`、字重↔字名互斥、1–1000 夹取 + `wght` 轴 | engine（egui 已有 / GPUI 缺） | `ccb33e0bb`（#2174）、`281918e2f` | `assets/style_vars.rs:35` 字段表无 `fontWeight`；`:69` scope 表无 `FONT_WEIGHT`；`cmd/styles.rs:55` spec 无 | **P0** | 0.5–1 d |
| A3 | REST 导入：描边分边权重绑定 + 节点级 `fills/strokes/effects/textRangeFills` 列表去重 | format | `d332c18a2`（#2441） | `format/src/rest/variables.rs:430` 无 `individualStrokeWeights` 分支；无 `bound_by_items` | **P0** | 0.5 d |
| A4 | 库接受更新时保留「仅样式绑定」的变量 | engine | `e9a477773`（#2416） | `assets/library.rs:1152` 存活种子缺 `styles.bound` | **P0** | 0.2 d |
| A5 | `variable.duplicate` 先按精确 id、再按名字匹配 | engine | `815cbf9fa` | `cmd/variables.rs:201` 一次 `x.id == v \|\| x.name == v` | P1 | 0.1 d |
| A6 | CSS 导出：仅样式使用的字重变量保持裸数字（不写成 px） | engine | `281918e2f` | 依赖 A2；`assets/token_roles.rs:29` 已把 `fontWeight` 归为无单位 | P1 | 随 A2 |
| A7 | 恢复被本地删除的 3 条编辑器回归测试（duplicate-by-id 1 条、stored-ids 2 条） | engine/test | `815cbf9fa`、`2f4701491`（#1613） | `variables_editor_tests.rs` 现在用 3 条 `variable.move` 测试替换了它们 | P2 | 0.2 d |
| A8 | Dev Mode `node.get` 读取 `variableModes` / `propertyReferences` / `exposedInstance` | engine | `inspect_fields_tests.rs`（上游 `NODE_KEYS` 113 项 vs 本地 101 项） | `automation/inspect.rs:9` 键表无这 3 项；无对应 `insert_prop` | P2 | 0.3 d |
| A9 | slot 文本属性不读取未绑定的基础属性 | engine | `slot_tests.rs` | 本地 `slot_tests` 无此测试 | P3 | 0.2 d |

---

## 4. B 类：本地领先，补的时候不要覆盖

| # | 本地实现 | 上游状态 | 处理 |
|---|---|---|---|
| B1 | `variable.move` / `variable.moveCollection` + 3 条测试 + GPUI 行拖拽（`panels/right/variables.rs:3628,3649`） | 上游没有这两个命令 | **保留**；移植上游 `variables.rs` 时按 hunk 加，不整文件替换 |
| B2 | `crates/format/src/fig/map/variables.rs`（168 行）：`variableConsumptionMap` → `boundVariables`、`variableModeBySetMap` → `explicitVariableModes`、每区域填充 | 上游 `fig/map.rs` 检索不到 `variableConsumptionMap` / `boundVariables` 映射 | **保留**；后续可反向上游化 |
| B3 | GPUI 变量编辑器（4,014 行：搜索、多选、批量菜单、模式列、详情、拖拽、Floating/Full） | 上游只有 egui | **保留**；补行为测试而不是重写 |
| B4 | 玻璃 `BOUND_FIELDS`（7 个字段）、Icon 节点、brush 体系 | 本地扩展 | 保留；`cmd/styles.rs` 的绑定字段表不要被上游版本覆盖 |
| B5 | `style.move {name?, type?}`、`wrapStyle` 等本地参数 | 上游没有 | 保留；A2 只改 `bind_style` / spec 的绑定段 |

---

## 5. C 类：语义分歧，先决策再动手

**C1 · `.fig` 与 Scale 因子（必须先定）**
本地 `.fig` 导入会把实例的 `uniformScaleFactor` 烘进子层长度（`fig/map/component.rs:455-460`），导出则用宽高比例猜因子（`export/component.rs:179`，写回在 `:300`）。上游 #2090 明确「不改 `.fig` 导入、不改 `.fig` 导出」。
→ 加 `Node.scale_factor` 后**不能**在 `.fig` 导入时再设一次因子，否则双重缩放。建议：跟随上游，`.fig` 继续烘焙 + 比例导出，只让 Scale 工具使用新字段；`.fig` 原生往返另立 PR。

**C2 · 文本字重的 `.fig` 往返**
上游接受 `no field for` 告警（二进制没有 `wght` 字段），字体名按已结算的字面回读。本地导出已写 `st.bound`（`export/assets.rs:198-210`），字重会走「按值烘出 + 告警」，与上游一致。不要为了消警改格式。

**C3 · egui 已就绪、引擎未接**
本地 `ui-egui/src/panels/variable_ui.rs:517` 与 `widgets/style_picker.rs:735,797` 已带字重入口（与上游同文件），但 `style_vars::field_type` 不认 `fontWeight` → 现在点字重会报 `the style has no field 'fontWeight' to bind`。补 A2 即通，不用改 egui。

**C4 · GPUI 的绑定入口差距**
`ui-gpui` 全仓 **0** 处 `style.bind`；行内绑定只覆盖 width/height/gap/opacity/fontSize（`sections.rs:2819,2935`）以及 fill/visible/characters；`fontFamily`/`fontStyle`/`fontWeight`、行高、字距没有 chip。这是 UI 差距，不是引擎差距（节点级 `variable.bind fontWeight` 引擎已支持，`cmd/variables.rs:742`）。

---

## 6. 逐项证据

### 6.1 A1 · Scale × 绑定长度（#2090）

**上游做了什么**（`3053b7711` + `0636a3e6a`）：

- `doc/node.rs` 加 `scale_factor: f64`（默认 1，序列化时 1 不落盘）；`collab/change.rs` 加 `ScaleFactor => scale_factor`。
- `cmd/arrange.rs`：`scale_node` 对实例乘因子并夹在 `1e-3..1e3`；普通层若在实例外、且有绑定，则丢掉它缩放的**长度类**绑定。
- `cmd/styles.rs`：`LENGTH_FIELDS`（width/height/gap/padding/radius/描边宽/字号/行高等 35 个）、`scale_of`（自身及祖先实例因子的乘积）、`bake_scale` / `bake_stale_scales`（脱离实例、转 frame、main 丢失时把长度落成实际值并解绑）、`apply_bindings_matching` 里数字长度按 `x * scale` 绘制、`detach_edited` 遇到因子变化直接跳过。
- 组件侧：`components/sync.rs` 重建时带因子；`assets/library.rs` main 丢失时 `bake_stale_scales`；`cmd/component/instance.rs` 的 detach / restore 走 `bake_scale`。

**本地现状（为什么是 bug）**：

- `derived.rs:56` 每次编辑后都会 `apply_bindings_to`，把变量值写回节点；`scale_node`（`cmd/arrange.rs:412`）先按 `k` 缩放了几何。
- 于是「实例里绑定的 gap=8，Scale 到 0.5」→ 长度先变 4 → normalize 又把 8 写回，**绑定值回弹、与未绑定长度错位**。这正是上游提交说明里描述的成因。
- 本地没有 `scale_factor` 字段，也没有任何「因子参与绑定解析」的路径，所以只能整块补，不能局部打补丁。

**移植顺序（缺一不可）**：

1. `doc`：字段 + serde 默认/跳过 + `Node::new` 初始化；
2. `collab`：`Prop::ScaleFactor`（否则协作端不同步该字段）；
3. `arrange`：`MIN/MAX_SCALE_FACTOR`、因子乘法、`covered`（嵌套实例只由外层因子覆盖）；
4. `styles`：4 个符号 + `apply_bindings_matching` 乘法 + `detach_edited` 跳过；
5. 组件 / 库 / cycles / instance 的 bake 调用；
6. `scale_bound_values_tests.rs`（14 条）注册进 `lib.rs`；
7. UI：Scale 面板读数是否需要显示因子（本地 `ui-egui/src/panels/scale_panel.rs` 已有面板，按上游对照）。

**验收**：14 条测试全绿，重点四条——`a_scaled_instance_draws_bound_gap_and_radius_at_the_factor_and_keeps_the_bindings`、`scaling_an_instance_with_the_tool_scales_bound_values_and_undo_takes_them_back`、`a_file_with_a_scaled_instance_keeps_its_factor_and_bound_gap_after_save_and_open`、`scaling_a_plain_frame_bakes_its_bound_gap_and_drops_the_binding`。

### 6.2 A2 · 文本样式数字字重（#2174）

**上游做了什么**：

- `assets/style_vars.rs`：`field_type` 的 Number 组加 `fontWeight`；scope 表加 `FONT_WEIGHT`；应用时把 1–1000 的数夹取，选最近的字面并写 `wght` 轴（`styled_name(weight_name(w), italic)`）；字名绑定后清 `wght`（`281918e2f` 的 review 修复）。
- `cmd/styles.rs`：`style.bind` 的 spec 增加 `fontWeight` 语义；`fontWeight` 与 `fontStyle` 互斥（绑一个就移除另一个）。
- 测试 5 条（`style_vars_tests.rs`）：字面 + 轴、越界夹取、字重压过字名、百分比行高按 px、`.fig` 往返。
- egui：`variable_ui.rs` / `style_picker.rs` 的字重入口（本地已与上游同文件，已具备）。

**本地现状**：

- `style_vars.rs:35` 的字段表只到 `letterSpacing`；`:69` scope 表没有 `FONT_WEIGHT`；应用分支没有 weight。
- 但 `cmd/styles.rs:591` 的 `style_sets` **已经**列了 `fontWeight` —— 半接入状态：校验层认、`field_type` 不认，最终仍在 `bind_style` 的 `field_type` 处报错。
- 节点级 `variable.bind` 对 `fontWeight` 已支持（`cmd/variables.rs:742`、`cmd/styles.rs:1194`），所以缺的只是**样式级**。

**验收**：5 条测试；`css_export_keeps_a_style_weight_bare` 一并转绿（`token_roles.rs:29` 已具备 Unitless 归类）。

### 6.3 A3 · REST 分边描边与列表去重（#2441）

`crates/format/src/rest/variables.rs` 的 `node_binding` 目前对 `individualStrokeWeights` 直接返回未识别 → 变量进入 `skipped`，导入后四条边的权重是**未绑定**的普通值，同时报一条「not supported」。
上游加了两块：

1. `individualStrokeWeights` 分支：同时认实时 API 的 `BORDER_TOP_WEIGHT`… 与公开 spec 的 `top/right/bottom/left`，落到 `strokeTopWeight` 等 4 个字段；
2. `bound_by_items` + `aliases`：节点级 `fills/strokes/effects/textRangeFills` 列表只是重复各 paint / stop / effect 自己绑的变量，全部命中时不再报「skipped」。

**验收**：`variables_bound_to_stroke_sides_bind_each_sides_weight`、`node_level_lists_the_paints_and_effects_bind_are_not_reported`。

### 6.4 A4 · 库更新误删样式专用变量（#2416）

`assets/library.rs` 的 `prune` 计算存活变量时只算了「层绑定 / 原型动作 / 本地变量别名与继承」，没算**样式绑定**。当某个变量只被一个样式绑定（层通过样式取色），接受库更新后该变量被删，样式悬空、颜色冻结。
修复只有一行：种子里加 `doc.styles.iter().flat_map(|s| s.bound.values().map(String::as_str))`。
注意本地 `library.rs` 与上游整体差 169 行（本地是另一版），**只搬这一行语义**，不要整文件替换。

### 6.5 A5 · `variable.duplicate` 精确 id 优先

本地 `cmd/variables.rs:201` 用 `x.id == v || x.name == v` 一次匹配；若前一个变量把后一个的 id 当成了自己的名字，会复制错对象。上游改成先 `position(|x| x.id == v)`，再按名字兜底。
连带 A7：上游同时补了 `duplicate_by_id_copies_that_variable_even_when_an_earlier_one_is_named_like_its_id`。

### 6.6 A7 · 被删的 stored-id 回归测试

`variables_editor_tests.rs` 本地 8 条 = 上游 8 条里去掉 3 条（duplicate-by-id、`stored_variable_ids_detach_deleted_aliases_per_mode`、`stored_variable_ids_keep_collection_edits_on_the_resolved_collection`），换成 3 条 `variable.move` 测试。
代码侧 `doc/styles.rs` 的 `get_by_id` / `collection_by_id` 与 `cmd/variables.rs` 的删除后按模式 detach（约 440–452 行）都在，**两条 stored-id 测试预计可直接恢复**；duplicate 一条需先修 A5。

### 6.7 A8 · Dev Mode 读取绑定与模式

上游 `automation/inspect.rs` 的 `NODE_KEYS` 有 113 项，本地 101 项；缺 `variableModes`、`propertyReferences`、`exposedInstance`，以及对应 `insert_prop`（上游 403/406/409 行）。`cmd/props.rs:1138` 只能**写** `variableModes`，读不回来。
这条属于「Dev Mode 能看见绑定/模式」的通道，不是画布绑定本身。

---

## 7. 补全计划

### 批次总览

| 批次 | 内容 | 依赖 | 验收 | 预估 |
|---|---|---|---|---|
| 0 | 建分支、记录基线、确认工作树 | — | 基线测试数入档 | 0.2 d |
| 1 | P0 三处小修：A3 / A4 / A5（+A7 两条 stored-id 测试） | 0 | 6 条上游测试转绿 | 0.5–1 d |
| 2 | P0 文本字重：A2 + A6 | 1 | 6 条测试转绿 | 0.5–1 d |
| 3 | P1 缩放×绑定：A1 | 1（与 2 可并行） | 14 条测试转绿 | 2–3 d |
| 4 | P2 通道与覆盖：A7 剩余、A8、GPUI 面板行为 | 1–3 | inspect 测试 + 面板用例 | 1–2 d |
| 5 | P3 上游化本地领先（可选） | 3 | 上游 PR 草稿 | 1 d |

### 批次 0 · 准备

- 从当前 HEAD 切 `sync/variable-binding-2026-11`；工作树的文本输入改动先单独提交/暂存，避免混入。
- 记录基线：`cargo test -p figcraft-engine variables`、`cargo test -p figcraft-engine style_vars`、`cargo test -p figcraft-format proto_variable`、`cargo test -p figcraft-ui-gpui variables` 的通过数。
- 确认 `git diff upstream/main -- crates/doc/src/styles.rs` 为空（模型同构的前提）。

### 批次 1 · P0 小修（A3 / A4 / A5）

1. `crates/format/src/rest/variables.rs`
   - 在 `node_binding` 加 `individualStrokeWeights` 分支（8 个键 → 4 个 `stroke*Weight`）；
   - 加 `bound_by_items` 与 `aliases`；`skipped.retain(|k| !bound_by_items(k, j, &found))`。
2. `crates/engine/src/assets/library.rs`
   - `prune` 的 seeds 加 `doc.styles.iter().flat_map(|s| s.bound.values())`。
3. `crates/engine/src/cmd/variables.rs`
   - `duplicate` 改「先 id、后 name」。
4. 测试
   - 搬 `proto_variable_tests.rs` 两条；`library_variables_tests.rs` 一条；`variables_editor_tests.rs` 恢复 duplicate 一条 + 两条 stored-id。

**Gate**：`cargo test -p figcraft-format proto_variable`、`cargo test -p figcraft-engine library_variables`、`cargo test -p figcraft-engine variables_editor` 全绿；既有测试零新增失败。

### 批次 2 · P0 文本字重（A2 / A6）

1. `crates/engine/src/assets/style_vars.rs`
   - `field_type`：Number 组加 `"fontWeight"`；
   - `scopes`：加 `"fontWeight" => &["FONT_WEIGHT"]`；
   - 应用：`("fontWeight", Number(x))` → 夹取 1–1000、`styled_name(weight_name(w as u16), italic)`、`variations["wght"]`；`"fontStyle"` 分支保持 `set_font_style`（绑名字后清 `wght`）。
2. `crates/engine/src/cmd/styles.rs`
   - `style.bind` spec 文本加 `fontWeight` 与互斥说明；
   - `bind_style`：绑 `fontWeight` 时移除 `fontStyle`，绑 `fontStyle` 时移除 `fontWeight`（与上游同语义）。
3. 测试（`style_vars_tests.rs`）
   - 5 条上游测试；额外确认 `css_export_keeps_a_style_weight_bare`。
4. UI（可选并入本批）
   - GPUI Typography 加 `fontWeight` 的绑定 chip（节点级 `variable.bind`，复用 `widgets::variable::bound_field`，不改冻结件）。

**Gate**：`cargo test -p figcraft-engine style_vars`、`cargo test -p figcraft-engine variables_io` 全绿；egui `style_picker` 既有测试不回退。

### 批次 3 · P1 缩放×绑定（A1）

按 §6.1 的 7 步走，单独 PR，提交信息建议：

```
feat(engine): Scale 工具在实例上保留变量绑定的长度，其它层烘值并解绑 (#2090)

- doc: Node::scale_factor（默认 1，不落盘）
- collab: Prop::ScaleFactor
- arrange: 实例因子乘法与 1e-3..1e3 夹取；嵌套实例只由外层覆盖
- styles: LENGTH_FIELDS / scale_of / bake_scale / bake_stale_scales；
  apply_bindings_matching 按 x*scale 绘制；detach_edited 跳过因子变化
- components/library/cycles/instance: 重建与转换时带因子或烘焙
```

**Gate**：`cargo test -p figcraft-engine scale_bound_values` 14/14；`cargo test -p figcraft-engine components_scale`、`-p figcraft-format fig` 零新增失败；手工验证「实例 gap/radius 绑 8/4，Scale 0.5 → 4/2，切 Dark → 6/3，撤销回 8/4」。

**风险**：`.fig` 双倍缩放（C1）；实例 override 行不能再乘因子；`detach_edited` 必须在因子变化时跳过，否则缩放即解绑。

### 批次 4 · P2 通道与覆盖

1. A8：`automation/inspect.rs` 的 `NODE_KEYS` 加 3 项 + 3 个 `insert_prop`；恢复 `inspect_fields_tests` 一条。
2. A7 收尾：确认 duplicate 测试进文件后不与 move 测试冲突。
3. GPUI 变量面板（对照 egui `variables_select_tests.rs` 23 条）：
   - 已有 4 条（Ctrl/Shift 选择、批量菜单）；
   - 待补：Escape 清选、选中行颜色、批量删除/改分组/改 scope、40 组菜单滚动、分组复制/重命名/嵌套、Edit variables 的描边与隐藏发布、scope 页签的开关与布尔特例。
   - 先逐条对照 GPUI 是否「有实现无测试」，再决定补测试还是补行为。
   - 进展（2026-11-27）：测试从 4 条补到 14 条并全部通过，新增普通点击、失效锚点的 Shift、Escape 清选、批量删除/移动/改 scope、混合类型 scope、分组复制（含子组唯一命名与继承成员跳过）、scope 逐项勾选等用例。其中三条补齐/修正了行为：批量改 scope 菜单原先把各类型的 scope 取**并集**（已改为交集）；分组右键菜单原先缺 **Duplicate group**（已补，按 egui `duplicate_group` 生成 `variable.duplicate` + `$ref` 改名的一步撤销批次）；详情面板原先只读展示已选 scope（已补逐项 checkbox，勾选/取消写 `variable.update`，全部取消即空数组=隐藏于取色器）。其余对照项（选中行颜色、40 组菜单滚动的 UI 用例、分组重命名/嵌套的 UI 用例）仍为待补。
   - 注：右栏的 `visible` 绑定行、Layout grid 逐字段绑定、Page 级 Variable modes / Local variables 入口，已在 `docs/right-panel-visibility-parity-plan.md`（D25 / D14 / D10 / D11）单独立项，本计划不重复排期。
4. GPUI 样式绑定入口（独立项，可另行排期）：`style.bind` 目前零引用；至少补 Text 样式的 `fontSize/lineHeight/letterSpacing/fontWeight/fontFamily/fontStyle`，Paint/Effect/Grid 次之。

### 批次 5 · P3 上游化（可选）

- `variable.move` / `variable.moveCollection` 反向提交上游（含 GPUI 拖拽用例）；
- `.fig` 的 `variableConsumptionMap` 映射反向提交上游（按当前上游代码，`.fig` 导入不产出绑定）。

---

## 8. 验收基线

### 8.1 测试数（15 个变量/绑定相关引擎文件）

| 文件 | 本地 | 上游 | 补全后 |
|---|---:|---:|---:|
| `variable_tests.rs` | 6 | 6 | 6 |
| `variable_props_tests.rs` | 16 | 16 | 16 |
| `variables_behaviour_tests.rs` | 17 | 17 | 17 |
| `variables_colour_tests.rs` | 15 | 15 | 15 |
| `variables_editor_tests.rs` | 8 | 8 | **11**（保留 3 条 move + 恢复 3 条） |
| `variables_import_shapes_tests.rs` | 2 | 2 | 2 |
| `variables_io_tests.rs` | 17 | 18 | 18 |
| `library_variables_tests.rs` | 10 | 11 | 11 |
| `style_vars_tests.rs` | 7 | 12 | 12 |
| `binding_lists_tests.rs` | 9 | 9 | 9 |
| `bound_alpha_tests.rs` | 9 | 9 | 9 |
| `scale_bound_values_tests.rs` | 0 | 14 | **14** |
| `stored_ids_tests.rs` | 3 | 3 | 3 |
| `paint_reorder_tests.rs` | 8 | 8 | 8 |
| `components_scale_tests.rs` | 16 | 16 | 16 |
| **合计** | **143** | **164** | **167** |

`figcraft-format` 另加 #2441 两条；GPUI 变量面板从 4 条起补。

### 8.2 命令

```bash
cargo test -p figcraft-engine variables
cargo test -p figcraft-engine style_vars
cargo test -p figcraft-engine scale_bound_values
cargo test -p figcraft-engine library_variables
cargo test -p figcraft-format proto_variable
cargo test -p figcraft-ui-gpui variables
cargo xtask ci   # fmt + clippy -D warnings + tests + layers + assets + wasm
```

### 8.3 手工验收（变量绑定专项）

1. 绑定 gap / radius 的实例：Scale 0.5 → 显示值减半且绑定仍在；切模式 → 按新因子重算；撤销 → 复原。
2. 绑定 gap 的普通 frame：Scale 0.5 → 值落成 4 且绑定消失。
3. 实例 Detach / Restore main / main 丢失 → 长度按绘制值保留、解绑。
4. 文本样式绑字重 700：选最近字面 + `wght` 轴；绑字名后 `wght` 清除；越界 -50/5000 → Thin/Black。
5. REST 导入含 `individualStrokeWeights` 的文件：四边权重绑定齐全、无告警。
6. 库更新：仅样式使用的变量不被删，样式颜色不冻结。
7. `.fig` 往返：按 `a_text_style_binding_round_trips_through_fig` 验证 fontSize/lineHeight 绑定保留、字重按值回读并告警（与上游一致）。

---

## 9. 实施注意

1. **不要整文件 `git checkout upstream/main -- ...`**：`cmd/styles.rs`、`assets/library.rs`、`variables_editor_tests.rs`、`fig/map/` 都有本地独有内容（B1–B5），整文件会回退 move/name/type/玻璃/Icon/slot。
2. **`scale_factor` 必须三处同改**：`Node` 字段 + `collab::change::Prop` + 序列化默认值；漏 collab 会让协作端丢字段。
3. **`.fig` 不能双重缩放**：导入已经烘焙 `uniformScaleFactor`；加字段后导入路径不要同时设因子（C1）。
4. **override 行不乘因子**：`scale_of` 只看「自身 + 祖先实例」，实例 override / derived 行给的值已是实际值；照上游判断，不要给所有 `bound` 统一乘。
5. **`detach_edited` 的跳过条件**：`n.scale_factor != old.scale_factor` 时不解绑，否则 Scale 一放下就掉绑定。
6. **字重夹取与互斥**：1–1000、写 `wght`、绑名字清 `wght`；`fontWeight`/`fontStyle` 只能留一个。
7. **测试文件只追加**：`variables_editor_tests.rs` 的上游 3 条与本地 3 条 move 测试共存；整文件替换会丢 move 覆盖。
8. **GPUI 冻结件**：`ui-gpui/src/widgets/` 已声明冻结；字重 chip 复用 `widgets::variable::bound_field`，不改 widget 本体。
9. **面板 23 条是「覆盖差」不是「全缺」**：先逐条对照实现，再决定补测试或补行为，避免重写。
10. **CRLF 警告**：本仓工作树有 `LF will be replaced by CRLF` 提示，diff 里的大段「行数变化」多为换行/格式化，判断行为差异时以符号与测试名为准。

---

## 10. 建议提交切分

| # | 提交 | 范围 |
|---|---|---|
| 1 | `fix(format): REST 导入绑定描边分边权重并去重节点级列表 (#2441)` | `format/src/rest/variables.rs` + 2 测试 |
| 2 | `fix(engine): 库更新保留样式绑定的变量 (#2416)` | `assets/library.rs` + 1 测试 |
| 3 | `fix(engine): variable.duplicate 精确 id 优先；恢复 stored-id 回归测试` | `cmd/variables.rs` + `variables_editor_tests.rs` |
| 4 | `feat(engine): 文本样式可绑定数字字重 (#2174)` | `style_vars.rs`、`cmd/styles.rs`、`style_vars_tests.rs`、`variables_io_tests.rs` |
| 5 | `feat(engine): Scale 工具保留实例的绑定长度 (#2090)` | doc / collab / arrange / styles / components / library / 14 测试 |
| 6 | `feat(dev): inspect 读取 variableModes、propertyReferences、exposedInstance` | `automation/inspect.rs` + 1 测试 |
| 7 | `feat(ui-gpui): Typography 字体族/字名/字重/行高/字距绑定 chip` | `sections.rs`、`widgets::variable` 复用 |

---

## 附录 A · 上游提交 → 文件/符号

| 提交 | 主题 | 关键文件 / 符号 |
|---|---|---|
| `3053b7711` | #2090 Scale 保留绑定长度 | `doc/node.rs::scale_factor`、`collab/change.rs::ScaleFactor`、`cmd/arrange.rs::scale_node`、`cmd/styles.rs::{LENGTH_FIELDS,scale_of}`、`components/sync.rs`、`library.rs`、`scale_bound_values_tests.rs` |
| `0636a3e6a` | #2134 review：嵌套实例与转换 | `cmd/styles.rs::{bake_scale,bake_stale_scales}`、`cmd/arrange.rs::covered`、`instance.rs`、`cycles.rs`、`library.rs` |
| `d332c18a2` | #2441 REST 分边描边 | `rest/variables.rs`（`individualStrokeWeights`、`bound_by_items`、`aliases`）+ 2 测试 |
| `ccb33e0bb` | #2174 文本字重 | `assets/style_vars.rs`、`assets/token_roles.rs`、`cmd/styles.rs`、`style_vars_tests.rs`、egui `variable_ui.rs`/`style_picker.rs` |
| `281918e2f` | #2174 review：字名清轴 + CSS 裸数字 | `style_vars.rs`、`token_roles.rs`、`variables_io_tests.rs`、`style_picker.rs` |
| `e9a477773` | #2416 库更新保留样式变量 | `assets/library.rs::prune` + `library_variables_tests.rs` |
| `815cbf9fa` | duplicate 精确 id | `cmd/variables.rs::duplicate` + `variables_editor_tests.rs` |
| `2f4701491` | #1613 存储引用保身份 | `doc/styles.rs::{get_by_id,collection_by_id}` + 2 条 editor 测试 |

## 附录 B · 绑定面 matrix（当前状态）

| 绑定面 | 上游 | 本地 | 备注 |
|---|---|---|---|
| 节点：填充 / 描边 / 渐变停止点 / 效果 / 网格 | ✅ | ✅ | `binding_lists` 9/9、`variable_props` 16/16 |
| 节点：组件属性（`componentProperties/<name>`） | ✅ | ✅ | `PROP_BINDING`、`check_prop_binding` |
| 节点：透明度 / 可见性 / 文本内容 / 宽高 / 间距 / 圆角 / 字号 / 行高 / 字重 / 字族 / 字名 | ✅ | ✅ | `cmd/variables.rs:742`、`cmd/styles.rs:1194` |
| 样式：Paint / Effect / Grid | ✅ | ✅ | `style.bind` |
| 样式：Text 的 fontSize / lineHeight / letterSpacing / fontFamily / fontStyle | ✅ | ✅ | — |
| 样式：Text 的 **fontWeight** | ✅ | ❌（半接入） | A2 |
| REST：`individualStrokeWeights` 分边 | ✅ | ❌ | A3 |
| REST：节点级列表去重 | ✅ | ❌ | A3 |
| `.fig`：`variableConsumptionMap` → 绑定 | ❌（按当前上游代码） | ✅ | B2，可反向上游化 |
| Scale：实例保留绑定 / 普通层烘值解绑 | ✅ | ❌ | A1 |

## 8. 执行记录（2026-11-26）

在 `sync/variable-binding-2026-11` 上按批次 1-4 落地，基线 `5cfe98448`。

| 批次 | 提交 | 内容 |
|---|---|---|
| 1 | `e6d74397a` | REST `individualStrokeWeights` 分边 + 节点级列表去重 |
| 1 | `68288471d` | 库更新 `prune` 保留样式绑定的变量 |
| 1 | `507ac0f71` | `variable.duplicate` 精确 id 优先 + 2 条 stored-id 回归 |
| 2 | `430c6b900` | 文本样式 `fontWeight` 绑定（字段表、scope、应用、互斥） |
| 2 | `21a6c1661` | CSS 导出把仅样式绑定的字重视为无单位 |
| 3 | `36a36f894` | 并发 UI 提交一并带入了本批的 doc/collab/styles/components 改动（`scale_factor`、`LENGTH_FIELDS`、`scale_of`、`bake_scale`、`bake_stale_scales`） |
| 3 | `99750c19a` | `scale_fits` 越界拒绝、因子 1 早退、拖拽停在最后一个可用因子 |
| 4 | `bc2cd57ad` | Dev Mode `node.get` 读 `variableModes` / `propertyReferences` / `exposedInstance` |

验证（`CARGO_INCREMENTAL=0`，`-j 2`，D 盘 target）：

- `cargo test -p figcraft-format proto_variable`：10/10
- `cargo test -p figcraft-engine --lib library_variables`：11/11
- `cargo test -p figcraft-engine --lib style_vars`：12/12
- `cargo test -p figcraft-engine --lib variables_editor`：11/11
- `cargo test -p figcraft-engine --lib scale`：62/62（含 `scale_bound_values_tests` 14/14）
- `cargo test -p figcraft-engine --lib modes_property_bindings`：1/1
- `cargo test -p figcraft-engine --lib variables`：106 通过，1 失败（`fonts::tests::lists_families_of_layers_ranges_styles_and_variables_once_each`，字体族去重/环境，未涉及变量绑定，未修）

留待拍板 / 后续：

1. **C1**：`.fig` 仍未读写 `scale_factor`（跟随上游现状：导入不烘焙因子，导出不带）。JSON 存档的因子往返已有测试覆盖；若产品要求 `.fig` 往返，另开一项。
2. **C4**：GPUI 仍无 `style.bind` 入口，文本样式字重绑定面板未做；egui 侧入口已在。因 GPUI 右栏正在并发改动，本期不动，避免冲突。
3. **A9**：上游 `variables_io_tests` 的 slot 测试（P3）未移植。

## 9. PR 与合并（2026-11-27）

- PR [#34](https://github.com/MonetDesigns/Monet/pull/34) 已从 Draft 转为 Ready，base `main`，head `sync/variable-binding-2026-11`。
- 合并 `origin/main`（`599f89187`），解决 6 处冲突：`library.rs`（bake 与时间线复制并存）、`clipboard.rs`/`overlays.rs`/`paste_motion_tests.rs`（取 origin 新版）、`app.rs`（`opacity_digit` 与 `key_box_doc`/`pick` 并存）、`ui-gpui/lib.rs`（`annotation.new` 与 `selectArea`、`layers.collapseAll`/`contextMenu`、Developer Mode 拒绝与取色归属并存）。
- 合并后发现并修复重复的 `key_box_key` 与 `selectArea` 分支（`c0b2d42dd`）；`cargo check -p figcraft-engine`、`cargo check -p figcraft-ui-gpui` 均通过。
- 批次 5 上游化草稿：`docs/upstream-variable-binding-proposal.md`。
- 2026-11-27 续：合并远端 `a369c2853`（Dev Mode 缓存/审计、text-wrap、导出格式等 14 文件）无冲突；`cargo check -p figcraft-ui-gpui` 通过。GPUI 变量面板测试 9/9 通过，并修复批量改 scope 菜单把各类型 scope 取并集的 bug（改为交集）。推送后最新 head `0e2c633b`，PR Ready、`mergeable=clean`，38 commits；CI 已触发，等待结果。


