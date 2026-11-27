// Monet — progressive layer blur. Eight pre-blurred silhouettes (one per CPU level) are mixed per
// pixel by the same bracket/255 weighting the CPU uses in `figcraft_render::blur::progressive`.
//
// The ramp is measured in device pixels: the CPU's `Ramp` has its endpoints mapped through the
// layer's affine, and the dot product is translation invariant, so using the device-space
// endpoints here gives the same `t` for the same pixel centre.

struct Inst {
  m: vec4<f32>,
  off: vec4<f32>,
  size: vec4<f32>,
  radii: vec4<f32>,
  params: vec4<f32>,
  p0: vec4<f32>,
  p1: vec4<f32>,
  clip: vec4<f32>,
  clip_radii: vec4<f32>,
  stop_pos: array<f32, 8>,
  stop_col: array<vec4<f32>, 6>,
};

struct Params {
  start: vec2<f32>,
  end: vec2<f32>,
  s0: f32,
  s1: f32,
  level_count: u32,
  pad0: u32,
  levels: array<vec4<f32>, 2>,
  color: vec4<f32>,
};

@group(0) @binding(0) var t0: texture_2d<f32>;
@group(0) @binding(1) var t1: texture_2d<f32>;
@group(0) @binding(2) var t2: texture_2d<f32>;
@group(0) @binding(3) var t3: texture_2d<f32>;
@group(0) @binding(4) var t4: texture_2d<f32>;
@group(0) @binding(5) var t5: texture_2d<f32>;
@group(0) @binding(6) var t6: texture_2d<f32>;
@group(0) @binding(7) var t7: texture_2d<f32>;
@group(0) @binding(8) var<uniform> params: Params;
@group(1) @binding(0) var<storage, read> insts: array<Inst>;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) @interpolate(flat) idx: u32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
  var pts = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  var o: VsOut;
  o.pos = vec4<f32>(pts[vi], 0.0, 1.0);
  o.idx = ii;
  return o;
}

// The same distance functions as `shape.wgsl`, so the mask lands on the same edge pixels.
fn sd_rrect(p: vec2<f32>, b: vec2<f32>, r: vec4<f32>) -> f32 {
  var rr: f32;
  if (p.y < 0.0) {
    rr = select(r.x, r.y, p.x > 0.0);
  } else {
    rr = select(r.w, r.z, p.x > 0.0);
  }
  rr = min(rr, min(b.x, b.y));
  let q = abs(p) - b + vec2<f32>(rr);
  return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - rr;
}

fn sd_ellipse(p: vec2<f32>, ab: vec2<f32>) -> f32 {
  let k0 = length(p / max(ab, vec2<f32>(1e-6)));
  if (k0 < 1e-6) {
    return -min(ab.x, ab.y);
  }
  let k1 = length(p / max(ab * ab, vec2<f32>(1e-6)));
  return (k0 - 1.0) * k0 / max(k1, 1e-9);
}

fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
  let pa = p - a;
  let ba = b - a;
  let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-9), 0.0, 1.0);
  return length(pa - ba * h);
}

fn vertex_at(k: f32, count: f32, r: f32, inner: f32, star: bool) -> vec2<f32> {
  if (star) {
    let a = -1.5707963268 + k * 3.1415926536 / max(count, 1.0);
    let odd = fract(k * 0.5) > 0.25;
    let rad = select(r, r * inner, odd);
    return vec2<f32>(cos(a), sin(a)) * rad;
  }
  let a = -1.5707963268 + k * 6.2831853072 / max(count, 1.0);
  return vec2<f32>(cos(a), sin(a)) * r;
}

fn sd_regular(p: vec2<f32>, r: f32, count: f32, inner: f32, star: bool) -> f32 {
  let segments = select(count, count * 2.0, star);
  var d = 1e9;
  var inside = false;
  var prev = vertex_at(0.0, count, r, inner, star);
  for (var k: u32 = 0u; k < 64u; k = k + 1u) {
    if (f32(k) >= segments) {
      break;
    }
    let next = vertex_at(f32(k) + 1.0, count, r, inner, star);
    d = min(d, seg_dist(p, prev, next));
    if ((prev.y > p.y) != (next.y > p.y)) {
      let x = (next.x - prev.x) * (p.y - prev.y) / (next.y - prev.y) + prev.x;
      if (p.x < x) {
        inside = !inside;
      }
    }
    prev = next;
  }
  return select(d, -d, inside);
}

fn level_at(i: u32) -> f32 {
  return params.levels[i / 4u][i % 4u];
}

fn level_weight(i: u32, k: u32, f: f32) -> f32 {
  if (i == k) {
    return 1.0 - f;
  }
  if (i == k + 1u) {
    return f;
  }
  return 0.0;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let inst = insts[in.idx];

  if (inst.clip.z > 0.0) {
    if (in.pos.x < inst.clip.x || in.pos.y < inst.clip.y ||
        in.pos.x >= inst.clip.x + inst.clip.z || in.pos.y >= inst.clip.y + inst.clip.w) {
      discard;
    }
  }

  let det = inst.m.x * inst.m.w - inst.m.z * inst.m.y;
  if (abs(det) < 1e-12) {
    discard;
  }
  // No SDF mask: the pre-blurred silhouettes already carry the shape and its spill; the CPU's
  // layer blur paints that blurred pixmap, not a masked copy of it.

  let n = params.level_count;
  if (n == 0u) {
    discard;
  }
  let delta = params.end - params.start;
  let len2 = dot(delta, delta);
  var sigma = params.s1;
  if (abs(params.s1 - params.s0) >= 0.05 && len2 > 1e-12) {
    let t = clamp(dot(vec2<f32>(in.pos.x, in.pos.y) - params.start, delta) / len2, 0.0, 1.0);
    sigma = params.s0 + (params.s1 - params.s0) * t;
  }
  if (!(sigma >= 0.0)) {
    discard;
  }

  // CPU `bracket`: lower_bound(levels, sigma) - 1, clamped to n - 2; `f` rounded to 255ths.
  var lower = 0u;
  for (var i: u32 = 0u; i < n; i = i + 1u) {
    if (level_at(i) <= sigma) {
      lower = i + 1u;
    }
  }
  var k = 0u;
  if (lower > 0u) {
    k = lower - 1u;
  }
  if (n >= 2u) {
    k = min(k, n - 2u);
  }
  let lo = level_at(k);
  let hi = level_at(min(k + 1u, n - 1u));
  var f = 0.0;
  if (hi > lo) {
    f = clamp((sigma - lo) / (hi - lo), 0.0, 1.0);
  }
  f = round(f * 255.0) / 255.0;

  let coord = clamp(vec2<i32>(in.pos.xy), vec2<i32>(0, 0), vec2<i32>(textureDimensions(t0)) - 1);
  let acc =
    level_weight(0u, k, f) * textureLoad(t0, coord, 0).a +
    level_weight(1u, k, f) * textureLoad(t1, coord, 0).a +
    level_weight(2u, k, f) * textureLoad(t2, coord, 0).a +
    level_weight(3u, k, f) * textureLoad(t3, coord, 0).a +
    level_weight(4u, k, f) * textureLoad(t4, coord, 0).a +
    level_weight(5u, k, f) * textureLoad(t5, coord, 0).a +
    level_weight(6u, k, f) * textureLoad(t6, coord, 0).a +
    level_weight(7u, k, f) * textureLoad(t7, coord, 0).a;
  let a = params.color.a * acc;
  if (!(a > 0.0)) {
    discard;
  }
  return vec4<f32>(params.color.rgb * a, a);
}
