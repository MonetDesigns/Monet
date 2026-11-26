// FigCraft — 投影 / 内阴影（外置 WGSL，contract v2「整层效果」通道）。
//
// 对照图：Figma 画布引擎 `compiled_wasm.wasm`（126.11.11 构建）里的三条阴影路径。
//
//   ① ROUNDED_RECTANGLE_BLUR / RECTANGLE_BLUR —— 圆角矩形的**解析**阴影，一遍、不读纹理。
//      顶点把 quad 四边各外扩 `blurRadius`；片元拿到 (frag - shadowCenter)/blurRadius。
//      `i(x)` 是三角核的累积分布（-1..0 是 0.5(x+1)^2，0..1 是 0.5-0.5x(x-2)），
//      两条边的 i 相减就是 1D 覆盖；圆角用 4 采样的角部积分 `aa()` 近似。
//      也就是说 Figma 的矩形阴影核 = **两次 box（三角核）**，不是高斯，也不是我们 CPU
//      路径的 3-box。`e()` 在最后给 2/255 的抖动（shadow 也抖，不只是渐变）。
//
//   ② MASK_KNOCKOUT_SHADOW_TILE —— 非矩形：mask 走 clamp 感知的 blur 链，
//      `mix(blurred*binary(alpha==0), ...)`；`useBinaryKnockoutMask` 走二值挖空。
//   ③ MASK_INNER_SHADOW_TILE —— `(1 - blurredMask.a) * shape.a * color`。
//
// 本文件：analytic=1 走 ①（矩形/圆角矩形 1:1，也是 Figma 自己给 frame 用的那条），
// analytic=0 走 ②③（形状来自 `content.a`，文本/矢量/椭圆用这条）。
//
// 接线（当前 runner 还差这一步，见 docs/figma-canvas-engine-wgsl.md）：
//   content : 本层内容，**预乘 alpha**，含 mip 链，覆盖外扩后的整块区域；
//   fields  : 形状 SDF（R 距离 / GB 法线 / A 覆盖），和 glass 同一张；
//   rect    : 比节点四边各外扩 reach = max(|offset|, blur, spread) + 4px；
//   合成    : 输出整层的 straight alpha；子树已烤进 content，所以**不要**再叠一层 `Step::Over`。
//             pass 的 target 以画布现有像素为底、走 ALPHA_BLENDING，除一次 over 就是对的。

struct Uniforms {
    resolution     : vec2<f32>,
    regionOriginPx : vec2<f32>,
    nodeSizePx     : vec2<f32>,
    cornerRadii    : vec4<f32>,   // 文档单位 [tl, tr, br, bl]（shade_gpu 就是这么打的）
    kind           : f32,         // 0 = 投影，1 = 内阴影
    dx             : f32,         // 文档 px
    dy             : f32,
    blur           : f32,         // 文档 px —— Figma 的 blurRadius，三角核半宽
    spread         : f32,         // 文档 px —— 外扩 shadowBounds，不改 blur
    color_r        : f32,
    color_g        : f32,
    color_b        : f32,
    color_a        : f32,
    knockout       : f32,         // 投影：1 = 被本层遮住的部分不画（Figma 默认）
    analytic       : f32,         // 1 = 圆角矩形解析路径
    dpr            : f32,
};

@group(0) @binding(0) var content : texture_2d<f32>;
@group(0) @binding(1) var fields  : texture_2d<f32>;
@group(0) @binding(2) var samp    : sampler;
@group(0) @binding(3) var<uniform> u : Uniforms;

