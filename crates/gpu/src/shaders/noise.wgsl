// Monet — mono grain. The CPU (`figcraft_render::grain::noise`) hashes each document cell with a
// 64-bit mix of the cell's integer coordinates and the node id, draws `density` of them, and rides
// them on the layer's own alpha. This pass does the same hash in WGSL (the 64-bit values are two
// `u32` limbs and multiply modulo 2^64), so the grain lands on exactly the same cells.

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

struct Params {
  seed: u32,
  density: f32,
  fill_alpha: f32,
  mode: u32,
  cell: vec2<f32>,
  opacity: f32,
  pad1: f32,
  color: vec4<f32>,
  secondary: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
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

// The same distance functions as `shape.wgsl`: the grain mask must land on the same pixels.
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

// --- 64-bit arithmetic on two little-endian 32-bit limbs -------------------------------------

fn u64_from_u32(v: u32) -> vec2<u32> {
  return vec2<u32>(v, 0u);
}

fn u64_from_i32(v: i32) -> vec2<u32> {
  let lo = u32(v);
  let hi = select(0u, 0xFFFFFFFFu, v < 0);
  return vec2<u32>(lo, hi);
}

fn u64_xor(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
  return vec2<u32>(a.x ^ b.x, a.y ^ b.y);
}

fn u64_add(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
  let lo = a.x + b.x;
  let carry = select(0u, 1u, lo < a.x);
  return vec2<u32>(lo, a.y + b.y + carry);
}

fn u64_shr(a: vec2<u32>, n: u32) -> vec2<u32> {
  if (n == 0u) {
    return a;
  }
  if (n >= 64u) {
    return vec2<u32>(0u, 0u);
  }
  if (n >= 32u) {
    return vec2<u32>(a.y >> (n - 32u), 0u);
  }
  return vec2<u32>((a.x >> n) | (a.y << (32u - n)), a.y >> n);
}

fn limb(a: vec2<u32>, k: u32) -> u32 {
  let word = select(a.x, a.y, k >= 2u);
  return (word >> ((k % 2u) * 16u)) & 0xFFFFu;
}

fn add_shift(r: vec2<u32>, prod: u32, shift: u32) -> vec2<u32> {
  if (shift == 0u) {
    return u64_add(r, vec2<u32>(prod, 0u));
  }
  if (shift == 16u) {
    return u64_add(r, vec2<u32>((prod & 0xFFFFu) << 16u, prod >> 16u));
  }
  if (shift == 32u) {
    return u64_add(r, vec2<u32>(0u, prod));
  }
  return u64_add(r, vec2<u32>(0u, prod << 16u));
}

fn u64_mul(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
  var r = vec2<u32>(0u, 0u);
  r = add_shift(r, limb(a, 0u) * limb(b, 0u), 0u);
  r = add_shift(r, limb(a, 0u) * limb(b, 1u), 16u);
  r = add_shift(r, limb(a, 1u) * limb(b, 0u), 16u);
  r = add_shift(r, limb(a, 0u) * limb(b, 2u), 32u);
  r = add_shift(r, limb(a, 1u) * limb(b, 1u), 32u);
  r = add_shift(r, limb(a, 2u) * limb(b, 0u), 32u);
  r = add_shift(r, limb(a, 0u) * limb(b, 3u), 48u);
  r = add_shift(r, limb(a, 1u) * limb(b, 2u), 48u);
  r = add_shift(r, limb(a, 2u) * limb(b, 1u), 48u);
  r = add_shift(r, limb(a, 3u) * limb(b, 0u), 48u);
  return r;
}

// `figcraft_render::grain::hash`, limb for limb. The constants are the CPU's three mix primes.
fn grain_hash(ix: i32, iy: i32, seed: u32) -> vec2<u32> {
  let k1 = vec2<u32>(0x7F4A7C15u, 0x9E3779B9u);
  let k2 = vec2<u32>(0x27D4EB4Fu, 0xC2B2AE3Du);
  let k3 = vec2<u32>(0x9E3779F9u, 0x165667B1u);
  let k4 = vec2<u32>(0xED558CCDu, 0xFF51AFD7u);
  let k5 = vec2<u32>(0x1A85EC53u, 0xC4CEB9FEu);
  var h = u64_mul(u64_from_i32(ix), k1);
  h = u64_xor(h, u64_mul(u64_from_i32(iy), k2));
  h = u64_xor(h, u64_mul(u64_from_u32(seed), k3));
  h = u64_xor(h, u64_shr(h, 47u));
  h = u64_xor(h, u64_mul(h, k4));
  h = u64_xor(h, u64_shr(h, 47u));
  h = u64_xor(h, u64_mul(h, k5));
  h = u64_xor(h, u64_shr(h, 47u));
  return h;
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

  // Local coordinates from this device pixel: invert the instance's affine.
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
  let coverage = clamp(0.5 - d / w, 0.0, 1.0);
  if (coverage <= 0.0) {
    discard;
  }

  // The CPU's cell: round() the local coordinate to 2^-20 so a point sitting on a cell boundary
  // does not flip with floating noise, then floor to the integer cell.
  let cx = local.x / max(params.cell.x, 1e-6);
  let cy = local.y / max(params.cell.y, 1e-6);
  let sx = floor(round(cx * 1048576.0) / 1048576.0);
  let sy = floor(round(cy * 1048576.0) / 1048576.0);
  if (!(abs(sx) < 2147483000.0 && abs(sy) < 2147483000.0)) {
    discard;
  }
  let ix = i32(sx);
  let iy = i32(sy);
  let hv = grain_hash(ix, iy, params.seed);
  let unit = f32(hv.x & 0xFFFFu) / 65536.0;
  if (unit >= params.density) {
    discard;
  }

  // The layer alpha the grain rides on is the node's own fill alpha under the mask, exactly what
  // the CPU's `grain::grains` multiplies by the grain colour's alpha.
  let layer_a = params.fill_alpha * coverage;
  var rgb = params.color.rgb;
  var ca = params.color.a;
  if (params.mode == 1u) {
    // Duo: `hv >> 63` picks the secondary colour, like the CPU's `hv >> 63 == 1`.
    let second = hv.y >= 2147483648u;
    rgb = select(params.color.rgb, params.secondary.rgb, second);
    ca = select(params.color.a, params.secondary.a, second);
  } else if (params.mode >= 2u) {
    // Multi: one colour per grain from the 16-bit fields at bits 16, 32 and 48; the grain's
    // alpha is the effect's own opacity, not any colour's alpha.
    rgb = vec3<f32>(
      f32((hv.x >> 16u) & 0xFFFFu) / 65536.0,
      f32(hv.y & 0xFFFFu) / 65536.0,
      f32((hv.y >> 16u) & 0xFFFFu) / 65536.0,
    );
    ca = params.opacity;
  }
  let a = ca * layer_a;
  if (!(a > 0.0)) {
    discard;
  }
  return vec4<f32>(rgb * a, a);
}
