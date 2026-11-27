// FigCraft — 画布网格 / 画板布局网格（外置 WGSL，覆盖层 pass）。
//
// 这不是「材质」，是**覆盖层**：由编辑器在自己的 overlay pass 里直接 composite_steps 驱动
// （不进 layers，也不绑节点），所以没有 content/backdrop 依赖，只读 u。
//
// 两种网格，分别对应 Figma 的两套东西：
//
//   ① mode 0 —— 像素网格（Figma 叫 render grid / "Pixel grid"，View → Pixel grid，
//      ≥ 约 500% 才出现）。1 设备像素的线，间隔 1 文档单位，颜色是主题里的一档低透明度墨色。
//      我们现在的实现是 `overlays.rs` 里 zoom > 5.07 时每文档单位画 1px 的
//      `Color32::from_black_alpha(18)`（≈7%）。Figma 的常量在 wasm 的 C++ 里，字符串表读不到，
//      所以这里把它做成 uniform：亮色 0,0,0,0.07，暗色 1,1,1,0.07。
//
//   ② mode 1/2/3 —— 布局网格（Layout grid，Figma 的 frame 属性）。模型直接来自 JS 包：
//        { offset: 0, sectionSize, color: {r:1,g:0,b:0,a:.1}, pattern: "GRID", gutterSize: 20 }
//      pattern = GRID（方格线）或 STRIPES（axis X = 列 / Y = 行，alignment
//      MIN/CENTER/STRETCH/MAX）。默认就是 **纯红 10%**、gutter 20 —— 我们 doc 里的
//      `LayoutGrid::default()` 也是 (1,0,0,0.1)，对得上。
//      画法：GRID 画 1 设备像素的线（不是 1 文档像素），STRIPES 画**色带**（不是线）；
//      两者都裁在画板矩形内、不随图层缩放变粗。
//
// 线宽单位说明：`u.line` 是文档 px，0 表示「1 设备像素发丝线」（Figma 的网格线是发丝线，
// 缩放时恒为 1px）。色带边缘按 1 设备像素做 AA。

struct Uniforms {
    resolution     : vec2<f32>,
    regionOriginPx : vec2<f32>,
    nodeSizePx     : vec2<f32>,
    mode           : f32,   // 0 像素网格，1 GRID，2 列，3 行
    cell           : f32,   // 文档 px，sectionSize
    gutter         : f32,
    offset         : f32,
    count          : f32,   // STRIPES 的段数
    align          : f32,   // 0 MIN 1 CENTER 2 STRETCH 3 MAX
    line           : f32,   // 文档 px，0 = 1 设备像素
    color_r        : f32,
    color_g        : f32,
    color_b        : f32,
    color_a        : f32,
    origin_x       : f32,   // 节点左上角在页面里的文档坐标（像素网格的锚点）
    origin_y       : f32,
    dpr            : f32,
};

@group(0) @binding(0) var<uniform> u : Uniforms;

// @fx id grid
// @fx label Canvas Grid
// @fx param u.mode float 0 3 1 1 "Mode (0 pixel 1 grid 2 cols 3 rows)"
// @fx param u.cell float 1 2000 1 8 "Cell"
// @fx param u.gutter float 0 2000 1 0 "Gutter"
// @fx param u.offset float -2000 2000 1 0 "Offset"
// @fx param u.count float 0 200 1 12 "Count"
// @fx param u.align float 0 3 1 0 "Align (0 min 1 center 2 stretch 3 max)"
// @fx param u.line float 0 10 0.5 0 "Line Width (0 = hairline)"
// @fx param u.color_r float 0 1 0.01 0 "Color R"
// @fx param u.color_g float 0 1 0.01 0 "Color G"
// @fx param u.color_b float 0 1 0.01 0 "Color B"
// @fx param u.color_a float 0 1 0.01 0.1 "Color A"
// @fx param u.origin_x float -100000 100000 1 0 "Origin X"
// @fx param u.origin_y float -100000 100000 1 0 "Origin Y"

struct VSOut {
    @builtin(position) pos : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi : u32) -> VSOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var out : VSOut;
    let xy = corners[vi];
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

