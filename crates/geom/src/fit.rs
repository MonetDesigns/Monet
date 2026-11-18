//! Fitting a hand-drawn polyline to a handful of smooth cubics.
//!
//! kurbo's `simplify_bezpath` only fits a cubic across joins whose tangents are nearly parallel
//! (its angle threshold is a tangent of 1e-3, about 0.06°). Adjacent pointer samples almost never
//! pass that, so a 300-sample pencil stroke came out as 299 straight lines and no curve at all —
//! hundreds of points, and a visibly faceted stroke. This module fits the whole stroke instead:
//! a least-squares cubic with the end tangents fixed, split at the worst sample only when the fit
//! is off by more than `accuracy`, with a few Newton steps on the parameterisation first. Every
//! accepted cubic therefore stays within `accuracy` of the samples, and a smooth stroke needs a
//! few segments rather than one per sample.

use kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveDeriv, ParamCurveNearest, Point, Vec2};

/// Most cubics one stroke is fitted with. Past this the stroke keeps its samples: a stroke that
/// cannot be fitted in this many segments is not one a hand drew smoothly.
const MAX_CUBICS: usize = 64;

/// Samples closer than this are the same point (pointer pixels are fractional page points).
const SAME: f64 = 1e-9;

/// The fewest samples worth fitting a curve through; below it the stroke stays a line.
const MIN_FIT: usize = 3;

/// How far, in page units, a sample is averaged with its neighbours before fitting. A pointer's
/// tremor is a pixel or two whatever the sampling rate, so the kernel is a distance, not a count:
/// dense samples are smoothed (a shaky stroke stops being traced sample by sample) while samples
/// several pixels apart — the pointer was moving fast, so there is no tremor to remove — stay put.
const SMOOTHING: f64 = 3.0;

/// `points` fitted with cubics that stay within `accuracy` of them, as an open path starting at the
/// first sample and ending at the last. `None` when there is nothing to draw.
pub fn fit_polyline(points: &[Point], accuracy: f64) -> Option<BezPath> {
    let (pts, _) = prepare_samples(points, SMOOTHING);
    fit_ready(&pts, accuracy)
}

/// The samples a fit is made from — deduped, then averaged with their neighbours within
/// `smoothing` — and, for each, the index of the sample it came from.
///
/// A width profile (pressure, tilt) is keyed by length along the stroke, and the curve that is
/// finally drawn is built from these samples: measuring the profile on the raw samples instead
/// makes it slide against the fitted curve (the two have different lengths), which shows up as
/// taper that does not line up with the hand.
pub fn prepare_samples(points: &[Point], smoothing: f64) -> (Vec<Point>, Vec<usize>) {
    let (mut pts, from) = dedupe_with_index(points);
    smooth(&mut pts, smoothing);
    (pts, from)
}

/// `points` (already through [`prepare_samples`]) fitted with cubics within `accuracy` of them.
pub fn fit_ready(points: &[Point], accuracy: f64) -> Option<BezPath> {
    let pts = points;
    if pts.len() < 2 {
        return None;
    }
    if pts.len() < MIN_FIT {
        let mut out = BezPath::new();
        out.move_to(pts[0]);
        out.line_to(pts[1]);
        return Some(out);
    }
    let accuracy = if accuracy.is_finite() { accuracy.max(1e-3) } else { 1.0 };
    // The end directions come from one step of the *smoothed* samples: the step is already an
    // average of the hand's tremor, and a direction measured over a longer stretch of an arc is
    // the arc's chord, which is off by half the angle it spans — that bent every end tangent and
    // split the arc into more segments.
    let t0 = unit(pts[1] - pts[0]);
    let t1 = unit(pts[pts.len() - 1] - pts[pts.len() - 2]);
    let mut fitter = Fitter { pts, accuracy, out: BezPath::new(), cubics: 0 };
    fitter.out.move_to(pts[0]);
    if !fitter.fit(0, pts.len() - 1, t0, t1) {
        // Not fittable within the segment budget: keep the samples rather than guess.
        let mut out = BezPath::new();
        out.move_to(pts[0]);
        for p in &pts[1..] {
            out.line_to(*p);
        }
        return Some(out);
    }
    Some(fitter.out)
}

/// `points` smoothed the way the fit smooths them, for a preview drawn while the stroke is still
/// being made: the raw samples carry the hand's tremor, and a preview that shows it looks nothing
/// like the curve the stroke becomes. Endpoints stay where they are.
pub fn smooth_points(points: &[Point], sigma: f64) -> Vec<Point> {
    let mut out = dedupe(points);
    smooth(&mut out, sigma);
    out
}

/// The samples without consecutive repeats (and without points that are not finite).
fn dedupe(points: &[Point]) -> Vec<Point> {
    dedupe_with_index(points).0
}

