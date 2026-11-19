//! WCAG 2.x contrast checking and the Figma-style accessibility layer: the 4 categories × 2
//! levels threshold table, the J3 category classifier, the JJ level picker, alpha compositing
//! before measuring, the inverse boundary used to draw the white "just passes" line on the SV
//! square, and the CIEDE2000-nearest compliant candidate used by the AA autofix button.
//!
//! All colour values here are [`Color`] = non-premultiplied sRGB in `0..=1`. The math mirrors
//! the reference implementation in the Figma colour picker:
//!   - module 434318: `m` threshold table, `J3(_)` classifier, `JJ(f)` level picker, `Ef(g)` ratio
//!   - module 245726: `o/u/d` inverse boundary sweep, `L$` nearest candidate
//!   - module 640649: `x` = CIEDE2000 / 0.002741735

use crate::{Color, Hsb};

// ── threshold table (module 434318 `m`) ─────────────────────────────────────────────────────

/// The four accessibility categories Figma distinguishes. Kept in the same order as the picker's
/// dropdown: auto is resolved to one of these by [`classify`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContrastCategory {
    LargeText,
    NormalText,
    Graphics,
    Mixed,
}

/// The two conformance levels. `graphics` has no AAA threshold (`None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContrastLevel {
    AA,
    AAA,
}

/// Minimum contrast ratio for a `(category, level)` pair. `None` means the level does not apply
/// (graphics has no AAA), which is what greys out "AAA" in the picker dropdown.
pub fn minimum(category: ContrastCategory, level: ContrastLevel) -> Option<f32> {
    match (category, level) {
        (ContrastCategory::NormalText, ContrastLevel::AA) => Some(4.5),
        (ContrastCategory::NormalText, ContrastLevel::AAA) => Some(7.0),
        (ContrastCategory::LargeText, ContrastLevel::AA) => Some(3.0),
        (ContrastCategory::LargeText, ContrastLevel::AAA) => Some(4.5),
        (ContrastCategory::Graphics, ContrastLevel::AA) => Some(3.0),
        (ContrastCategory::Graphics, ContrastLevel::AAA) => None,
        (ContrastCategory::Mixed, ContrastLevel::AA) => Some(3.0),
        (ContrastCategory::Mixed, ContrastLevel::AAA) => Some(4.5),
    }
}

// ── J3 category classifier (`_`) ────────────────────────────────────────────────────────────

/// One text run's metrics, as read from the selected node.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextRun {
    pub font_size: f32,
    pub font_weight: u32,
}

/// `is_text == false` (a shape / icon) → `Graphics`. A text node whose **every** run is at least
/// 24 px, or at least 19 px and bold (weight ≥ 700), is `LargeText`; otherwise `NormalText`.
/// An empty run list falls back to `NormalText`, matching `t && t.length > 0 && t.every(…)`.
pub fn classify(is_text: bool, runs: &[TextRun]) -> ContrastCategory {
    if !is_text {
        return ContrastCategory::Graphics;
    }
    if !runs.is_empty()
        && runs.iter().all(|s| s.font_size >= 24.0 || (s.font_size >= 19.0 && s.font_weight >= 700))
    {
        ContrastCategory::LargeText
    } else {
        ContrastCategory::NormalText
    }
}

// ── JJ level picker (`f`) ───────────────────────────────────────────────────────────────────

/// Resolve the threshold to enforce for a level and a set of selected categories.
///
/// Mirrors `f(e, t)`: with an explicit level the category set only decides the threshold when it
/// is a single category; a mixed selection containing normal text has no single standard; the
/// remaining mixtures fall back to `mixed`'s thresholds.
pub fn resolve(level: ContrastLevel, categories: &[ContrastCategory]) -> Option<(ContrastCategory, Option<f32>)> {
    let mut unique: Vec<ContrastCategory> = Vec::new();
    for &c in categories {
        if !unique.contains(&c) {
            unique.push(c);
        }
    }
    let cat = if unique.len() == 1 {
        Some(unique[0])
    } else if categories.iter().any(|&c| c == ContrastCategory::NormalText) {
        None
    } else {
        Some(ContrastCategory::Mixed)
    };
    cat.map(|c| (c, minimum(c, level)))
}

// ── alpha compositing + ratio (`Ef`) ────────────────────────────────────────────────────────

