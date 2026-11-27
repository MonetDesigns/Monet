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
};

@vertex
fn vs(
  @location(0) p: vec2<f32>,
  @location(1) color: vec4<f32>,
  @location(2) clip: vec4<f32>,
) -> VsOut {
  var o: VsOut;
  o.pos = vec4<f32>(p / globals.size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
  o.color = color;
  o.clip = clip;
  return o;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  if (in.clip.z > 0.0) {
    if (in.pos.x < in.clip.x || in.pos.y < in.clip.y ||
        in.pos.x >= in.clip.x + in.clip.z || in.pos.y >= in.clip.y + in.clip.w) {
      discard;
    }
  }
  if (in.color.a <= 0.0) {
    discard;
  }
  return in.color;
}