/// [`dedupe`], keeping the index each kept sample came from.
fn dedupe_with_index(points: &[Point]) -> (Vec<Point>, Vec<usize>) {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    let mut from: Vec<usize> = Vec::with_capacity(points.len());
    for (i, p) in points.iter().enumerate() {
        if !(p.x.is_finite() && p.y.is_finite()) {
            continue;
        }
        if out.last().is_none_or(|l| (*l - *p).hypot() > SAME) {
            out.push(*p);
            from.push(i);
        }
    }
    (out, from)
}

/// `v` as a unit vector; the x axis for a vector of no length.
fn unit(v: Vec2) -> Vec2 {
    let len = v.hypot();
    if len > SAME { v / len } else { Vec2::new(1.0, 0.0) }
}

/// Average each sample with the neighbours within `sigma` of it, by a Gaussian of their distance.
/// A hand's tremor is a pixel or two whatever the sampling rate, so a dense stroke is smoothed and
/// a sparse one (a fast stroke, or a small one) is barely touched: smoothing shrinks a curve by
/// about `sigma²·curvature`, which is nothing at three points.
///
/// The ends are mirrored rather than left with a one-sided window: a one-sided average pulls the
/// first samples *along* the stroke, which made the start of an arc disagree with its own end
/// tangent — the fit then split there and the stroke began with a straight piece.
fn smooth(points: &mut [Point], sigma: f64) {
    if points.len() < MIN_FIT || !sigma.is_finite() || sigma <= 0.0 {
        return;
    }
    let src = points.to_vec();
    let n = src.len();
    let reach = sigma * 3.0;
    // A sample outside the stroke, mirrored across its end, so the kernel stays symmetric there.
    let at = |i: isize| -> Point {
        let base = |p: Point, anchor: Point| Point::new(2.0 * anchor.x - p.x, 2.0 * anchor.y - p.y);
        if i < 0 {
            let k = (-i) as usize;
            return base(*src.get(k.min(n - 1)).unwrap_or(&src[0]), src[0]);
        }
        if i as usize >= n {
            let j = (2 * (n - 1)).saturating_sub(i as usize).min(n - 1);
            return base(src[j], src[n - 1]);
        }
        src[i as usize]
    };
    let weight = |d: f64| (-(d * d) / (2.0 * sigma * sigma)).exp();
    // The two ends stay exactly where they were: the stroke starts and ends on the pointer.
    for i in 1..n.saturating_sub(1) {
        let (mut sum, mut x, mut y) = (0.0, 0.0, 0.0);
        // Walk out while the samples are still within the kernel's reach, both ways.
        let mut first = i as isize;
        while first > i as isize - 128 && (at(first - 1) - src[i]).hypot() <= reach {
            first -= 1;
        }
        let mut last = i as isize;
        while last < i as isize + 128 && (at(last + 1) - src[i]).hypot() <= reach {
            last += 1;
        }
        for m in first..=last {
            let p = at(m);
            let w = weight((p - src[i]).hypot());
            sum += w;
            x += w * p.x;
            y += w * p.y;
        }
        if sum > 0.0
            && let Some(p) = points.get_mut(i)
        {
            *p = Point::new(x / sum, y / sum);
        }
    }
}

struct Fitter<'a> {
    pts: &'a [Point],
    accuracy: f64,
    out: BezPath,
    cubics: usize,
}

