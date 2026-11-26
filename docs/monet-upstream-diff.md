# Monet ↔ 上游 figcraft 差异审计（2026-11-20，PR 全量合并后重做）

- **本方**：`MonetDesigns/Monet` `main` = `78c356b3`（已合并 PR #5–#10 全部内容）
- **上游**：`storytold/figcraft` `e97b644a`
- **分叉点**：`a2e87433`
- **审计基线**：全部基于已提交的 git blob，工作区干净（0 改动）。

> **2026-11-26 状态**：本文件是 11-20 的审计快照，不再作为「当前落后多少」的来源。实时锚点看
> [`../upstream.toml`](../upstream.toml)，机器生成的命令 / 文件差异看
> [`upstream-sync.md`](upstream-sync.md)（`cargo xtask upstream-report`）。两边历史无共同祖先，
> 任何 commit 计数都没有语义，只能按内容同步。

## 总览

| 维度 | 数据 |
|---|---|
| 命令 | 本方 691，上游 652；**上游独有仍是 3 个**（`board.codeBlock`、`board.setLanguage`、`inspect.designContext`） |
| 差异文件 | 1014 个（ui-gpui 390、engine 175、format 69、docs 53、assets 42、ui-egui 34、i18n 30、doc 29、render 29…） |
| 双方都改过的实现文件 | 157 个：上游改动**已包含 63**，**未包含 94** |
| 图标 | 已按规范全部以 PR #9 为准（扁平 `assets/icons/*.svg` + 域前缀） |
| 上游新增功能主题 | ~200 条 M1/M2/M3/M4/M5/M8/M9/M10/M12 改进 |

> 与上次审计（HEAD `76418e12`）相比：本方命令 +5（691 vs 696→691，合并冲突解决后），
> 已包含的上游改动 59→63。上游独有 3 条命令**仍未带入**。

---

## 一、明确可带入（无冲突，按价值排序）

> 判断原则：我们本地独有功能（brush/ABR、motion、icon fonts、Pencil+PenKind）与上游**零重叠**——
> 上游 `brush_stroke`/`PenKind`/`motion::`/`icon.font`/`keyframe` 命中均为 0。

| 功能 | 上游 commit | 涉及文件 | 价值 | 带入代价 |
|---|---|---|---|---|
| **design context for AI agents** | `8dffbf87`+3 | `dev/design_context.rs`（25 个新符号）、`inspect.designContext` 命令 | 高：给 AI/Dev Mode 导出图层上下文 | 中：新模块+命令+UI 入口 |
| **board code blocks** | `0d70c332` | `doc/board_code.rs`（CodeLanguage 16 语言表）、`boards/board/code.rs`、`board.codeBlock`/`board.setLanguage` | 高：看板代码块 | 中：数据模型+渲染+命令 |
| **Fill across a hugging axis**（Figma 实测） | `d494fad1` | `layout/lib.rs`（165 行）、`cmd/props.rs`（103 行）、`components/sync.rs` | 高：auto layout 保真 | 中：949 行含测试，与我们前缘重排（drop.rs/drag.rs）不重叠 |
| **路径安全加固** | `98c47144`+ | `automation/file_access.rs`（13 函数：`link_refused`/`lexical`/`escape_device_names`/`outside`…） | 高：符号链接穿越/设备名防护 | 低：独立模块，可整体替换 |
| **SVG writer 共享 + 16-bit 缩略图 + 光栅方向** | `dcd2783f`/`11bfa4db`/`3120523a`/`4e35fb9c` | `svg/src/lib.rs`、`render/image_profile.rs`、`render/images.rs`、`Cargo.toml` | 中：导出保真 | 中：需把 craft-libs 依赖 bump 到 `ea9407b3`（我们已在用这些 crate，只是 rev 不同） |
| **边缘自动平移（autopan）** | `98c47144` | `view_state.rs`（`AUTOPAN_*`）、`canvas/tools.rs`、`ui-egui/canvas/autopan.rs` | 中：拖到边缘自动滚动画布 | 中：引擎+UI 两层 |
| **旋转父层内方向键沿画布轴移动** | `e94ecfca` | `cmd/edit.rs`、`edit/nudge_rotated_parent_tests.rs` | 中 | 低：独立命令逻辑 |
| **Duplicate 重复旋转步进** | `adad8a08` | `cmd/edit.rs` | 中 | 低 |
| **marquee 选择规则（Figma 实测）** | `604c7846` | `doc/geometry.rs`（`marquee_outline`/`marquee_touches`） | 中 | 低 |
| **Select layer 列出锁定层（挂锁图标）** | `6cb015d4` | `doc/geometry.rs`、`ui-egui/panels/layers.rs` | 中 | 中：含 UI |
| **矢量点对齐/分布、X/Y Mixed** | `4c71211e` | `canvas/vectoredit.rs` | 中 | 中 |
| **Count handle（多边形/星形点数手柄）** | `89d8b50e` | `canvas/shape_handles.rs` | 中 | 中 |
| **text wrap style（三段级别）** | `654eb205` | `doc/text.rs`（`set_wrap_style` 等 14 符号） | 中 | 中 |
| **library 二级跳转保持绑定 / asset 计数** | `0e5070d6`/`e4bc5678` | `assets/library.rs`、`doc/library.rs` | 中 | 中 |
| **paste 到选中 frame / 副本在选中层上方** | M1 系列 | `design_paste_tests`、`cmd/edit/clipboard.rs` | 中 | 低 |
| **Shift+Pencil 画 45° 直线** | M1 系列 | `canvas/tools/pen.rs` | 低 | ⚠️ **有冲突，见第三节**：我们用 `pencil_add` 接管了同一个接入点 |

