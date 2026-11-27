// Monet — M1 shape pipeline.
//
// One instanced unit quad per filled shape. The vertex stage maps the shape's local box (0..w, 0..h in
// document units) to device pixels with the instance's affine matrix; the fragment stage evaluates the
// shape's signed distance field **in local space** and turns it into a one-pixel anti-aliased coverage
// with `fwidth`, then evaluates the paint (solid or one of the four Figma gradients) and writes a
// premultiplied colour. Blending is One / OneMinusSrcAlpha, so overlapping shapes accumulate exactly
// like the CPU's painter's order.
//
// Why `fwidth(d)` instead of a fixed-width falloff: `d` is computed from the interpolated local
// position, so its screen-space derivative already folds in zoom, rotation and (for a similarity)
// scale. The edge stays one device pixel at any zoom.
//
// Limits kept deliberately explicit: no strokes, no text, no images, no effects, no non-normal
// blending, no masks. Those frames are refused in Rust (`Scene::unsupported`) and drawn on the CPU.

struct Globals {
  size: vec2<f32>,
  pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

// How far (device pixels) the quad is drawn beyond the shape's box, so multisample coverage never
// cuts into the SDF's antialiased edge.
const EXPAND_PX: f32 = 4.0;

struct Inst {
  // Local → device: x' = m.x*x + m.z*y + off.x, y' = m.y*x + m.w*y + off.y
  m: vec4<f32>,
  // off = (translation.x, translation.y, node opacity, shape kind)
  off: vec4<f32>,
  // size = (width, height in document units, paint kind, stop count)
  size: vec4<f32>,
  // corner radii tl, tr, br, bl in document units
  radii: vec4<f32>,
  // params = (polygon/star count, inner ratio, unused, unused)
  params: vec4<f32>,
  // gradient handles in the unit box: from.xy, to.xy
  p0: vec4<f32>,
  // width handle.xy
  p1: vec4<f32>,
  // device-pixel clip [x, y, w, h]; w < 0 means no clip
  clip: vec4<f32>,
  // four corner radii of the clip in device pixels (top-left, top-right, bottom-right, bottom-left)
  clip_radii: vec4<f32>,
  // stop positions (up to 8 slots, at most MAX_STOPS used)
  stop_pos: array<f32, 8>,
  // stop colours, premultiplied
  stop_col: array<vec4<f32>, 6>,
};
@group(1) @binding(0) var<storage, read> insts: array<Inst>;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) local: vec2<f32>,
  @location(1) @interpolate(flat) idx: u32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
  var corners = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
  );
  let inst = insts[ii];
  let c = corners[vi];
  // The quad is drawn a few device pixels larger than the shape on every side. The target is
  // multisampled (for tessellated paths), and a quad exactly on the shape's border would give the
  // border pixels partial sample coverage *and* the SDF's coverage — double antialiasing. With the
  // margin every pixel within the SDF's edge band is fully covered by samples, so the shader's
  // coverage is what the resolve averages.
  let sx = max(length(inst.m.xy), 1e-6);
  let sy = max(length(inst.m.zw), 1e-6);
  let margin = vec2<f32>(EXPAND_PX / sx, EXPAND_PX / sy);
  let l = vec2<f32>(c.x * (inst.size.x + 2.0 * margin.x) - margin.x, c.y * (inst.size.y + 2.0 * margin.y) - margin.y);
  let d = vec2<f32>(
    inst.m.x * l.x + inst.m.z * l.y + inst.off.x,
    inst.m.y * l.x + inst.m.w * l.y + inst.off.y,
  );
  var o: VsOut;
  // Device pixels (y down) → NDC (y up).
  o.pos = vec4<f32>(d / globals.size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
  o.local = l;
  o.idx = ii;
  return o;
}

// Rounded rectangle with per-corner radii, p relative to the centre.
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

// Ellipse: first-order distance in normalised space, exact enough near the edge (where the coverage
// is decided) and monotone inside.
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

// Vertex k of a regular polygon (point up) or a star (alternating outer/inner, 2*count vertices).
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

// Exact signed distance to a regular polygon or star: nearest edge segment, inside by even-odd
// crossing (a star is not convex, so a half-plane test would fill its notches).
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

