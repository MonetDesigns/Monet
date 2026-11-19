# Monet ↔ 上游 figcraft 差异表

- **本方**：`MonetDesigns/Monet` main = `ba572a2d`（2026-11-19）
- **上游**：`storytold/figcraft` main = `b45255bb`（2026-11-19，标尺 `yelintong/FigCraft-Monet` 与此一致）
- **分叉点**：`a2e87433`（2026-11-09）
- **commit 数**：本方领先 183，上游领先 985

---

## 一、命令层差异（精确对照 696 vs 657）

### 1.1 上游有、本方没有（3 个，未带入）

| 命令 | 功能 | 所在模块 | 带入价值 |
|---|---|---|---|
| `board.codeBlock` | 看板上插入代码块（随行数撑高） | `engine/boards/board` + `doc` | 高：新看板对象类型 |
| `board.setLanguage` | 设置代码块语言（Python/C++ 等，`CodeLanguage` 表） | 同上 | 高：与上者配套 |
| `inspect.designContext` | 导出图层设计上下文给 AI / Dev Mode（含变量合并、资源归属） | `engine/dev` | 高：独立新能力 |

### 1.2 本方有、上游没有（42 个，本地独有，保留）

| 类别 | 命令 | 说明 |
|---|---|---|
| Motion（18） | `motion.timeline`、`motion.clip*`、`motion.keyframe*`、`motion.track`、`motion.autoKey`、`motion.export`、`motion.preset`、`motion.replace`、`animation.transitionPoses`、`inspect.motionCode` | 动画编辑器全套，上游无对应 |
| Brush（5） | `brush.list/import/reload`、`tool.brush`、`tool.brushPreset` | 外置 `.abr` 笔刷体系 |
| Icon fonts（13） | `icon.create/delete/detach/exportFont/font.*/generateFont/insertText/list/move/rename/setCodepoint/update` | 图标字体生成与管理 |
| 其它（6） | `tool.drawStyle`、`tool.penKind`、`pref.accent`、`variable.move`、`variable.moveCollection`、`window.merge` | 本地行为/偏好扩展 |

---

## 二、实现层差异（按 crate / 文件数）

上游改过而本方仍有差异的文件（三方合并判定"上游改动未完全包含"）：

| Crate | 差异文件数 | 主要内容 |
|---|---|---|
| `ui-gpui` | 206 | 上游也有 GPUI 实现但路线不同；本方为 1:1 照抄 ui-egui 的自有实现 |
| `engine` | 180 | 见下方"重点未带入项" |
| `format` | 69 | REST import 变量/描边边绑定、`.fig` ruler guides、共享 Sketch/Penpot 依赖 |
| `ui-egui` | 39 | 参照端细节差异 |
| `i18n` | 30 | 文案差异 |
| `doc` | 29 | CodeBlock 数据模型、库统计、圆角点半径规则 |
| `render` | 26 | 平移条带并行渲染、HiDPI 细节、椭圆四锚点 |
| `svg` | 15 | SVG 导入细节 |
| `text` | 14 | 文本布局修复 |
| `collab-net` | 13 | 协作网络 |
| 其它 | 各 1–10 | `pathops` / `abr` / `mcp` / `pdf` / `layout` / `geom` 等 |

> 其中多数差异是**本方独有功能造成的正常分叉**（brush/motion/icon、board 派生、本地 layout hug 行为），不是遗漏。

---

## 三、重点未带入项（实现级，非命令）

| 模块 | 缺失函数 / 能力 | 性质 | 建议 |
|---|---|---|---|
| `engine/automation/file_access.rs` | `link_refused`、`escape_device_names`、`is_native_name`、`is_rooted_without_drive`、`lexical`、`outside`、`on_disk`、`device_name`、`has_trimmed_name`、`is_volume`、`nothing_there`、`refused_spelling`、`refused_unresolved`（13 个） | 符号链接穿越 / 设备名 / 路径规范化**安全加固** | **优先带入** |
| `engine/canvas/vectoredit/ops.rs` | `close_bridged_loops`、`smooth_sharp_point` | 闭合桥接环、平滑尖点 | 值得带入 |
| `engine/boards` + `doc` | `BoardItem::CodeBlock`、`CodeLanguage` 表 | 配套三个新命令的数据模型与渲染 | 与命令一起带 |
| `engine/dev` | design context 的变量合并与资源归属逻辑 | 配套 `inspect.designContext` | 与命令一起带 |
| `format` | REST import 的描边边变量绑定、`node.get` 的 dashCap 字段、`.fig` ruler guides 写出 | 格式保真改进 | 逐项评估 |

---

## 四、已带入的上游特性（本方已覆盖，供对照）

| 特性 | 来源问题 | 状态 |
|---|---|---|
| `.fig` schema 5 字段 + LeadingTrim + TextPathStart | 导出报 `no field` | 已带入 |
| `render_area_over` / `render_part` / `render_columns` | 背景模糊导出、局部重绘并行 | 已带入 |
| `LibraryLink.relative` | 库链接跨文件夹存活 | 已带入 |
| `page.setOpening` | 指定打开页 | 已带入 |
| mind map（`board.mindMap*`、`MIND_*`） | #2226 | 已带入（3 个几何测试受本地 layout hug 影响未平） |
| 自动布局按前缘+方向重排 | 上游实测规则 | 已带入（10/10 测试通过） |
| Align 对 auto-layout 子层置灰 | Figma 实测 | 已带入 |
| codegen 渐变辅助（Compose/Flutter/Swift） | 只写实心填充 | 已带入 |
| `pref.invertZoom`、`component.createSet` | 最后两个缺的命令 | 已带入 |
| Annotation / tidy / per-page views / vector clipboard / Escape 清选择 | UI parity | 已带入 |

---

## 五、结论

- **命令覆盖**：657 个上游命令中 654 个已带入（99.5%）；缺的 3 个都是最近新增（code block 2 个、design context 1 个）。
- **实现覆盖**：核心功能已对齐；剩余差异以本地独有功能和刻意保留的本地行为为主。
- **真正值得继续带**：路径安全加固（13 函数）+ 代码块看板对象 + design context + 2 个矢量编辑函数。
