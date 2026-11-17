# Figma 原型查看器网页 — 布局提取规格

来源：`C:/Users/YeLinTong/Downloads/新建文件夹 (17)/FigCraft-UI-ui-vars - Current UI.html`
（Figma `b1EkkwnSXLctuttceBJ61n`，node `1-617`，另存时间 2026-11-17）

目标：用 GPUI 复刻**这个网页本身的界面布局**（不是画布里 Figma 设计稿的内容）。
画布内是 wasm 从 API 动态加载的设计稿，与本次布局复刻无关。

## 数据来源与可信度

- DOM 骨架：HTML 第 1779 行起 `<div id="react-page">` 的标签树（真实节点，含 inline style）。
- 布局 CSS：`FigCraft-UI-ui-vars - Current UI_files/prototype_app-92a14b65b01bb7e3.min.css.br`
  （Figma 另存的 `.br` 实为明文 CSS/JS，可直接 grep）。
- 尺寸/位置：HTML inline style 里的 `width/height/margin-left/left/top/--x-*` 数值。
- 颜色/间距：CSS 变量 token（`--toolbar-height` 等）。

## 整体结构（DOM → 布局）

```
<body style="color-scheme: light/dark; overscroll-behavior:none">
  <div id="react-page">
    <div class="in_app_page" data-preferred-theme="dark" style="color-scheme:dark">
      <div class="prototype--documentationContainer">
        <div class="prototype--background" style="background-color: rgb(0,0,0)">   ← 黑底画布区
          <div class="header--header header--headerWithSidebar" role="region">      ← 顶部工具栏（浮在画布上）
            left column | center column | right column
          </div>
          <div class="prototype--content">   ← position:absolute; inset:0; display:flex; flex-direction:row
            <div id="prototype-sidebar-panel" style="width:240px; margin-left:-240px">  ← 左侧流程栏（收起）
            <div class="prototype--contentMiddle">  ← flex:1
              <div id="viewerContainer" class="prototype--viewer">
                <canvas width=3298 height=1682 style="width:2093.97px; height:1067.94px">
              <div class="prototype--footerContainer">  ← 底部工具栏
            </div>
            <div class="right_sidebar_panel">  ← 右侧评论栏
          </div>
        </div>
      </div>
    </div>
  </div>
  <iframe src=".../saved_resource.html" style="position:fixed;width:0;height:0">  ← 隐藏 iframe，忽略
```

## 精确尺寸 / 样式

### 顶部 header
- CSS `.header--header`: `background-color: var(--color-bg-toolbar, rgba(0,0,0,.75)); transition: opacity .35s`
- `.fullscreen_view` 内 `--toolbar-height: 40px`；`body / [data-editor-theme=design]` 为 `48px`
  → **原型查看器用 40px**（fullscreen override）。
- 三列 `.header--column`: `height:100%; display:flex; align-items:center; box-sizing:border-box`
- 左列：Figma logo（12×18 SVG，5 色）、评论按钮（24×24）、"Skip to content"（登录态下隐藏）、
  头像动画位 `margin-left:-103.156px`
- 中列：标题 `FigCraft-UI-ui-vars` + 下拉 caret（24×24）
- 右列：多人头像行、**"分享原型"按钮**、选项菜单、全屏按钮
  - 分享按钮：`button__primary` + `button__lg` + `button__hug`；`--btn-height:2rem(32px)`，
    `padding: 0 var(--spacer-2-5)`；主色 `--color-bg-brand`
  - 全屏按钮：`.header--fullScreenButton`，`svg 24×24`，`margin: 0 8px`
- 顶层固定层：`position:fixed; top:0; left:0; width:100%; height:48px; z-index:9; pointer-events:none`

### 左侧流程栏（`prototype-sidebar-panel`）
- inline: `width:240px; margin-left:-240px`（**收起态**，负边距藏起）
- `.sidebar_panel--container`: `margin: var(--toolbar-height) 0 0 0; display:flex; flex-direction:column;
  background:var(--color-bg); border-right-color:var(--color-border-menu);
  z-index:var(--z-index-sidebar); transition: margin-left .2s`
