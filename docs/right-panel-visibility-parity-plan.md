# 右栏控件显隐与功能差异补全计划

> 审计日期：2026-11-26
> 上游基线：
>
> `storytold/figcraft`
>
>  的 
>
> `crates/ui-egui`
>
> （本地只读参照副本，2026-11-25 22:38；关键段落已与 GitHub 
>
> `main`
>
>  核对）
> Monet 基线：
>
> `feat/crash-report@5cfe98448`
>
>  \+ 
>
> `crates/ui-gpui/src/panels/right/sections.rs`
>
>  未提交 WIP（多选 Create component 菜单）
> 范围：右栏 Design 标签，包括顶栏 / View Bar / 空选 Page / 选中后的全部属性 section。Prototype、Inspect 只核对到 section 级；Motion 右栏、左栏、画布不在本轮。

## 0. 结论

右栏的**骨架**已经对齐：section 顺序、主 section 的粗粒度出现条件、View Bar 的 Zoom / 显示隐藏开关都与上游一致。剩余差异不是 “整块重做”，而是四类口子：



1. **该出现的被折叠或漏画**：Constraints 行、Frame 头部的 Fit / 状态图标、Frame 的开发状态面板、页面背景不透明度。

2. **该隐藏 / 该禁用的没有处理**：矢量编辑无选中点时显示 Position / Arc / Star、实例内的 Position / Auto layout 方向仍可编辑、PassThrough 多出 Blend 行、无效的 Create component set 菜单项。

3. **整块控件缺失**：Page 变量模式与 Local variables、Mask 类型、Layout grid 设置、多选间距、Grid cell、文本 range fill、Text on path、组件描述。

4. **内容或目标算错**：混合选区（文本 + 非文本）的 Typography 用了全文本假设，显示默认值且命令打到整个选区。

共 **33 项**：P0 4 项、P1 15 项、P2 14 项。建议先做 P0 + 一批 “只改条件” 的 P1（约 1–2 天），再补缺失控件，最后做三个功能块（Layout grid 设置、多选间距 / Grid cell、文本 range fill）。

## 1. 判定口径

本轮把 “显示 / 隐藏” 拆成 5 种，避免把 “禁用” 误判成 “隐藏”：



1. **Section 级显隐**：整个区块出不出现（如 Typography 只在选区含文本时出现）。

2. **控件级显隐**：section 内的行、字段、header 图标出不出现（如 Fill 的 `+`、Stroke 的 weight 行、Frame 的 fit 图标）。

3. **可用态**：上游是 “显示但置灰 / 只读 + tooltip”，Monet 不能直接隐藏。实例内的 Position、Auto layout 方向属于这一类。

4. **折叠 / 展开**：section 默认展开还是折叠、行内展开还是收进弹窗（如 Selection colors、Constraints、Paragraph spacing）。

5. **功能**：点下去是否有命令、命令参数是否正确、菜单项是否按条件增减。

## 2. Section 级对照



