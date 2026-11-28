//! Layer and background blurs: uniform (one Gaussian) or progressive (the radius ramps linearly
//! along a line, from a start radius at its start point to an end radius at its end point).
//!
//! A progressive blur blurs the pixels at a few radii between its smallest and largest radius,
//! spaced geometrically (steps between small radii show the most), and blends every pixel between
//! the two levels around its own radius. Each level is one Gaussian-like blur (three box passes,
//! linear in the pixel count whatever the radius), so the cost is a handful of uniform blurs.

use figcraft_doc::Blur;
use figcraft_geom::{Affine, Point};
use tiny_skia::Pixmap;

use crate::fx;

/// Most blur levels a progressive blur draws.
const MAX_LEVELS: usize = 8;
/// Ratio between neighbouring levels' radii (above one pixel).
const LEVEL_RATIO: f64 = 1.5;

/// Blur `p` with `b`. `size` is the layer's box (local units, for the progressive line's
/// unit-box points), `xf` maps local units to `p`'s pixels and `scale` is device pixels per unit.
pub fn apply(p: &mut Pixmap, b: &Blur, size: (f64, f64), xf: Affine, scale: f64) {
    let b = b.sanitized();
    let sigma = |r: f64| r / 2.0 * scale;
    if !b.is_progressive() {
        fx::blur(p, sigma(b.radius));
        return;
    }
    let at = |u: (f64, f64)| xf * Point::new(u.0 * size.0, u.1 * size.1);
    progressive(p, sigma(b.start_radius), sigma(b.radius), at(b.start), at(b.end));
}

/// The blur's standard deviation along the line: `s0` at `a` and before it, `s1` at `b` and past
/// it, linear in between (pixels).
struct Ramp {
    s0: f64,
    s1: f64,
    /// t at pixel (0, 0)'s centre and its change per pixel across and down.
    t0: f64,
    tx: f64,
    ty: f64,
}

impl Ramp {
    fn new(s0: f64, s1: f64, a: Point, b: Point) -> Option<Ramp> {
        let d = b - a;
        let len2 = d.hypot2();
        if !(len2.is_finite() && len2 > 1e-12 && a.x.is_finite() && a.y.is_finite()) {
            return None;
        }
        let (tx, ty) = (d.x / len2, d.y / len2);
        Some(Ramp { s0, s1, t0: (0.5 - a.x) * tx + (0.5 - a.y) * ty, tx, ty })
    }
    fn sigma(&self, x: f64, y: f64) -> f64 {
        let t = (self.t0 + x * self.tx + y * self.ty).clamp(0.0, 1.0);
        self.s0 + (self.s1 - self.s0) * t
    }
}

/// The radii blurred for a ramp from `lo` to `hi` (pixels): `lo`, then geometric steps from one
/// pixel (or `lo`) up to `hi`, at most [`MAX_LEVELS`].
pub(crate) fn levels(lo: f64, hi: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(MAX_LEVELS);
    let base = lo.max(1.0);
    if lo < base {
        out.push(lo);
    }
    if hi <= base {
        if hi > lo || out.is_empty() {
            out.push(hi);
        }
        return out;
    }
    let steps = ((hi / base).ln() / LEVEL_RATIO.ln()).ceil().clamp(1.0, (MAX_LEVELS - 2) as f64) as usize;
    out.extend((0..steps).map(|k| base * (hi / base).powf(k as f64 / steps as f64)));
    out.push(hi);
    out
}

/// The level below `s` and the weight (0–255) of the level above it.
fn bracket(levels: &[f64], s: f64) -> (u8, u8) {
    let n = levels.len();
    if n < 2 {
        return (0, 0);
    }
    let k = levels.partition_point(|v| *v <= s).saturating_sub(1).min(n - 2);
    let (Some(lo), Some(hi)) = (levels.get(k), levels.get(k + 1)) else { return (0, 0) };
    let f = if hi > lo { ((s - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    (k as u8, (f * 255.0).round() as u8)
}

/// A progressive blur of `p`: standard deviation `s0` at pixel point `a` (and before it) to `s1`
/// at `b` (and past it). A degenerate line blurs with `s1` everywhere.
pub fn progressive(p: &mut Pixmap, s0: f64, s1: f64, a: Point, b: Point) {
    let cap = |s: f64| if s.is_finite() { s.clamp(0.0, fx::MAX_SIGMA) } else { 0.0 };
    let (s0, s1) = (cap(s0), cap(s1));
    let Some(ramp) = Ramp::new(s0, s1, a, b).filter(|_| (s1 - s0).abs() >= 0.05) else {
        fx::blur(p, s1);
        return;
    };
    let (w, h) = (p.width(), p.height());
    // The levels span the whole ramp, not just the part on this pixmap: a layer cut by a band or
    // the view blends the same levels as the whole layer would.
    let (lo, hi) = (s0.min(s1), s0.max(s1));
    let lv = levels(lo, hi);
    // Each level's pixels: the box they lie in (inclusive), where it is used at all.
    let mut used: [Option<[u32; 4]>; MAX_LEVELS] = [None; MAX_LEVELS];
    let mut mark = |k: usize, x: u32, y: u32| {
        if let Some(u) = used.get_mut(k) {
            *u = Some(match *u {
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
                None => [x, y, x, y],
            });
        }
    };
    let mut sel = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        for x in 0..w {
            let (k, f) = bracket(&lv, ramp.sigma(f64::from(x), f64::from(y)));
            mark(usize::from(k), x, y);
            if f > 0 {
                mark(usize::from(k) + 1, x, y);
            }
            sel.push((k, f));
        }
    }
    // Each pixel: Σ weight × level value, the weights of its two levels summing to 255.
    let mut acc = vec![0u16; p.data().len()];
    for (k, s) in lv.iter().enumerate() {
        let Some(Some([x0, y0, x1, y1])) = used.get(k).copied() else { continue };
        // A level is blurred only around the pixels that use it: a pixel's blur reads at most
        // 3 sigma + 3 pixels away (see `crate::backdrop_reach`), so the result is exact.
        let reach = (s * 3.0).ceil() as u32 + 3;
        let (cx, cy) = (x0.saturating_sub(reach), y0.saturating_sub(reach));
        let (cw, ch) = ((x1 + reach + 1).min(w) - cx, (y1 + reach + 1).min(h) - cy);
        let Some(mut level) = tiny_skia::IntRect::from_xywh(cx as i32, cy as i32, cw, ch).and_then(|r| p.clone_rect(r)) else { continue };
        fx::blur(&mut level, *s);
        let ld = level.data();
        let row = cw as usize * 4;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = (y * w + x) as usize;
                let Some(&(j, f)) = sel.get(i) else { continue };
                let j = usize::from(j);
                let wgt = if j == k {
                    255 - u16::from(f)
                } else if j + 1 == k {
                    u16::from(f)
                } else {
                    continue;
                };
                let li = (y - cy) as usize * row + (x - cx) as usize * 4;
                if let (Some(px), Some(out)) = (ld.get(li..li + 4), acc.get_mut(i * 4..i * 4 + 4)) {
                    for (o, c) in out.iter_mut().zip(px) {
                        *o += wgt * u16::from(*c);
                    }
                }
            }
        }
    }
    for (d, a) in p.data_mut().iter_mut().zip(&acc) {
        *d = ((u32::from(*a) + 127) / 255) as u8;
    }
}

