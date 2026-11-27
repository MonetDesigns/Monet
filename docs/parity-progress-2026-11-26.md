# 对齐进度（2026-11-26）

> 基线：本地 `main`（`ff8e5e617` 起）↔ 上游 `storytold/figcraft` `main`（`ef2a9eeef`）
> 本文件记录已落地的改动与验证情况；缺口总表仍见 `full-parity-audit-2026-11-25.md`、
> `multiselect-batch-gap-plan.md`。

---

## 一、已完成（已编译通过）

### 1. 画布键盘交互（`full-parity-audit` §6，K1–K6 / K10 / K11 / K14）

| 项 | 行为 | 改动位置 |
|---|---|---|
| K1 | 方向键移动选中图层 | `ui-gpui/src/lib.rs` 新增 `arrow_key()`；画布 `on_key_down` 接入 |
| K2 | Cmd/Ctrl+方向键改尺寸 | 同上（`transform.nudgeSize`） |
| K3 | Cmd/Ctrl+Alt+方向键旋转 1°/15° | 同上（`transform.rotate`，绕 `origin_point`） |
| K4 | 无选区方向键平移视图 | 同上（`view.pan`） |
| K5 | 形状工具已选 + 无选区方向键移动放置十字线 | 同上（`tool.crosshair`） |
| K6 | 两位数字键设不透明度 | `lib.rs` 新增 `OPACITY_CHAIN_SECS` / `digit()` / `opacity_key()`；`app.rs` 加 `opacity_digit` 字段 |
| K10 | Enter（形状/框架工具已选）放置形状 | `run_binding()` 加 `select.children if draws_shape → tool.place` |
| K11 | Enter/Escape 退出裁剪模式 | 画布 `on_key_down` 开头清 `app.crop` |
| K14 | Escape 取消多选间距拖拽 | 画布 `on_key_down` 调 `canvas::selection_spacing::cancel` |
| — | **焦点链兜底** | 根节点 `on_key_down` 也调用 `arrow_key`（焦点在右栏时方向键仍生效） |

**背景**：上游把这段硬编码在 `ui-egui/src/shortcuts.rs::key_event`（:483-522），不走 `keymap` 的 `k()` 表；GPUI 之前只移植了 `keymap::lookup`，所以方向键无反应。`control.rs:281` 的方向键是控制端口注入，不是真实键盘。

**注意**：根节点兜底只在「无对话框、无文字编辑」时生效；焦点在文本输入框时输入框会先消费并停止冒泡，不会误移动图层。

### 2. 锁定层批量规则（`full-parity-audit` / `multiselect` #1–#3）

| 项 | 行为 | 改动位置 |
|---|---|---|
| #1 | `transform.nudgeSize` 跳过锁定层及其子层 | `engine/src/cmd/edit/select.rs::nudge_size` |
| #2 | `object.toggle`（Lock/Unlock）用 `locked_in` 判断，并跳过锁定父层的子层 | 同文件 `toggle()`（`under_lock` 闭包） |
| #3 | 注册 `mod lock_behaviour_tests;`，让这批测试真正运行 | `engine/src/lib.rs` |

### 3. Component 多选下拉（`multiselect` #5）

- `ui-gpui/src/panels/right/sections.rs` 新增 `component_button()`：多选（`count > 1`）时把「Create component」换成下拉，提供 `component.create` / `component.createMultiple` / `component.createSet`；单选维持原按钮。

### 4. 拖动选中 frame 的子层带走整个选区（`multiselect` #7，上游 `4806b95e8` #2360）

| 改动 | 位置 |
|---|---|
| 加 `in_selected` 闭包，并入 `behind_sel` | `engine/src/canvas/tools/drag.rs` |
| `keep` 判定加 `in_selected(*c)` | 同上 |
| 单选也要收窄：`keep.filter(|_| sel.len() > 1)` → `keep.filter(\|k\| sel.as_slice() != [*k])` | 同上 |
| 点击不拖动时同样收窄：`sel.len() > 1 && …` → `sel.as_slice() != [k] && …` | `engine/src/canvas/tools/draw.rs` |
| 移植上游两个测试 | `engine/src/selection_behaviour_tests.rs`（`frame_with_child` + 两个用例） |

