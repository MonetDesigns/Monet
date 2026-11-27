# 完整 GPU 管线立项（Monet / FigCraft）

> 分支：`ai/gpu-pipeline`（worktree `D:\Projektdateien\GPUI\branches\gpu-pipeline`，从 `main` 拉出）
> 状态：阶段 1（图元与填充）、阶段 2（路径与描边）、阶段 3（文字）与阶段 4（投影、内阴影、uniform 与
> 分级图层模糊、背景模糊、Mono/Duo/Multi 颗粒与纹理粗糙边）已作为**过渡实现**落地——能力探测、
> `auto/gpu/cpu` 三档偏好（引擎命令 + Preferences ▸ Rendering）、场景 IR、实例化
> SDF 图元（圆角矩形/椭圆/多边形/星形）、四类渐变、裁剪、路径填充与描边网格（4× MSAA）、文字字形
> 网格、离屏剪影 + 六 pass 盒模糊的投影/内阴影/图层模糊、八级加权混合的分级模糊、按段 resolve 的背景
> 模糊回读、逐 cell 64 位哈希的 Mono/Duo/Multi 颗粒、整像素散射的纹理粗糙边、单叶组透明度、动画导出
> 切到 `figcraft-gpu`、圆角裁剪、离屏组层透明度、大图分块导出、CPU↔GPU 像素 diff（24 项对照测试），UI 画布已接线并整帧回退。**所有内置效果按产品计划弃用，后续效果转到外置 WGSL，
> 本分支不再新增或扩展内置效果**；M5–M7 的非效果核心为设计与接口。
> 日期：2026-11-27

## 0. 一句话结论

画布从「CPU 栅格 → 纹理上传」迁到「GPU 直接出帧」，**不是换渲染器，而是加一条与 CPU 并行的
出图通道**：同一份 `Document`，两条路径产出同一张 `Pixmap`，逐特性做像素 diff，任何一条画不准
的帧整帧回退 CPU。默认 `auto`：探测到硬件 GPU 走 GPU，否则（含软件适配器、设备丢失、场景含未
支持特性）走 CPU。

本分支交付：`figcraft-gpu` crate（能力探测 / 后端选择 / 场景 IR / 实例化 SDF 渲染 / 回读导出 /
diff 工具）、引擎与偏好面板的开关接线、CPU↔GPU 像素回归。M2 起按第 4 节推进。

## 1. 现状盘点（为什么不是从零开始）

| 已有 | 位置 | 对本项的作用 |
|---|---|---|
| CPU 出图（tiny-skia） | `crates/render`（`Retained`、`draw.rs`、`glass.rs`…） | 对照基准与回退路径，**不动** |
| 窗口与 UI（GPUI；egui 版已移出默认构建） | `crates/ui-gpui`（画布 worker、WGSL 材质、导出通道） | 本轮 GPU 画布的接线点；wgpu 设备与 `graphics_backend` 偏好都在这条线上 |
| 最小 GPU 场景渲染（纯色圆角矩形） | `crates/ui-gpui/src/gpu.rs`（`feature = "gpu"`） | 实例化 + SDF + 回读的既有写法，M1 在其之上扩成通用管线 |
| 外置 WGSL 材质跑 GPU | `ui-gpui/wgsl_runner.rs`、`shade_gpu.rs`、`render::wgsl_effect` | 效果阶段的着色器加载/绑定约定 |
| 效果着色器数学 | `assets/wgsl/{shadow,layer_blur,background_blur,glass,noise}.wgsl` | M4 直接复用 |
| 文字排版与字形轮廓 | `crates/text`（skrifa / harfrust，`TextLayout`、`Glyph`、glyph outlines） | M3 的字形图集来源 |
| 路径代数 | `crates/pathops`、`figcraft_geom`（kurbo 形状构造） | M2 的三角化输入；`lyon_tessellation` 只做填充三角化，描边语义仍来自 pathops |

结论：M1 的「图元 + 填充」已有可复用的内核，缺的是**与 UI 解耦的公共 crate、完整的图元/填充
表达、逐特性回退判定和回归口径**。

## 2. 架构

### 2.1 分层

```text
Document (crates/doc)
   │  doc_scene::build(doc, page|ids, view)      ← 唯一做「文档语义 → 绘制语义」的地方
   ▼
Scene (crates/gpu/src/scene.rs)                   ← 纯数据：宽高、底色、图元、裁剪、未支持项
   │  renderer::render(scene)                    ← 只认 Scene，不认 Document，可单测
   ▼
PassGraph（层 → 效果 → 合成）                     ← M4/M5
   ▼
GPU 纹理（Rgba8UnormSrgb 工作，预乘混合）
   │  readback / encoder
   ▼
Pixmap / PNG / JPEG / WebP / 视频帧
```

