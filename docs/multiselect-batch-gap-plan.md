# 多选 / 批量操作缺口清单与补全计划

> 审计日期：2026-11-25
> 基线：本地 `main` HEAD `e0cf9e50f` ↔ 上游 `storytold/figcraft` `main` `ef2a9eeef`
> 用途：后续「一次选中多个画布组件并统一处理」相关补全工作的计划底稿
> 性质：只读审计结论，未改动代码

---

## 0. 结论摘要

**底层能力基本齐了，缺的是行为细节和前端入口。**

- 命令名层面：上游与本地在 `select.*` / `align.*` / `distribute.*` / `arrange.*` / `bool.*` / `object.*` 上**完全一致**，无遗漏命令。
- 引擎 `selection/` 五个文件（rename / tidy / colors / reorder / spacing）与上游逐字节相同。
- 选择模型本身就是多选：`DocState.selection: Vec<NodeId>`（`crates/engine/src/doc_state.rs:73`），统一入口 `targets()/req_targets()`（`crates/engine/src/automation/params.rs:128-169`）让几十条命令不传 `ids` 就作用于整个选区。
- 缺口集中在三处：① 上游已合并进主线、本地手工移植时漏掉的行为细节；② GPUI 前端没接的入口/UI 请求；③ 上游仍停在 open PR、两边都还没有的改进。

---

## 1. 缺口总表

> 优先级：P0 = 多选行为错误/主入口缺失，P1 = 功能通道断裂，P2 = 交互保真，P3 = 健壮性/待观察
> 类别：A = 上游已合并、本地缺（确定性缺口，应补）；B = 上游 open PR、两边都无（待观察）；C = EGUI 有、GPUI 缺（移植缺口）；D = 上下游都一样的既有行为缺口（自研要改，非带入问题）

