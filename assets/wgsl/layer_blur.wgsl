// FigCraft — 图层模糊（外置 WGSL，contract v2）。
//
// 对照 Figma：layer blur 没有单独的片元——图层先渲进 tile，再走
// downsample → clamped box blur → composite（`SHADER_SOURCE_DOWNSAMPLE_TILE_FRAGMENT` +
// `SHADER_SOURCE_CLAMPED_BLUR_TILE_NODESPACE_FRAGMENT`，一次 64 tap、可分离、带 clamp 权重）。
//
// 我们这边不等价地重跑 64 tap：backdrop 已经带整条 mip 链，content 也要求带链，
// 2×2 box 降采样 + 采样器线性上采样 = Figma 那条 downsample→upsample 链，
// 一次 `textureSampleLevel` 就是一个半径 2^L 的模糊（和 glass 的 Frost 同一套）。
// 半径 → 级：level = log2(radius px)。要贴 Figma 的「box 半宽 = blurRadius」语义，
// 把 BLUR_SEMANTICS 改成 2.0（下面留了常数）。
//
// 接线（见 docs/figma-canvas-engine-wgsl.md）：
//   content : 本层内容，**预乘 alpha**，含 mip 链，覆盖外扩后的区域；
//   rect    : 四边各外扩 reach = blur + 4px；
//   合成    : straight alpha 输出整层；子树已烤进 content，不要再叠 `Step::Over`。

struct Uniforms {
    resolution     : vec2<f32>,
    regionOriginPx : vec2<f32>,
    nodeSizePx     : vec2<f32>,
    blur           : f32,   // 文档 px
    dpr            : f32,
};

@group(0) @binding(0) var content : texture_2d<f32>;
@group(0) @binding(1) var samp    : sampler;
@group(0) @binding(2) var<uniform> u : Uniforms;

// @fx id layer_blur
// @fx label Layer Blur
// @fx stage layer
// @fx reach u.blur
// @fx param u.blur float 0 200 1 12 "Blur"

/// 本项目 frost 的约定：半径 2^L 设备像素取 level L。
/// Figma 的 box 半宽等于 blurRadius，要按那个语义填 2.0。
const BLUR_SEMANTICS : f32 = 1.0;

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

@fragment
fn fs(in : VSOut) -> @location(0) vec4<f32> {
    let radius_px = max(u.blur, 0.0) * u.dpr;
    let max_lod = f32(max(textureNumLevels(content), 1u) - 1u);
    let lod = clamp(log2(max(radius_px * BLUR_SEMANTICS, 1.0)), 0.0, max_lod);
    // content 是预乘的：模糊可以直接在预乘空间做（透明区不会把颜色拉黑）。
    let p = textureSampleLevel(content, samp, texel_uv(in.uv), lod);
    // 还原成直通 alpha 交回 runner。
    return vec4<f32>(p.rgb / max(p.a, 1e-5), p.a);
}