- **`Scene` 是契约**。GPU 侧不读 `Document`，CPU 侧也不读 `Scene`：两侧都从 `Document` 各自出图，
  diff 才有意义（否则是拿同一份中间表示自证）。
- **`Unsupported` 是显式数据**，不是 `Option::None`。`Scene.unsupported` 列出「这一帧哪里画不准」，
  `doc_scene` 收集它，渲染器见非空即拒绝整帧——保证画布上不会出现「看着差不多、其实错」的像素。
- **不混帧**。一帧内一部分图元 GPU、一部分 CPU 需要坐标/混合/AA 口径完全一致，代价远大于收益；
  M1–M6 一律「整帧 GPU 或整帧 CPU」，M7 后再评估按层混排（离屏层作为纹理交给 CPU 合成）。

### 2.2 crate 边界

```text
figcraft-gpu
├─ backend.rs     能力探测（适配器、后端名、硬件/软件分类），OnceLock 缓存
├─ prefs.rs       RenderBackend::{Auto,Gpu,Cpu}、进程级选择、resolve() 规则
├─ scene.rs       Scene / ShapeKind / Paint / Clip / Unsupported（无 wgpu 依赖）
├─ doc_scene.rs   Document → Scene（阶段 1 子集；不支持项逐条登记）
├─ render.rs      wgpu 设备/管线/绘制/回读（feature = "wgpu"）
├─ shaders/shape.wgsl
└─ diff.rs        CPU/GPU 像素比对（无 wgpu 依赖）
```

依赖：`figcraft-geom`、`figcraft-color`、`figcraft-doc`（`doc_scene`）、可选 `wgpu` / `pollster` /
`bytemuck`。**默认 feature 为空**：不把 wgpu 拖进默认 workspace 构建；UI 侧显式开 `wgpu`。
`ui-egui`、`ui-gpui` 共享同一 crate，GPU 代码只写一份。

### 2.3 关键接口（M1 已实现）

```rust
figcraft_gpu::prefs::set(RenderBackend::Auto);          // 偏好/启动时设置
figcraft_gpu::prefs::effective();                       // 当前生效后端（含降级原因）
figcraft_gpu::doc_scene::build(&doc, page, &view) -> Scene;
match figcraft_gpu::render::Renderer::global().render(&scene) { Ok(rgba) => …, Err(_) => CPU }
```

## 3. 后端选择与回退契约（三条铁律）

1. **偏好三档**：`auto`（默认）/ `gpu` / `cpu`。
   - `auto`：探测到**硬件**适配器才用 GPU；`DeviceType::Cpu`（WARP / lavapipe / Swiftshader）与
     `DeviceType::Other` 一律当「没有 GPU」，走 CPU。理由：软件适配器像素正确但比 tiny-skia 慢，
     用它只会让画布更卡。
   - `gpu`：明确要求。没有可用适配器时**不失败**，记录原因后走 CPU，并在 UI 上说明。
   - `cpu`：强制 CPU（排查 GPU 差异、旧驱动、远程桌面）。
2. **逐帧回退**：`Scene.unsupported` 非空、设备创建失败、管线验证失败、映射读回失败 → 本帧 CPU。
   回退是**每帧判定**，不是一次性开关：文档里加一个文字层不会让整次会话掉到 CPU。
3. **失败锁定**：设备丢失（`DeviceLost`）或连续 3 帧 GPU 出错 → 本会话锁定 CPU，直到用户改偏好或
   重启；同时写日志（`log::warn!`）与「关于/偏好」里的可见状态。绝不静默降级。

## 4. 分阶段计划

每阶段都以「能 diff 通过 + 能回退」为完成线，不以「看着像」为完成线。

### M1 图元与填充（本分支已落地）

- 图元：矩形（四角独立半径）、椭圆（整圆）、正多边形、星形；任意仿射（旋转/斜切）由实例矩阵承担。
- 填充：纯色；线性 / 径向 / 角度 / 菱形渐变（Figma 三手柄：`from`/`to`/`width`，单位盒坐标）；
  多层填充按 Figma 顺序叠加；`paint.opacity`、节点 `opacity`、可见性、祖先裁剪矩形（单层圆角
  裁剪见 M5）。
