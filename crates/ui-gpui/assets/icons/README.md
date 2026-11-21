# 图标资源规范

`assets/icons/` 下的 SVG 全部**平铺在这一层**，不再按面板/用途分子目录。任何模块都能直接引用任意图标，复用不受目录限制。

## 命名

```
<域>-<名字>.svg          kebab-case，全小写
```

| 域 | 含义 | 例子 |
|---|---|---|
| `ui-` | 通用界面控件：按钮、箭头、勾选、可见性、窗口控制 | `ui-plus` `ui-chevron-down` `ui-eye` `ui-win-restore` |
| `data-` | 变量与数据类型 | `data-variable` `data-number` `data-boolean` |
| `shape-` | 可成为图层的对象类型与形状 | `shape-rectangle` `shape-frame` `shape-component` `shape-union` |
| `tool-` | 画布工具（不产生独立图层语义） | `tool-move` `tool-pen` `tool-shape-builder` `tool-eraser` |
| `layout-` | 对齐、分布、布局模式、边距、圆角 | `layout-align-left` `layout-h` `layout-grid` `layout-padding-top` `layout-corner-tl` |
| `layer-` | 图层树专用字形（与主图标区分时保留） | `layer-frame` `layer-mask` `layer-group` |
| `typ-` | 排版对话框专用 | `typ-align-left` `typ-line-height` |
| `picker-` | 取色器专用 | `picker-solid` `picker-gradient` |

规则：

- **名字里不重复域词**。例如形状工具合并器是 `tool-shape-builder`，不是 `shape-shape-builder`。
- 同一图形只保留**一份物理文件**。多个逻辑图标共用时，在代码的资源表里让它们指向同一文件（如 `ui-panel`/`ui-settings`/`ui-trash` 共用 `ui-dot.svg`）。
- 新增图标先归域，再命名；域内重名时改用更具体的名字，不要加数字后缀。

## 绘制

- `viewBox` 取 16（图层树/小控件）或 24（工具栏/大图标），保持原生网格，不要缩放改写。
- 所有 `stroke-width` 写 `1`（`ICON_STROKE`）。渲染时 `rescale_strokes` 会按目标尺寸补偿。
- 颜色一律 `#000`（`black` 亦可）。**预览是黑色，实际颜色由使用处的 `text_color` 注入**；默认墨色取 `tokens().icon`（浅色主题 `#1E1E1E`，约 90% 黑）。
- 不要使用 `#fff`/`white`：浅色主题下会不可见，也无法被统一着色。

## 加载

- 图标通过 `IconAssets`（`widgets/icon.rs`）的 `ICON_FILES` 表编译期内联，运行时零 I/O。
- 路径形如 `icons/<stem>.svg`，`@<size>` 后缀触发描边重算（`icons/ui-plus@16.svg`）。
- `brand/` 下的产品标识不在本目录，保持独立。

## 使用流程（强制，唯一合法路径）

一个图标从资源到屏幕，**只有**下面四步。任何跳过其中一步的写法都是 bug。

1. **资源**：把 SVG 放进本目录（遵守上面的命名 / 绘制规则）。
2. **注册 + 枚举**：在 `widgets/icon.rs` 的 `ICON_FILES` 注册文件名，并在 `Icon` 枚举里加变体、`asset_rel()` 里加映射。**没有枚举变体的图标等于不存在**——不允许"只注册资源、直接按路径画"。
3. **调用处只引用枚举**：一律走 `icon::paint_in_slot(icon, slot, color)`。
   - `slot` 是**槽位**大小（16 / 24 / 32），尺寸换算由 `slot_glyph` 负责：
     - 24 槽：画图标的原生尺寸（24 网格画 24，16 网格仍画 16）。
     - 16 / 32 槽：图标缩放到槽大小；线宽由 `rescale_strokes` 反比补偿，视觉恒为 1pt。
     - 若将来某图标新增了专门的 `<stem>@<size>` 变体，会优先命中它，调用处不用改。
   - `color` 传 `Some(tokens().x)` 指定；传 `None` 时默认取 `tokens().icon`。
   - 需要裸尺寸时用 `icon::default_glyph(icon)` / `natural_size(icon)`。
4. **尺寸与线宽不自行计算**：`rescale_strokes` 在加载时按目标尺寸反比补偿 SVG 的 `stroke-width`，保证 16 / 24 / 32 下视觉线宽恒为 `ICON_STROKE`（1pt）。

```rust
// 正确：只认枚举 + 槽位，颜色可省
.child(paint_in_slot(Icon::PaddingTop, 24.0, None))

// 正确：指定颜色
.child(paint_in_slot(Icon::Eye, 16.0, Some(rgba(tokens().icon))))

// 错误：绕过枚举直接按路径画 —— 尺寸和线宽都没人管
.child(svg().path("icons/layout-padding-top.svg").w(px(16.0)).h(px(16.0)))
```

### 禁止事项

- ❌ `svg().path("icons/…")` 在业务代码里直接画：这条路绕过了 `default_glyph` 和 `rescale_strokes`，尺寸只能手填，一旦资源网格从 16 升到 24 就会被压小（历史上圆角 / 边距 / 描边 lead 图标就是这样坏的）。
- ❌ 硬编码 `w(16) / w(24)` 之类的字形尺寸：槽位大小与字形原生尺寸是两回事，交给 `default_glyph`。
- ❌ 只往 `ICON_FILES` 加资源、不进 `Icon` 枚举：调用处就没法用统一入口。
- ❌ 指定颜色写死在 SVG 里：颜色必须由使用处注入。

### 已知遗留（按本规范逐步迁移）

`Slot::Custom` + `named_icon()` 这条老路径仍被部分字段 lead / 菜单触发图标使用，它们是上述禁令的历史例外，**不是**新代码可以照抄的范例。新增或改动时应改为枚举变体 + `paint_icon`。
