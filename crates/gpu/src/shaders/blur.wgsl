// Monet — one separable box-blur pass, matching `figcraft_render::fx::box_blur_row` byte for byte.
//
// The CPU blurs a premultiplied RGBA8 pixmap with three box passes along the rows, transposes it and
// does three more down the columns. Each pass sums the window (zero outside the pixmap), divides
// with a floor and writes the byte back. This pass reproduces that on integer bytes, so a shadow's
// silhouette blurred here is the same image the CPU's blur produces.

struct Params {
  // Window radius in pixels; 0 copies the source.
  radius: i32,
  // 1 = along x (rows), 0 = along y (columns).
  horizontal: i32,
  pad0: i32,
  pad1: i32,
};
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
  // One oversized triangle covers the whole target.
  var pts = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  var o: VsOut;
  o.pos = vec4<f32>(pts[vi], 0.0, 1.0);
  return o;
}

fn byte_of(v: f32) -> u32 {
  return u32(round(clamp(v, 0.0, 1.0) * 255.0));
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let dims = vec2<i32>(textureDimensions(src, 0));
  let p = vec2<i32>(i32(floor(in.pos.x)), i32(floor(in.pos.y)));
  let r = max(params.radius, 0);
  var sum = vec4<u32>(0u, 0u, 0u, 0u);
  for (var i = -r; i <= r; i = i + 1) {
    var q = p;
    if (params.horizontal != 0) {
      q.x = q.x + i;
    } else {
      q.y = q.y + i;
    }
    if (q.x < 0 || q.y < 0 || q.x >= dims.x || q.y >= dims.y) {
      continue;
    }
    let v = textureLoad(src, q, 0);
    sum = sum + vec4<u32>(byte_of(v.r), byte_of(v.g), byte_of(v.b), byte_of(v.a));
  }
  let div = f32(2 * r + 1);
  let out = vec4<f32>(f32(sum.x), f32(sum.y), f32(sum.z), f32(sum.w)) / div / 255.0;
  return out;
}