| 区块                           | 上游出现条件                         | Monet 等价条件                     | 结论                                                                     |
| ---------------------------- | ------------------------------ | ------------------------------ | ---------------------------------------------------------------------- |
| 顶栏 + View Bar                | 始终                             | 始终                             | 基本对齐；Zoom 菜单 14 项一致                                                    |
| Page                         | 空选区                            | 空选区                            | 部分：缺页面变量模式、Local variables，背景透明度只读                                     |
| Header（类型 + 快捷动作）            | 非空选区                           | 非空选区                           | 部分：缺 Fit / 状态图标、组件名；Create set 未按条件过滤                                  |
| Component（实例 / 组件 / 变体 / 绑定） | 选中组件、实例、实例内层                   | 同左                             | 基本对齐；组件 / 组件集缺 Description                                             |
| Section（开发状态）                | 所有 `has_status` 节点             | 仅 Section 节点                   | 缺口：顶层 Frame 的状态 / Resize 在 Design 页不可达                                 |
| Position / Scale / Points    | 非矢量编辑 → Position；矢量编辑 → Points | 非矢量编辑 → Position；有选中点 → Points | 缺口：矢量编辑但无选中点时走了 Position                                               |
| Auto layout                  | 非 Points 选中                    | 非 Points 选中                    | 部分：Constraints 折叠、Group 无 `+`、实例内方向可编辑、缺 Selection spacing / Grid cell |
| Arc / Star                   | 全椭圆 / 全多边形且非矢量编辑               | 全椭圆 / 全多边形且非 Points 选中         | 条件基本对齐；矢量编辑无点时多显示；缺 caption                                            |
| Appearance                   | 始终                             | 始终                             | 部分：缺 Mask 类型、可见性变量行、iOS 预设；PassThrough 多一行                             |
| Variable modes               | 选区有模式                          | 选区有模式                          | 对齐；Page 级缺失                                                            |
| Typography                   | 选区中**任一**文本                    | 选区中任一文本                        | 部分：数据按 “全是文本” 读取，混合选区内容错；缺 Path / 样式联动                                 |
| Fill / Stroke                | 始终                             | 始终                             | 部分：缺文本 range fill /region fill；混合填充无提示                                 |
| Effects                      | 始终                             | 始终                             | 基本对齐；设置从行内展开改为弹窗                                                       |
| Selection colors             | ≥2 色，或 1 色且有子层                 | 同左                             | 显隐对齐；默认折叠（用户要求）                                                        |
| Layout grid                  | 全部 frame-like                  | 全部 frame-like                  | 显隐对齐；整块设置缺失                                                            |
| Export                       | 始终                             | 始终                             | 对齐                                                                     |
| Prototype                    | 有选区 / 空选区                      | 有选区 / 空选区                      | 空选 Flow starting points 为有意偏离                                          |
| Inspect                      | 有选区                            | 有选区                            | 本轮不展开                                                                  |

## 3. 差异清单

### 3.1 P0（先修，直接影响 “看起来少了一块”）



| ID  | 差异                                            | 上游                                                        | Monet 现状                                             | 证据                                                            | 批次 |
| --- | --------------------------------------------- | --------------------------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------------- | -- |
| D01 | Constraints 行默认不出现                            | `in_frame` 时直接在 X/Y 下画 Constraints                        | 收进 X/Y 行尾的 LayoutGrid 按钮，点开才画                        | 上游 `right.rs:727-728`；Monet `panels.rs:2544-2580`             | 1  |
| D02 | 矢量编辑但无选中点：显示 Position / Arc / Star，而不是 Points | 只要 `vector_edit=Some` 就画 Points；Position / Arc / Star 都不画 | 用 `points_selected` 判断；无点时落回 Position，Arc / Star 也出现 | 上游 `right.rs:283-301`；Monet `panels.rs:2465-2471`、`2643-2650` | 1  |
| D03 | 顶层 Frame 的开发状态 / Resize 面板缺失                  | `has_status` 节点都画；单 Frame 的状态在 header，非单 Frame 在 section  | `sections::section` 只对 Section 节点生效                  | 上游 `section_panel.rs:81-90`；Monet `sections.rs:3390-3397`     | 1  |
| D04 | 页面背景不透明度只读                                    | 背景色和透明度都可编辑                                               | 显示固定 “100%” 标签                                       | 上游 `right.rs:338-359`；Monet `sections.rs:4996-5017`           | 1  |

### 3.2 P1（缺失控件 / 错误禁用 / 整块功能）