// @fx id shadow
// @fx label Shadow
// @fx stage layer
// @fx reach u.blur
// NOTE: @fx param 的 token 之间只能有一个空格（parse_param 先 splitn 再过滤空串）。
// @fx param u.kind float 0 1 1 0 "Kind (0 drop 1 inner)"
// @fx param u.dx float -500 500 1 0 "Offset X"
// @fx param u.dy float -500 500 1 0 "Offset Y"
// @fx param u.blur float 0 200 1 8 "Blur"
// @fx param u.spread float -200 200 1 0 "Spread"
// @fx param u.color_r float 0 1 0.01 0 "Color R"
// @fx param u.color_g float 0 1 0.01 0 "Color G"
// @fx param u.color_b float 0 1 0.01 0 "Color B"
// @fx param u.color_a float 0 1 0.01 0.25 "Color A"
// @fx param u.knockout float 0 1 1 1 "Knockout"
// @fx param u.analytic float 0 1 1 1 "Analytic Rect"

const DITHER: f32 = 0.007843; // 2/255 — Figma 的 e()

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

fn texel_uv(uv : vec2<f32>) -> vec2<f32> {
    let t = 0.5 / max(u.resolution, vec2<f32>(1.0, 1.0));
    return clamp(uv, t, vec2<f32>(1.0, 1.0) - t);
}

fn dither3(c : vec3<f32>, frag : vec2<f32>) -> vec3<f32> {
    let h = fract(sin(dot(frag, vec2<f32>(12.9898, 78.233))) * 43758.547);
    let o = (h - 0.5) * DITHER;
    return min(abs(c + vec3<f32>(o, o, o)), vec3<f32>(1.0, 1.0, 1.0));
}

/// Figma 的 `i(x)`：三角核（两次 box）的 CDF，x 以 blurRadius 为单位。
fn edge_cdf(x : f32) -> f32 {
    if (x <= -1.0) { return 0.0; }
    if (x < 0.0) { return 0.5 * (x + 1.0) * (x + 1.0); }
    if (x < 1.0) { return 0.5 - 0.5 * x * (x - 2.0); }
    return 1.0;
}

/// 一条轴上的覆盖：矩形 [-half, half] 在 d 处的解析积分。
fn axis_cov(d : f32, half : f32) -> f32 {
    return edge_cdf(d + half) - edge_cdf(d - half);
}

/// Figma `aa()` 的角部积分：沿垂直于主轴的 p 方向取 4 个样本，
/// 每个样本的边线位置由圆角弧给出（`u = half_o - q + sqrt(q^2 - t^2)`）。
fn corner_edge_cov(o : f32, p : f32, half_o : f32, half_p : f32, q : f32) -> f32 {
    let t = min(half_p - q - abs(p), 0.0);
    let u = half_o - q + sqrt(max(q * q - t * t, 0.0));
    return edge_cdf(o + u) - edge_cdf(o - u);
}

fn corner_cov(o : f32, p : f32, half_o : f32, half_p : f32, q : f32) -> f32 {
    let lo = clamp(p - half_p, -1.0, 1.0);
    let hi = clamp(p + half_p, -1.0, 1.0);
    if (hi <= lo) { return 0.0; }
    let step = (hi - lo) * 0.25;
    var acc = 0.0;
    var wsum = 0.0;
    var t = lo + step * 0.5;
    for (var i = 0; i < 4; i = i + 1) {
        let w = max(0.0, 1.0 - abs(t));
        acc = acc + corner_edge_cov(o, p - t, half_o, half_p, q) * w;
        wsum = wsum + w;
        t = t + step;
    }
    if (wsum <= 0.0) { return 0.0; }
    return acc / wsum * (edge_cdf(hi) - edge_cdf(lo));
}

/// 圆角矩形被三角核模糊后的覆盖；p/half/q 都以 blurRadius 为单位。
fn rect_coverage(p : vec2<f32>, half : vec2<f32>, q : f32) -> f32 {
    // Figma 的快路：离边超过一个 blurRadius + 圆角，内部就是 1。
    if (all(abs(p) < half - vec2<f32>(q + 1.0))) { return 1.0; }
    if (q < 0.01) {
        return axis_cov(p.x, half.x) * axis_cov(p.y, half.y);
    }
    if (abs(p.x) > abs(p.y)) {
        return corner_cov(p.x, p.y, half.x, half.y, q);
    }
    return corner_cov(p.y, p.x, half.y, half.x, q);
}

