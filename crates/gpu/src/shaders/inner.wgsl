// Monet — draw the blurred silhouette of a shape as an inner shadow, or as a drop shadow that
// must not show behind its caster.
//
// For an inner shadow the CPU builds the shape at the shadow's offset in a padded buffer, inverts
// its alpha, blurs it, crops it back and multiplies it by the shape's own coverage before tinting.
// The blur chain here already inverted the first pass; this pass multiplies the blurred alpha by
// the shape's coverage (the second texture, drawn at zero offset) and applies the tint, premultiplied.
// For a drop shadow with `show_behind == false`, `mode` selects `1 - coverage` instead.

struct Globals {
  size: vec2<f32>,
  pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Tint {
  color: vec4<f32>,
  // Device-pixel clip [x, y, w, h]; w < 0 means no clip, w == 0 clips everything.
  clip: vec4<f32>,
  // x = 0: multiply by the caster coverage (inner shadow). x = 1: multiply by 1 - coverage
  // (a drop shadow that does not show behind its caster).
  mode: vec4<f32>,
};
@group(1) @binding(0) var blurred: texture_2d<f32>;
@group(1) @binding(1) var cover: texture_2d<f32>;
@group(1) @binding(2) var samp: sampler;
@group(1) @binding(3) var<uniform> tint: Tint;

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
  let b = textureSample(blurred, samp, in.uv);
  let cover = textureSample(cover, samp, in.uv);
  let keep = select(cover.a, 1.0 - cover.a, tint.mode.x > 0.5);
  let a = b.a * keep * tint.color.a;
  if (a <= 0.0) {
    discard;
  }
  return vec4<f32>(tint.color.r * a, tint.color.g * a, tint.color.b * a, a);
}
