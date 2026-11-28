// Monet — draw a blurred shadow texture as one tinted, premultiplied quad.
//
// The shadow texture holds the blurred silhouette's alpha (white, premultiplied). The tint's alpha
// is the shadow's strength; the result is premultiplied so it composites under the node exactly like
// the CPU's tinted shadow pixmap.

struct Globals {
  size: vec2<f32>,
  pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Tint {
  color: vec4<f32>,
  // Device-pixel clip [x, y, w, h]; w < 0 means no clip, w == 0 clips everything.
  clip: vec4<f32>,
  // Unused here (the cover variant uses the inner shader); kept so the two tint buffers agree.
  mode: vec4<f32>,
};
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var<uniform> tint: Tint;

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
  let c = tint.clip;
  if (c.z >= 0.0) {
    if (in.pos.x < c.x || in.pos.y < c.y || in.pos.x >= c.x + c.z || in.pos.y >= c.y + c.w) {
      discard;
    }
  }
  let s = textureSample(tex, samp, in.uv);
  let a = s.a * tint.color.a;
  if (a <= 0.0) {
    discard;
  }
  return vec4<f32>(tint.color.r * a, tint.color.g * a, tint.color.b * a, a);
}