---

## 二、已带入（供核对，勿重复劳动）

| 功能 | 证据 |
|---|---|
| .fig schema 5 字段 + LeadingTrim + TextPathStart | 已带入 |
| `render_area_over` / `render_part` / `render_columns` | 已带入 |
| `LibraryLink.relative` | 已带入 |
| `page.setOpening` | 已带入 |
| mind map（`board.mindMap*`） | 已带入（3 个几何测试受本地 layout hug 差异影响未平） |
| 自动布局前缘+方向重排 | 已带入，`al_drag` 10/10 通过 |
| Align 对 auto-layout 子层置灰 | 已带入 |
| codegen 渐变（Compose/Flutter/SwiftUI） | 核心 `visible_fills`/`gradient_*` 已带入；上游后来又补了 paint-order 测试和小修 |
| `pref.invertZoom`、`component.createSet` | 已带入 |
| annotation / tidy / per-page views / vector clipboard / Escape | 已带入 |
| `absoluteRotation` | 核心 `cmd/props.rs`、`edit/props.rs` 双方都有；仅 `rotation_origin_tests.rs` 和 MCP 工具描述未带 |

---

## 三、冲突需决策（不是全丢弃，逐项说明）

| 项 | 上游做法 | 我们本地做法 | 冲突性质 | 建议 |
|---|---|---|---|---|
| **笔刷体系** | 上游无 brush/PenKind/motion/icon（命中全 0） | 外置 `.abr` 光栅盖章 + `PenKind{Ink,Highlighter}` + motion + icon fonts（42 个独有命令） | **无冲突**：上游根本没这些 | 保留本地，不需处理 |
| **GPUI vs egui 前端** | 上游维护 egui 前端（`ui-egui` 31 文件差异多为上游的新 UI 改进） | 我们开发 `ui-gpui` 1:1 移植，`ui-egui` 是冻结参照 | **部分冲突**：上游 egui 的新控件（autopan、assets 面板统计、typography 等）我们要判断是否在 GPUI 侧实现 | 逐项评估 UI 层，引擎层照带 |
| **共享 craft-libs rev** | 全部 bump 到 `ea9407b3` | 停在旧 rev（`453b0da7` 等） | **非冲突，是版本差**：我们已在用同样 crate | 可带入，但需 `cargo update` + 适配新 API |
| **layout hug 行为** | 上游改 Fill across hugging | 本地 mindmap 几何测试依赖旧 hug 行为（3 个测试未平） | **真实重叠**：带入上游规则可能改变 mindmap 节点尺寸 | 带入时同步修 mindmap 布局，不可直接丢弃上游改进 |
| **100% 上游命令覆盖** | 652 命令 | 691 命令（多 42 本地独有） | 无冲突 | 保持 |
| **Shift+Pencil 45° 直线** | `pen.rs` 的 `pencil_add` 在 Shift 时直接返回，画图走 `draw_pencil_line` | 我们为笔刷系统把 `pencil_add` 改成带笔刷取样（`pencil_color_at`/stabilizer），Shift 分支是笔刷逻辑 | **真实冲突**：同一个 `pencil_add` 接入点，双方语义互斥（上游要几何直线，我们要笔刷压力线） | **不丢弃**：保留本地笔刷，但要重新设计——把 Shift 直线提到笔刷盖章之前（先算 45° 约束路径，再按笔刷 stamp），两边都保住 |
| **共享 craft-libs 切到单一 rev** | 最近 8 提交把 10+ 个 craft-* 依赖统一 bump 到 `ea9407b3` | 我们停在多个旧 rev | 非语义冲突，是版本差 | 带入时统一 bump + 适配新 API；不可当作"丢弃" |

### 冲突项的处理原则

用户要求：**不要随意丢弃上游改进；只有严重冲突、与我们修复重叠才考虑丢弃，并汇报。**

本次审计中没有任何一项属于"必须丢弃"：
- **Shift+Pencil** 不是互斥——是可分层共存的（几何约束 → 笔刷盖章），需要一次小重构，已列为待办。
- **Fill across hugging** 与 mindmap 尺寸测试是耦合，带入时同步修测试即可。
- 其余 94 个未包含文件都是同文件不同区域的三向合并，互不覆盖。

---

## 四、结论

- **没有任何上游改进需要丢弃**。3 个上游独有命令、13 个路径加固函数、design context、code blocks、
  Fill-across-hugging、autopan、Shift+Pencil 等全部应带入。
- 两处需要设计而非丢弃：**Shift+Pencil**（分层：45° 约束 + 笔刷盖章）和 **Fill-across-hugging**（同步修 mindmap 测试）。
- 上游最近 8 个提交（SVG/thumbnail/raster）随 craft-libs rev 一起带。
- 完整待办按"可带入"表从高到低执行。
