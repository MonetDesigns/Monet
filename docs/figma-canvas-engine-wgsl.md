# Figma 画布引擎拆解 → 我们的外置 WGSL

本文记录 `compiled_wasm.wasm`（Figma 编辑器画布渲染器）的实际拆解结果，以及把它里面的

着色数学搬进 FigCraft 外置 WGSL 时的改写方式、接线点和 1:1 边界。

对象与版本（2026-11-26 核对）：



| 来源                                                                         | 大小         | SHA-256           | 说明                                          |
| -------------------------------------------------------------------------- | ---------- | ----------------- | ------------------------------------------- |
| `static.figma.com/fullscreen/3c49878a…/fullscreen-wasm/compiled_wasm.wasm` | 54,904,274 | `139B285B…247A6A` | 线上仍在服务，与本机 Figma 126.11.11 的缓存解压结果**逐字节一致** |
| 用户提供的 `compiled_wasm.wasm`                                                 | 53,241,904 | `9CCC1CF9…790D97` | 同代构建（shader 表与 uniform 名全同），体积差来自构建变体       |

> 更新的构建拿不到：
>
> `Fig.fullscreenURLs`
>
>  只在
>
> **登录态**
>
> 编辑器 bootstrap 里下发；未登录
> `www.figma.com/files`
>
>  只返回登录页，
>
> `static.figma.com/fullscreen/<hash>/`
>
>  的 hash 无法枚举。
> 想拿最新：在 Figma 里打开任意文件刷新一次缓存，再从
> `%APPDATA%\Figma\DesktopProfile\v44\Cache\Cache_Data\index`
>
>  里取新的 
>
> `fullscreen/<hash>`
>
> 。

## 1. 它是什么



* **不是 WGSL 引擎。** 画布绘制全部是 **GLSL ES 3.00**：wasm 里 79 处 `gl_FragColor`

  （同一批 shader 的 GL2 / GL3 / ES 三个变体），viewer 包里 64 处。

* wasm 里只有 6 段 **WGSL compute**（`@compute @workgroup_size(256)` 的前缀和归约），

  用于 GPU 统计，不是画布材质。

* 着色器源码是**明文字符串**存在 wasm 的数据段里，可以直接抓出来（本次即如此）；

  参数名也被保留（`uHalfSizeOverBlurRadius`、`uCount`、`uBlurDelta`、`noiseType`…），

  但**常量值**（网格颜色、dot 灰度）是 C++ 里的 f32 立即数，字符串表里没有。

抓取方式：在 `Cache_Data\f_000076`（brotli 压缩流）里解出 wasm，按 `#version 300 es` /

`precision highp float` 切字符串；或在 JS 包的 `viewer-*.min.js` 里按

`SHADER_SOURCE_*="…"` 取同一批 shader。

## 2. 画布网格（Figma Design / FigJam）

Figma 的「画布网格」有三个不同的东西，别混：

### 2.1 像素网格（View → Pixel grid，内部名 `renderGrid`）



* 只在很高倍率出现；命令表里的 synonym 是 `hide pixel grid` / `pixel gridlines`。

* 画法：每 **1 文档单位**一条 **1 设备像素**的线（发丝线，缩放不变粗），两条轴取并集。

* 颜色：主题里的一档低透明度墨色。我们的 `canvas/overlays.rs` 现用

  `Color32::from_black_alpha(18)`（≈7.05%），开关阈值是 `ViewState::PIXEL_GRID_ZOOM = 5.07`

  （`crates/engine/src/view_state.rs:136`）。注意这里我们自己也不自洽：`overlays.rs:114`

  的注释和 `docs/target-app-parity.md` 都写「400% 起」，常量却是 507% —— 要么改常量要么改注释。

  Figma 自己的常量和颜色值同样读不到（在 wasm 的 C++ 立即数里），所以 `grid.wgsl`

  把这些都做成 uniform：亮色 `0,0,0,0.07`，暗色 `1,1,1,0.07`。

### 2.2 FigJam 点阵（内部名 `showDotGrid`，`checkerboardStyle == 2`）

这段 shader 在 wasm 里能整段读到（`dotBackgroundBaseColor` / `dotBackgroundZoomedOutColor` /

`dotColorDiff` / `gridData` / `pageColor`），数学是完整的：



```
step_doc = 16 × {32,16,8,4,2,1}     // 按 zoomScale < {1/32,1/16,1/8,1/4,1/2} 取档
fade     = smoothstep(0.5, 1.0, zoomScale)     // 50% 以下淡出
dot_col  = dotBackgroundZoomedOutColor + dotColorDiff * fade
p        = round(frag / step_doc) * step_doc   // 最近格点
dist_px  = length(frag - p) * zoomScale
radius_px= max(0.5, 1.0 * zoomScale)           // 50% 时 0.5px，100% 时 1px
coverage = 1 - smoothstep(0, 1, dist_px - radius_px + 0.5)
color    = mix(dotBackgroundBaseColor, dot_col, coverage)
```