- 抗锯齿：局部空间 SDF + `fwidth` 屏幕空间覆盖，1px 边缘，任意缩放下线宽恒定。
- **验收**：矩形/椭圆/多边形/星形 × 纯色/四种渐变 × 旋转 × 裁剪，CPU↔GPU 像素 diff 达标（第 6 节）。
- 未做（登记为 Unsupported，走 CPU）：图片填充、图案填充、圆角 smoothing、弧形/环形椭圆、
  文字、效果、非 Normal 混合、组透明度（仅单叶与无裁剪/无效果组的离屏层子集）、蒙版；圆角裁剪只支持单层、无效果、均匀缩放的
  SDF/网格子树。

### M2 路径与描边（本分支已落地）

- 几何与 CPU **同源**：不引入 lyon 的描边器，直接取 `figcraft_render::node_outline`（填充轮廓）与
  `figcraft_render::stroke_fill`（描边轮廓，`figcraft_pathops::stroke_outline_of` 已把 align、
  join、cap、dash、per-side、arrowhead 全部烘进 `BezPath`）。GPU 只负责三角化——所以只有 AA 口径
  的差别，没有第二套描边语义。
- 三角化：`lyon_tessellation::FillTessellator`（tolerance 0.05），`tiny_skia::FillRule`
  直接映射；顶点在**设备像素**里烘焙节点变换，网格着色器只需帧尺寸，无矩阵。
- 覆盖形状：任意填充轮廓（矢量网络、布尔结果、线段、圆角 smoothing、弧形/环形椭圆）、任意描边
  （内/中/外、miter/round/bevel、虚线、per-side）；布尔节点的子节点是操作数，不递归绘制。
- 抗锯齿：整帧 **4× MSAA**（`Rgba8Unorm`，单采样 resolve 供回读），SDF 图元四边形按局部像素尺度
  外扩 4px，避免「样本覆盖率 × SDF 覆盖率」双重抗锯齿。
- 仍登记为 Unsupported：路径上的渐变填充 / 渐变描边、文字、图标、切片（走 CPU）。
- **验收**：新增 2 项 CPU↔GPU 对照（描边圆角矩形 + 虚线描边；环形椭圆 + 星形描边），
  内区 `per_channel = 2`、`bad_ratio = 0`（`diff::compare_interiors` 自动跳过两图任一颜色台阶
  的 1–2px 边带）；另有 8 项场景单测 + 4 项三角化单测。
- 风险：MSAA 的 4 档覆盖率与 tiny-skia 的解析覆盖率在边带上不可能逐级相等 → 边带跳过、内部严格；
  `lyon` 与 tiny-skia 的 miter 极限角度差异被同一「跳过边带」口径吸收。

### M3 文字

- `figcraft_text::TextLayout` 已有字形与位置；用 skrifa 取轮廓 → 栅格进 **R8 字形图集**（按
  `(font, size, subpixel offset, gamma)` 缓存，LRU 上限），GPU 侧只画四边形 + UV + 颜色。
- 颜色字形（emoji，`text::ColorBitmap`）单独一层 RGBA 图集。
- 混排/换行/字距全部沿用 CPU 的 `TextLayout`，GPU **不重新排版**——排版只有一份。
- **验收**：字形覆盖、亚像素定位、斜体/描边文字与 CPU diff；图集重建（新增字形）不闪帧。
- 风险：iOS 上 `skrifa` 与 CoreText 的 hinting 差异 → 只比覆盖率，不比 hinting。

### M3 文字（本分支已落地）

- 轮廓**不再重新排版**：直接取 `figcraft_text::layout_node(n)` 产出的字形与装饰线轮廓（与 CPU 同一个
  `TextLayout`、同一个 `dy`），按 run 的 `style` 分组成 `BezPath`，用 Winding 三角化。
- 每个 run 用自己的 fills（`TextStyle.fills`，缺省回落到节点 `n.fills`），只支持纯色；文字描边复用
  `figcraft_pathops::stroke_outline_of`（closed = true）与 CPU 的 `stroke_rule`（内/外/虚线 → even-odd）。
- 仍登记为 Unsupported：彩色字形（emoji 位图、COLR 分层）、文字渐变/图案填充、文字虚线渐变描边。
- 已知差异：CPU 的 LOD `greeked`（低缩放下画灰条）GPU 未实现——GPU 始终画真实字形；只影响低缩放
  预览的观感，不影响 100% 导出。列入 M7 对齐项。
- **验收**：单测 `text_glyphs_become_meshes`；对照测试 `text_glyphs_match_the_cpu`（覆盖率总量误差
  < 3%、内区 `per_channel = 2`）。已用 `CRAFT_FONTS_DIR` 指向 Arial 的临时字体目录跑过一次真机对照
  （不再 SKIP，字形轮廓、亚像素位置与装饰线均一致）；无字体环境仍打印 `SKIP`。`parity` 的 GPU 设备
  创建/销毁用一把测试锁串行化：并行创建十几个 adapter/device 会卡驱动（安装真字体后暴露）。