/// Figma's layer/background blur is a **separable box**, not a mip-level sample: a box of
/// half-width `radius_px` is applied horizontally, then vertically. Edges clamp to the border
/// (the reference's `blurClamp`), so a transparent border cannot pull black into the result.
///
/// The input and output are **premultiplied** RGBA. This is deliberately the exact box, not a
/// finite tap approximation: a 1D running sum makes the cost linear in pixels even for a radius
/// of hundreds of pixels. `radius_px` is in device pixels (document radius × scale).
pub fn box_premultiplied(src: &[u8], w: u32, h: u32, radius_px: f64) -> Vec<u8> {
    let len = (w as usize).saturating_mul(h as usize).saturating_mul(4);
    if len == 0 || src.len() < len {
        return src.to_vec();
    }
    let r = radius_px.round();
    if !(r >= 1.0) || w == 0 || h == 0 {
        return src[..len].to_vec();
    }
    let r = (r as usize).min(usize::MAX / 4);
    let denom = (2 * r + 1) as f64;

    // Horizontal pass. The running window repeats the first/last column outside the border,
    // exactly like clamping each tap coordinate.
    let mut tmp = vec![0u8; len];
    for y in 0..h as usize {
        let row = y * w as usize * 4;
        let mut sum = [0.0f64; 4];
        for i in 0..=(2 * r) {
            let x = i.saturating_sub(r).min(w as usize - 1);
            let p = row + x * 4;
            for c in 0..4 {
                sum[c] += f64::from(src[p + c]);
            }
        }
        for x in 0..w as usize {
            let out = row + x * 4;
            for c in 0..4 {
                tmp[out + c] = (sum[c] / denom).round().clamp(0.0, 255.0) as u8;
            }
            if x + 1 < w as usize {
                let add = (x + 1 + r).min(w as usize - 1);
                let del = x.saturating_sub(r);
                for c in 0..4 {
                    sum[c] += f64::from(src[row + add * 4 + c]);
                    sum[c] -= f64::from(src[row + del * 4 + c]);
                }
            }
        }
    }

    // Vertical pass, same running window over rows.
    let mut out = vec![0u8; len];
    let stride = w as usize * 4;
    for x in 0..w as usize {
        let mut sum = [0.0f64; 4];
        for i in 0..=(2 * r) {
            let y = i.saturating_sub(r).min(h as usize - 1);
            let p = y * stride + x * 4;
            for c in 0..4 {
                sum[c] += f64::from(tmp[p + c]);
            }
        }
        for y in 0..h as usize {
            let p = y * stride + x * 4;
            for c in 0..4 {
                out[p + c] = (sum[c] / denom).round().clamp(0.0, 255.0) as u8;
            }
            if y + 1 < h as usize {
                let add = (y + 1 + r).min(h as usize - 1);
                let del = y.saturating_sub(r);
                for c in 0..4 {
                    sum[c] += f64::from(tmp[add * stride + x * 4 + c]);
                    sum[c] -= f64::from(tmp[del * stride + x * 4 + c]);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod box_tests {
    use super::box_premultiplied;

    #[test]
    fn a_constant_survives_any_radius() {
        let src = vec![10, 20, 30, 255].repeat(16);
        let out = box_premultiplied(&src, 4, 4, 3.0);
        assert_eq!(out, src);
    }

    #[test]
    fn a_single_pixel_spreads_by_one_and_clamps_at_the_border() {
        // 3×1: the centre is white, the two sides are black. Clamped box radius 1 gives every
        // pixel (0 + 255 + 0) / 3 = 85, including the border pixels (their outside tap repeats
        // the border, not the centre).
        let src = vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255];
        let out = box_premultiplied(&src, 3, 1, 1.0);
        assert_eq!(out, vec![85, 85, 85, 255, 85, 85, 85, 255, 85, 85, 85, 255]);
    }

    #[test]
    fn radius_below_half_a_pixel_is_a_no_op() {
        let src = vec![1, 2, 3, 4];
        assert_eq!(box_premultiplied(&src, 1, 1, 0.4), src);
    }
}
