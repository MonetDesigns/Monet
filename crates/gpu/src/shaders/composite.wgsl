// Monet — composite one resolved group layer into its parent.
//
// The group's children were drawn into a transparent, full-frame layer. This full-screen triangle
// samples that layer and multiplies all four premultiplied channels by the group's alpha; the pass
// blend then stacks it in painter's order exactly once, which is the CPU's `save_layer` semantics.

struct Globals {
  size: vec2<f32>,
  pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Params {
  opacity: vec4<f32>,
};
@group(1) @binding(0) var src: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var<uniform> params: Params;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
  var pts = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  let p = pts[vi];
  var o: VsOut;
  o.pos = vec4<f32>(p, 0.0, 1.0);
  o.uv = vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
  return o;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let s = textureSample(src, samp, in.uv);
  return s * params.opacity.x;
}
