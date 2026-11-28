// Monet — rough-edge texture. The CPU (`figcraft_render::grain::texture`) moves each destination
// pixel one whole-pixel step from a 64-bit cell hash, copies the layer at that offset, and optionally
// multiplies the copy by the destination alpha (`clip_to_shape`). This pass does the same over the
// node's own fill, with the identical hash and pixel rounding.

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
  clip_radii: vec4<f32>,
  stop_pos: array<f32, 8>,
  stop_col: array<vec4<f32>, 6>,
};

struct Params {
  seed: u32,
  radius: f32,
  clip_to_shape: u32,
  pad0: u32,
  cell: vec2<f32>,
  pad1: vec2<f32>,
  color: vec4<f32>,
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

  if (inst.clip.z >= 0.0) {
    if (in.pos.x < inst.clip.x || in.pos.y < inst.clip.y ||
      in.pos.x >= inst.clip.x + inst.clip.z || in.pos.y >= inst.clip.y + inst.clip.w) {
      discard;
    }
  }

  // Destination local coordinates from this device pixel: invert the instance's affine.
  let det = inst.m.x * inst.m.w - inst.m.z * inst.m.y;
  if (abs(det) < 1e-12) {
    discard;
  }
  let dx = in.pos.x - inst.off.x;
  let dy = in.pos.y - inst.off.y;
  let local = vec2<f32>((inst.m.w * dx - inst.m.z * dy) / det, (-inst.m.y * dx + inst.m.x * dy) / det);
  // One device pixel in the layer's local units. Different fragments scatter by different
  // whole-pixel offsets, so `fwidth(sd)` would measure those jumps instead of the shape edge;
  // a fixed one-pixel footprint keeps the edge coverage correct at the displaced source pixel.
  let scale_x = length(vec2<f32>(inst.m.x, inst.m.y));
  let scale_y = length(vec2<f32>(inst.m.z, inst.m.w));
  let aa_local = 1.0 / max(max(scale_x, scale_y), 1e-6);

  // The CPU's cell from the destination pixel, rounded to 2^-20 before floor.
  let cx = local.x / max(params.cell.x, 1e-6);
  let cy = local.y / max(params.cell.y, 1e-6);
  let sx = floor(round(cx * 1048576.0) / 1048576.0);
  let sy = floor(round(cy * 1048576.0) / 1048576.0);
  if (!(abs(sx) < 2147483000.0 && abs(sy) < 2147483000.0)) {
    discard;
  }
  let ix = i32(sx);
  let iy = i32(sy);
  let hv = grain_hash(ix, iy, params.seed ^ 0x7E57u);

  // A uniform point in the disc of radius `params.radius`: one whole-pixel move per cell.
  let ang = (f32(hv.x & 0xFFFFu) / 65536.0) * 6.2831853072;
  let unit16 = f32((hv.x >> 16u) & 0xFFFFu) / 65536.0;
  let dist = sqrt(unit16) * params.radius;
  let mx = floor(0.5 + cos(ang) * dist);
  let my = floor(0.5 + sin(ang) * dist);

  // Read the fill at the moved pixel: the layer content is exactly this node's fill.
  let sdx = in.pos.x + mx - inst.off.x;
  let sdy = in.pos.y + my - inst.off.y;
  let slocal = vec2<f32>((inst.m.w * sdx - inst.m.z * sdy) / det, (-inst.m.y * sdx + inst.m.x * sdy) / det);
  let half = inst.size.xy * 0.5;
  let sp = slocal - half;
  let kind = i32(round(inst.off.w));
  var sd = 0.0;
  if (kind == 0) {
    sd = sd_rrect(sp, half, inst.radii);
  } else if (kind == 1) {
    sd = sd_ellipse(sp, half);
  } else {
    let hn = max(half, vec2<f32>(1e-6));
    sd = sd_regular(sp / hn, 1.0, inst.params.x, inst.params.y, kind == 3);
  }
  let sw = max(aa_local, 1e-5);
  let scov = clamp(0.5 - sd / sw, 0.0, 1.0);
  var src = vec4<f32>(params.color.rgb * (params.color.a * scov), params.color.a * scov);

  // `clip_to_shape` multiplies the copied pixel by the layer alpha at the destination, exactly
  // like the CPU's `keep = src[i + 3]`.
  if (params.clip_to_shape != 0u) {
    let p = local - half;
    var dd = 0.0;
    if (kind == 0) {
      dd = sd_rrect(p, half, inst.radii);
    } else if (kind == 1) {
      dd = sd_ellipse(p, half);
    } else {
      let hn = max(half, vec2<f32>(1e-6));
      dd = sd_regular(p / hn, 1.0, inst.params.x, inst.params.y, kind == 3);
    }
    let dw = max(aa_local, 1e-5);
    let dcov = clamp(0.5 - dd / dw, 0.0, 1.0);
    let keep = params.color.a * dcov;
    src = src * keep;
  }

  if (!(src.a > 0.0)) {
    discard;
  }
  return src;
}