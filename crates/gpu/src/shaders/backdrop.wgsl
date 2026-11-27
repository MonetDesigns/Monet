// Monet — a background blur: the frame's backdrop, blurred by the box chain, shown through one SDF
// box. The CPU blurs everything under the layer and masks it by the node's outline; this pass reads
// the resolved frame (sampled in device pixels), and keeps the same SDF mask, clip and coverage.

struct Inst {
  // Local → device, same layout as `shape.wgsl`.
  m: vec4<f32>,
  off: vec4<f32>,
  size: vec4<f32>,
  radii: vec4<f32>,
  params: vec4<f32>,
  p0: vec4<f32>,
  p1: vec4<f32>,
  clip: vec4<f32>,
  stop_pos: array<f32, 8>,
  stop_col: array<vec4<f32>, 6>,
};
@group(0) @binding(0) var src: texture_2d<f32>;
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

// The same distance functions as `shape.wgsl`: the mask must land on the same pixels.
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

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let inst = insts[in.idx];

  if (inst.clip.z > 0.0) {
    if (in.pos.x < inst.clip.x || in.pos.y < inst.clip.y ||
      in.pos.x >= inst.clip.x + inst.clip.z || in.pos.y >= inst.clip.y + inst.clip.w) {
      discard;
    }
  }

  // The box's local coordinates from this device pixel: invert the instance's affine.
  let det = inst.m.x * inst.m.w - inst.m.z * inst.m.y;
  if (abs(det) < 1e-12) {
    discard;
  }
  let dx = in.pos.x - inst.off.x;
  let dy = in.pos.y - inst.off.y;
  let local = vec2<f32>((inst.m.w * dx - inst.m.z * dy) / det, (-inst.m.y * dx + inst.m.x * dy) / det);
  let half = inst.size.xy * 0.5;
  let p = local - half;
  let kind = i32(round(inst.off.w));
  var d = 0.0;
  if (kind == 0) {
    d = sd_rrect(p, half, inst.radii);
  } else if (kind == 1) {
    d = sd_ellipse(p, half);
  } else {
    let hn = max(half, vec2<f32>(1e-6));
    d = sd_regular(p / hn, 1.0, inst.params.x, inst.params.y, kind == 3);
  }

  let w = max(fwidth(d), 1e-5);
  let coverage = clamp(0.5 - d / w, 0.0, 1.0) * inst.off.z;
  if (coverage <= 0.0) {
    discard;
  }

  let at = vec2<i32>(i32(floor(in.pos.x)), i32(floor(in.pos.y)));
  let dims = vec2<i32>(textureDimensions(src, 0));
  if (at.x < 0 || at.y < 0 || at.x >= dims.x || at.y >= dims.y) {
    discard;
  }
  return textureLoad(src, at, 0) * coverage;
}