- 内容区 `.sidebar_panel--content`: `flex-grow:1; display:flex; flex-direction:column; overflow-y:auto`
- 头部："流程"（11px，`text--ui2FontSize11`）+ 关闭按钮（24×24）
- 列表容器 inline `height:23px`，`--x-width:240px`
- 描述区："No description"（Quill 只读），`--x-height:1018.95px`
- resize 线：`.sidebar_panel--edge`: `cursor:ew-resize; position:fixed; right:0; top:42px;
  height:100%; width:6px; margin-left:-3px`（inline `left:240px`）

### 中间 viewer
- `.prototype--content`: `position:absolute; height:100%; width:100%; flex row`
- `.prototype--viewer`: `width:100%; height:100%; position:relative`
- canvas inline: `width:2093.97px; height:1067.94px`（bitmap 3298×1682，约 1.575x DPR）
- 画布定位 inline: `left:47.996px; top:59.9901px; width:2093.97px; height:1067.94px`
- 背景 `.prototype--background`: `flex:1; background:var(--color-bg); font-size:11px`

### 底部 footer
- `.footer--footer`: `position:absolute; left:0; right:0; bottom:0; display:flex;
  justify-content:space-between; align-items:center; z-index:10; height:48px;
  font-size:11px; background:transparent; color:var(--color-text)`
- 左侧 debug 区（空）+ 中间 stepper：
  - `1 / 1`（Stepper，role=toolbar，aria-orientation=horizontal）
  - 设备下拉 `.options_menu--deviceName`: `MacBook Pro 14"` + chevron（8×7 SVG）
- 右侧 Restart 按钮：`.footer--pill` `height:30px; border-radius:73px;
  background:var(--color-prototypeloadingbackdrop, rgba(0,0,0,.8))`；
  内容 = Restart 图标 + "Restart" + `R` 快捷键标签

### 右侧评论栏
- `.right_sidebar_panel--sidebarContainer`: flex 列
- 头部 `.comments_sidebar_header`（11px）
  - 搜索框：放大镜 24×24 + `<input placeholder="搜索">` + 清除按钮
  - 排序/筛选按钮（16×16）+ 关闭按钮（24×24）
- 提示文字："提供反馈，提出问题，或只是留下鼓励的话。点击文件中的任意位置，即可留下评论。"
- 空评论列表区 `--x-height:1018.95px`，顶部留白 `height:50px; margin-top:var(--spacer-1)`
- 评论锚点按钮 inline `--x-left:1094.996px; --x-top:593.990px`

## 关键 CSS token（用于取色）

- `--toolbar-height: 40px`（fullscreen / prototype）
- `--color-bg-toolbar: rgba(0,0,0,.75)`
- `--color-prototypeloadingbackdrop: rgba(0,0,0,.8)`
- `--color-bg-brand: var(--ramp-blue-500)`（亮）/ `var(--ramp-blue-400)`（暗）
- `--spacer-1`、`--spacer-2-5`、`--radius-medium/large`
- 画布背景：`rgb(0,0,0)`（inline，实际由 `data-preferred-theme=dark` 决定）

## 复刻要点（GPUI）

1. 窗口主体 = 黑色画布区，header/footer 半透明黑浮在画布上下。
2. header 高 40，左中右三列 flex；"分享原型"是主色实心按钮（32 高，圆角）。
3. 左栏 240 宽，可收起（负 margin 等价：宽 0 + 6pt resize edge，`ew-resize` 光标）。
4. 中间 viewer 铺满剩余空间，内含缩放后的画布局部（2093.97×1067.94，起点 48,60）。
5. footer 高 48，三段：debug（空）| stepper `1 / 1` + 设备下拉 | Restart 胶囊按钮。
6. 右栏评论面板：搜索 + 排序/筛选 + 关闭 + 提示文字 + 空列表。
