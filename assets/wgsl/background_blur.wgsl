// FigCraft — 背景模糊（外置 WGSL）。这个文件在**当前** contract 下就能直接跑：
// 只读 backdrop + fields，输出画在内容之下，正好是 `MASK_BACKGROUND_BLUR_TILE` 的位置。
//
// 对照 Figma：`SHADER_SOURCE_MASK_BACKGROUND_BLUR_TILE_FRAGMENT`
//   if (shape.a == 0.0) discard;  return blurredBackdrop;
// 模糊本身走 `@fx blur backdrop u.blur`：宿主先做 Figma 的可分离 box（水平、垂直各一遍，
// 边缘 clamp），再把糊好的 backdrop 绑进来。这个片元只负责 mask，不再自己做 mip 近似。

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
// @fx param u.blur float 1 100 1 4 "Blur"
// @fx reach u.blur
// @fx blur backdrop u.blur

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
    let raw = textureSampleLevel(backdrop, samp, texel_uv(in.uv), 0.0);
    // backdrop 是预乘的；先还原成直通，否则透明边缘的颜色会被 alpha 再乘一次，四边发黑。
    let c = vec4<f32>(raw.rgb / max(raw.a, 1e-5), raw.a);
    // 直通 alpha：runner 用 ALPHA_BLENDING 盖在原来的底图上。
    return vec4<f32>(c.rgb, coverage);
}