| ID  | 差异                                                        | 上游                                                                             | Monet 现状                                                           | 证据                                                                            | 批次 |
| --- | --------------------------------------------------------- | ------------------------------------------------------------------------------ | ------------------------------------------------------------------ | ----------------------------------------------------------------------------- | -- |
| D05 | 实例内 Position 仍可编辑                                         | 整个 Position 字段禁用 + tooltip “In an instance…”                                   | 无禁用，可改 X/Y/W/H                                                     | 上游 `right.rs:653-659`；Monet `panels.rs:2537-2631`                             | 1  |
| D06 | 实例内 Auto layout 方向仍可编辑                                    | 方向控件禁用 + tooltip                                                               | 方向分段控件始终可点                                                         | 上游 `layout_panel.rs:55-57`；Monet `panels.rs:2738-2758`                        | 1  |
| D07 | Frame 头部缺 Fit to contents + 开发状态图标                        | 单选 Frame 时 header 有 swap /fit/status 三个动作                                      | 只有 swap                                                            | 上游 `right.rs:521-529`；Monet `sections.rs:3651-3725`                           | 1  |
| D08 | 多选 Create component 菜单始终含 Create component set，且无 chevron | 仅 `can_create_set` 时增加该项；按钮 34pt 带 chevron                                     | 三项固定；24pt 图标按钮                                                     | 上游 `right.rs:560-611`；Monet `sections.rs:3567-3577`（WIP）                      | 1  |
| D09 | 混合选区（文本 + 非文本）Typography 内容错                              | 只取选区中的文本节点；命令只作用于文本 ids                                                        | `PanelData.text` 仅 “全部是文本” 才 `Some`；混合时走默认值，命令打整个选区                | 上游 `typography.rs:96-102`、`right.rs:309`；Monet `data.rs:2467-2468`            | 2  |
| D10 | Page 缺变量模式                                                | 空选时画页面级 Variable modes                                                         | 只有选区级；Page 不画                                                      | 上游 `right.rs:377-379`；Monet `sections.rs:4975-5062`                           | 2  |
| D11 | Page 缺 Local variables 入口                                 | 有 “Local variables” section + Open variables 控件                                | 无入口                                                                | 上游 `right.rs:396-404`；Monet `sections.rs:5036-5045`                           | 2  |
| D12 | 全 Mask 选区缺 Mask 类型行                                       | `is_mask` 时画 Mask 下拉（Alpha / Luminance / Vector）                               | 完全没有                                                               | 上游 `paints.rs:37-50`；Monet `sections.rs:2739-2841`                            | 2  |
| D13 | 文本选区 Fill 未切换为 range fill                                 | 编辑文本且有 range 时，Fill 变成单行 range fill + “Use the layer's fill”                   | 仍按图层 fills 画，编辑打到整层                                                | 上游 `right/paints.rs:313-319`；Monet `sections.rs:1435-1490`                    | 2  |
| D14 | Layout grid 设置整块缺失                                        | 行内 Grid 按钮展开：size /count/type /width-height/margin-offset /gutter/color + 变量绑定 | 只有类型下拉、显隐、删除；无任何尺寸 / 间距设置                                          | 上游 `right/effects.rs:395-433`、`474-573`；Monet `sections.rs:2231-2270`         | 3  |
| D15 | 多选缺 Selection spacing 行                                   | 多选成行 / 列时显示 Spacing + H/V 数值                                                   | 只有画布手柄，右栏无字段                                                       | 上游 `layout_panel.rs:71`、`canvas/selection_spacing.rs:120-139`；Monet 无         | 3  |
| D16 | Grid layout 子层缺 Grid cell 控件                              | 父级是 grid 且非 absolute 时显示 Column / Row、span、对齐、Auto                             | 完全没有                                                               | 上游 `layout_panel.rs:72-75`、`629-661`；Monet 无                                  | 3  |
| D17 | 组件 / 组件集缺 Description                                     | Properties 节里可编辑描述                                                             | 只在实例里只读显示主组件描述                                                     | 上游 `component_panel.rs:78-87`；Monet `component.rs:133-208`                    | 2  |
| D18 | 缺 Text on path 行                                          | 文本在路径上时显示 start /flip/remove；文本 + 路径时显示 Put text on path                       | 完全没有                                                               | 上游 `typography.rs:121`、`126-158`；Monet `sections.rs:2900-2990`                | 3  |
| D19 | Blend 行不跟随选区                                              | 每帧从选区算 blend；非 Normal 即显示，PassThrough 除外                                       | `blend_label` 只在手动选过 blend 时更新；选中已有 Multiply 图层不显示；PassThrough 仍显示 | 上游 `paints.rs:37-51`、`134-136`；Monet `sections.rs:2845`、`panels.rs:2030-2031` | 2  |

### 3.3 P2（细节 / 次要控件 / 表现）