/// 该像素所在象限的圆角（文档单位）。cornerRadii = [tl, tr, br, bl]。
fn quadrant_corner(p : vec2<f32>) -> f32 {
    if (p.x >= 0.0) {
        return select(u.cornerRadii.y, u.cornerRadii.z, p.y >= 0.0);
    }
    return select(u.cornerRadii.x, u.cornerRadii.w, p.y >= 0.0);
}

fn unpack(p : vec4<f32>) -> vec4<f32> {
    return vec4<f32>(p.rgb / max(p.a, 1e-5), p.a);
}

/// straight-alpha 的 src over dst。
fn over(dst : vec4<f32>, src : vec4<f32>) -> vec4<f32> {
    let a = src.a + dst.a * (1.0 - src.a);
    if (a <= 1e-5) { return vec4<f32>(0.0, 0.0, 0.0, 0.0); }
    return vec4<f32>((src.rgb * src.a + dst.rgb * dst.a * (1.0 - src.a)) / a, a);
}

/// mask 路径的模糊级：半径 2^L 像素取 level L（和 glass 的 frost 同一条链）。
fn blur_lod(radius_px : f32) -> f32 {
    return log2(max(radius_px, 1.0));
}

@fragment
fn fs(in : VSOut) -> @location(0) vec4<f32> {
    let frag = u.regionOriginPx + in.uv * max(u.resolution, vec2<f32>(1.0, 1.0));
    let centre = 0.5 * u.nodeSizePx;
    let off = vec2<f32>(u.dx, u.dy) * u.dpr;

    let size_px = max(u.nodeSizePx, vec2<f32>(1.0, 1.0));
    let r_px = max(u.blur * u.dpr, 1.0);          // 三角核半宽（设备像素）
    let half_px = 0.5 * size_px + u.spread * u.dpr;

    let content_c = unpack(textureSampleLevel(content, samp, texel_uv(in.uv), 0.0));
    let cov_shape = textureSampleLevel(fields, samp, in.uv, 0.0).a;

    let tint = vec4<f32>(u.color_r, u.color_g, u.color_b, u.color_a);
    var shadow = vec4<f32>(0.0, 0.0, 0.0, 0.0);

    if (u.analytic > 0.5) {
        // ① 解析圆角矩形（Figma 的 ROUNDED_RECTANGLE_BLUR）。
        let p = (frag - centre - off) / r_px;
        let half = half_px / r_px;
        let q = max(quadrant_corner(p), 0.0) * u.dpr / r_px;
        let cov = rect_coverage(p, half, min(q, min(half.x, half.y)));
        // drop = cov × (1 - 形状)；inner = (1 - cov) × 形状。
        let alpha = select(cov * (1.0 - cov_shape * u.knockout), (1.0 - cov) * cov_shape, u.kind > 0.5);
        shadow = vec4<f32>(dither3(tint.rgb, frag), tint.a * clamp(alpha, 0.0, 1.0));
    } else {
        // ②③ mask 路径：content.a 就是形状覆盖（文本/矢量/椭圆的真实 alpha）。
        let lod = blur_lod(u.blur * u.dpr);
        let mask = textureSampleLevel(content, samp, texel_uv(in.uv - off / max(u.resolution, vec2<f32>(1.0, 1.0))), lod).a;
        if (u.kind > 0.5) {
            // 内阴影：(1 - 模糊 mask) × 形状覆盖。
            let a = max(1.0 - mask, 0.0) * cov_shape;
            shadow = vec4<f32>(dither3(tint.rgb, frag), tint.a * a);
        } else {
            let a = mask * (1.0 - cov_shape * u.knockout);
            shadow = vec4<f32>(dither3(tint.rgb, frag), tint.a * a);
        }
    }

    if (u.kind > 0.5) {
        // 内阴影画在内容之上。
        return over(content_c, shadow);
    }
    // 投影画在内容之下。
    return over(shadow, content_c);
}