/// Composite `fg` (non-premultiplied, possibly translucent) over an opaque `bg`: the picker
/// always measures the colour you actually see, so a 50%-alpha foreground is blended first.
pub fn composite(fg: Color, bg: Color) -> Color {
    let a = fg.a;
    Color::new(
        fg.r * a + bg.r * (1.0 - a),
        fg.g * a + bg.g * (1.0 - a),
        fg.b * a + bg.b * (1.0 - a),
        1.0,
    )
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

/// The measured ratio of `fg` over `bg`, rounded to two decimals (Figma shows e.g. `2.79 : 1`).
pub fn contrast_ratio(fg: Color, bg: Color) -> f32 {
    round2(composite(fg, bg).contrast(bg))
}

/// The result the picker shows for a foreground against a background at a required ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContrastResult {
    pub ratio: f32,
    pub aa_pass: bool,
    /// `None` when the category/level pair has no threshold (graphics AAA).
    pub aaa_pass: Option<bool>,
}

/// `Ef(g)`: ratio + pass flags for `fg` over `bg`, with the AA/AAA minima supplied by the table.
pub fn measure(fg: Color, bg: Color, aa_min: f32, aaa_min: Option<f32>) -> ContrastResult {
    let ratio = contrast_ratio(fg, bg);
    ContrastResult {
        ratio,
        aa_pass: ratio >= aa_min,
        aaa_pass: aaa_min.map(|m| ratio >= m),
    }
}

// ── inverse boundary (module 245726 `o/u/d`) ────────────────────────────────────────────────

/// The target relative-luminance boundary on the light or dark side of `bg` for `ratio`.
///
/// `L = ratio·(Lbg + 0.05) − 0.05` (lighter) or `(Lbg + 0.05)/ratio − 0.05` (darker).
pub fn boundary_luminance(bg: Color, ratio: f32, lighter: bool) -> Option<f32> {
    let lbg = bg.luminance();
    let l = if lighter {
        ratio * (lbg + 0.05) - 0.05
    } else {
        (lbg + 0.05) / ratio - 0.05
    };
    if l.is_finite() && (0.0..=1.0).contains(&l) {
        Some(l)
    } else {
        None
    }
}

/// Binary-search the HSV brightness `v` (at fixed hue `h`, saturation `s`) whose fully-opaque
/// colour, composited over `bg`, lands on luminance `target`. `None` when even v=1 cannot reach
/// it (e.g. the light side of a saturated hue).
fn solve_value(h: f32, s: f32, target: f32, lighter: bool) -> Option<f32> {
    let lum = |v: f32| Hsb { h, s, b: v }.to_color().luminance();
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    let l0 = lum(0.0);
    let l1 = lum(1.0);
    let reaches = if lighter { l1 >= target } else { l0 <= target };
    if !reaches {
        return None;
    }
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        let l = lum(mid);
        // Along increasing v, luminance is monotonic. Find the crossing where l meets target;
        // the side (lighter/darker) is already encoded in `target` itself.
        if l < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some((lo + hi) * 0.5)
}

/// A point on the SV square: `s` is saturation 0..1, `v` is brightness 0..1. These map directly
/// to `x = s·328`, `y = (1 − v)·328` in the 328×328 Figma square (scaled to our 240×180 box).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryPoint {
    pub s: f32,
    pub v: f32,
}

/// Sweep saturation 0→1 in 0.02 steps and solve the matching brightness for the light and dark
/// boundary luminances. This is the polyline drawn as the white "just passes" line on the square.
pub fn boundary_line(bg: Color, ratio: f32, hue: f32) -> (Vec<BoundaryPoint>, Vec<BoundaryPoint>) {
    let mut lighter = Vec::new();
    let mut darker = Vec::new();
    let l_light = boundary_luminance(bg, ratio, true);
    let l_dark = boundary_luminance(bg, ratio, false);
    let mut s = 0.0_f32;
    while s <= 1.0 + 1e-6 {
        if let Some(target) = l_dark {
            if let Some(v) = solve_value(hue, s, target, false) {
                darker.push(BoundaryPoint { s, v });
            }
        }
        if let Some(target) = l_light {
            if let Some(v) = solve_value(hue, s, target, true) {
                lighter.push(BoundaryPoint { s, v });
            }
        }
        s += 0.02;
    }
    (lighter, darker)
}

// ── CIEDE2000 (`_W/x`, module 640649) ───────────────────────────────────────────────────────

const XYZ: [[f64; 3]; 3] = [
    [0.4124, 0.3576, 0.1805],
    [0.2126, 0.7152, 0.0722],
    [0.0193, 0.1192, 0.9505],
];
const WHITE: [f64; 3] = [0.95047, 1.0, 1.08883];

