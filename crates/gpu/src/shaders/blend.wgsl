// Monet — composite one resolved group layer over the frame with a non-normal blend.
//
// The layer is isolated exactly like `composite.wgsl`, but a blend like Multiply cannot be written
// with fixed-function blending: tiny-skia's formulas multiply source *and* destination colours,
// which a `BlendState` cannot express. This pass therefore reads a snapshot of the frame under the
// group, blends in the shader with tiny-skia's premultiplied formulas, and *replaces* the frame:
// the returned value already contains the destination, so the pipeline has blending disabled.
//
// The formulas are the stages in tiny-skia 0.11.4 `pipeline/highp.rs`; the HSL modes follow the
// same `set_sat`/`set_lum`/`clip_color` helpers so the bytes line up with the CPU.

struct Params {
  // x = opacity, y = mode code (1..=17), z/w unused.
  params: vec4<f32>,
};
@group(0) @binding(0) var under: texture_2d<f32>;
@group(0) @binding(1) var layer: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: Params;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
  var pts = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  var o: VsOut;
  o.pos = vec4<f32>(pts[vi], 0.0, 1.0);
  return o;
}

fn lum(c: vec3<f32>) -> f32 {
  return c.r * 0.30 + c.g * 0.59 + c.b * 0.11;
}

fn sat(c: vec3<f32>) -> f32 {
  return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
}

fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
  let diff = l - lum(c);
  return c + vec3<f32>(diff, diff, diff);
}

fn sat_scale(v: f32, mn: f32, span: f32, s: f32) -> f32 {
  if (span == 0.0) { return 0.0; }
  return (v - mn) * s / span;
}

fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
  let mn = min(c.r, min(c.g, c.b));
  let mx = max(c.r, max(c.g, c.b));
  let span = mx - mn;
  return vec3<f32>(sat_scale(c.r, mn, span, s), sat_scale(c.g, mn, span, s), sat_scale(c.b, mn, span, s));
}

fn clip_channel(v: f32, mn: f32, mx: f32, l: f32, a: f32) -> f32 {
  var x = v;
  if (mx < 0.0 && l != mn) {
    x = l + (x - l) * l / (l - mn);
  }
  if (mx > a && mx != l) {
    x = l + (x - l) * (a - l) / (mx - l);
  }
  return max(x, 0.0);
}

fn clip_color(c: vec3<f32>, a: f32) -> vec3<f32> {
  let mn = min(c.r, min(c.g, c.b));
  let mx = max(c.r, max(c.g, c.b));
  let l = lum(c);
  return vec3<f32>(clip_channel(c.r, mn, mx, l, a), clip_channel(c.g, mn, mx, l, a), clip_channel(c.b, mn, mx, l, a));
}

fn inv(x: f32) -> f32 { return 1.0 - x; }

// Separable channels, premultiplied; `mode` is 3 Darken .. 13 Exclusion, 1 PlusDarker.
fn separable(mode: i32, s: f32, d: f32, sa: f32, da: f32) -> f32 {
  switch mode {
    case 1: { // PlusDarker
      return s * inv(da) + d * inv(sa) + max(sa * d + da * s - sa * da, 0.0);
    }
    case 3: { // Darken
      return s + d - max(s * da, d * sa);
    }
    case 4: { // Multiply
      return s * inv(da) + d * inv(sa) + s * d;
    }
    case 6: { // ColorBurn
      if (d >= da) { return d + s * inv(da); }
      if (s <= 0.0) { return d * inv(sa); }
      return sa * (da - min(da, (da - d) * sa / s)) + s * inv(da) + d * inv(sa);
    }
    case 7: { // Lighten
      return s + d - min(s * da, d * sa);
    }
    case 8: { // Screen
      return s + d - s * d;
    }
    case 10: { // ColorDodge
      if (d <= 0.0) { return s * inv(da); }
      if (s >= sa) { return s + d * inv(sa); }
      return sa * min(da, d * sa / (sa - s)) + s * inv(da) + d * inv(sa);
    }
    case 11: { // Overlay
      var b = s * inv(da) + d * inv(sa);
      if (2.0 * d <= da) { b += 2.0 * s * d; } else { b += sa * da - 2.0 * (da - d) * (sa - s); }
      return b;
    }
    case 12: { // SoftLight, straight-space W3C formula (same result as tiny's premul stages)
      let cs = select(0.0, s / sa, sa > 0.0);
      let cd = select(0.0, d / da, da > 0.0);
      var b = 0.0;
      if (cd <= 0.5) {
        b = cd - (1.0 - 2.0 * cs) * cd * (1.0 - cd);
      } else {
        var dd = sqrt(cd);
        if (cd <= 0.25) { dd = ((16.0 * cd - 12.0) * cd + 4.0) * cd; }
        b = cd + (2.0 * cs - 1.0) * (dd - cd);
      }
      return sa * b + d * inv(sa);
    }
    case 13: { // HardLight
      var b = s * inv(da) + d * inv(sa);
      if (2.0 * s <= sa) { b += 2.0 * s * d; } else { b += sa * da - 2.0 * (da - d) * (sa - s); }
      return b;
    }
    case 14: { // Difference
      return s + d - 2.0 * min(s * da, d * sa);
    }
    case 15: { // Exclusion
      return s + d - 2.0 * s * d;
    }
    default: { return s; }
  }
}

fn non_separable(mode: i32, s: vec3<f32>, d: vec3<f32>, sa: f32, da: f32) -> vec3<f32> {
  var rr = vec3<f32>(0.0, 0.0, 0.0);
  switch mode {
    case 16: { // Hue
      rr = s * sa;
      rr = set_sat(rr, sat(d) * sa);
      rr = set_lum(rr, lum(d) * sa);
      rr = clip_color(rr, sa * da);
    }
    case 17: { // Saturation
      rr = d * sa;
      rr = set_sat(rr, sat(s) * da);
      rr = set_lum(rr, lum(d) * sa);
      rr = clip_color(rr, sa * da);
    }
    case 18: { // Color
      rr = s * da;
      rr = set_lum(rr, lum(d) * sa);
      rr = clip_color(rr, sa * da);
    }
    case 19: { // Luminosity
      rr = d * sa;
      rr = set_lum(rr, lum(s) * da);
      rr = clip_color(rr, sa * da);
    }
    default: { rr = vec3<f32>(0.0, 0.0, 0.0); }
  }
  return s * inv(da) + d * inv(sa) + rr;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let p = vec2<i32>(floor(in.pos.xy));
  let dims = textureDimensions(layer);
  if (p.x < 0 || p.y < 0 || p.x >= i32(dims.x) || p.y >= i32(dims.y)) {
    return vec4<f32>(0.0);
  }
  let s = textureLoad(layer, p, 0) * params.params.x;
  let d = textureLoad(under, p, 0);
  let sa = s.a;
  let da = d.a;
  let mode = i32(params.params.y);
  if (mode == 5) { // PlusLighter clamps the sum, alpha included.
    return clamp(vec4<f32>(min(s.rgb + d.rgb, vec3<f32>(1.0)), min(sa + da, 1.0)), vec4<f32>(0.0), vec4<f32>(1.0));
  }
  var rgb = vec3<f32>(
    separable(mode, s.r, d.r, sa, da),
    separable(mode, s.g, d.g, sa, da),
    separable(mode, s.b, d.b, sa, da),
  );
  if (mode >= 16) {
    rgb = non_separable(mode, s.rgb, d.rgb, sa, da);
  }
  let alpha = sa + da - sa * da;
  return clamp(vec4<f32>(rgb, alpha), vec4<f32>(0.0), vec4<f32>(1.0));
}