两点要注意：底色 / 点色都是**灰度 float**（不是 RGB），所以 FigJam 的点阵是单色的；

具体灰度值同样在 C++ 里，包里只有 uniform 名。

### 2.3 画板布局网格（Layout grid，frame 属性）

模型直接来自 JS 包的默认值：



```
{ offset: 0, sectionSize, color: { r: 1, g: 0, b: 0, a: 0.1 }, pattern: "GRID", gutterSize: 20 }
```



* `pattern`：`GRID`（方格）或 `STRIPES`（`axis: "X"` 列 / `"Y"` 行）。

* `alignment`：`MIN` / `CENTER` / `STRETCH` / `MAX`；`STRETCH` 时段宽 =

  `(extent - gutter*(n-1))/n`。

* 画法：**GRID 画 1 设备像素的线**（不是 1 文档像素），**STRIPES 画色带**（不是线）；

  色带的 alpha 就是 `color.a`（默认 10% 纯红）；两者都裁在画板矩形内，不随图层缩放变粗。

* 我们 doc 里的 `LayoutGrid::default()` 是 `(1,0,0,0.1)`、gutter 20，和 Figma 对得上。

`grid.wgsl` 把 2.1 与 2.3 合在一个覆盖层 pass 里（mode 0/1/2/3），2.2 的点阵没有搬

（本项目没有 FigJam 画布；需要时按上面四行公式单独加一个 mode 即可）。

## 3. 五个效果的真实算法

### 3.1 投影 / 内阴影

矩形（Figma 给 frame/rect 用的就是这条，`ROUNDED_RECTANGLE_BLUR` + `RECTANGLE_BLUR`）：



* **解析积分，一遍，不读任何纹理。** 顶点把 quad 四边各外扩 `blurRadius`，

  片元取 `p = (frag - shadowCenter)/blurRadius`。

* 核是 `i(x)` 的导数：`i` 在 `-1..0` 是 `0.5(x+1)²`、`0..1` 是 `0.5-0.5x(x-2)`

  —— 这是**三角核**（两次 box）的 CDF，不是高斯。

* 1D 覆盖 = `i(d + half) - i(d - half)`；圆角用 `aa()` 的 4 采样角部积分近似：

  每个样本的边线位置 `u = half_o - q + sqrt(q² - t²)`，`t = min(half_p - q - |p|, 0)`。

* `spread` 不是改核：它把 **shadowBounds** 撑大，`maskBounds` 不动（wasm 里两组

  uniform：`shadowHalfSize` / `maskHalfSize`、`shadowCornerRadius` / `maskCornerRadius`）。

* 最后 `e()`：把颜色加 `2/255` 抖动再 `min(abs(c),1)` —— 阴影也抖，不只是渐变。

* 非矩形走 mask：mask 先过 `CLAMPED_BLUR_TILE_NODESPACE`（见 3.2），

  再 `MASK_INNER_SHADOW_TILE = (1 - blurredMask.a) * shape.a * color`、

  `MASK_KNOCKOUT_SHADOW = blurred * (shape.a == 0)`（`useBinaryKnockoutMask` 时是二值）。

### 3.2 模糊链（图层模糊 / 背景模糊共用）

`DOWNSAMPLE_TILE_FRAGMENT` 只是一次带线性采样的拷贝（2×2 归约）；

`CLAMPED_BLUR_TILE_NODESPACE_FRAGMENT` 是主角：



* uniform：`uCount`（tap 数，最多 63）、`uBlurDelta`（每 tap 的节点空间步长）、

  `blurClamp`（把采样钳在源矩形里）。

* 沿**一个轴**取 ±i 的 63 个 tap，权重全 1（box），两端用 clamp 后的坐标做加权合并；

  水平、垂直各跑一遍 → **可分离两遍 box**。

* `progressiveBlur` 是另一支：`startCount`/`endCount` + `blurGradientStart/End`，

  按到渐变线的距离在「原图」和「模糊图」之间按 `n = j²`、`o = ((j-1)²+2(j-1))/2` 加权 —— 这就是

  Figma 的渐进模糊。

### 3.3 图层模糊

图层先渲进 tile，再走 3.2，再合成。没有单独的片元；半径 = box 半宽。

### 3.4 背景模糊

`MASK_BACKGROUND_BLUR_TILE_FRAGMENT` 只有两句：形状外 `discard`，形状内返回**已经模糊好的**

backdrop。模糊本身在 3.2 的链上。

### 3.5 噪点 / 纹理

`NOISE_EFFECT_FRAGMENT`，整套数学（哈希、梯度噪声、三层 fbm、density 窗口、三种模式）