| ID  | 差异                                  | 上游                                                            | Monet 现状                              | 证据                                                         | 批次 |
| --- | ----------------------------------- | ------------------------------------------------------------- | ------------------------------------- | ---------------------------------------------------------- | -- |
| D20 | 矢量点选区缺 Alignment 行                  | 有点时画 Alignment 行（左 / 中 / 右、上 / 中 / 下、分布）                      | 无                                     | 上游 `vector_panel.rs:106-109`；Monet `sections.rs:3007-3096` | 2  |
| D21 | Width 工具无选中点时标题 / 提示错               | 画 “Width point” + “Drag the stroke to add a width point”      | 落到 “Point” 且提示 “Select points…”       | 上游 `vector_panel.rs:52-60`；Monet `sections.rs:3011-3020`   | 2  |
| D22 | Group 不能从右栏加 Auto layout            | `frame-like OR Group` 都可加，`+` 按钮出现                            | 只认 `all_frame`，Group 无 `+`            | 上游 `layout_panel.rs:33,44`；Monet `panels.rs:605`           | 4  |
| D23 | 混合 Fill / Stroke 无提示                | 头部下有一行 “Click + to replace mixed content”                     | 空列表只画置灰头部                             | 上游 `right/paints.rs:364-365`；Monet `sections.rs:1486-1490` | 4  |
| D24 | 组件头部不显示组件名 / 图标                     | 单选组件显示 icon + 名称                                              | 只显示 “Component / Component set” 类型标签  | 上游 `right.rs:480-499`；Monet `sections.rs:3637-3648`        | 4  |
| D25 | 可见性变量绑定行缺失                          | `visible` 绑了变量时画 bound row                                    | 只在组件绑定场景画按钮                           | 上游 `paints.rs:35`；Monet `sections.rs:2739-2757`            | 2  |
| D26 | Corner smoothing 缺 iOS 预设           | 平滑行里有 “iOS” 按钮（0.6）                                           | 弹窗只有滑杆                                | 上游 `paints.rs:129-131`；Monet 弹窗无                           | 2  |
| D27 | Section 面板细节缺失                      | 选中色环、Hide contents 复选、Changed reason + Resolve、Last change 说明 | 无选中色环、下拉式 Hide contents、只有 Resolve 下拉 | 上游 `section_panel.rs:95-158`；Monet `sections.rs:3401-3461` | 3  |
| D28 | Arc / Star 缺字段 caption 与 Polygon 标题 | Arc 有 Start / Sweep / Ratio；Star 按类型用 Star / Polygon          | 只有三个无 caption 字段；标题恒为 Star            | 上游 `right.rs:884-967`；Monet `sections.rs:2692-2701`        | 4  |
| D29 | Page 缺 “Color profile” 子标题          | 背景后先 subheader，再下拉                                            | 直接画下拉                                 | 上游 `right.rs:380`；Monet `sections.rs:5039`                 | 4  |
| D30 | Page 样式节命名不一致                       | “Local styles”                                                | “Styles”，并把 `+` 放在 header             | 上游 `right.rs:407`；Monet `sections.rs:5046`                 | 4  |
| D31 | 文本样式已链接时仍显示字体行                      | 有 text style 时用 chip 代替字体行                                    | chip 只在 header，字体行始终显示                | 上游 `typography.rs:115-119`；Monet `sections.rs:2911-2938`   | 2  |
| D32 | 文本字符变量绑定行缺失                         | `characters` 绑变量时画 bound row                                  | 无                                     | 上游 `typography.rs:114`；Monet 无                             | 2  |
| D33 | 矢量编辑 region 选中时 Fill 未切换            | region fills 单行替换                                             | 仍按图层 fills                            | 上游 `right/paints.rs:323-333`；Monet 无                       | 3  |

## 4. 补全计划

### 批次 1：显隐纠偏（D01–D08，1–2 天）

目标：只改条件与渲染入口，不动数据模型的大结构。



1. **D01 Constraints**：删掉 `constraints_toggle` 这个中间态，`in_frame` 时直接把 Constraints 子行挂在 X/Y 行下面；实例内跟随 Position 一起禁用。

2. **D02 矢量编辑入口**：把右栏的 “是否替换 Position” 从 `data.vector.has_selection` 改成 `data.vector.is_some()`；`vector_points` 在无点时画 Width / Points 的说明态；Arc / Star 的 gate 改成 `!vector_active`。布局节仍按 `!points_selected` 决定是否显示，保持上游行为。

3. **D03 开发状态面板**：`sections::section` 的 gate 从 `is_section` 改为 `d.section.is_some()`；`is_section` 只控制色板 /section 专属内容；单 Frame 不重复画状态行（状态在 header），多 Frame / Section 画状态 + Resize + reason /last change。

4. **D04 页面背景透明度**：`PageData` 增加背景 alpha；把 “100%” 标签换成可编辑字段，提交 `page.setBackground { color }` 的 8 位 hex。

