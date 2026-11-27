# 全量行为对齐审计（2026-11-25）

> 基线：本地 `main` HEAD `ff8e5e617` ↔ 上游 `storytold/figcraft` `main` `ef2a9eeef`
> 分叉：本地独有 377 提交 / 上游独有 4681 提交；780 个文件不同（引擎+UI）
> 方法：以 `#[test]` 函数名差集为行为覆盖的硬指标，再逐条验证「真缺失」还是「改名/手工移植已带」
> 性质：只读审计，未改代码

---

## 0. 量化总览

| 维度 | 上游 | 本地 | 上游独有 | 说明 |
|---|---|---|---|---|
| 引擎+支撑 crate 测试函数 | 4,305 | 4,266 | **327** | 15 个非 UI crate |
| ├ 整块文件本地没有（高置信真缺） | — | — | **27**（10 个文件） | 见 §2 |
| └ 文件在、测试名不同（多为改名/移植） | — | — | 300（83 个文件） | 需逐条验，见 §3 |
| UI 测试函数（egui 前端） | 1,354 | GPUI 503 / 冻结 egui 1,310 | GPUI 缺 **1,335** / 两边都缺 **44** | 见 §4 |
| 命令名集合（`spec!`） | 652 | 702（多 50 条本地独有） | **0** | 上游独有命令为 0，命令名完全对齐 |

> 所有数字用统一方法复核：两边都用 `git grep -h -A2 '#[test]' <rev> -- <paths>` 提取函数名后 `comm -23`，避免 grep/git grep 行为差异造成假差集。

**一句话结论**：命令名层面完全对齐；引擎层约 27 个高置信真缺失行为 + 300 个待验证（多数是手工移植改名）；UI 层 GPUI 相对上游 egui 缺约 1,335 个测试覆盖的行为，其中 44 个连本地冻结的 egui 参照也没有（纯上游新增）。

---

## 1. 方法说明（为什么不能只看提交数/git diff）

- 本地大量上游功能是**手工移植**而非 cherry-pick：`adad8a08a`（重复旋转步进）、`e94ecfca7`（旋转父层方向键）、`d6bfe6893`（frame 内粘贴）、`4c71211e2`（矢量点对齐）、`cfd6bf527`（tidy 保序）等提交号都不在本地，但功能和测试都在。
- `git diff HEAD upstream/main` 显示 19.5 万行删除，绝大部分是本地独有 `ui-gpui`（上游根本没有这个 crate）和品牌重命名（Monet ↔ FigCraft），不是"落后"。
- 测试函数名差集是最接近"行为差"的指标：每条测试对应一个可验证契约；但仍需区分改名、重构、手工移植。

---

## 2. 引擎层：27 个高置信真缺失行为（上游有测试、本地连文件都没有）

| 文件 | 测试数 | 行为 | 优先级 |
|---|---|---|---|
| `crates/text/src/embedded.rs` | 5 | 内嵌字体：压缩字节往返、损坏流拒绝、size 上限、解码缓存、缺字体只报一次 | 中 |
| `crates/format/src/fig/guides_tests.rs` | 6 | .fig 导出 ruler guides：页面/frame 的 guides、偏移非有限值剔除、instance override、schema 编号 | 中 |
| `crates/engine/src/library_asset_counts_tests.rs` | 7 | 库用量统计：应用/复制/复制粘贴/实例各计一次、Detach 才减、本地样式不计、撤销回退、消息上限覆盖 | 中 |
| `crates/engine/src/pencil_shift_tests.rs` | 2 | Shift+Pencil 画 45° 直线（已知与本地笔刷体系冲突，需分层：几何约束→笔刷盖章） | 低（有冲突） |
| `crates/engine/src/automation/batch.rs` | 2 | 批量引用扩容上限：大列表入队前拒绝、引用拷贝有界 | 中 |
| `crates/engine/src/library_style_hops_tests.rs` | 1 | 跨库 style hop：第二个库的组件用第一个库样式时取绑定副本而非冻结副本（#2414） | 中 |
| `crates/engine/src/cmd/file/export.rs` | 1 | 导出文件名在后缀之后、编号之前做设备名转义 | 低 |
| `crates/engine/src/extensions/widgets.rs` | 1 | widget 的多张图片共享解码像素预算 | 低 |
| `crates/collab-net/src/net.rs` | 1 | loopback-only dial 跳过非本机地址 | 中 |
| `crates/collab-net/src/requests.rs` | 1 | server key 不通过明文 TCP 发给其他机器 | 中 |

---

## 3. 引擎层：300 个共享文件内的测试名差（需逐条判定）

按文件分布（TOP 20，测试数）：