impl Fitter<'_> {
    /// Fit `pts[lo..=hi]` with `t0` / `t1` as the travel directions at its ends, appending the
    /// segments to `out`. `false` when the segment budget runs out.
    fn fit(&mut self, lo: usize, hi: usize, t0: Vec2, t1: Vec2) -> bool {
        if hi <= lo {
            return true;
        }
        if hi - lo == 1 {
            self.out.line_to(self.pts[hi]);
            return true;
        }
        if self.cubics >= MAX_CUBICS {
            return false;
        }
        let mut u = self.chord_params(lo, hi);
        let curve = self.generate(lo, hi, &u, t0, t1);
        let (error, split) = self.max_error(lo, hi, &curve);
        // Re-space the samples on the curve and fit again: most misses are a poor parameterisation,
        // not a bad shape. Newton can overshoot (the parameters are kept ordered, which can crowd
        // them), so the best fit seen is the one kept — a step that makes it worse must not cost a
        // segment, because every extra segment is another control point on the stroke.
        let mut best = (curve, error, split);
        for _ in 0..8 {
            if best.1 <= self.accuracy {
                break;
            }
            self.reparameterize(lo, hi, &mut u, &best.0);
            let next = self.generate(lo, hi, &u, t0, t1);
            let (e, s) = self.max_error(lo, hi, &next);
            if e < best.1 {
                best = (next, e, s);
            }
        }
        let (curve, error, split) = best;
        if error <= self.accuracy {
            self.out.curve_to(curve.p1, curve.p2, curve.p3);
            self.cubics += 1;
            return true;
        }
        let split = split.clamp(lo + 1, hi - 1);
        let mid = self.pts[split];
        let left = unit(mid - self.pts[split - 1]);
        let right = unit(self.pts[split + 1] - mid);
        self.fit(lo, split, t0, left) && self.fit(split, hi, right, t1)
    }

    /// Sample parameters by chord length, 0 at the first and 1 at the last.
    fn chord_params(&self, lo: usize, hi: usize) -> Vec<f64> {
        let mut u = Vec::with_capacity(hi - lo + 1);
        u.push(0.0);
        let mut total = 0.0;
        for i in lo + 1..=hi {
            total += (self.pts[i] - self.pts[i - 1]).hypot();
            u.push(total);
        }
        if total > 0.0 {
            for v in &mut u {
                *v /= total;
            }
        } else {
            let n = (hi - lo).max(1) as f64;
            for (i, v) in u.iter_mut().enumerate() {
                *v = i as f64 / n;
            }
        }
        u
    }

    /// How long the run is along its samples (not its chord): a stroke that comes back near where
    /// it started has no chord, but its handles still need a length.
    fn run_length(&self, lo: usize, hi: usize) -> f64 {
        (lo + 1..=hi).map(|i| (self.pts[i] - self.pts[i - 1]).hypot()).sum()
    }

    /// The least-squares cubic through the samples with the end tangents fixed (Schneider's
    /// `generateBezier`): the two handle lengths are solved for, and fall back to a third of the
    /// run when the system is degenerate or asks for a handle that would loop.
    fn generate(&self, lo: usize, hi: usize, u: &[f64], t0: Vec2, t1: Vec2) -> CubicBez {
        let (p0, pn) = (self.pts[lo], self.pts[hi]);
        let chord = (pn - p0).hypot();
        let scale = self.run_length(lo, hi).max(chord).max(SAME);
        let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (k, &uk) in u.iter().enumerate() {
            let om = 1.0 - uk;
            let a1 = t0 * (3.0 * om * om * uk);
            // The end handle is `end - t1 * alpha`, so its basis vector carries the minus sign:
            // solving for it with a plus gave handles of the wrong length and fits an order of
            // magnitude worse than they should be (a quarter arc took two segments instead of one).
            let a2 = t1 * (-3.0 * om * uk * uk);
            c00 += a1.dot(a1);
            c01 += a1.dot(a2);
            c11 += a2.dot(a2);
            // The base curve of the least-squares system is the chord with no handles (P0, P0,
            // Pn, Pn); the solved alpha values are then the handle lengths themselves. Using a
            // base with third-of-the-chord handles made them offsets, so every handle came out
            // short by a third of the chord — arcs flattened and were split into extra segments.
            let base = CubicBez::new(p0, p0, pn, pn).eval(uk);
            let rest = self.pts[lo + k] - base;
            x0 += a1.dot(rest);
            x1 += a2.dot(rest);
        }
        let det = c00 * c11 - c01 * c01;
        let mut al = scale / 3.0;
        let mut ar = scale / 3.0;
        if det.abs() > 1e-12 {
            let l = (x0 * c11 - x1 * c01) / det;
            let r = (c00 * x1 - c01 * x0) / det;
            if l.is_finite() && r.is_finite() {
                al = l;
                ar = r;
            }
        }
        let floor = (scale * 1e-3).max(1e-6);
        let ceiling = (scale * 2.0).max(floor);
        if !(al > floor && ar > floor) {
            al = scale / 3.0;
            ar = scale / 3.0;
        }
        let (al, ar) = (al.clamp(floor, ceiling), ar.clamp(floor, ceiling));
        CubicBez::new(p0, p0 + t0 * al, pn - t1 * ar, pn)
    }

    /// The sample that misses `curve` most, and by how far: `(distance, index)`.
    fn max_error(&self, lo: usize, hi: usize, curve: &CubicBez) -> (f64, usize) {
        let (mut worst, mut at) = (0.0f64, (lo + hi) / 2);
        for i in lo..=hi {
            let d2 = curve.nearest(self.pts[i], 1e-6).distance_sq;
            if d2 > worst {
                worst = d2;
                at = i;
            }
        }
        (worst.sqrt(), at)
    }

    /// One Newton step per interior sample, keeping the parameters ordered.
    fn reparameterize(&self, lo: usize, hi: usize, u: &mut [f64], curve: &CubicBez) {
        let d1 = curve.deriv();
        let d2 = d1.deriv();
        let mut prev = 0.0;
        for i in lo + 1..hi {
            let idx = i - lo;
            let Some(&ui) = u.get(idx) else { continue };
            let diff = curve.eval(ui) - self.pts[i];
            let dq = d1.eval(ui).to_vec2();
            let den = dq.dot(dq) + diff.dot(d2.eval(ui).to_vec2());
            let next = if den.abs() < 1e-12 { ui } else { (ui - diff.dot(dq) / den).clamp(0.0, 1.0) };
            let ordered = next.max(prev + 1e-9).min(1.0 - 1e-9);
            if let Some(slot) = u.get_mut(idx) {
                *slot = ordered;
            }
            prev = ordered;
        }
    }
}