**关键点**：`select_target` 的 `top_is_board` 分支本来就返回子层，所以「点击选中子层」缺的其实是 `sel.len() > 1` 这个门控——单选时会挡住收窄。

### 5. 带圆角形状转矢量保留顶点圆角（`multiselect` #33/#34，用户 2026-11-25 提出）

| 改动 | 位置 |
|---|---|
| 新增 `semantic_network(n)`：矩形有圆角时用「直角骨架 + 每点 `Vertex.radius`」构网络 | `engine/src/canvas/vectoredit.rs` |
| `ensure_vector` 改用它；层的 `radii` 清零但保留 `smoothing`（`drawn_network` 靠它还原圆角） | 同上 |
| 矢量编辑态粘贴形状时也用它，圆角数据不再丢失 | `engine/src/cmd/edit/vector_clipboard.rs::outline_network` |
| 新增测试：转矢量后 4 顶点各带 radius、`smoothing` 保留、轮廓面积不变、`vector.edit.cornerRadius` 仍可调 | `engine/src/canvas/vectoredit/tests.rs` |

**原理**：`node_outline` 返回的是**已画好圆角**的路径，`from_path` 反推只能得到曲线段、顶点 radius 全 0。改为从 `rounded_rect(w, h, [0;4], 0)` 的直角路径取点、再把 `corners.radii` 写进 `Vertex.radius`；绘制结果由 `drawn_network` 按 radius 还原，与转换前一致（测试断言面积差 < 0.5）。

---

## 二、验证

- `cargo check -p figcraft-ui-gpui --lib` → **通过**（0 error）。
- `cargo check -p figcraft-engine --lib --tests` → **通过**（0 error，含新注册的 `lock_behaviour_tests`）。
- `cargo test -p figcraft-engine --lib lock_behaviour_tests` → **5 passed / 0 failed**。
- `cargo test -p figcraft-engine --lib a_child_of_a_selected_frame` → **2 passed / 0 failed**
  （`a_drag_from_a_child_of_a_selected_frame_moves_the_frame`、
  `a_click_on_a_child_of_a_selected_frame_selects_the_child`）。

---

## 三、未完成（下一批）

### 键盘剩余

| 项 | 行为 | 说明 |
|---|---|---|
| K7 | Layers 面板聚焦时方向键导航图层 | 需要图层面板的焦点判定，未做 |
| K8 | Alt+Space / F8 键盘框选 | 需实现 `selectArea` UI 请求 + 框选状态机 |
| K9 | Tab + 方向键 + Space 键盘拾取 | 需实现 pick-select 状态机 |
| K12 | Delete 删除选中的原型交互 | 需接 proto 面板的交互选择状态 |
| K13 | Escape 取消 Design 面板高亮的属性行 | GPUI 暂无「属性行高亮」状态 |
| K15 | Dev Mode 下方向键提示 | 目前已消费按键但不移动；提示 toast 待补 |

### 多选 / 批量

| 项 | 行为 |
|---|---|
| #4 | 混合选区相对编辑（`+10` / `*2` / `50%` 按每层自身值算）——需让面板数据携带每层值 |

### UI 请求接线

`selectArea` / `layers.collapseAll` / `panel.layers` / Copy as 四项 / `present.options` / `window.fullscreen` 等（见 `full-parity-audit` §3）。

### 纯上游新增

UI 44 项（autopan 14、present/HIDPI 10、Shift 范围跨 frame 4、library 用量统计 4、矢量点对齐 UI 4、variant 值 3、其他 5）；引擎 27 项（text/embedded 5、fig guides 6、library asset counts 7、pencil shift 2、automation/batch 2 等）。

---

## 四、方法提醒

- **提交前用路径白名单**，别 `git add -A`（仓库有 `target_old/`、`.tmp_shots/` 等历史垃圾）。
- `cargo test -p figcraft-engine --lib` 的**链接**极慢（30 分钟以上），日常验证优先用 `cargo check --lib --tests`。
- 判缺口不能只看 git 提交/测试名：本地大量功能是手工移植，`tool.place`、`place()` 其实早已存在（本次一度重复添加，已删除）。
