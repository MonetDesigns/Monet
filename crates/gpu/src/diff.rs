//! CPU ↔ GPU pixel comparison: the acceptance test of every milestone.
//!
//! Both sides hand back **premultiplied RGBA8 at the same size**, which is also what an export writes.
//! The report keeps the three numbers the plan grades on: the largest channel difference, the mean,
//! and how many pixels are worse than the tolerance.

/// How different two images may be and still count as the same picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// A channel inside this difference is fine (`2` = invisible steps of a 0–255 ramp).
    pub per_channel: u8,
    /// The share of pixels allowed to exceed it (0.02 = two percent: the anti-aliased edge band).
    pub max_bad_ratio: f64,
}

impl Tolerance {
    /// The edge band's tolerance: an analytic `tiny-skia` edge and an SDF `fwidth` edge can differ by
    /// a few dozen levels on one pixel, so the edge is graded loosely and the interior strictly.
    pub const EDGE: Self = Self { per_channel: 24, max_bad_ratio: 0.02 };

    /// Interior of a solid fill.
    pub const SOLID: Self = Self { per_channel: 2, max_bad_ratio: 0.0 };

    /// Interior of a gradient (the two sides interpolate in slightly different spaces).
    pub const GRADIENT: Self = Self { per_channel: 6, max_bad_ratio: 0.0 };
}

/// What the comparison found.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub pixels: usize,
    /// The largest single channel difference anywhere.
    pub max_delta: u8,
    /// The mean channel difference over every channel of every pixel.
    pub mean_delta: f64,
    /// Pixels with any channel past the tolerance.
    pub bad: usize,
    pub bad_ratio: f64,
}

impl Report {
    pub fn passes(&self, tolerance: Tolerance) -> bool {
        self.bad_ratio <= tolerance.max_bad_ratio
    }

    /// A sentence for a failed assertion or a log line.
    pub fn summary(&self) -> String {
        format!(
            "{} px, max {}, mean {:.2}, {} over tolerance ({:.2}%)",
            self.pixels,
            self.max_delta,
            self.mean_delta,
            self.bad,
            self.bad_ratio * 100.0
        )
    }
}

/// Compare two premultiplied RGBA8 buffers. `None` when they cannot be compared (different lengths or
/// not a whole number of pixels).
pub fn compare(cpu: &[u8], gpu: &[u8], tolerance: Tolerance) -> Option<Report> {
    if cpu.len() != gpu.len() || cpu.len() % 4 != 0 {
        return None;
    }
    let pixels = cpu.len() / 4;
    let mut max_delta = 0u8;
    let mut sum = 0u64;
    let mut bad = 0usize;
    for (a, b) in cpu.chunks_exact(4).zip(gpu.chunks_exact(4)) {
        let mut pixel_bad = false;
        for k in 0..4 {
            let d = a[k].abs_diff(b[k]);
            sum += u64::from(d);
            max_delta = max_delta.max(d);
            if d > tolerance.per_channel {
                pixel_bad = true;
            }
        }
        if pixel_bad {
            bad += 1;
        }
    }
    let channels = (pixels * 4) as u64;
    Some(Report {
        pixels,
        max_delta,
        mean_delta: if channels == 0 { 0.0 } else { sum as f64 / channels as f64 },
        bad,
        bad_ratio: if pixels == 0 { 0.0 } else { bad as f64 / pixels as f64 },
    })
}

/// `data`'s pixel at `(x, y)`, for a focused assertion.
pub fn pixel(data: &[u8], width: usize, x: usize, y: usize) -> Option<[u8; 4]> {
    let at = (y.checked_mul(width)? + x).checked_mul(4)?;
    let slice = data.get(at..at + 4)?;
    Some([slice[0], slice[1], slice[2], slice[3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_buffers_have_no_difference() {
        let a = vec![10u8, 20, 30, 255, 0, 0, 0, 0];
        let r = compare(&a, &a, Tolerance::SOLID).expect("comparable");
        assert_eq!((r.max_delta, r.bad), (0, 0));
        assert!(r.passes(Tolerance::SOLID));
    }

    #[test]
    fn a_difference_on_one_channel_is_reported() {
        let a = vec![10u8, 20, 30, 255];
        let b = vec![10u8, 20, 30, 200];
        let r = compare(&a, &b, Tolerance::SOLID).expect("comparable");
        assert_eq!(r.max_delta, 55);
        assert_eq!(r.bad, 1);
        assert!(!r.passes(Tolerance::SOLID));
    }

    #[test]
    fn an_edge_only_image_may_fail_the_strict_tolerance() {
        // One pixel of 100 pixels is 1%: inside the edge band's 2%.
        let a = vec![255u8; 400];
        let mut b = a.clone();
        b[0] = 200;
        let r = compare(&a, &b, Tolerance::EDGE).expect("comparable");
        assert!(r.passes(Tolerance::EDGE));
        assert!(!r.passes(Tolerance::SOLID));
    }

    #[test]
    fn mismatched_buffers_are_not_a_comparison() {
        assert!(compare(&[1, 2, 3, 4], &[1, 2, 3], Tolerance::SOLID).is_none());
    }
}