### M4 效果（过渡实现；所有内置效果已按产品计划弃用）

> **范围变更（2026-11-27）**：产品计划弃用**所有内置效果**，后续效果转到外置 WGSL。因此本分支不再新增
> 或扩展任何内置效果（玻璃、阴影、模糊、颗粒、纹理均不再作为后续目标）；已落地部分只保留为过渡实现
> 与 parity 参考，不删除。M5–M7 的非效果核心继续。

- **剪影**：投影要求「轮廓 == 节点自身填充轮廓」——SDF 可表达的盒子、无可见描边、无子节点
  （或 `clip` 且自身有填充的 frame-like）、全部可见填充为不透明纯色、节点 `opacity = 1`。内阴影只
  要求节点 `opacity = 1` 与 SDF 盒子：CPU 的内阴影从节点**轮廓**而非像素起算，填充、子节点、描边
  都只是画在它上面。其余投影/内阴影登记 `effect` 并整帧回退。
- **模糊**：为与 CPU 逐字节一致，不做「两 pass 高斯」，而是复刻 `figcraft_render::fx::blur` 的三盒近似：
  `box_radii(sigma)` 与 CPU `box_sizes` 同一公式（`sigma = radius / 2 * scale`），六个单盒 pass
  （水平 r0/r1/r2 → 垂直 r0/r1/r2）；每个 pass 逐像素 `textureLoad` 窗口、`u32` 累加、整除、
  越界按透明 0。内阴影在第一个 pass 读入 `255 - byte`（CPU 的 `fx::invert_alpha` 在模糊前执行），
  其余五个 pass 与投影共用。`shaders/blur.wgsl`。
- **投影合成**：剪影先画进每阴影独立的 4× MSAA 目标并 resolve，模糊链在 `ping/pong` 单采样纹理间
  ping-pong（写入纹理与读回纹理每 pass 互换，最后落在 `pong`）；主 pass 用全屏三角形按剪影 alpha
  乘以投影色（straight sRGB）预乘输出，`show_behind` 由「先画影、后画节点本体」的画家顺序自然满足；
  投影随节点的 `clip` 裁剪。`shaders/shadow.wgsl`。
- **内阴影合成**：额外把同一 SDF 盒子按零偏移画进一张 mask 目标（4× MSAA → resolve）；主 pass 在
  填充之后、子节点与描边之前，用 `shaders/inner.wgsl` 把「模糊后的反转剪影 alpha × mask alpha ×
  tint alpha」预乘输出。与 CPU 的 `inner_shadow`（反转 → 模糊 → 按覆盖裁剪 → tint）同一顺序。
- **图层模糊（uniform）**：只接受「一个可见的 uniform `LayerBlur` + SDF 可表达的盒子 + 无子节点 + 无可见
  描边 + 节点与祖先都无 `clip` + 恰好一个可见纯色填充 + 无投影/内阴影 + 模糊 reach 完全落在画布内」。
  该节点的普通填充被跳过，改为把盒子按零偏移画成白色剪影，走同一套六 pass 盒模糊（`sigma` 同式），
  再以「填充色 × 节点 `opacity`」为 tint 合成——CPU 的层内容就是这层填充，模糊后再按层 `op` 合成。
  复用 `Cmd::Shadow` 与 `shaders/shadow.wgsl`，因此与投影共用同一条模糊链，不额外建管线。
- **背景模糊（uniform）**：只接受「一个可见的 uniform `BackgroundBlur`（radius > 0）+ SDF 盒子 + 恰好一个
  可见填充 + 无子节点/可见描边/`clip` + 无图层模糊 + 节点 `opacity = 1`、Normal 混合」。该节点先把已画内容
  收束成一个主 pass 段：结束时 resolve 4× MSAA，把 resolve 作为六 pass 盒模糊的输入，再重开一个
  `Load` 主 pass，用 `shaders/backdrop.wgsl` 的 SDF 覆盖把模糊结果画进盒子（随后投影、填充、子节点照旧
  覆盖其上）。模糊用 CPU 同一 `sigma = radius / 2 * scale` 与同一盒宽；mask 与 `shape.wgsl` 同一组 SDF
  函数。多段/多个背景模糊各建一条链，读取的都是当时的帧内容。