都在 `assets/wgsl/noise.wgsl` 里逐函数移植了，要点：



* 坐标 = `(节点空间位置 - nodeSpaceCenterPoint) * noiseSize`：锚在节点中心、随文档缩放。

* 3 层梯度噪声，权重 `1, 1/2, 1/4`；每层 seed =

  `fract(sin(seed*91.3458 + octave*57.123 + 12.345) * 43758.5453)`。

* `density` 是**窗口**不是阈值：`lo = 10*(0.25 - d/4)`（duo 第二色用 `0.75`），

  `hi = lo + 10*(d/2)`，`keep = step(lo, v*10) * (1 - step(hi, v*10))`。量程 **0..100**。

* `type`：0 = multi（RGB 各一份噪声 + 第四份做 alpha）、1 = mono（`0.213/0.715/0.072`）、

  2 = duo（两个互补窗口、两个颜色）。

* alpha = `density * color.a * 图层 alpha`。

## 4. 我们的外置 WGSL 能不能直接吃下

结论：**只有网格和背景模糊能直接插；另外四个不行，卡在通道语义上，不是数学上。**



| 效果        | 现在能跑吗                        | 卡点                                                                                                                  |
| --------- | ---------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| 画布 / 画板网格 | ⚠️ 能跑，但没必要                   | 我们的网格是 egui painter 画的（`canvas/overlays.rs`），不是 shader；`grid.wgsl` 的价值是**把画法固化成公式**（将来覆盖层上 GPU 时直接用），当下改 CPU 那两处更划算 |
| 背景模糊      | ✅（backdrop + fields，输出在内容之下） | 无                                                                                                                   |
| 投影        | ❌                            | ① 需要本层 alpha（content）；② 材质通道会**替换节点自己的填充**（`draw.rs:713 draw_children_only`），填充会被抹掉                                 |
| 内阴影       | ❌                            | 内阴影画在内容**之上**，材质通道画在内容**之下**；同样需要 content                                                                           |
| 图层模糊      | ❌                            | 模糊对象是本层内容，通道里没有 content                                                                                             |
| 噪点        | ❌                            | 同上，且噪点要盖在内容上                                                                                                        |

也就是说：`assets/wgsl` 现在这套是 **「底图材质」通道**（glass / 背景模糊那一类，

替换的是节点下面那层漆），不是 \*\*「图层效果」通道 \*\*。Figma 的五个效果全是**内容派生**的，

所以必须补一条内容输入 —— 也就是下面 §5 的 contract v2，不是再写几个 shader 能绕过去的。

## 5. Contract v2：最小接线

新增一个 `@fx` 头标记区分两种通道（不改现有 glass 的行为）：



```
// @fx stage layer          // 缺省 = backdrop（现有语义）
// @fx reach u.blur         // 外扩半径取哪个参数（文档 px），host 乘 dpr
// @fx blur content u.blur  // host 先按该字段做可分离 box，shader 收到糊后的 content
```

运行期需要 6 处改动：



1. `render/src/shade.rs` — `ShadeRequest` 增加

   `content: Option<&[u8]>`（BGRA8，**预乘 alpha**，尺寸 = region）与 `reach_px: f32`。

2. `ui-gpui/src/shade_gpu.rs` — 上传 content 纹理并建 mip 链（和 backdrop 一样），

   region 四边各外扩 `reach_px`，`origin_px` 可以为负；矩形仍传 `rect_px` 做 scissor。

3. `ui-gpui/src/wgsl_runner.rs` — 绑定扫描里加：名字 `content` → content 纹理

   （现在的规则是「非 fields 的纹理一律 backdrop」，`wgsl_runner.rs:508`；不改会把

   content 静默绑成 backdrop，出图是错的）。

4. `render/src/draw.rs` — 绑了 `stage=layer` 的材质时：先把本层内容（含子树）画进

   pixmap，把 content 传上去算，然后**不要再 push&#x20;**`Step::Over`（`wgsl_runner.rs:733`

   的「pass 的子树画在 pass 之上」）—— 子树已经烤进 content，再画一遍会叠两次；同时跳过

   CPU 的 `inner` / `shadow` / `layer_effects`。合成用不着改成 Replace：`out` 纹理是以

   canvas 现有像素为底、每个 pass 都是 `Load` + `ALPHA_BLENDING`（`wgsl_runner.rs:566`），

   整层结果按 straight alpha 除一次 `over` 就是对的。

5. `render/src/wgsl_effect.rs` — 解析 `@fx stage` / `@fx reach`；`stage=layer`

   的 effect 必须有 content 绑定，缺了要在加载时就报错，不要等到出图才发现没绑上。

6. **面板** — `stage=layer` 的材质只出现在「图层效果」列表，不出现在「填充材质」列表。

