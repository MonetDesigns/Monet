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