| 测试数 | 文件 | 初步判定 |
|---|---|---|
| 20 | `engine/src/components_behaviour_tests.rs` | 组件行为，功能已在（有文件） |
| 14 | `text/src/fontdb_tests.rs` | 字体数据库 |
| 13 | `engine/src/duplicate_spacing_tests.rs` | 重复步进（已手工移植，测试名差） |
| 12 | `engine/src/automation/file_access/tests.rs` | 路径安全加固（已带，符号存在） |
| 11 | `format/src/fig/board_import_tests.rs` | .fig 看板导入（文件在本地） |
| 10 | `engine/src/dev/codegen.rs` | 代码生成 |
| 10 | `geom/src/tests.rs` | 几何 |
| 8 | `engine/src/board_tests.rs` | 看板 |
| 8 | `engine/src/view_behaviour_tests.rs` | 视图 |
| 8 | `render/src/image_profile_tests.rs` | 图片 profile |
| 7 | `engine/src/al_behaviour_tests.rs` | 自动布局 |
| 7 | `engine/src/board_tables_tests.rs` | 看板表格 |
| 7 | `engine/src/export_behaviour_tests.rs` | 导出 |
| 7 | `format/src/fig/text_path_tests.rs` | 文本路径（文件在本地） |
| 6 | `doc/src/text.rs` | 文本模型 |
| 6 | `engine/src/slot_tests.rs` | slot |
| 5 | `engine/src/shape_handles_tests.rs` | 形状手柄 |
| 5 | `engine/src/style_vars_tests.rs` | 样式变量 |
| 5 | `engine/src/text_tail_tests.rs` | 文本尾部 |
| 5 | `engine/src/transform_behaviour_tests.rs` | 变换 |

> 完整逐条判定见各分路代理结果（下方 §3.1–§3.3）。这些文件本地都存在且有其他测试，**大多数是上游新增测试/改名，不代表功能缺失**，但需抽查确认。

---

## 4. UI 层：GPUI 未还原行为

### 4.1 量化

| 指标 | 数值 |
|---|---|
| 上游 egui 测试函数 | 1,354 |
| 本地 GPUI 测试函数 | 503 |
| 本地冻结 egui 测试函数 | 1,315 |
| GPUI 相对上游缺失 | **1,335** |
| 两边（GPUI + 冻结 egui）都缺 = 纯上游新增 | **44** |

### 4.2 纯上游新增的 44 个（两边都没带）按主题

| 主题 | 测试数 | 行为 |
|---|---|---|
| autopan / 边缘自动平移 | 14 | 拖到边缘自动滚动画布、速度随距离、Hand 工具/未成拖动/缩放时不触发、shape/selection box 在边缘增长、200% 下平移一致 |
| present / HIDPI 视图细节 | 10 | tall frame 在 hidpi 按设备像素绘制、detail 保留/重建、整块纹理替换、固定设备与 sticky 到页面末端 |
| Shift 范围跨 frame（#2330） | 4 | anchor 被选中 frame 覆盖时保留 frame、跨 frame 反向范围扩展、单 frame 内不变 |
| library 用量统计 | 4 | totals/period/list、旧 server 不支持的 kind 无 totals、period 行求和 |
| 矢量点对齐 UI（#2116） | 4 | 点编辑模式显示对齐行、三点的分布菜单、Alt+D 对齐点 |
| variant 值展示 | 3 | 选中 variant 下拉显示自身值、超长截断省略号、hover 显示完整、读屏保留完整值 |
| 其他 | 5 | 类型排版上下标、页面菜单复制链接、安装的 identity 稳定 |

### 4.3 GPUI 相对冻结 egui 的未覆盖（按 UI 区域）

按测试所在文件统计 TOP 20：

| 未还原测试数 | 上游 UI 测试文件 | 区域 |
|---|---|---|
| 42 | `field_tests.rs` | 数值字段编辑 |
| 33 | `home_tests.rs` | 首页 |
| 32 | `layers_behaviour_tests.rs` | 图层面板 |
| 28 | `i18n/tests.rs` | 国际化 |
| 25 | `control/capture_tests.rs` | 控制通道 |
| 25 | `paste_image_tests.rs` | 图片粘贴 |
| 25 | `present/tests.rs` | 演示 |
| 25 | `property_rows_tests.rs` | 属性行 |
| 23 | `canvas/zoom_preview_tests.rs` | 缩放预览 |
| 23 | `present_links_tests.rs` | 演示链接 |
| 23 | `variables_select_tests.rs` | 变量选择 |
| 22 | `comments_tests.rs` | 评论 |
| 22 | `ime_tests.rs` | 输入法 |
| 19 | `a11y_tests.rs` | 无障碍 |
| 19 | `idle_tests.rs` | 空闲状态 |
| 19 | `windows_tests.rs` | 多窗口 |
| 17 | `panels/selection_colors.rs` | 选择色 |
| 17 | `selection_spacing_tests.rs` | 多选间距 |
| 15 | `board_ui_tests.rs` | 看板 UI |
| 14 | `home_recents_tests.rs` / `live_share_tests.rs` / `panel_width_tests.rs` / `present/behaviour_tests.rs` / `save_as_tests.rs` / `widgets/style_picker/tests.rs` | 各区域 |

---

## 5. 与专项审计的关系

本文件是全量量化补充；多选/批量相关的详细缺口（含 #1–#34）见 `multiselect-batch-gap-plan.md`。两份文件中的重叠项以专项文件为准。

---

## 6. 画布键盘交互整块缺失（2026-11-26 用户实测追加）

