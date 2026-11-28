// Monet — M2 mesh pipeline.
//
// One draw per tessellated path: the vertices are already in **device pixels** (the document and view
// transforms are baked in when the scene is built), and the colour is premultiplied. Coverage comes
// from the 4× multisampled target, so this shader writes a flat colour and lets the samples be the
// antialiasing — the same job the SDF's `fwidth` coverage does on the other pipeline.
//
// A per-vertex clip rectangle discards fragments outside the ancestor clips (vertex-flat, because it
// is the same for the whole path).

struct Globals {
  size: vec2<f32>,
  pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) color: vec4<f32>,
  @location(1) @interpolate(flat) clip: vec4<f32>,
  @location(2) @interpolate(flat) clip_radii: vec4<f32>,
};

@vertex
fn vs(
  @location(0) p: vec2<f32>,
  @location(1) color: vec4<f32>,
  @location(2) clip: vec4<f32>,
  @location(3) clip_radii: vec4<f32>,
) -> VsOut {
  var o: VsOut;
  o.pos = vec4<f32>(p / globals.size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
  o.color = color;
  o.clip = clip;
  o.clip_radii = clip_radii;
  return o;
}

// Coverage of the ancestor clip at `pos`; see the shape shader for the rounded-corner SDF.
fn clip_mask(pos: vec2<f32>, clip: vec4<f32>, radii: vec4<f32>) -> f32 {
  if (clip.z < 0.0) {
    return 1.0;
  }
  if (clip.z == 0.0 || clip.w == 0.0) {
    return 0.0;
  }
  let rounded = radii.x > 0.0 || radii.y > 0.0 || radii.z > 0.0 || radii.w > 0.0;
  if (!rounded) {
    if (pos.x < clip.x || pos.y < clip.y || pos.x >= clip.x + clip.z || pos.y >= clip.y + clip.w) {
      return 0.0;
    }
    return 1.0;
  }
  let half = clip.zw * 0.5;
  let p = pos - (clip.xy + half);
  var r = radii.x;
  if (p.x >= 0.0) {
    if (p.y < 0.0) { r = radii.y; } else { r = radii.z; }
  } else if (p.y >= 0.0) {
    r = radii.w;
  }
  let q = abs(p) - half + vec2<f32>(r);
  let sd = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
  let w = max(fwidth(sd), 1e-5);
  return clamp(0.5 - sd / w, 0.0, 1.0);
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let clip_a = clip_mask(in.pos.xy, in.clip, in.clip_radii);
  if (clip_a <= 0.0) {
    discard;
  }
  if (in.color.a <= 0.0) {
    discard;
  }
  return in.color * clip_a;
}
