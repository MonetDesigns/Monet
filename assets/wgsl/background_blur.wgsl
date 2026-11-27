// FigCraft — 背景模糊（外置 WGSL）。这个文件在**当前** contract 下就能直接跑：
// 只读 backdrop + fields，输出画在内容之下，正好是 `MASK_BACKGROUND_BLUR_TILE` 的位置。
//
// 对照 Figma：`SHADER_SOURCE_MASK_BACKGROUND_BLUR_TILE_FRAGMENT`
//   if (shape.a == 0.0) discard;  return blurredBackdrop;
// 模糊本身在另一条链上（downsample → clamped box taps），这里换成 backdrop 的 mip 链：
// 2×2 box 降采样 + 线性上采样，一次 `textureSampleLevel` 完成（glass 的 Frost 用的同一招）。
// Figma 的 mask 片元**不抖动**；这里也不抖，保持 1:1。若平色区出现色带，在返回前加
// glass.wgsl 里的 ditherize(·, frag_px) 即可（2/255，同一个 hash）。
//
// 半径语义：Figma 的 blurRadius 是 box 半宽；本项目 frost 的约定是 level = log2(radius px)。
// 要按 Figma 的几何走，把 BLUR_SEMANTICS 填 2.0。

struct Uniforms {
    resolution     : vec2<f32>,
    regionOriginPx : vec2<f32>,
    nodeSizePx     : vec2<f32>,
    blur           : f32,   // 文档 px
    dpr            : f32,
};

@group(0) @binding(0) var backdrop : texture_2d<f32>;
@group(0) @binding(1) var samp     : sampler;
@group(0) @binding(2) var fields   : texture_2d<f32>;
@group(0) @binding(3) var<uniform> u : Uniforms;

// @fx id background_blur
// @fx label Background Blur
// @fx param u.blur float 0 200 1 12 "Blur"

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
    let coverage = textureSampleLevel(fields, samp, in.uv, 0.0).a;
    if (coverage <= 0.001) {
        discard;
    }
    let radius_px = max(u.blur, 0.0) * u.dpr;
    let max_lod = f32(max(textureNumLevels(backdrop), 1u) - 1u);
    let lod = clamp(log2(max(radius_px * BLUR_SEMANTICS, 1.0)), 0.0, max_lod);
    let c = textureSampleLevel(backdrop, samp, texel_uv(in.uv), lod);
    // 直通 alpha：runner 用 ALPHA_BLENDING 盖在原来的底图上。
    return vec4<f32>(c.rgb, coverage);
}