fn srgb_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB (composited over white if translucent) → CIE L*a*b* (D65), the input to ΔE2000.
pub fn lab(c: Color) -> [f64; 3] {
    let c = composite(c, Color::WHITE);
    let rgb = [srgb_linear(f64::from(c.r)), srgb_linear(f64::from(c.g)), srgb_linear(f64::from(c.b))];
    let mut xyz = [0.0_f64; 3];
    for i in 0..3 {
        xyz[i] = XYZ[i][0] * rgb[0] + XYZ[i][1] * rgb[1] + XYZ[i][2] * rgb[2];
    }
    let f = |t: f64| {
        if t > 216.0 / 24389.0 {
            t.cbrt()
        } else {
            (24389.0 / 27.0 * t + 16.0) / 116.0
        }
    };
    let fx = f(xyz[0] / WHITE[0]);
    let fy = f(xyz[1] / WHITE[1]);
    let fz = f(xyz[2] / WHITE[2]);
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 colour difference between two colours (composited over white first). Standard
/// Sharma/Wu/Dalal implementation with kL = kC = kH = 1.
pub fn delta_e(c1: Color, c2: Color) -> f64 {
    let [l1, a1, b1] = lab(c1);
    let [l2, a2, b2] = lab(c2);
    let deg = std::f64::consts::PI / 180.0;
    let c1ab = (a1 * a1 + b1 * b1).sqrt();
    let c2ab = (a2 * a2 + b2 * b2).sqrt();
    let c_bar = (c1ab + c2ab) * 0.5;
    let g = 0.5 * (1.0 - (c_bar.powi(7) / (c_bar.powi(7) + 25.0_f64.powi(7))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();
    let h1p = if c1p == 0.0 { 0.0 } else { b1.atan2(a1p).rem_euclid(2.0 * std::f64::consts::PI) / deg };
    let h2p = if c2p == 0.0 { 0.0 } else { b2.atan2(a2p).rem_euclid(2.0 * std::f64::consts::PI) / deg };
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p <= h1p {
        h2p - h1p + 360.0
    } else {
        h2p - h1p - 360.0
    };
    let dh_cap = 2.0 * (c1p * c2p).sqrt() * ((dh * deg) * 0.5).sin();
    let l_bar = (l1 + l2) * 0.5;
    let c_bar_p = (c1p + c2p) * 0.5;
    let h_bar_p = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) * 0.5
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) * 0.5
    } else {
        (h1p + h2p - 360.0) * 0.5
    };
    let t = 1.0 - 0.17 * (h_bar_p - 30.0).cos() + 0.24 * (2.0 * h_bar_p).cos() + 0.32 * (3.0 * h_bar_p + 6.0).cos()
        - 0.20 * (4.0 * h_bar_p - 63.0).cos();
    let dtheta = 30.0 * (-(((h_bar_p - 275.0) / 25.0).powi(2))).exp();
    let rc = 2.0 * (c_bar_p.powi(7) / (c_bar_p.powi(7) + 25.0_f64.powi(7))).sqrt();
    let sl = 1.0 + (0.015 * (l_bar - 50.0).powi(2)) / (20.0 + (l_bar - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * c_bar_p;
    let sh = 1.0 + 0.015 * c_bar_p * t;
    let rt = -rc * (2.0 * dtheta * deg).sin();
    let term_l = dl / sl;
    let term_c = dc / sc;
    let term_h = dh_cap / sh;
    (term_l * term_l + term_c * term_c + term_h * term_h + rt * term_c * term_h).sqrt()
}

/// `x`: ΔE2000 normalised by ~1% grey-step in Lab, the "perceptual steps" the candidate picker
/// minimises.
pub fn perceptual_delta(c1: Color, c2: Color) -> f64 {
    delta_e(c1, c2) / 0.002741735
}

// ── `L$`: the nearest compliant colour on the boundary ──────────────────────────────────────

/// Pick the colour that just passes `ratio` (plus a small margin) with hue locked, minimising
/// perceptual distance from `fg`. This is what makes the dot "run to the nearest point on the
/// white line" when AA is clicked. `None` when no boundary point is reachable in gamut.
pub fn autocorrect_candidate(fg: Color, bg: Color, ratio: f32, hue: f32) -> Option<Color> {
    // e + .05 gives a safety margin so the result is comfortably over the line, never 4.4999.
    let (lighter, darker) = boundary_line(bg, ratio + 0.05, hue);
    let mut best: Option<(f64, Color)> = None;
    for p in lighter.iter().chain(darker.iter()) {
        let rgb = Hsb { h: hue, s: p.s, b: p.v }.to_color();
        let d = perceptual_delta(fg, rgb);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, rgb));
        }
    }
    best.map(|(_, c)| c)
}

#[cfg(test)]
mod contrast_tests {
    use super::*;

