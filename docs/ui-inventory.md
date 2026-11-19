# UI 界面清单（EGUI ↔ GPUI 对照）

一张可以长期查的界面元素总表：**每个界面元素叫什么、绑的是哪个引擎命令、EGUI 有没有、GPUI 有没有、在什么情况下出现/隐藏、实现在哪个文件**。

- 表格：`ui-inventory.csv`（UTF-8 with BOM，Excel / Numbers 直接打开不乱码）
- 数据源：`ui-inventory-src/*.tsv`（每个模块一份，`|` 不用，用 Tab 分隔）
- 生成：`python docs/ui-inventory-src/merge.py` → 重新写出 `docs/ui-inventory.csv`

## 列的含义

| 列 | 含义 |
|---|---|
| 分类 | 模块 / 区域，如 `菜单·File`、`工具栏·主工具栏`、`右面板·Design 描边`、`画布覆盖·overlays`、`对话框` |
| 界面元素 | 界面上的真实名称（英文原文，取自代码里的 label / 常量） |
| 绑定命令 | 该元素点击 / 修改时走的引擎命令 id（`catalog.rs` 的 id，或 `spec!` 注册的命令）。纯展示元素写 `—` |
| EGUI | `✓` = EGUI 后端有这个界面；空 = 没有（目前只有 GPUI 独有的 Motion 动画系统） |
| GPUI | `✓` = GPUI 后端有对应界面；空 = GPUI 尚未实现 |
| 行为 / 显示规则备注 | 什么时候出现、什么时候隐藏 / 置灰、特殊规则、快捷键。备注里的「置灰」= 命令未注册或 `!available` 时不可点；「Dev Mode 拒」= Developer Mode 只读时拒绝执行 |
| 源文件 | 该元素在 EGUI 里的实现文件（GPUI 特有的见备注 / 分类里的 `GPUI 特有`） |

## 覆盖与核对

- **主菜单 223 / 223**：与 `crates/engine/src/catalog.rs` 的 `MENU`（非分隔符项）逐条比对，差集为 0。
- 另有 1 条本仓库新增、上游 catalog 没有的项：`tool.brush`（画笔工具，见 `菜单·Tools`）。
- 面板 / 对话框 / 工具栏按 `crates/ui-egui/src/panels/*`、`dialogs/*`、`toolbar.rs` 逐文件核对，除 `panels/list_drag.rs`（图层面板拖拽的内部辅助，不是独立界面）外都有对应行。
- 总计 **1016 行**（含表头 1017 行）。


## 上游能力：已带入 / 未带入（2026-11-19 快照）

### 已带入（分支 `feat/ui-parity-annotations-tidy`，commit `ab105f5c`，尚未推送）

| 能力 | 界面/行为落点 |
|---|---|
| Annotation 工具 | Shift+T、`tool.annotation`、Inspect 面板的标注分类（`Right panel·Inspect` 各行） |
| Tidy up | `Arrange > Tidy up` 与 `Design > Position` 的 Distribute and tidy 菜单，走 `selection/tidy.rs` 完整实现（行/列/网格） |
| 切页记忆视图 | `View > Previous/Next page`、页列表点击：回到该页上次的缩放与位置 |
| 矢量编辑剪贴板 | 矢量编辑模式下的 `edit.copy/cut/paste`（点/段级） |
| Zoom to selection 留白 | 缩放选区避开底部浮动工具栏（`view.bottomInset`） |
| absoluteRotation | Design 面板 Rotation 字段按**页面角**写入（父级旋转一并抵消） |
| Polygon/Star 点数手柄 | 画布上 Count 手柄（3~60 点） |
| Escape 直接清空 | 原「先选父级」改为一次清空（Figma 实测行为），测试同步更新 |
| 描边高级设置 | 枚举行改下拉、端帽两行各 8 种、宽度剖面 6 个 preset（PR #823 形态） |

### 未带入 · GPUI 层（CSV 里「GPUI」列为空、本地状态 = `GPUI 未实现`）

- 跨端协作 / 在线文件：Join live session、Publish to server、File published、Remove from server、Rename on the server、Pending open card
- 插件：Plugin install/update confirm、Plugin parameters form、Plugin panels、Save file、Create from color
- Assets 面板的 Libraries 子视图（4 行）
- 版本历史的 Branch menu（3 行）
- Inspect 面板的 Plugins 分区

共 **19 行**，都在 CSV 里能筛出来（「本地状态」列）。

### 未带入 · engine 层（行为与上游不同）

- **auto-layout normalize**：本地仍是 `7f5745ed`（CodeRabbit PR#32）的版本——`run` / `box_changed` / frontier 逐级传播，并带 `board::settle_mind_maps` 钩子；上游是精简的一次遍历。差异表面化在「auto layout 把 item 放回」这类断言（`components_behaviour_tests::what_instances_cannot_override_is_refused`，60 vs 4）。**未带入**，因为它与并行进行的 mind map 工作流在同一段代码上。
- **grid 线型**：上游删掉了 `Slash` / `Ring` 两种布局网格线，本地保留——这是**本地多**于上游的地方，不是缺失。

### GPUI 独有（EGUI 没有，CSV 里「EGUI」列为空）

Motion 动画编辑系统 13 行：MotionInspector、AnimationSection、KeyframeFields、MotionTimelineBar、TimelineRuler+Playhead、TimelineLayerRows+Tracks、KeyframeDiamonds、ClipBars、DurationHandle、EditableTimeBoxes、ClipSettingsPopover、EasingEditor、MotionOverlay。

## 已知边界（别把这张表当唯一真相）

1. **GPUI 列的 19 处空白**是核实过「GPUI 确实没有」的，不是漏填：
   - 左面板 Assets 的 Libraries 子视图（4 项）
   - 版本历史的分支菜单（3 项）
   - Inspect 的 Plugins 分区（1 项）
   - 跨端协作 / 在线文件 / 插件运行类对话框（11 项：Join live session、Publish to server、File published、Pending open card、Plugin install confirm、Plugin parameters、Plugin panels、Save file、Remove from server、Rename on the server、Create from color）
2. **EGUI 列的 13 处空白**是 GPUI 独有的 Motion 动画编辑系统（时间线 / 关键帧 / 片段 / 缓动 / 运动路径叠加）。EGUI 只有原型转场动画，没有独立时间线。
3. 命令 id 来自 `catalog.rs` 与引擎 `spec!` 注册表；UI 命令（如 `view.toggleUi`）在引擎里是 UI 请求，不一定有对应的文档修改。
4. 这张表是 **2026-11-19 的源码快照**。代码大改后重跑 `merge.py` 前，需要先同步更新对应 `.tsv`。

## 怎么维护

```bash
# 改了某个模块的界面后：编辑 docs/ui-inventory-src/<模块>.tsv，再重新生成
python docs/ui-inventory-src/merge.py
```

- 新增一行：在对应 `.tsv` 里加一行，4 列用 Tab 分隔：`分类<TAB>界面元素<TAB>命令<TAB>备注`
- 源文件列由 `merge.py` 的 `SRC` 表按分类前缀映射，新增分类时在那里加一条
- 对话框的源文件映射在 `DIALOG_SRC`（按元素名精确到文件）
- GPUI 缺口的判定在 `merge.py` 的 `GPUI_MISSING_DIALOGS` / `GPUI_MISSING_ASSETS`
