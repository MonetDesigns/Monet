// FigCraft — 图层模糊（外置 WGSL，contract v2）。
//
// 对照 Figma：layer blur 没有单独的片元——图层先渲进 tile，再走
// downsample → clamped box blur → composite（`SHADER_SOURCE_DOWNSAMPLE_TILE_FRAGMENT` +
// `SHADER_SOURCE_CLAMPED_BLUR_TILE_NODESPACE_FRAGMENT`，一次最多 63 tap、可分离、带 clamp）。
//
// 模糊由宿主统一做：`@fx blur content u.blur` 让 runner 对 content 做严格的水平 + 垂直
// box（边缘 clamp），再把结果作为 content 绑进来。这个片元只负责把预乘 content 还原成直通
// alpha 交回 runner，不再自己采样 mip。
//
// 接线（见 docs/figma-canvas-engine-wgsl.md）：
//   content : 本层内容，**预乘 alpha**，覆盖外扩后的区域；
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
// @fx blur content u.blur
// @fx param u.blur float 1 100 1 4 "Blur"

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
    // content 已是宿主 box 模糊后的预乘结果；还原成直通 alpha 交回 runner。
    let p = textureSampleLevel(content, samp, texel_uv(in.uv), 0.0);
    return vec4<f32>(p.rgb / max(p.a, 1e-5), p.a);
}