- **颗粒（Mono / Duo / Multi）**：只接受「一个可见的 `Noise`（density > 0，Normal 混合）+ SDF 盒子 +
  恰好一个可见纯色填充 + 无子节点/可见描边/`clip` + 节点 `opacity = 1` + 没有其它可见效果」。grain 的
  层 alpha 就是该填充的 alpha（颜色 × 填充 opacity）；`shaders/noise.wgsl` 在片元里复刻 CPU 的 64 位
  cell 哈希（`ix*K1 ^ iy*K2 ^ seed*K3` 再三轮 `xorshift/mul`，用两条 `u32` limb 模 2^64 乘加），按 2^-20
  舍入后 floor 到整数 cell，`unit(h,0) < density` 才画：Mono 输出 `color × 层 alpha`；Duo 用哈希最高位
  选 `secondary`；Multi 用 bit 16/32/48 的三个 16 位字段当 RGB、`opacity` 当 alpha，均输出预乘值。不新增
  中间纹理，也不依赖模糊链。
- **纹理粗糙边**：只接受「一个可见的 `Texture` + SDF 盒子 + 恰好一个可见纯色填充 + 无子节点/可见
  描边/`clip` + 节点 `opacity = 1` + 没有其它可见效果」。它与图层模糊一样**替换**节点本身的填充：
  `shaders/texture.wgsl` 用同一 64 位 cell 哈希（seed 先 XOR `0x7E57`）算每个目标像素的整像素位移
  （`floor(0.5 + cos/sin * radius)`），在源像素处用同一 SDF 求填充覆盖率，`clip_to_shape` 时再乘
  目标像素的填充 alpha（CPU 的 `keep`）。半径 < 0.5 像素时 CPU 不动层，GPU 也保留普通填充。
- **共享**：每个阴影/模糊用一条独立实例（内阴影另有零偏移实例），六组 blur bind group 与一组 draw/inner
  bind group 覆盖纹理、采样器与 tint/clip uniform；颗粒和纹理复用主实例缓冲，各建一组 uniform
  bind group。本帧现建，暂不做跨帧缓存池。
- **分级图层模糊**：同一套剪影 + 六 pass 盒模糊，但按 CPU 的 `levels` 取至多 8 个半径；每个半径
  模糊成一层（一层共享的 ping/pong + 一张八层纹理），最后 `progressive.wgsl` 按设备像素上的
  `Ramp` 求 `sigma`，用 CPU 的 `lower_bound - 1` / 255 量化权重混合相邻两层。它同样**替换**节点
  填充，且不套 SDF mask——CPU 画的是已模糊的层像素，会把模糊外溢一起画出来；端点重合或半径差
  < 0.05 时退化成 `sigma1` 的单层。
- **玻璃**：产品已决定弃用，移出本分支范围，不再实现；UI 仍按现有 CPU 路径绘制。
- 仍登记为 Unsupported：带子节点/描边/`clip` 的图层模糊、
  颗粒/纹理叠在子节点/描边/其它效果上；spread ≠ 0、非
  Normal 混合、组/文字/半透明/带描边剪影的投影、组内阴影。
- **验收**：`box_radii` 单测对 CPU 近似公式的三组值；对照 `a_drop_shadow_matches_the_cpu_blur`（圆角
  矩形 radius 12 / offset (6,10) / 55% 黑投影）、`an_inner_shadow_matches_the_cpu_blur`（圆角矩形
  radius 10 / offset (4,6) / 60% 黑内阴影，断言填充先于内阴影）、`a_layer_blur_matches_the_cpu_blur`
  （radius 8 的实心卡片 + radius 6 的 80% 填充 × 0.65 层透明卡片，断言卡片上方有模糊外溢）、
  `a_background_blur_matches_the_cpu_blur`（红/蓝硬边背景上一张 radius 10 的 35% 白卡 + 第二张
  radius 6 的 30% 白卡，断言接缝被抹开且两次 resolve/续画都一致）、`mono_noise_matches_the_cpu`
  （radius 10 的绿色卡片 + `size 1.2 / density 0.55` 的 35% 深色颗粒，断言两侧都真的画出了 grain）、
  `duo_and_multi_noise_match_the_cpu`（Duo 红/蓝双色 35%/65%、Multi 彩色 + `opacity 0.45`，断言两张卡
  都有彩色颗粒且像素 diff 达标）、`a_progressive_layer_blur_matches_the_cpu_blur`（顶端 0、底端 12 的
  渐变卡片，断言顶端仍锐、底端有外溢且像素 diff 达标）与 `a_rough_texture_matches_the_cpu`（70% 填充的
  裁剪纹理卡片 + 不裁剪的纹理卡片，断言前者的外溢被裁掉、后者确实溢出到形状上方）。阴影/内阴影为
  `per_channel = 2`、2%；模糊因两侧量化放宽到 `3 / 3%`；颗粒因 CPU 先量化 8 位层 alpha 用 `3 / 2%`；纹理是逐像素整搬、
  边缘两侧 AA 不同，用 `24 / 3%`（`Tolerance::EDGE` 的口径）。
