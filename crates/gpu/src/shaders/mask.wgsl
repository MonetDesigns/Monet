// Monet — composite a mask run: content sampled through a mask layer.
//
// Both sides were drawn into transparent, full-frame layers, like the CPU's `draw_masked`:
//   * mask alpha     -> the mask layer's alpha byte
//   * mask luminance -> ceil((0.2126*r + 0.7152*g + 0.0722*b) * 255) / 255
// The CPU's tiny-skia `Mask::from_pixmap(Luminance)` demultiplies its 8-bit pixels, applies that
// formula and re-multiplies by alpha, which is exactly the premultiplied RGB dot product; it then
// ceils to a mask byte. `textureLoad` keeps the lookup pixel-exact (no filtering).

struct Globals {
  size: vec2<f32>,
  pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Params {
  // x = 1 for luminance, 0 for alpha.
  kind: vec4<f32>,
};
@group(1) @binding(0) var content: texture_2d<f32>;
@group(1) @binding(1) var mask: texture_2d<f32>;
@group(1) @binding(2) var<uniform> params: Params;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
  var pts = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  let p = pts[vi];
  var o: VsOut;
  o.pos = vec4<f32>(p, 0.0, 1.0);
  return o;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let p = vec2<i32>(floor(in.pos.xy));
  let c = textureLoad(content, p, 0);
  let m = textureLoad(mask, p, 0);
  var coverage = m.a;
  if (params.kind.x > 0.5) {
    let luma = dot(m.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    coverage = ceil(luma * 255.0) / 255.0;
  }
  return c * coverage;
}
