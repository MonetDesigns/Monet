# 上游化提案：变量行移动与 `.fig` 变量消费映射

面向 `storytold/figcraft` 的两项本地领先能力，建议拆成两个独立 PR，互不依赖。基线为上游 `e1361cb15`（2026-11-26）。

## 1. `variable.move` / `variable.moveCollection`

### 现状

上游变量编辑器只有创建、重命名、改值、删除与模式操作；变量和集合在列表里的顺序只能由删除后重建改变。本地（Monet `cmd/variables.rs`）补了两条命令，对应 Variables 面板的行拖拽：

```text
variable.move {variable, before?, group?} → {name}
variable.moveCollection {collection, before?} → {name}
```

语义：

- `variable` 接受 id 或名字；`before` 接受同集合的变量 id/名字，缺省或 null 表示放到该集合末尾。
- `group` 是目标文件夹路径；缺省时跟随 `before` 所在文件夹，否则保持原文件夹。`""`/null 表示移出所有文件夹。
- 移动与换文件夹是一个原子操作（一次 undo），因为表格按名字分组，行落在别的组里就必须属于那个组。
- 继承变量（alias 的 `extends`）拒绝移动；跨集合移动也拒绝，需先 `variable.moveCollection` 或用 `variable.duplicate`。
- `variable.moveCollection` 同理：`before` 为 null/缺省放末尾。

### 上游接入点

| 文件 | 改动 |
|---|---|
| `crates/engine/src/cmd/variables.rs` | 注册两条 `spec!`；实现 `move_variable` / `move_collection`；沿用 `bad`/`inherited` 错误形状 |
| `crates/engine/src/variables_editor_tests.rs` | 3 条本地测试：行移动、换文件夹、移动集合；断言 undo 一步还原 |
| `crates/ui-egui/src/panels/variable_ui.rs` | 行拖拽落到 `variable.move`；集合头拖拽落到 `variable.moveCollection` |

### 兼容性

- 纯新增命令，不改存储模型与文件格式；旧客户端不认识的请求按未知命令拒绝。
- 排序是 `doc.variables.variables` 的 Vec 顺序，REST/`.fig` 导入按文件顺序落地，无需迁移。

## 2. `.fig` 的 `variableConsumptionMap` → `boundVariables`

### 现状

上游 `.fig` 导入只读取变量与集合定义、模式，不把层上的 `variableConsumptionMap` 变成 `Node.bound`；导出也不写回。结果是打开一个带变量绑定的 Figma 文件后，绑定全部丢失，只剩变量定义。

本地 `crates/format/src/fig/map/variables.rs` 做了完整映射：

- 每个 `entries[]` 的 `variableData`（GUID）解析为变量 id，`variableField` 按字段表映射到绑定键：
  - `BORDER_TOP_WEIGHT` / `BORDER_RIGHT_WEIGHT` / `BORDER_BOTTOM_WEIGHT` / `BORDER_LEFT_WEIGHT` → `strokeTopWeight` / `strokeRightWeight` / `strokeBottomWeight` / `strokeLeftWeight`；
  - `CORNER_RADIUS` → 四个圆角键；
  - 文本字段进入 `textStyle.boundVariables`（`fontFamily` / `fontStyle` / `fontSize` / `fontWeight` / `lineHeight` / `letterSpacing`）；
  - 其余按 `binding_key` / `text_binding_key` 表落到 `fills/<i>/color`、`strokes/<i>/color`、`effects/<i>/<field>`、`layoutGrids/<i>/<field>` 等。
- `explicitVariableModes` → `Node.var_modes`；库资产里的变量 GUID 解析到文件内最新副本。
- 导出侧 `crates/format/src/fig/export/{layer,assets,common}.rs` 反向写 `variableConsumptionMap`，未知字段落空对象，保持 Figma 可读。

### 上游接入点

| 文件 | 改动 |
|---|---|
| `crates/format/src/fig/map/variables.rs` | 新增 `variable_bindings`（约 90 行）与 `binding_key` / `text_binding_key` 字段表 |
| `crates/format/src/fig/map/mod.rs` | 在层映射处调用，合并 `boundVariables` 与文本样式绑定 |
| `crates/format/src/fig/export/layer.rs` | 写回 `variableConsumptionMap.entries` |
| `crates/format/src/fig/assets_tests.rs` | 3 条本地测试：读绑定、读模式、导出字段 id 稳定 |

### 兼容性 / 风险

- 只影响 `.fig` 编解码，不碰 REST 与内部模型；未绑定层输出空 map，与 Figma 默认一致。
- 字段表是白名单：不认识的 `variableField` 跳过而不是报错，避免新版 Figma 字段导致整文件导入失败。
- 圆角与分边描边是展开映射（一个字段对多个键），导出时按优先级取第一个非空键，避免重复条目。

## 建议的提交顺序

1. PR A：`variable.move` + `variable.moveCollection`（命令 + 测试 + egui 拖拽），最小可审。
2. PR B：`.fig` 变量消费映射（仅 format crate + 测试），与 A 无耦合。

两项都不改 `Node`/`Variable` 模型，适合作为独立上游 PR；本地已在 `sync/variable-binding-2026-11` 验证。