- 风险：剪影边缘是 4× MSAA 覆盖率、CPU 是解析覆盖率，1 级差会随模糊扩散几像素——对照口径因此
  放宽到 `per_channel = 2` 且允许 2% 像素；不透明度/描边/子节点等剪影不唯一的场景一律回退。

### M4 效果（已冻结：全部内置效果计划弃用）

- 阴影/内阴影/uniform 与分级图层模糊/背景模糊/颗粒（Mono/Duo/Multi）/纹理粗糙边已在上一节作为**过渡实现**
  落地；本节的「两 pass 高斯」是针对模糊核的替代方案讨论，当前仍以 CPU 精确复刻（三盒近似）为准。
- 内置效果已整体冻结：玻璃、带内容的图层模糊、效果叠加顺序等不再做新的 GPU 实现；未来由外置 WGSL
  效果承担。
- 颗粒的过渡实现已覆盖 Mono/Duo/Multi；后续同样随内置效果一起退役。
- **验收**：每种效果单独 diff；效果叠加顺序与 `figcraft_render::effect_order_tests` 一致。
- 风险：模糊核在不同后端（DX12/Metal/Vulkan）的采样精度差异 → 核系数用 f16 存储时先量化。

### 外置 WGSL 效果（后续方向，未在本 PR 实现）

- 内置效果冻结后，效果不再作为 `Item` 的新变体进入本 crate；宿主负责把效果作用的层 / backdrop
  纹理、参数 buffer、采样器和裁剪区准备好，外置 WGSL 只声明自己的 bindings 与 entry point。
- 加载时做 WGSL 校验与 uniform 布局校验（版本号、字段对齐、纹理维度）；任一失败或渲染失败都整帧
  回退 CPU，不输出半张图。
- 效果顺序由文档的效果列表决定；跨效果共享中间纹理，避免每个效果都重新分配整帧。
- 本 PR 只保留现有过渡实现作为 parity 参考，不新增内置效果，也不提前固化外置 ABI。

### M5 组合

- **已落地（过渡子集）**：单叶组透明度。组自身无可见填充/描边/裁剪/蒙版/效果、`Normal` 或
  `PassThrough`，且恰好一个可见子节点；子节点可以是同样的纯组链，最终落到一个叶子。叶子须
  **恰好一个可见漆面（填充或描边）**、无效果、`Normal` 或 `PassThrough`。满足时把整条链的
  `opacity` 乘进子叶的填充、路径填充、描边或文字，不建离屏层。其余任何组透明度整帧回退。
- **已落地（过渡子集）**：圆角裁剪。仅当被裁剪节点自身有圆角、`clip = true`、均匀缩放、无外层
  或嵌套裁剪、整棵子树没有可见效果时，把四角半径折进 `Clip`，由 SDF 与网格 shader 用圆角矩形
  SDF 求覆盖；矩形裁剪仍走原来的硬 discard。嵌套裁剪、与另一裁剪相交、非均匀缩放、子树含
  投影/模糊/颗粒/纹理时登记 `rounded-clip-*` 整帧回退。
- 同一判据也用于节点自身的 `opacity`：恰好一个可见漆面（单个填充或单条描边）、无效果时直接
  折进漆面；填充+描边、多填充等会重叠的漆面登记 `node-opacity` 整帧回退，避免逐漆面乘 alpha
  而 CPU 是整层乘一次。
- **已落地（过渡子集）**：离屏组层透明度。当组自身无可见填充/描边/裁剪/蒙版/效果、`Normal` 或
  `PassThrough`、子树内没有 `BackgroundBlur`（层内读不到文档背景）时，子节点先画进一张全帧透明
  `Rgba8Unorm` 4× MSAA 层，resolve 后由 `shaders/composite.wgsl` 把四个预乘通道一起乘组
  `opacity`，再按画家顺序合成一次。嵌套组按「父组先登记、子组后登记」的逆序准备各自的层，父层
  合成时子层已 resolve；重叠的多子节点因此先完成组内合成，再乘一次 alpha，与 CPU 的
  `save_layer` 语义一致。组内投影/内阴影/模糊/颗粒/纹理仍走既有 pass；组内 `BackgroundBlur`
  与组自身的裁剪/效果登记 `group-opacity` 整帧回退。
- 非 Normal 混合模式 / 蒙版：**离屏层扩展**。组内容画进层纹理，再以 `opacity` + `blend` 合成；
  蒙版用 stencil（Metal 的 `depthStencil` 等价）或 `mask_type` 对应的
  alpha/outline/luminance 采样。