不改这 6 处，`shadow.wgsl` / `layer_blur.wgsl` / `noise.wgsl` 挂上去会得到一张错图

（content 被当 backdrop 绑）。`background_blur.wgsl` 和 `grid.wgsl` 不受影响。

### 已做的校验



* `naga`（wgpu 的 WGSL 前端，30.0.1）逐个文件 `Validation successful`。

* 项目自己的加载器（`figcraft_render::wgsl_effect`，直接编译 `crates/render/src/wgsl_effect.rs`

  跑的探针）全部 `OK`，参数全被识别、`set_param` 往返正常、uniform 打包尺寸

  `shadow 96 / layer_blur 32 / background_blur 32 / noise 80 / grid 80` 字节，都在 256 以内。

* **没有跑 GPU 出图**：这些数字是解析和布局层面的，像素是否 1:1 还得上夹具截图比对（见 §7）。

## 6. 1:1 的边界（哪些会对不上）



* **模糊核**：Figma 是「可分离两遍 box，63 tap，clamp 加权」；我们走 mip 链

  （2×2 降采样 + 线性上采样）。小半径几乎一致，大半径核形状不同（都收敛高斯）。

  要逐像素对齐，就得放弃 mip、照 `CLAMPED_BLUR_TILE` 跑两遍（v2 的单 pass 装不下，

  需要 `fs_blurH/V` 那种多 pass）。

* **阴影核**：Figma 矩形是**三角核**（两次 box）；我们 CPU 是 3-box 高斯。GPU 走

  `shadow.wgsl` 的 analytic 路径才和 Figma 一致，CPU 路径要同步改才不打架。

* **mask 路径的 spread**：Figma 撑的是 shadowBounds；我们 mask 路径没有做 dilate，

  `spread` 只在 analytic 路径生效。

* **颜色空间**：两边都在非 sRGB 的 8bit 目标上混合（我们 `Bgra8Unorm`），这一点是一致的；

  但 Figma 的 tile 是预乘存储，我们的 content 也要求预乘，否则半透明边缘会发黑。

* **抖动**：Figma 在阴影 / 渐变 / 玻璃上加 `2/255` 抖动；背景模糊的 mask 片元不抖。

  三个新 shader 里只有阴影抖了，和它一致。

* **噪点合成**：Figma 是层内一道 paint（alpha 叠加）；我们保持层覆盖、按 alpha 占比推颜色。

  不透明度 100% 的层上两者一致；半透明层上会有细微差别。

## 7. 校验建议



* 夹具：100×100 圆角 12 的矩形 + 每档 blur（0/4/8/16/32/64）、offset、spread；

  同一文档分别用 Figma 导出 2× PNG 和本项目 GPU 路径出图，做 `max |Δ|` 与

  `p99 |Δ|`，阈值建议 `max ≤ 2/255`、`p99 ≤ 1/255`（抖动本身的量级）。

* 网格：像素网格用 500%/800% 两档截图数线宽（必须是 1 设备像素）；

  布局网格用 GRID/STRIPES × 四种 alignment 各一张，核对色带边界与 gutter。

* 噪点：固定 seed，导出 1× 与 4×，确认图案锚在节点中心且随缩放不变。
## 8. 落地状态（2026-11-26 夜）

* **接口已落地**：`crates/render/src/wgsl_effect.rs` 新增 `Stage { Backdrop, Layer }`，解析
  `@fx stage layer|backdrop` 与 `@fx reach <field>`，对外 `stage(id)` / `reach(id)` /
  `needs_content(id)`。五个 shader 补了标记；探针实测 shadow / layer_blur / noise 为
  `Stage::Layer`，shadow / layer_blur 的 reach 为 `u.blur`，background_blur / grid / glass 为
  `Stage::Backdrop`。**这层只改元数据，不改现有 glass 的行为。**
* **band 精度已改**：`crates/render/src/band.rs` 不再在页面坐标里建 f32 路径，先把 `page_xf`
  乘完再收窄；新增“1e9 页面单位外的带子和原点同一块带子像素一致”的回归测试。
* **卡住**：`crates/ui-gpui/src/panels/right/sections.rs` 有 5 个既有编译错误
  （`ModeRow` 未定义，`FieldRow` / `AnyElement` / `Section` 类型不匹配），外加
  `crates/plugin-figma-quickjs/Cargo.toml` 只有清单、没有 `src/`，整个 workspace
  `cargo check` 直接失败。所以 runner / draw 那 6 处接线**没有落地**——改了也验不了。
* **未做**：白板点阵按 Figma 公式改写（`dots.rs` 圆形 AA + `board_bar.rs` 16/六档/smoothstep）；
  `Drag::Gap` 只认被拖的那一条（现在用 `key: "gap"` 匹配，会把同类 gap 全铺上）。