// Piecewise-linear in premultiplied space, like the CPU's gradient shader.
fn stop_color(inst: Inst, n: i32, t: f32) -> vec4<f32> {
  var pos = inst.stop_pos;
  var col = inst.stop_col;
  if (n <= 1) {
    return col[0];
  }
  var i: i32 = 0;
  for (var k: i32 = 1; k < 6; k = k + 1) {
    if (k >= n) {
      break;
    }
    if (t >= pos[k]) {
      i = k;
    }
  }
  if (i >= n - 1) {
    return col[n - 1];
  }
  let t0 = pos[i];
  let t1 = pos[i + 1];
  let f = clamp((t - t0) / max(t1 - t0, 1e-6), 0.0, 1.0);
  return mix(col[i], col[i + 1], f);
}

// The paint at `local` (document units), premultiplied.
fn paint_color(inst: Inst, local: vec2<f32>) -> vec4<f32> {
  let kind = i32(round(inst.size.z));
  let n = i32(round(inst.size.w));
  if (kind == 0) {
    let c = inst.stop_col[0];
    return c;
  }
  let size = max(inst.size.xy, vec2<f32>(1e-6));
  let u = local / size;
  let gstart = inst.p0.xy;
  let gend = inst.p0.zw;
  // The third handle is absolute; the frame's second axis runs from `gstart` to it (the CPU's
  // `handle_frame` maps (0,0)→from, (1,0)→to, (0,1)→width in the unit box).
  let gwidth = inst.p1.xy;
  let ax = gend - gstart;
  let bx = gwidth - gstart;
  let det = ax.x * bx.y - ax.y * bx.x;
  let dp = u - gstart;
  var q = vec2<f32>(0.0, 0.0);
  if (abs(det) > 1e-9) {
    q = vec2<f32>((bx.y * dp.x - bx.x * dp.y) / det, (-ax.y * dp.x + ax.x * dp.y) / det);
  }
  var t = 0.0;
  if (kind == 1) {
    t = q.x;
  } else if (kind == 2) {
    t = length(q);
  } else if (kind == 3) {
    t = fract(atan2(q.y, q.x) / 6.2831853072 + 1.0);
  } else {
    t = abs(q.x) + abs(q.y);
  }
  return stop_color(inst, n, clamp(t, 0.0, 1.0));
}

// Coverage of the ancestor clip at `pos`: 1 inside, 0 outside, an antialiased edge on a rounded
// corner. A plain rectangle keeps the hard test the shader always had.
fn clip_mask(pos: vec2<f32>, clip: vec4<f32>, radii: vec4<f32>) -> f32 {
  if (clip.z <= 0.0) {
    return 1.0;
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
  let inst = insts[in.idx];

  // Ancestor clip rectangles/rounded rects (device pixels).
  let clip_a = clip_mask(in.pos.xy, inst.clip, inst.clip_radii);
  if (clip_a <= 0.0) {
    discard;
  }

  let half = inst.size.xy * 0.5;
  let p = in.local - half;
  let kind = i32(round(inst.off.w));
  var d = 0.0;
  if (kind == 0) {
    d = sd_rrect(p, half, inst.radii);
  } else if (kind == 1) {
    d = sd_ellipse(p, half);
  } else {
    // Polygons and stars stretch to the box (the CPU builder uses rx = w/2, ry = h/2), so the SDF is
    // evaluated in the box's normalised space where the shape is a regular polygon of radius 1.
    let hn = max(half, vec2<f32>(1e-6));
    d = sd_regular(p / hn, 1.0, inst.params.x, inst.params.y, kind == 3);
  }

  // One-pixel screen-space coverage, independent of zoom and rotation.
  let w = max(fwidth(d), 1e-5);
  let coverage = clamp(0.5 - d / w, 0.0, 1.0);
  if (coverage <= 0.0) {
    discard;
  }

  let pm = paint_color(inst, in.local);
  let alpha = coverage * inst.off.z * clip_a;
  let out = pm * alpha;
  if (out.a <= 0.0) {
    discard;
  }
  return out;
}