- **验收**：`blend` 16 种模式逐项 diff；组透明度与 CPU 的「先合成再乘 alpha」一致；蒙版边缘与
  CPU `mask_bounds_tests` 一致；圆角裁剪的角部与 CPU 裁剪路径一致（已完成 SDF/网格两项对照）；
  离屏组层已完成重叠双叶与嵌套双组两项对照。

### M6 导出链路

- **已落地（过渡）**：动画导出的 `set_export_frame_hook` 从旧 `ui-gpui::gpu` 的纯色矩形场景切到
  `figcraft-gpu`：`doc_scene::build_subtree` 判支持度、`render::render_document_if_gpu` 出帧，再包成
  `Pixmap` 交给原编码链路。用户强制 CPU 或无适配器时仍回落 CPU。带外置材质的区域导出继续走
  CPU 多 pass + `shade_gpu`，两条路径互不替代。
- **已落地（过渡）**：大图分块。`Renderer::render_document_tiled` 按
  `max_texture_dimension_2d` 把视口切成不重叠的方格，每格单独 build + render，再把行原位拷进整帧
  缓冲。纯图元/网格/文字/组层可以安全切分（组层按格建全帧层，合成语义逐像素可分离）；投影、
  图层/分级/背景模糊、颗粒与纹理读格外的像素或按设备坐标锚定图案，整帧登记 `tiled-effect` 回退
  CPU，避免接缝与图案漂移。对照测试用 64px 格切 200px 文档，与不切分 GPU 结果逐字节一致。
- 取消令牌（`AtomicBool`）与导出进度回调（**未做**）；取消后释放纹理。
- **验收**：同一文档 GPU 导出与 CPU 导出 diff 达标（椭圆帧已加对照）；分块与整帧逐字节一致
  （已加两组对照：可切分文档、含投影拒绝切分）；取消在 1 帧内生效、4K/8K 不 OOM（**未做**）。

### M7 一致性

- 每个特性一组「金样文档 + 三种缩放 + 两种背景」的 diff 用例（第 6 节），CI 无适配器时跳过并标记
  `SKIP: no adapter`，不算通过也不算失败。
- 新增特性必须同时提交：`doc_scene` 支持判定、WGSL 实现、diff 用例、Unsupported 回退用例。
- **验收**：`cargo test -p figcraft-gpu --features wgpu` 全绿；`--no-default-features` 下纯逻辑测试全绿。

## 5. 跨平台

| 平台 | 后端 | 说明 |
|---|---|---|
| Windows | DX12（默认）/ Vulkan | WARP 视为软件适配器 → `auto` 不用；`gpu` 强制时仍可跑 |
| Linux | Vulkan（默认）/ GL | lavapipe 视为软件；远程桌面（llvmpipe）同理 |
| iOS | Metal | 无表面设备（离屏纹理）；`MTLCommandQueue` 单线程提交，回读用 shared buffer |
| macOS | Metal | 同 iOS |
| wasm | WebGPU / WebGL2 | **本阶段不支持**：WebGL2 无 storage buffer，`figcraft-gpu` 不参与 web 构建 |

设备请求：`Limits::downlevel_defaults().using_resolution(adapter.limits())` 起步，额外要求
`max_storage_buffers_per_shader_stage >= 4`（实例缓冲在片元里读）。不满足 → 视为不可用（CPU）。

## 6. 一致性回归（diff 口径）

指标（预乘 RGBA8，逐通道）：

- `max_delta`：全图最大通道差；
- `mean_delta`：全图平均通道差；
- `bad_ratio`：通道差 > `per_channel` 的像素占比；
- 分级容差：
  - **实心内部**（离边缘 ≥ 2px）：`per_channel = 2`，`bad_ratio = 0`；
  - **边缘带**（SDF 覆盖率 0–1 的 1–2px 环）：`per_channel = 24`，`bad_ratio ≤ 2%`
    （tiny-skia 解析 AA 与 SDF `fwidth` AA 的天然差异）；
  - **渐变内部**：`per_channel = 6`（插值空间差；见风险 1）。

边缘带用「CPU 结果的 alpha 梯度」自动圈定，不手写坐标。金样存 `crates/gpu/tests/golden/`（PNG，
仅存 CPU 基准，GPU 结果现算，避免二进制随平台漂移）。

网格（路径/描边）另用 `diff::compare_interiors`：在**任一**图里相邻通道差 > 8 的像素两侧各扩 1px
算边带并跳过，其余按 `per_channel = 2`、`bad_ratio = 0` 判——MSAA 4 档覆盖率与解析覆盖率的差别
只允许存在于边带。