5. **D05 / D06 实例内禁用**：`PanelData` 增加 `nested_in_instance`（注意是 `nested`，不是 `in_instance` 的整体）；Position 整块 `add_enabled_ui(false)`，Auto layout 方向同理，tooltip 用上游原文。

6. **D07 Frame 头部**：单选 Frame 时在 swap 后补 `resizeToFit` 与状态图标；状态菜单复用 `d.section.status`。

7. **D08 Create 菜单**：`HeaderData` 增加 `can_create_set`；`component_button` 只在允许时追加第三项；多选按钮宽度改 34pt 并画 chevron。

**验收场景**：顶层 Frame（有 dev status）；实例内矩形；矢量编辑无选中点的椭圆；空选 Page；多选 Group；多选两个可建组的组件。

### 批次 2：缺失控件（D09–D13、D17、D19–D21、D25–D26、D31–D32，2–4 天）



1. **D09 混合文本**：`PanelData.text` 改成 “选区含文本就有值”，值只从文本 ids 统计；Typography 的行命令显式带 `ids = text_ids`，不再依赖整选区。补一个 “文本 + 矩形” 的快照测试。

2. **D10 / D11 Page 变量**：Page 节顺序恢复为背景 → 页面 Variable modes → Color profile → Local variables → Local styles → Export；`PageData.local_variables` 已经在读，直接接入口。

3. **D12 Mask**：`PanelData` 增加 `all_mask`、`mask_type`；Appearance 在 opacity 前画 Mask 行，提交 `node.set { maskType }`。

4. **D13 文本 range fill**：`PanelData` 增加 `selected_text_range`；`paints()` 命中 range 时改画单行 range fill（颜色 + Use the layer's fill），不再画图层 fills。

5. **D17 组件描述**：`properties_section` 增加 Description 多行字段，`node.set { description }`；实例里的只读描述保持不变。

6. **D19 Blend**：`PanelData` 增加 `blend` 快照；Appearance 直接用快照判断并同步下拉 label，不再依赖 `f.blend_label`；条件按上游排除 `PassThrough`。

7. **D20 / D21 矢量点**：补 Alignment 行；`VectorData` 增加 `width_tool`，Width 工具但无点时保留 “Width point” 标题和拖拽提示。

8. **D25 / D32 变量行**：`bound` 里已有 `visible` / `characters` 时，补对应 bound row。

9. **D26 iOS**：平滑弹窗加 “iOS” 按钮，写入 0.6。

10. **D31 样式联动**：`PanelData` 增加 text style link；有链接时隐藏 font rows，只留 style chip。

**验收场景**：文本 + 矩形混选；页面有 2 个 collection mode；全 Mask；文本 range 选中；组件 / 组件集描述编辑；已有 Multiply 图层选中；Width 工具无点；绑定了 visible /characters 变量；应用 text style 后字体行消失。

### 批次 3：功能块（D14–D16、D18、D27、D33，4–7 天）



1. **D14 Layout grid 设置**：把 `GridRow` 从 “标签 + 显隐” 扩成完整快照；行内 Grid 按钮展开设置；按上游 `grid_settings` 的三种 kind /align 分支画 size、count、type、width-height、margin /offset、gutter、color；绑定 key 用 `layoutGrids/{i}/{field}`。

2. **D15 Selection spacing**：右栏快照 `selection_spacing(doc, ids)`；在 Auto layout 末尾画 “Spacing” + H/V 字段，提交 `arrange.spacing { ids, horizontal|vertical, fromGesture: true }`。

3. **D16 Grid cell**：父级 grid 且 child absolute=false 时，画 Column / Row、Column span / Row span、H/V align、Auto 重置；命令 `node.set { gridColumn, gridRow, gridColumnSpan, gridRowSpan, gridAlignH, gridAlignV }`。

4. **D18 Text on path**：`TextData` 增加 path start /flip/has\_path；Typography 末尾接 path row，文本 + 路径双选时画 “Put text on path”（命令 `text.onPath`）。

5. **D27 Section 面板细节**：选中色环、真 checkbox、Changed reason 文本 + Resolve、Last change 说明。

6. **D33 Region fill**：矢量编辑 region 选中时，Fill 只画 region 单行 + `+`。

**验收场景**：Grid / Columns / Rows 三种布局各展开一次设置并改值；多选 3 个横排图层改 H spacing；grid 子层改 column /span/align；文本 + 路径；矢量 region 选中。