> 用户实测：画布选中组件后按方向键不动。排查后确认这不是单个键的问题，而是**上游 egui 在 `key_event` 里的一段硬编码键盘处理整体没移植到 GPUI**。

### 6.1 根因（已逐行确认）

上游 `crates/ui-egui/src/shortcuts.rs` 的 `key_event()` 在 `keymap::lookup` 之后有一段**不走 keymap 表、直接按 Key 枚举分派**的代码（约 :483-522）：方向键 → nudge / nudgeSize / rotate / pan / crosshair。这段是硬编码在 UI 层，不在 `keymap.rs` 的 `k()` 绑定表里。

GPUI 的链路是：画布 `on_key_down` → `session.tool_key()`（只处理矢量编辑/参考线/形状构建器）→ `keymap::lookup(Context::Canvas)`（只扫 `KEYS` 的 `k()` 绑定）→ `run_binding`（只有 `select.none`、`edit.delete` 两个特判）。而 `keymap.rs` 里方向键**只出现在 GESTURES 文档表**（`:370-371`，用于快捷键面板展示），不在 `k()` 绑定中。

所以普通画布选中图层按方向键：`tool_key` 返回 false → `lookup` 返回 None → 什么都不发生。**`control.rs:281` 里那段方向键代码是控制端口 `ui.key` 注入，真实键盘不走那里**，容易误判成"已实现"。

### 6.2 缺失清单（上游 `key_event` 分支逐条对照）

| # | 按键 | 上游行为 | 上游命令 | GPUI 现状 |
|---|---|---|---|---|
| K1 | ← ↑ → ↓ | 移动选中图层（Shift 大步） | `transform.nudge` | **完全缺失** |
| K2 | Cmd/Ctrl + 方向键 | 按 nudge 量改尺寸 | `transform.nudgeSize` | **缺失** |
| K3 | Cmd/Ctrl + Alt + 方向键 | 旋转 1°（Shift 15°），绕旋转原点 | `transform.rotate` | **缺失** |
| K4 | 方向键（无选区） | 平移视图 | `view.pan` | **缺失**（命令在，无键路径） |
| K5 | 方向键（形状工具已选、无选区） | 视图中心十字线移动，Enter 放置 | `tool.crosshair` | **缺失**（命令在，0 引用） |
| K6 | 两位数字键 | 直接设不透明度（4 5 → 45%） | `object.set` opacity | **缺失** |
| K7 | 方向键（Layers 面板聚焦） | 图层键盘导航（上下游走） | `select.set` | **缺失**（`layer_tree.rs` 无方向键处理） |
| K8 | Alt+Space / F8 / Ctrl+Space | 键盘框选 | `select.area` → UI `selectArea` | **缺失**（已知，UI 请求未接） |
| K9 | Tab + 方向键 + Space | 键盘拾取（pick select） | — | **缺失**（0 引用） |
| K10 | Enter（形状工具已选） | 在视图中心放置形状 | `tool.place` | **缺失**（Enter 只在文字编辑里处理） |
| K11 | Enter / Escape | 退出裁剪模式 | — | **缺失**（`app.crop` 状态在，无键盘退出） |
| K12 | Delete / Backspace | 删除选中的原型交互 | `proto.*` | **缺失**（0 引用） |
| K13 | Escape | 取消 Design 面板高亮的属性行 | `property_rows::escape` | **缺失**（0 引用） |
| K14 | Escape | 取消多选间距拖拽 | `selection_spacing::cancel` | **函数在但无调用点** |
| K15 | 方向键（Dev Mode） | 拒绝并提示 | `dev_mode_refuses` | **缺失**（0 引用） |

### 6.3 实施注意（比补齐分支更容易踩的坑）

1. **焦点链**：GPUI 按键沿焦点链派发，画布 `on_key_down` 只覆盖画布子树。焦点在右栏输入框/列表行时（改完属性、提交对话框后很常见），方向键到不了画布分支。上游 egui 是全局 `key_event`，没有这个问题。补齐时**必须在根节点 `on_key_down` 也兜底方向键**（参考现有 `edit.undo`/`edit.redo` 的根节点兜底做法，`lib.rs:8807` 附近），否则会变成"有时能动有时不能动"。
2. **输入框不能抢**：根节点兜底时必须排除「焦点在文本输入框/内联改名/变量面板编辑」的情况，否则方向键会被用来移动图层而不是移光标。
3. **nudge 量**：`prefs.nudge_small` / `nudge_big`（Shift），不要写死 1px。
4. **矢量编辑模式优先**：`session.tool_key` 已经在矢量编辑模式下处理方向键（推点），新分支必须放在它之后，不要抢。
5. **Dev Mode**：方向键应先走 `dev_mode_refuses` 提示，不能真的移动。

### 6.4 建议

P0，一次补全 K1–K5（同一个方向键分派块），K6–K15 随后。补完加一组回归测试：普通选中移动、Cmd 改尺寸、Cmd+Alt 旋转、无选区平移、形状工具十字线，各一条；再加一条「焦点在右栏时方向键仍能移动选中层」。