## 7. 导出

- 回读：`copy_texture_to_buffer` + `map_async` + `device.poll(Wait)`；行对齐按
  `COPY_BYTES_PER_ROW_ALIGNMENT`（256）补列后裁掉。
- 编码：PNG/JPEG/WebP 走 `crates/render` 既有 `encode_export`；视频帧走 `motion-codec`
  （`craft-video` h264-encode 已是依赖）。
- 进度/取消：`ExportJob { done: AtomicU32, total: u32, cancel: AtomicBool }`；每 band 检查一次，
  取消后 `cancel` 返回 `Err(Cancelled)`，已用纹理立即 drop。

## 8. 偏好设置

| 项 | 值 |
|---|---|
| 键 | `Prefs.render_backend`（`"auto"` 默认 / `"gpu"` / `"cpu"`；手改文件容错为 `auto`） |
| 命令 | `pref.renderBackend {backend}`（`auto` / `gpu` / `cpu`） |
| UI | Preferences ▸ **Rendering**：Auto / GPU / CPU 三选一；下方灰字显示**当前实际生效**的后端与原因（如「GPU：NVIDIA RTX 4070（DX12）」/「CPU：没有可用的 GPU 适配器」） |
| 生效 | 立即（下一帧），无需重启；已缓存的设备保留，切到 `cpu` 只是不再调用 |
| 与 `graphics_backend` 的区别 | 后者选 **窗口** 的图形 API（dx12/vulkan/gl/metal），前者选 **画布** 是否用 GPU 出图 |
| 环境变量 | `FIGCRAFT_RENDER=cpu` 覆盖偏好（排查用，优先级最高） |
| 默认 | `auto` |

接线（本分支已做）：`Prefs.render_backend` 在 `pref.renderBackend` 与 `Session::apply_view_prefs`
里同时写入 `figcraft_render::set_render_backend()`；`figcraft_gpu::prefs::requested()` 依次看
`FIGCRAFT_RENDER` 环境变量 → 进程内 `set()` → 这个镜像值（所以 GPU crate 不必依赖 engine）。
`figcraft-ui-gpui` 的 `canvas/render_worker::run_gpu` 每帧问一次 `render_if_gpu`，返回 `None`
就整帧让回 tiny-skia——偏好只决定「先试哪边」，决定不了「画不出来还硬画」。

## 9. 分支与构建

- 分支 `ai/gpu-pipeline` 从 `main`（`c3bf6c1c`）拉出，worktree 在统一分支根
  `D:\Projektdateien\GPUI\branches\gpu-pipeline`。
- 按 `docs/local-branch-policy.md`，功能分支默认不构建；**本分支经 owner 明确授权**允许在
  worktree 内自行 `cargo check/test`（等价于一个普通 PR 分支的自测），因此本分支的构建不违反
  「主分支唯一构建」的初衷，但产物仍落 `A:\Temp\monet-target`，不新建 `target/`。
- 合入前照常：`Backup-Refs.ps1` → `Merge-ToMain.ps1`（`--no-ff`）。合并后才允许启动测试。

## 10. 风险与开放问题

1. **颜色空间与插值**：tiny-skia 的渐变在预乘 sRGB 上插值，GPU 侧按 sRGB 加权；两端在中间色上
   可能差 2–6 级。M1 先用容差覆盖，M7 前决定是否统一到线性空间（会影响 CPU 既有输出，需单独立项）。
2. **AA 口径**：解析覆盖率 vs SDF `fwidth` 在 1px 边缘的差异不可消除，只能定容差与边缘带。
3. **非均匀缩放下的圆角**：SDF 在局部空间求值，非均匀缩放会把圆角拉成椭圆（与 CPU 真实几何不同）。
   M2 用三角化后不再有该问题；M1 遇到 `scale` 工具的非均匀缩放登记 Unsupported。
4. **iOS 线程模型**：Metal 的 `MTLCommandQueue` 不适合多线程并发提交；导出 worker 需串行化
   （`Mutex<Renderer>`，与现有 `FRAME_GPU` 同策略）。
5. **性能基线**：M1 只保证正确性；实例化 + storage buffer 的吞吐在 10k 图元级别实测后再定
   「何时该走 GPU」的阈值（现在 `auto` 只要有硬件就走 GPU，可能在小画布上不如 CPU）。
6. **stencil 与 Metal**：Metal 支持 stencil，但 wgpu 的 `depth_stencil` 需要显式格式；
   M5 的蒙版要确认 `Stencil8` 在 iOS 可用。