| # | 缺口 | 类别 | 层级 | 上游依据 | 本地证据 | 优先级 |
|---|---|---|---|---|---|---|
| 1 | `nudge_size` 未跳过锁定层 | A | 引擎 | `24cf70329` (#1560/#2219) | `cmd/edit/select.rs:221` 无 `locked_in` 守卫 | P0 |
| 2 | 批量 lock/unlock 不认「锁定父层的子层」 | A | 引擎 | `24cf70329` | `cmd/edit/select.rs:243 toggle()` 用 `n.locked` | P0 |
| 3 | `lock_behaviour_tests.rs` 未注册，测试没跑 | A | 引擎/测试 | 上游 `lib.rs:438` 有 `mod` | 文件在，本地 `lib.rs` 无注册 | P0 |
| 4 | 混合选区相对编辑 `+10`/`*2`/`50%` | C | UI | `ui-egui/panels/right.rs:71-93 field_of()` | `ui-gpui/lib.rs:3101-3113` 同值发全部 ids | P0 |
| 5 | `component.createMultiple/createSet` 无头部下拉 | C | UI | `ui-egui/panels/right.rs:560,601-625` | `ui-gpui` 全仓 0 引用 | P0 |
| 6 | `pasteOver` 多选插在第一层 | B | 引擎 | PR #2608（open，依赖 #2471） | `cmd/edit/clipboard.rs:363-377 sel.first()` | P0 |
| 7 | 拖动选中 frame 的子层不带走选区 | A | 引擎 | `4806b95e8` (#2360) | `canvas/tools/drag.rs` 无 `in_selected` | P1 |
| 8 | 键盘框选 Alt+Space/F8 无效（`selectArea` 未接） | C | UI 请求 | `ui-egui/app/commands.rs:241` | `ui-gpui` 零命中 | P1 |
| 9 | `layers.collapseAll` 未接 | C | UI 请求 | `ui-egui/app/commands.rs:239` | `cmd/edit.rs:326` 发出，GPUI 零命中 | P1 |
| 10 | `panel.layers` 请求未接 | C | UI 请求 | `ui-egui/app/commands.rs:168` | `panels/extra.rs` 无分支 | P1 |
| 11 | Copy as CSS/SVG/PNG/Link 永久灰显 | C | UI | `ui-egui/menus.rs:33-48,114-160` | 无 `spec!` 注册，`can_run()` 恒 false | P1 |
| 12 | Layout 面板多选 Spacing 数字框 | C | UI | `ui-egui/canvas/selection_spacing.rs:120-137 panel()` | `ui-gpui/canvas/selection_spacing.rs` 无 `panel()` | P1 |
| 13 | 图层树 eye/padlock 批量拖拽扫掠 | C | UI | `ui-egui/panels/layers.rs:472-482,895-910,979-1047` | `ui-gpui/layer_tree.rs:788-824` 固定单 id | P1 |
| 14 | Tidy up 无 `can_tidy` 灰显门控 | C | UI | `ui-egui/panels/right.rs:777-798` | `ui-gpui/panels.rs:2140-2142`，`can_tidy` 0 引用 | P2 |
| 15 | `layout.add` 头部按钮漏掉 Group | C | UI | `ui-egui/panels/layout_panel.rs:30-52` | `ui-gpui/panels.rs:598-612 all_frame` 不含 Group | P2 |
| 16 | 对齐行「整行不画」而非灰显+说明 | C | UI | `ui-egui/panels/right.rs:743-773` | `ui-gpui/panels.rs:2561-2565` | P2 |
| 17 | 布尔按钮直点即执行（EGUI 只开菜单） | C | UI | `ui-egui/panels/right.rs:577-600` | `ui-gpui/panels/right/sections.rs:3669-3676` | P2 |
| 18 | 批量重命名对话框无 Enter 提交 | C | UI | — | `ui-gpui/dialogs.rs:502-527` 仅按钮 action | P2 |
| 19 | 图层行内改名无 Tab/Shift+Tab | C | UI | `ui-egui/panels/layers.rs:1143-1160 take_tab` | `ui-gpui/layer_tree.rs` 无 | P2 |
| 20 | 布尔运算不处理无描边文本轮廓 / 圆角点烘焙 | A | 引擎 | `64b0b4510` (#2405), `641cc4159` (#2195) | `cmd/structure.rs` 缺 `flatten(NodeKind::Text)` 分支 | P2 |
| 21 | Layers Shift 范围跨 frame 不保留 frame | A | UI | `b5d43a7de` (#2330) | `ui-gpui/layer_tree.rs:1347-1381` 按可见行全取 | P2 |
| 22 | 批量步骤被拒时不带步骤号 | A | 引擎 | `2462a8625` (#2181) | `error.rs:38-41` 仅 BadParams/Other | P3 |
| 23 | 批量引用无 8MB 扩容上限 | A | 引擎 | `3b9bf8c9a` | `automation/batch.rs:15` 仅 MAX_DEPTH=64 | P3 |
| 24 | 框选分支：聚焦画框 / widget 过滤 | A | 引擎 | `cb5262141` (gap-4) | `canvas/tools/pointer.rs:52` 改用 `display_doc` | P3 |
| 25 | `select.layersAt` 不返回 `locked` 字段 | A | 引擎 | 上游 `cmd/edit.rs` | 本地无 | P3 |
| 26 | text-on-path / pathOptions / removeFromPath | C | UI | `ui-egui/panels/typography.rs:126-157` | `ui-gpui` 全仓 0 引用 | P3 |
| 27 | 多选导出无 job 文件名列表（最多 6 个） | C | UI | `ui-egui/dialogs/export.rs:113-130` | `ui-gpui/dialogs.rs:630-657` 静态文案 | P3 |
| 33 | **带圆角形状转矢量时圆角被烘焙进路径，未保留为顶点圆角数据** | D | 引擎 | 上游同样如此；issue #2519（open，相关但未覆盖） | `canvas/vectoredit.rs:324-346 ensure_vector` 走 `node_outline`→`from_path` | P1 |
| 34 | 矢量编辑态粘贴圆角矩形同样丢顶点圆角（与 #33 共根因） | D | 引擎 | 上游同样 | `cmd/edit/vector_clipboard.rs:186 outline_network` 同样烘焙 | P1 |
| 35 | **画布方向键移动选中层整块缺失**（K1–K5 + 10 条同类键盘行为） | D | UI/引擎 | 上游 `ui-egui/src/shortcuts.rs:483-522 key_event` 硬编码分派 | GPUI `run_binding` 只有 2 个特判，方向键不在 keymap `k()` 表 | P0 |
| 28 | 粘贴属性保留样式/变量/layout grids | B | 引擎 | PR #2620（open） | 两边都无 | 待观察 |
| 29 | Selection colors 的 Styles 按钮 | B | UI | PR #2615（open） | 两边都无 | 待观察 |
| 30 | 多个同旋转角图层的单一旋转框 | B | UI | PR #2329（open） | 两边都无 | 待观察 |
| 31 | Shift+双击深入且保留选区 | B | 引擎/UI | #2483（open） | 两边都无 | 待观察 |
| 32 | Ctrl 连续选择不工作（bug） | B | UI | #1761（open） | 两边都无 | 待观察 |

---

## 2. 引擎层缺口（类别 A，应补）

### 2.1 锁定层批量规则 `24cf70329`（#1560/#2219，已在上游 main）

上游 `crates/engine/src/cmd/edit/select.rs` 的三处改动，本地只带了第一处：

1. `nudge()`（普通方向键微移）已有 `locked_in` 过滤 —— 本地 `:182` 已带。
2. `nudge_size()`（Cmd+方向键改尺寸）循环开头缺：
   ```rust
   if crate::tools::locked_in(s.doc()?, i) { continue; }
   ```
   本地 `:221` 无。
3. `toggle()`（批量 lock/unlock）本地仍用 `n.locked` 判断，且直接 `for i in &ids`；上游改用 `locked_in(doc, *i)` 计算值，并用 `under_lock` 闭包跳过锁定父层的子层：
   ```rust
   let under_lock = |i| doc.ancestors(i).into_iter().any(|a| doc.node(a).is_some_and(|n| n.locked));
   let skip = if what == "locked" && explicit.is_none() { ... } else { Vec::new() };
   ```

**注意这个坑**：`crates/engine/src/lock_behaviour_tests.rs` 文件在本地存在（6602 字节），但 `crates/engine/src/lib.rs` 里**没有** `mod lock_behaviour_tests;`（上游 `lib.rs:438` 有）。所以这批测试根本没在跑，缺口一直被掩盖。补功能时先把 `mod` 注册上，让测试先红再修。

GPUI 侧还有配套的 UI 缺口：图层树没有 `locked_above` 概念（EGUI `panels/layers.rs:504,613,996` 用它做锁定父层子项的灰显和 padlock sweep 过滤）。

### 2.2 拖动选中 frame 的子层 `4806b95e8`（#2360，已在上游 main）

上游 `canvas/tools/drag.rs` 加了 `in_selected` 闭包：
```rust
let in_selected = |t| sel.iter().any(|s| *s != t && doc.is_ancestor(*s, t));
```
并把它并入 `behind_sel` 和 `keep` 判定，使「按在选中 frame 的子层上拖动 → 带走整个选区；单击 → 选中该子层；Cmd/Ctrl 保持深入选取」成立。本地只有反向的 `doc.is_ancestor(t, *s)` 判断。

上游新增的两个测试本地都没有：
- `a_drag_from_a_child_of_a_selected_frame_moves_the_frame`
- `a_click_on_a_child_of_a_selected_frame_selects_the_child`

### 2.3 布尔运算的几何保真

- `64b0b4510`（#2405）：无描边文本做 boolean union 时应 outline 出字形。本地 `cmd/structure.rs` 缺 `flatten(NodeKind::Text)` 分支（上游在 `outline_stroke` 里）。
- `641cc4159`（#2195）：圆角点要烘焙进路径，上游用 `figcraft_doc::geometry::drawn_network(network, smoothing)`，本地 `collect_flatten_parts` 直接 `network.clone()`。注意本地别处有 2 处 `drawn_network` 引用，但不在 boolean 路径上。

### 2.4 批量命令的健壮性

- `2462a8625`（#2181）：`EngineError::at_step()` 上游新增 `Disabled`/`Internal` 两个分支，使批量中因「未选中图层」失败的步骤也带 `step N:`。本地 `error.rs:38-41` 只处理 `BadParams`/`Other`。
- `3b9bf8c9a`：上游 `automation/batch.rs` 加 `MAX_EXPANDED` / `weight()` / `resolve_in(.., left)` 的字节上限（8MB），本地 `:15` 只有 `MAX_DEPTH=64`。防止 `$step`/`$ref` 大结果被反复复制放大。
- `select.layersAt`：上游返回项带 `locked`（含祖先锁定），本地无。

### 2.5 框选分支 `cb5262141`

上游 `canvas/tools/pointer.rs:52 marquee_selection` 有聚焦画框的 `marquee(.., Some(f))` 分支和 widget 过滤，本地改用 `display_doc`。gap-4 几何本身已在 `crates/doc/src/geometry.rs`（与上游仅差 Icon 分支），所以只需核对分支语义。

---

## 3. GPUI UI 请求接线缺口（类别 C）

引擎通过 `request_ui(kind, data)` 发请求（`engine/src/session/undo.rs:202`）。GPUI 有四层分发：`lib.rs:1873 sync_ui_requests` → `lib.rs:1911 match` / `dialogs.rs:502 ui_action` / `lib.rs:2026 ui.set` / `panels/extra.rs:96 ui_action`。EGUI 基准是 `ui-egui/src/app/commands.rs:120 handle_ui_requests`。

### 3.1 确认零命中（14 项）

| 请求名 | 引擎发出 | EGUI 处理 | 与批量的关系 |
|---|---|---|---|
| `selectArea` | `cmd/edit/select.rs:48` | `commands.rs:241 start_key_box()` | **键盘框选，直接相关** |
| `layers.collapseAll` | `cmd/edit.rs:326` | `commands.rs:239` + `panels/layers.rs:734` | **图层批量折叠，直接相关** |
| `panel.layers` | `cmd/view.rs:379` (Alt+1) | `commands.rs:168` | 图层面板是批量操作的操作面 |
| `panel.leftSidebar` | `cmd/view.rs:385` | `commands.rs:149` | — |
| `contextMenu` | `cmd/view.rs:432` (Shift+F10) | `commands.rs:240` | 键盘打开右键菜单 |
| `annotation.new` | `canvas/tools/pointer.rs:213`、`cmd/view.rs:258` | `commands.rs:269` | — |
| `comment.new` | `canvas/tools/pointer.rs:170` | `commands.rs:281` | — |
| `comment.open` | `cmd/comment.rs:509` | `commands.rs:264` | GPUI 唯一命中是主动发命令，不是消费 |
| `open.url` | `cmd/text.rs:707` | `commands.rs:234` | 文本链接点击无反应 |
| `open.help` | `cmd/view.rs:572` | `commands.rs:231` | — |
| `window.fullscreen` | `cmd/view.rs:376` (F11) | `commands.rs:296` | GPUI 只接了 window.new/focus/merge |
| `dialog.pluginSave` | `extensions/plugins.rs:364` | `commands.rs:294` | 落 `dialog.*` 兜底成通用消息框 |
| `dialog.cursorChat` | `sharing/live/join.rs:174` | `commands.rs:295` | 同上 |
| `present.options` | `cmd/proto.rs:157` | `commands.rs:200-224` | 演示选项失效 |

### 3.2 部分接（4 项）

- `home.templates` / `home.gettingStarted`：`panels/extra.rs:112` 只接 `"home"`，不开到对应分区。
- `panel.libraries`：`extra.rs:105` 并入 `LeftTab(1)`，**丢弃 `publish:true` 参数**，不打开发布库界面。
- `annotation.new` 的落点：EGUI 有「仅选中非页面层才聚焦」的前置条件，GPUI 无。

### 3.3 键盘选择交互的连锁缺口

GPUI `run_binding`（`lib.rs:308-317`）只有 `select.none`、`edit.delete` 两个特判；EGUI `shortcuts.rs:397-422` 更丰富，还有独立的 `key_box` / `key_pick`（`shortcuts.rs:441,531-642`）。GPUI 全局搜 `key_box|key_pick|start_pick_select` 零命中。所以：
- `select.area` 键盘框选（Alt+Space/F8）无 UI 落地。
- Pick select 光标模式无落地。
- `select.children` 的 Enter 特判无落地。

---

## 4. GPUI UI 交互 / 门控缺口（类别 C）

### 4.1 多选主入口

**Component 下拉**（最高优先）：`panels/right/sections.rs:3656-3660` 无条件只放 `component.create`。EGUI 在 `sel.len() > 1` 时把按钮变成 34px 带 chevron，弹 `component.create` / `component.createMultiple` / `component.createSet`（后者按 `can_create_set` 门控）。引擎已实现三项（`cmd/component.rs:93-109`），GPUI 全仓 0 引用。主菜单因从 catalog 生成（`main_menu.rs:301`）仍能点到，但 Design 主入口缺。

**混合选区相对编辑**：EGUI `field_of()`（`ui-egui/panels/right.rs:71-93`）对 Mixed 字段按**每层自己的值**算相对编辑，再放进一次 interaction 提交。GPUI `lib.rs:3101-3113` 把 Pending 里的一个 `(Prop, f64)` 作为**同一个绝对值**发给所有 ids；Mixed 字段 `kept=""`（`panels.rs:1745`），相对值解析后也只得到一个常数。结果：多选且值不一致时输入 `+10` 被吞或把全部层设成同一值。

### 4.2 右栏批量控件

- **Layout Spacing 数字框**：EGUI `selection_spacing.rs:120-137 panel()` 画 H/V spacing，跑 `arrange.spacing {ids}`。GPUI 同名文件只有 `layout / handles / input / draw`（`:38-364`），没有 `panel()`，右栏 Layout 节也没有调用点（`panels.rs:2910` 的 Spacing 是 auto-layout 的 gap，不是多选间距）。
- **Tidy up 门控**：EGUI `right.rs:777-798` 按 `tidy_available()` 灰显并给 hover 说明；GPUI `panels.rs:2140-2142` 三条一律可点，`on_pick` 直接映射 `arrange.tidy`，`can_tidy` 全仓 0 引用（引擎有 `selection/tidy.rs:46`）。点了被引擎拒绝且无 toast。
- **`layout.add` 门控**：EGUI `layout_panel.rs:30-52` 用 `is_frame_like() || NodeKind::Group`；GPUI `panels.rs:598-612 visibility::layout_header()` 用 `sel.all_frame`（`lib.rs:1539` 只取 `is_frame_like()`，`node.rs:401-406` 不含 Group）。测试 `panels.rs:3245 layout_header_button_follows_egui_visibility` 还把错误规则锁住了——补的时候要一起改测试。
- **对齐行显隐**：EGUI 总画 caption + 六按钮，用 `add_enabled_ui(can_align)` 灰显并给不同 hover 文案；GPUI `panels.rs:2561-2565` 在 `!can_align` 时整行不画。能力同源，保真缺失。
- **布尔按钮**：EGUI `right.rs:577-600` 点图标只开菜单；GPUI `sections.rs:3669-3676` 在 `bool_union_enabled(count>=2)` 时 `on_click` 直接推 `bool.union`。`panels.rs:593` 的注释与当前 EGUI 源码不符，属描述滞后。

### 4.3 图层树

- **eye/padlock 扫掠**：EGUI 有 `Sweep`（`panels/layers.rs:472-482`）、`finish_sweep()`（`:895-910`，松手时 `object.toggleVisible/Lock {"ids": sw.ids}`）、`eye_lock()`（`:979-1047` 两图标都 `click_and_drag()`）。GPUI `layer_tree.rs:788-824` 两图标只挂 `on_mouse_down`，`:1310-1317` 固定 `{"ids":[id]}`。拖动经过多行不能批量显隐/锁定。
- **Shift 范围跨 frame**：上游 `b5d43a7de` 的 EGUI 逻辑在 `panels/layers.rs`，gpuI `selection_after_click()`（`layer_tree.rs:1347-1381`）按可见行取全范围，无 frame / 被覆盖子层的区分，也无对应测试。
- **行内改名 Tab**：EGUI `panels/layers.rs:1143-1160 take_tab`；GPUI 无。上游 `2408d0aa8`（#2311）就是这个。

### 4.4 画了不能点

`edit.copyAsCss` / `copyAsSvg` / `copyAsPng` / `copyLink` 只在 `catalog.rs:63-66`、`keymap.rs:316`、测试里出现，**没有 `spec!` 注册**（验证：grep `spec!("edit.copyAs` 零命中）。GPUI `context_menu.rs:82-86 can_run()` 用 `find_command(id).is_some()` 判定 → 四项永久灰显；主菜单从 catalog 生成，同样是点了无声失败。EGUI 在 `menus.rs:33-48,114-160` 自己实现剪贴板写入。

### 4.5 其他

- 批量重命名对话框无 Enter 提交（`dialogs.rs:502-527` 只有按钮 derived action）；`docs/gpui-egui-parity-plan.md:1046-1048` 对此标注准确。
- `text.onPath` / `text.pathOptions` / `text.removeFromPath`：GPUI 全仓 0 引用（EGUI `panels/typography.rs:126-157`）。
- 多选导出：主体已补（`dialogs.rs:630-657` 的 Export spec 带 `file.exportLayers` + 8 个预设，`sections.rs:2277-2293` 多选 `export.update` 带 ids，`sections.rs:2441-2445` 显示 `Export {n} layers`，`lib.rs:3988-4015` 原生保存/文件夹选择）。仍缺 EGUI `dialogs/export.rs:113-130` 的最多 6 个 job 文件名列表和动态 `{n} layers as one file` 文案，以及 PDF print options（GPUI 只有 Preview 位）。

---

## 4.5 带圆角形状转矢量：圆角被烘焙进路径（类别 D）

> 用户 2026-11-25 提出。这条不是「上游有我们没带」——**上游和本地代码完全一样**，是产品行为本身的问题，要自己改，故单列类别 D。

**现象**：矩形拉了圆角后双击进入矢量编辑（转矢量路径），圆角被写死成路径上的一段段曲线/描边，而不是保留成矢量节点的圆角数据。结果是圆角不再可调——你看不到「这个节点 radius = 20」，只看到一串锚点和曲线。

**根因**：`crates/engine/src/canvas/vectoredit.rs:324-346 ensure_vector()`

```rust
let (path, rule) = figcraft_render::node_outline(doc, &n).ok_or_else(...)?;
let mut net = VectorNetwork::from_path(&path);   // ← 圆角在这一步已经变成贝塞尔曲线
...
node.kind = NodeKind::Vector { network: net };
node.corners = Corners::default();               // ← 层的圆角数据清空
double-click → enter_vector_edit → ensure_vector
```

`node_outline()` 返回的是**已按圆角绘制好的路径**，再 `VectorNetwork::from_path()` 从这个路径反推顶点，圆角自然成了曲线段，每个顶点的 `corner_radius` 保持 0。

**正确做法**：矢量顶点本身就支持圆角数据——`crates/doc/src/vector.rs:41` 的 `Vertex.corner_radius`（0 = sharp）、`:53` 的 `corner_smoothing`。`crates/doc/src/tests.rs:768 closed_vectors_smooth_each_rounded_point_with_its_own_radius` 已证明：一个四顶点、每点 radius=20 的网络，绘制结果精确等于圆角矩形（`area` 误差 <0.1）。所以矩形/椭圆等规则形状转矢量时，应直接构造「保留语义顶点 + 每点 radius」的网络，而不是从烘焙路径反推。命令 `vector.edit.cornerRadius`（`vectoredit.rs:1143`）已有，改完后这些点就能继续用该命令调圆角。

**注意区分两类圆角**：这条讲的是「转矢量时保留为**顶点**圆角数据」，与第 20 项 `641cc4159`（#2195，Flatten 时把顶点圆角**烘焙进路径**）方向相反、互不矛盾——Flatten 是终态导出，烘焙是对的；进入矢量编辑是可逆编辑，必须保留数据。别改混了。

**上游状态**：已核对，上游 `vectoredit.rs` 的 `ensure_vector` 与本地逐字一致，同样是烘焙行为；相关 issue #2519「Corner radius values keeps vanishing」是 open，但描述的是「值随机归零 + 颜色闪烁」，不是这个转换问题，**不能指望上游修这条**。这是自研项。

**实施规格（用户 2026-11-25 确认的方案）**：**路径按直角烘焙，圆角写进点的数据里，确保圆角不丢。**

- 具体说：转矢量时不要拿 `node_outline()` 那条「已画圆角」的路径，而是拿形状的**直角骨架**（`figcraft_geom::rounded_rect(w, h, [0.0;4], 0.0)` 在半径全 0 时正好返回直角矩形，`crates/geom/src/lib.rs:120-128`），用 `VectorNetwork::from_points()`（`crates/doc/src/vector.rs:221`）构造语义顶点；然后把层圆角分别写进每个顶点的 `Vertex.radius`（四角顺序见 `crates/doc/src/node.rs` / `Radii` 定义，测试里 `[top-left, top-right, bottom-right, bottom-left]`，`crates/doc/src/tests.rs:749-756 rounded_square` 就是现成范例），`smoothing` 写进 `Vertex.smoothing`。这样几何是直角、圆角是数据，绘制时由 `drawn_network()` 还原出同样的视觉，且 `vector.edit.cornerRadius` 能继续调。
- 这个机制本项目**双向都在用**：`.fig` 导出 `crates/format/src/fig/export/shape.rs:211` 写 `("cornerRadius", fl(v.radius))`，导入侧 #2114/#2115 读点 radius（测试在 `export_tests.rs:216,923`）。所以不是新设计，是把转矢量这条路径接回既有机制。
- `Vector`/`BooleanOp` 本身没有「层圆角」语义，维持现有 from_path 逻辑即可。

**同类点排查结果（2026-11-25）**：把 `node_outline → VectorNetwork::from_path` 的全部调用点过了一遍（render/pdf 是绘制、svg/fig/pathops 是导入/导出，不算）：

| 路径 | 文件:行 | 是否同类 bug | 判定 |
|---|---|---|---|
| 进入矢量编辑（双击/Enter/`vector.edit`） | `canvas/vectoredit.rs:334 ensure_vector` | **是，就是 #33** | 唯一入口，`enter_vector_edit` 只在 `:537` 调它，修一处全覆盖 |
| 矢量编辑态粘贴形状 | `cmd/edit/vector_clipboard.rs:186 outline_network` | **是，同类（次优先）** | 把带圆角矩形粘进矢量时同样烘焙；且上游 `placed()` 只搬点 radius，粘进来的 shape 点 radius 已是 0，**圆角永久丢**。建议与 #33 共用同一个"形状→语义网络"构造函数 |
| Flatten（打平） | `cmd/structure.rs:531 outline_part` | 否 | 终态导出，烘焙正确（且 #2195 就是要烘焙得更准） |
| Outline stroke | `cmd/structure.rs:598,960` | 否 | 描边转填充，几何即结果 |
| Boolean 结果 / 合并 | `cmd/structure.rs:830-912` | 否 | 布尔是几何运算；圆角由 group 的 `corners` 单独处理（`boolean_radius_tests.rs`） |
| Frame from selection | `cmd/structure.rs:485` | 否 | 取框轮廓 |
| 画笔/形状构造器 | `canvas/tools/pen.rs:407`、`canvas/shapebuilder.rs:271` | 否 | 输入本来就是路径，无层圆角语义 |

**结论**：真正同类只有两处，且共用同一个根因（shape → 网络时用了烘焙路径）。建议抽一个公共函数 `network_from_shape_semantic(doc, node)`（直角骨架 + 顶点 radius），`ensure_vector` 和 `outline_network` 都调它。

**建议优先级**：P1。补测试：① 转矢量后顶点 `radius` 非空、`vector.edit.cornerRadius` 能继续改、绘制与转换前一致；② 矢量编辑态复制圆角矩形再粘贴，圆角数据仍在。

---

## 5. 上游 open PR（类别 B，待观察，不要当漏带处理）

这些连上游主线都还没进，本地没有是正常的。等合并后再评估。

| PR | 内容 | 状态 |
|---|---|---|
| #2608 | `pasteOver` 多选时插在最后一个可见选中层之后，新增 `paste_over_tests`×5 | open，依赖先合 #2471 |
| #2620 | Paste properties 保留样式链接、变量绑定、layout grids | open |
| #2615 | Selection colors 行的 Styles 按钮打开统一样式选择器 | open |
| #2329 | 多个同旋转角图层应显示单一旋转框（Figma 是页面对齐、一个旋转中心） | open |
| #2483 | Shift+双击深入且保留其余选择 | open |
| #1761 | Ctrl 连续选择不工作（bug） | open |

---

## 6. 已带入，勿重复劳动

判缺口时的血泪教训：**git 提交不在本地不等于功能缺失**。本地大量上游功能是手工移植的，提交号不在但功能+测试都在。以下都已核实带入：

- `adad8a08a`（#2293/#2313）重复旋转步进 + 取消选择清步骤：`cmd/edit.rs:614 Duplicated`、`:630 step()`、`:717 Step {dx,dy,angle}`，`duplicate_spacing_tests.rs` 全套。
- `e94ecfca7`（#2372）旋转父层内方向键沿画布轴移动：`cmd/edit/nudge_rotated_parent_tests.rs` 存在且测试齐全。
- `d6bfe6893`（#2355）frame 内选中层粘贴到旁边：`paste_sibling_tests.rs` 在本地。
- `4c71211e2`（#2109）矢量编辑模式点对齐/分布：`canvas/vectoredit/transform.rs` 有 `align_points_in_edit_mode` / `distribute_points_in_edit_mode`。
- `cfd6bf527` Tidy up 保序保尺寸：`selection/tidy.rs` 与上游逐字节相同。
- `d6bfe6893` / `87454911f` 相关框架行为：`frameSelection` 在右键菜单可达。
- 自动布局前缘+方向重排、Align 对 auto-layout 子层置灰（`cmd/arrange.rs:171 align_blocked_by_auto_layout`）、`absoluteRotation`、annotation / tidy / per-page views / vector clipboard / Escape、批量重命名主体、多选导出主体、原生路径选择——均已落地。
- 本地 `cmd/arrange.rs:228 distribute()` 比上游多一个 `points` 参数，让 `packH/packV` 走图层路径，属**本地更强**，不要"对齐上游"时改回去。

文档时效：`docs/monet-upstream-diff.md` 是 11-20 审计（基线 `78c356b3`），`docs/gpui-egui-parity-plan.md` 部分条目也滞后于代码。判现状一律以源码为准。

---

## 7. 建议执行顺序

1. **先把测试灯打开**：注册 `mod lock_behaviour_tests;`，跑出真实失败面。
2. **P0 行为错误**：锁定层批量规则（#1/#2/#3）→ 混合选区相对编辑（#4）→ component 下拉（#5）。
3. **P1 通道断裂**：`selectArea`（键盘框选）→ `layers.collapseAll` → Copy as 接线（或明确决定不接，把菜单项移除而不是留死条目）→ eye/lock 扫掠。
4. **P1 引擎**：拖动 frame 子层（#7）→ 圆角保真（#33/#34，类别 D，自研项，别等上游）：抽公共 `network_from_shape_semantic()`（直角骨架 + 顶点 radius），`ensure_vector` 和矢量粘贴 `outline_network` 共用。
5. **P0 画布键盘**（#35，2026-11-26 用户实测）：方向键 nudge / nudgeSize / rotate / pan / crosshair 一次补全（同一分派块），再到 opacity 数字键、图层导航、键盘框选/拾取、Enter 放置、crop 退出、原型删交互等 10 条。**必须在根节点 on_key_down 也兜底**（GPUI 按键沿焦点链派发，焦点在右栏时画布分支收不到）。详见 `full-parity-audit-2026-11-25.md` §6。
6. **P2/P3** 按上表顺序批量带入；每带一项同步核对 GPUI 对应入口。
7. **待观察 PR** 等上游合并后再统一评估，不要提前做。

带入方式提醒：本仓库历史垃圾多（`871` 误存 PNG、`.tmp_shots/` 104MB 未 ignore），提交时用路径白名单，别 `git add -A`。