    #[test]
    fn threshold_table_matches_figma() {
        assert_eq!(minimum(ContrastCategory::NormalText, ContrastLevel::AA), Some(4.5));
        assert_eq!(minimum(ContrastCategory::NormalText, ContrastLevel::AAA), Some(7.0));
        assert_eq!(minimum(ContrastCategory::LargeText, ContrastLevel::AA), Some(3.0));
        assert_eq!(minimum(ContrastCategory::LargeText, ContrastLevel::AAA), Some(4.5));
        assert_eq!(minimum(ContrastCategory::Graphics, ContrastLevel::AA), Some(3.0));
        assert_eq!(minimum(ContrastCategory::Graphics, ContrastLevel::AAA), None);
        assert_eq!(minimum(ContrastCategory::Mixed, ContrastLevel::AA), Some(3.0));
        assert_eq!(minimum(ContrastCategory::Mixed, ContrastLevel::AAA), Some(4.5));
    }

    #[test]
    fn classifier_j3() {
        assert_eq!(classify(false, &[]), ContrastCategory::Graphics);
        assert_eq!(classify(true, &[]), ContrastCategory::NormalText);
        assert_eq!(classify(true, &[TextRun { font_size: 24.0, font_weight: 400 }]), ContrastCategory::LargeText);
        assert_eq!(classify(true, &[TextRun { font_size: 19.0, font_weight: 700 }]), ContrastCategory::LargeText);
        assert_eq!(classify(true, &[TextRun { font_size: 19.0, font_weight: 400 }]), ContrastCategory::NormalText);
        // every() — one small run drags the whole text to normal.
        assert_eq!(
            classify(true, &[TextRun { font_size: 24.0, font_weight: 400 }, TextRun { font_size: 12.0, font_weight: 700 }]),
            ContrastCategory::NormalText
        );
    }

    #[test]
    fn level_picker_jj() {
        assert_eq!(resolve(ContrastLevel::AA, &[ContrastCategory::Graphics]), Some((ContrastCategory::Graphics, Some(3.0))));
        assert_eq!(resolve(ContrastLevel::AAA, &[ContrastCategory::Graphics]), Some((ContrastCategory::Graphics, None)));
        assert_eq!(
            resolve(ContrastLevel::AA, &[ContrastCategory::NormalText, ContrastCategory::LargeText]),
            None
        );
        assert_eq!(
            resolve(ContrastLevel::AAA, &[ContrastCategory::LargeText, ContrastCategory::Graphics]),
            Some((ContrastCategory::Mixed, Some(4.5)))
        );
    }

    #[test]
    fn wcag_reference_ratios() {
        // WCAG's canonical examples.
        assert_eq!(contrast_ratio(Color::BLACK, Color::WHITE), 21.0);
        assert_eq!(contrast_ratio(Color::WHITE, Color::WHITE), 1.0);
        let gray = Color::from_rgb8_const(0x77, 0x77, 0x77);
        let r = contrast_ratio(gray, Color::WHITE);
        assert!(r > 4.4 && r < 4.6, "gray 777777 on white ≈ 4.48, got {r}");
    }

    #[test]
    fn alpha_composite_before_measure() {
        // A fully transparent foreground is just the background → ratio 1.
        let clear = Color::TRANSPARENT;
        assert_eq!(contrast_ratio(clear, Color::WHITE), 1.0);
    }

    #[test]
    fn boundary_and_candidate_land_on_the_line() {
        let bg = Color::WHITE;
        let fg = Color::from_rgb8_const(0xE6, 0x4D, 0x4D);
        let hsb = Hsb::from_color(fg);
        let cand = autocorrect_candidate(fg, bg, 4.5, hsb.h).expect("a candidate must exist on white for 4.5");
        // The corrected colour comfortably passes AA, and the hue is preserved.
        assert!(contrast_ratio(cand, bg) >= 4.5, "candidate must pass 4.5:1");
        let cand_h = Hsb::from_color(cand).h;
        let dh = (cand_h - hsb.h).abs().min(360.0 - (cand_h - hsb.h).abs());
        assert!(dh < 1.0, "hue must stay locked, drift {dh}");
    }

    #[test]
    fn ciede2000_identity_and_symmetry() {
        let a = Color::from_rgb8_const(0x0D, 0x99, 0xFF);
        let b = Color::from_rgb8_const(0xE9, 0x1E, 0x63);
        assert!(delta_e(a, a) < 1e-9);
        assert!((delta_e(a, b) - delta_e(b, a)).abs() < 1e-6);
        assert!(delta_e(a, b) > 20.0);
    }
}