/// 到最近一条线的距离（设备像素），线在 `period` 的整数倍处。
fn line_coverage(pos_px : f32, period_px : f32, half_px : f32) -> f32 {
    let m = pos_px - floor(pos_px / period_px + 0.5) * period_px;
    return clamp(half_px + 0.5 - abs(m), 0.0, 1.0);
}

/// 色带：起点 start、宽 w、周期 pitch，取 [0,count) 段。
fn band_coverage(pos_px : f32, start_px : f32, w_px : f32, pitch_px : f32, count : f32) -> f32 {
    let k = floor((pos_px - start_px) / pitch_px);
    if (k < 0.0 || k >= count) { return 0.0; }
    let l = pos_px - start_px - k * pitch_px;
    return clamp(min(l, w_px - l) + 0.5, 0.0, 1.0);
}

/// 正数取模（WGSL 的 `%` 跟被除数同号，负坐标会算出负值）。
fn wrap(x : f32, p : f32) -> f32 {
    let m = x % p;
    return select(m, m + p, m < 0.0);
}

/// cell 局部坐标里到最近一条边（左/右）的距离；落进 cell 右边外的 gutter 就是超出量。
fn edge_dist(l : f32, w : f32) -> f32 {
    if (l <= w) {
        return min(l, w - l);
    }
    return l - w;
}

@fragment
fn fs(in : VSOut) -> @location(0) vec4<f32> {
    let frag = u.regionOriginPx + in.uv * max(u.resolution, vec2<f32>(1.0, 1.0));
    let size = max(u.nodeSizePx, vec2<f32>(1.0, 1.0));
    let scale = max(u.dpr, 1e-4);
    let ink = vec4<f32>(u.color_r, u.color_g, u.color_b, u.color_a);

    let mode = i32(round(u.mode));
    var cov = 0.0;

    if (mode == 0) {
        // 像素网格：每 1 文档单位一条，线宽 1 设备像素；两条轴上的线取并集。
        let d = frag + vec2<f32>(u.origin_x, u.origin_y) * scale;
        let cx = line_coverage(d.x, scale, 0.5);
        let cy = line_coverage(d.y, scale, 0.5);
        cov = max(cx, cy);
    } else {
        // 栅格化的网格只画在画板矩形内。
        if (any(frag < vec2<f32>(0.0)) || any(frag > size)) { return vec4<f32>(0.0); }
        let half = max(u.line * scale * 0.5, 0.5);
        if (mode == 1) {
            // GRID：每个 cell 是四边描线的方块，gutter 是空档（gutter=0 就退化成整片网格）。
            // 线在 cell 的两条边上，所以要在 cell 局部坐标里算「到最近的边」。
            let pitch = max((u.cell + u.gutter) * scale, 0.5);
            let cw = min(u.cell * scale, pitch);
            let start = u.offset * scale;
            let lx = wrap(frag.x - start, pitch);
            let ly = wrap(frag.y - start, pitch);
            let cx = clamp(half + 0.5 - edge_dist(lx, cw), 0.0, 1.0);
            let cy = clamp(half + 0.5 - edge_dist(ly, cw), 0.0, 1.0);
            let in_x = select(0.0, 1.0, lx <= cw);
            let in_y = select(0.0, 1.0, ly <= cw);
            // 竖边只画在 cell 高度内，横边只画在 cell 宽度内 → 拐角处不互相越界。
            cov = max(cx * in_y, cy * in_x);
        } else {
            // STRIPES：色带。
            let axis = select(frag.y, frag.x, mode == 2);
            let extent = select(size.y, size.x, mode == 2);
            let n = max(u.count, 1.0);
            var w = u.cell * scale;
            if (i32(round(u.align)) == 2) {
                w = max((extent - u.gutter * scale * (n - 1.0)) / n, 0.0);
            }
            let total = n * w + u.gutter * scale * (n - 1.0);
            var start = u.offset * scale;
            let a = i32(round(u.align));
            if (a == 1) {
                start = (extent - total) * 0.5 + u.offset * scale;
            } else if (a == 3) {
                start = extent - total + u.offset * scale;
            }
            let pitch = max(w + u.gutter * scale, 0.5);
            cov = band_coverage(axis, start, w, pitch, n);
        }
    }

    return vec4<f32>(ink.rgb, ink.a * clamp(cov, 0.0, 1.0));
}