### 批次 4：细节与表现（D22–D24、D28–D30，0.5–1 天）



1. **D22**：`layout_header` 的容器判定改成 `frame-like OR Group`。

2. **D23**：混合 Fill / Stroke 头部下补提示行。

3. **D24**：单选组件时 header 用 `layer_name` + 组件 icon，不再用类型标签。

4. **D28–D30**：Arc / Star caption 与 Polygon 标题；Page 的 Color profile 子标题；Local styles 命名恢复（或明确保留 “Styles”）。

### 批次 5：需确认后再动（不做默认修改）

见第 6 节。不要在批次 1–4 里顺手 “修” 这些，否则会推翻此前的设计决定。

## 5. 验收与测试



1. **纯显隐单测**：把条件收敛到 `panels::visibility` 风格的表驱动测试，至少覆盖 12 个场景：空选 Page、顶层 Frame、实例内层、Group、多选、文本 + 非文本、矢量编辑 0 点、全 Mask、Text range、Grid 子层、全 frame-like、混合 Fill。

2. **快照测试**：`PanelData::read` 增加上述场景的快照断言，重点断言 `text` / `section` / `vector` / `mask` / `selected_text_range` / `grid_cell` 这些 “条件输入”，避免 UI 层再散写条件。

3. **命令集成测试**：对每个新增可编辑控件跑一次 `Session::execute`，确认参数与目标 ids 正确；尤其 D09 的文本 ids、D13 的 range、D15 的 spacing、D16 的 grid cell。

4. **手工视觉验收**：右栏 240pt、明 / 暗两套主题，每种选区截一张；重点看 “默认态是否出现”“实例内是否置灰而不是消失”“空态是否只剩置灰头部 + `+`”。

5. **回归保护**：把第 6 节的有意偏离写成测试或注释锚点，避免后续 parity 审计反复误报。

## 6. 有意偏离（默认保留，需产品确认）



| 编号 | Monet 现状                                    | 上游                         | 说明                                                                                                        |
| -- | ------------------------------------------- | -------------------------- | --------------------------------------------------------------------------------------------------------- |
| C1 | Appearance 只有 Eye / Visibility / Blend 两颗按钮 | 上游是三颗：Eye、Variable、Blend   | 2026-11-16 按用户要求移除 Variable；本次核对上游 `main` 的 `paints.rs` 确实有三颗，若以代码为准应补回。**需确认当时是否误把 Figma 当成了 figcraft。** |
| C2 | 文本 Resizing 放在 Layout 节                     | 上游在 Typography 节末尾         | 2026-11-11 用户要求移到 Layout，保留                                                                               |
| C3 | Paragraph spacing 收进 Type settings 弹窗       | 上游在 Typography 节内          | 用户要求行高 / 字距留右栏，段间距进弹窗；保留                                                                                  |
| C4 | Selection colors 默认折叠，带 swatch 摘要           | 上游始终展开                     | 用户要求的折叠样式，保留                                                                                              |
| C5 | Prototype 空选不画 Flow starting points         | 上游画 flows + 页面设置           | 用户重写了空选原型面板，保留                                                                                            |
| C6 | 右栏 tab 在动画 / 开发模式显示 “动画 / 开发”               | 上游始终 Design / Prototype    | 用户要求，保留                                                                                                   |
| C7 | Layout grid 支持 Diagonal / Rings             | 上游只有 Grid / Columns / Rows | Monet 额外能力，不算缺口                                                                                           |

## 7. 已知但不在本轮



* 顶栏高度：上游 48pt，Monet 40pt；内容整体上移 8pt（视觉度量差异，不是显隐规则）。

* 个别图标用近似替代（Justify、Type settings、Corner smoothing），属于图标资产缺口。

* Effects 的类型切换从行内下拉改成设置弹窗；功能可达，但交互路径与上游不同。

* Prototype / Inspect 的深度功能对齐（交互编辑器、检查面板）另开批次。

* Motion 右栏是独立 inspector，不在本计划。

## 8. 建议的下一步

先确认第 6 节的 C1（Appearance 变量按钮）—— 这是唯一一个 “上游代码与 Monet 现状直接冲突、且可能源于误读” 的点。其余 P0 / P1 可以按批次 1 → 2 → 3 直接开工；每批完成后跑第 5 节的 12 个场景，再进入下一批。