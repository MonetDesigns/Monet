use super::handles::selected_end_at;
use super::{
    Affine, Document, Gesture, Guide, Node, NodeId, NodeKind, PENCIL_STROKE, PenPath, Point, PointerEvent, Result, Session, StrokeAlign, ToolId,
    Vec2, VectorNetwork, at_45_degrees, frame_at, json, pen_handles,
};
use crate::vectoredit::VTool;

/// Default stroke weight (pt) for a brush stroke. The pencil's 2 pt default reads as a thin line
/// that looks just like a pencil stroke, so a brush starts wider unless the user sets a weight.
const BRUSH_DEFAULT_WEIGHT: f64 = 8.0;

/// How far, in screen pixels, the hand's tremor is averaged out before a freehand stroke is fitted.
/// A pointer's tremor is a pixel or two on screen whatever the zoom, so the kernel is a *screen*
/// distance: a kernel in page units (which is what this used to be) smoothed a stroke four times as
/// hard at 400% as at 100%, and shrank a stroke drawn zoomed in out of existence.
pub(crate) const SMOOTH_PX: f64 = 3.0;

/// How far, in screen pixels, the fitted curve may miss the samples. In page units it did the same
/// thing to detail: at 400% a 1 pt tolerance is 4 screen pixels of corner cutting.
const FIT_PX: f64 = 1.0;

/// The pressure a sample is taken to have when it reports none but the stroke has pressure (a pen
/// that only reports pressure on some events): the middle of the range, which draws the weight.
const NOMINAL_PRESSURE: f64 = 0.5;

/// The narrowest and widest a pressure of 0 / 1 draws, as multiples of the stroke weight.
const PRESSURE_MIN: f64 = 0.1;
const PRESSURE_MAX: f64 = 2.0;

/// How far, in page units, the pressure signal is averaged. A pen's reading is noisy and it is a
/// physical quantity, so the kernel is a distance on the page, not on the screen: a width that
/// follows the signal sample by sample is a jagged edge, not a brush.
const PRESSURE_SMOOTH: f64 = 2.0;

/// The most knots a pressure profile keeps (`WidthProfile::MAX_POINTS` is 64).
const PROFILE_KNOTS: usize = 48;

/// How far the ends of a brush stroke thin out, in screen pixels, and how much a faster stroke
/// lengthens that ramp. A hand that stops moving leaves less ink behind: a slow lift lifts short,
/// a flick carries a long thin tail.
const TAPER_PX: f64 = 8.0;
const TAPER_SPEED_GAIN: f64 = 12.0;
/// The narrowest a tapered end goes, as a multiple of the stroke weight.
const TAPER_MIN: f64 = 0.55;
/// How far the taper may reach into a stroke, as a fraction of its length (a short stroke would
/// otherwise be all taper).
const TAPER_SHARE: f64 = 0.3;

/// The arc length at each point, and the stroke's total length. `None` when the stroke has none.
fn lengths_of(points: &[Point]) -> Option<(Vec<f64>, f64)> {
    let mut lengths = Vec::with_capacity(points.len());
    lengths.push(0.0);
    for w in points.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
        lengths.push(lengths.last().copied().unwrap_or(0.0) + (*b - *a).hypot());
    }
    let total = lengths.last().copied().unwrap_or(0.0);
    (total.is_finite() && total > 0.0).then_some((lengths, total))
}

/// `p` (0..1) as a multiple of the stroke weight: half pressure draws the weight itself, none a
/// hairline, full pressure twice it.
fn scale_of_pressure(p: f64) -> f64 {
    (p / NOMINAL_PRESSURE).clamp(PRESSURE_MIN, PRESSURE_MAX)
}

/// Average `v` along the stroke by a Gaussian of the distance between samples.
fn smooth_signal(v: &[f64], lengths: &[f64], sigma: f64) -> Vec<f64> {
    if v.len() < 3 || !(sigma.is_finite() && sigma > 0.0) || v.len() != lengths.len() {
        return v.to_vec();
    }
    let reach = sigma * 3.0;
    let mut out = Vec::with_capacity(v.len());
    for (i, here) in lengths.iter().enumerate() {
        let (mut sum, mut wsum) = (0.0, 0.0);
        let mut lo = i;
        while lo > 0 && here - lengths[lo - 1] <= reach {
            lo -= 1;
        }
        let mut hi = i;
        while hi + 1 < v.len() && lengths[hi + 1] - here <= reach {
            hi += 1;
        }
        for (m, at) in lengths.iter().enumerate().take(hi + 1).skip(lo) {
            let d = at - here;
            let w = (-(d * d) / (2.0 * sigma * sigma)).exp();
            sum += w * v[m];
            wsum += w;
        }
        out.push(if wsum > 0.0 { sum / wsum } else { v[i] });
    }
    out
}

/// The value of `values` at arc length `s`, interpolated between the samples around it.
fn at_length(values: &[f64], lengths: &[f64], s: f64) -> f64 {
    let i = lengths.partition_point(|l| *l < s).min(lengths.len().saturating_sub(1));
    let (Some(&b), Some(&vb)) = (lengths.get(i), values.get(i)) else { return 1.0 };
    match i.checked_sub(1).and_then(|j| lengths.get(j).zip(values.get(j))) {
        Some((&a, &va)) if b > a => va + (vb - va) * ((s - a) / (b - a)),
        _ => vb,
    }
}

/// The pressure of every point as a multiple of the stroke weight, averaged along the stroke.
/// `None` when no sample carries pressure (a mouse draws a plain stroke).
pub(crate) fn pressure_scales(points: &[Point], pressure: &[Option<f32>]) -> Option<Vec<f64>> {
    if pressure.iter().all(Option::is_none) {
        return None;
    }
    let (lengths, _) = lengths_of(points)?;
    let mut raw: Vec<f64> = (0..points.len())
        .map(|i| pressure.get(i).copied().flatten().filter(|p| p.is_finite()).map_or(NOMINAL_PRESSURE, |p| f64::from(p).clamp(0.0, 1.0)))
        .collect();
    // A pen reports its pressure collapsing as it leaves the tablet. Kept, the stroke ends in a
    // needle the hand never drew; the strike is dropped and the stroke ends where the pen still was.
    let n = raw.len();
    if n >= 2 && raw[n - 1] < 0.08 && raw[n - 2] > 0.25 {
        raw[n - 1] = raw[n - 2];
    }
    let smoothed = smooth_signal(&raw, &lengths, PRESSURE_SMOOTH);
    Some(smoothed.iter().map(|p| scale_of_pressure(*p)).collect())
}

/// How fast the hand was moving at each end of the stroke, in screen pixels per sample: the
/// average step over the first and last few samples. The pointer reports a sample every frame, so
/// the spacing between samples *is* the speed — and measuring it on the screen keeps a stroke drawn
/// at 400% as fast as the same gesture at 100%.
fn end_speeds(points: &[Point], zoom: f64) -> (f64, f64) {
    let n = points.len();
    let step = |i: usize| -> f64 { (points[i] - points[i - 1]).hypot() * zoom };
    let take = n.saturating_sub(1).min(3);
    if take == 0 {
        return (0.0, 0.0);
    }
    let head: f64 = (1..=take).map(step).sum::<f64>() / take as f64;
    let tail: f64 = ((n - take)..n).map(step).sum::<f64>() / take as f64;
    (head, tail)
}

/// The taper along a stroke: 1 in the middle, [`TAPER_MIN`] at the two ends, ramping in over a
/// length that grows with how fast the hand was moving there.
fn taper_scales(points: &[Point], lengths: &[f64], total: f64, zoom: f64) -> Vec<f64> {
    let (head, tail) = end_speeds(points, zoom);
    let reach = (total * zoom * TAPER_SHARE).max(1.0);
    let ramp_len = |speed: f64| (TAPER_PX + TAPER_SPEED_GAIN * speed.clamp(0.0, 4.0)).min(reach).max(1.0);
    let (len_in, len_out) = (ramp_len(head), ramp_len(tail));
    let ramp = |d: f64, len: f64| {
        let u = (d / len).clamp(0.0, 1.0);
        // Smoothstep: a taper with a corner at each end looks like a broken tip.
        let k = u * u * (3.0 - 2.0 * u);
        TAPER_MIN + (1.0 - TAPER_MIN) * k
    };
    lengths.iter().map(|s| ramp(*s * zoom, len_in).min(ramp((total - s) * zoom, len_out))).collect()
}

/// The width of a brush stroke at every sample: the pressure (1.0 when the pen reports none) times
/// the end taper. This is what the preview draws and what the stroke is written with.
pub(crate) fn brush_scales(points: &[Point], pressure: &[Option<f32>], zoom: f64) -> Option<Vec<f64>> {
    let (lengths, total) = lengths_of(points)?;
    let pressure = pressure_scales(points, pressure);
    let taper = taper_scales(points, &lengths, total, zoom);
    Some(taper.iter().enumerate().map(|(i, t)| pressure.as_ref().map_or(1.0, |p| p.get(i).copied().unwrap_or(1.0)) * t).collect())
}

/// The brush stroke's shape as width profile points `[t, scale]`, at most [`PROFILE_KNOTS`] of them,
/// keyed by length along the stroke.
pub(crate) fn brush_profile(points: &[Point], pressure: &[Option<f32>], zoom: f64) -> Option<Vec<[f64; 2]>> {
    let scales = brush_scales(points, pressure, zoom)?;
    profile_knots(points, &scales)
}

/// Pen pressure along drawn samples as width profile points `[t, scale]`: `t` by length, at most
/// [`PROFILE_KNOTS`] points, medium pressure (0.5) at the stroke weight, from a tenth to twice it.
/// `None` when no sample has pressure.
pub(crate) fn pressure_profile(points: &[Point], pressure: &[Option<f32>]) -> Option<Vec<[f64; 2]>> {
    let scales = pressure_scales(points, pressure)?;
    profile_knots(points, &scales)
}

/// `scales` as width profile knots: [`PROFILE_KNOTS`] of them, evenly spaced by length.
fn profile_knots(points: &[Point], scales: &[f64]) -> Option<Vec<[f64; 2]>> {
    let (lengths, total) = lengths_of(points)?;
    let n = points.len().clamp(2, PROFILE_KNOTS);
    Some((0..n).map(|k| {
        let t = k as f64 / (n - 1) as f64;
        [t, at_length(scales, &lengths, t * total)]
    }).collect())
}


/// Drop samples that cannot be a hand movement.
///
/// A fast stroke moves tens of points between two pointer events, so a *long step* is normal and
/// must be kept — dropping it truncated the stroke ("release quickly and the curve is lost"). What
/// a hand cannot do is go far out and straight back: that is a stray pointer position (a captured
/// move over a panel, a display change). So a sample is dropped only when it is the far end of a
/// sharp reversal, or when it teleports past [`TELEPORT`] (coordinates from another space).
pub(crate) fn despike(points: &[Point], pressure: &[Option<f32>]) -> (Vec<Point>, Vec<Option<f32>>) {
    /// A step this long is not a hand movement at any zoom: it is a coordinate from another space.
    const TELEPORT: f64 = 2000.0;
    if points.len() < 3 {
        return (points.to_vec(), pressure.to_vec());
    }
    let mut out = Vec::with_capacity(points.len());
    let mut out_pressure = Vec::with_capacity(points.len());
    out.push(points[0]);
    out_pressure.push(pressure.first().copied().flatten());
    for i in 1..points.len() - 1 {
        let (prev, here, next) = (points[i - 1], points[i], points[i + 1]);
        let (incoming, outgoing) = (here - prev, next - here);
        let (li, lo) = (incoming.hypot(), outgoing.hypot());
        let reversal = li > 1.0 && lo > 1.0 && (incoming.dot(outgoing) / (li * lo)).clamp(-1.0, 1.0) < -0.5;
        // Out and back, and far: the sample is the tip of a spike, so the stroke continues from
        // `prev`. The distance matters — a hand-drawn V turns back too, and its tip must stay.
        if reversal && li > 60.0 && lo > 60.0 {
            continue;
        }
        if li > TELEPORT {
            continue;
        }
        out.push(here);
        out_pressure.push(pressure.get(i).copied().flatten());
    }
    // The release point is where the hand ended up; keep it unless it is a coordinate from
    // another space.
    let last = points[points.len() - 1];
    if out.last().is_none_or(|p| (last - *p).hypot() <= TELEPORT) {
        out.push(last);
        out_pressure.push(pressure.last().copied().flatten());
    }
    (out, out_pressure)
}

/// A frame or section drawn around layers takes those of its siblings that lie fully inside it
/// (visible and unlocked ones, in their order; a frame never takes a section). Not in auto layout,
/// where the new frame is one more item of the flow.
pub(super) fn take_enclosed(doc: &mut Document, drawn: NodeId) -> Result<()> {
    let Some(n) = doc.node(drawn) else { return Ok(()) };
    let section = matches!(n.kind, NodeKind::Section { .. });
    let (Some(parent), Some(area)) = (n.parent.and_then(|p| doc.node(p)), doc.world_bounds(drawn)) else { return Ok(()) };
    if parent.layout.is_some() {
        return Ok(());
    }
    let inside: Vec<NodeId> = parent
        .children
        .iter()
        .copied()
        .filter(|c| *c != drawn && doc.node(*c).is_some_and(|k| k.visible && !k.locked && (section || !matches!(k.kind, NodeKind::Section { .. }))))
        .filter(|c| doc.world_bounds(*c).is_some_and(|b| area.contains(b.origin()) && area.contains(Point::new(b.x1, b.y1))))
        .collect();
    for c in inside {
        doc.reparent(c, drawn, None)?;
    }
    Ok(())
}

impl Session {
    /// The pen tool's snapped position for `ev`: its path's vector (or, before it has one, its first
    /// point) counts among the targets.
    pub(super) fn snap_pen_tool(&mut self, ev: &PointerEvent) -> Result<(Point, Vec<Guide>)> {
        let (id, extra) = match &self.tool.pen {
            Some(pen) if pen.id.is_some() => (pen.id, Vec::new()),
            Some(pen) => (None, pen.points.iter().map(|(p, _, _)| *p).collect()),
            None => (None, Vec::new()),
        };
        self.snap_pen(ev, id, &extra)
    }

    pub(super) fn pen_down(&mut self, ev: &PointerEvent) -> Result<()> {
        self.tool.overlay.guides.clear();
        // The segment being placed shows as itself while the press lasts.
        self.tool.overlay.rubber = None;
        self.tool.overlay.rubber_handle = None;
        let p = if ev.shift { ev.p() } else { self.snap_pen_tool(ev)?.0 };
        let (doc, page) = {
            let ds = self.ds()?;
            (ds.doc.clone(), ds.page)
        };
        // With an open path selected, a click on one of its end points continues it in edit mode.
        if self.tool.pen.is_none()
            && let Some((id, v)) = selected_end_at(&doc, &self.selection(), ev.p(), 8.0 / ev.zoom.max(1e-6))
        {
            return self.edit_vector_with(id, VTool::Pen, Some(v));
        }
        let pen = self.tool.pen.get_or_insert_with(PenPath::default);
        // Clicking the first anchor closes the path.
        if pen.points.len() > 2
            && let Some((first, _, _)) = pen.points.first()
            && (*first - p).hypot() * ev.zoom < 8.0
        {
            pen.closed = true;
            self.pen_update()?;
            return self.pen_finish();
        }
        // Clicking another of its own anchors (not the last) connects there and ends the path.
        let near = |q: &Point| (*q - p).hypot() * ev.zoom < 8.0;
        if let Some(k) = (1..pen.points.len().saturating_sub(1)).find(|k| pen.points.get(*k).is_some_and(|(q, _, _)| near(q))) {
            pen.join = Some(k);
            self.pen_update()?;
            return self.pen_finish();
        }
        // A press on the last anchor (a double-click's second press) adds no point on top of it.
        if pen.points.last().is_some_and(|(q, _, _)| near(q)) {
            return Ok(());
        }
        if pen.parent.is_none() {
            pen.parent = Some(frame_at(&doc, page, p, &[]).unwrap_or(page));
        }
        let mut p = p;
        if ev.shift
            && let Some((last, _, _)) = pen.points.last()
        {
            p = at_45_degrees(*last, p);
        }
        pen.points.push((p, Vec2::ZERO, Vec2::ZERO));
        pen.undone.clear();
        self.tool.gesture = Gesture::PenDrag { anchor: p };
        self.pen_update()
    }

    /// Undo while the pen draws (#1745): its last anchor goes (before two anchors, with the layer
    /// they made; with the only one, the whole path). Returns whether there was a path.
    pub(crate) fn pen_undo(&mut self) -> Result<bool> {
        let Some(pen) = self.tool.pen.as_mut() else { return Ok(false) };
        let Some(last) = pen.points.pop() else {
            self.tool.pen = None;
            return Ok(false);
        };
        pen.undone.push(last);
        let drop_layer = pen.points.len() < 2 && pen.id.take().is_some();
        let empty = pen.points.is_empty();
        self.tool.gesture = Gesture::None;
        self.tool.overlay.rubber = None;
        self.tool.overlay.rubber_handle = None;
        if drop_layer {
            self.cancel_interaction()?;
        }
        if empty {
            self.pen_finish()?;
            return Ok(true);
        }
        self.pen_update()?;
        Ok(true)
    }

    /// Redo while the pen draws: the last anchor undo took back returns. Returns whether there was
    /// one.
    pub(crate) fn pen_redo(&mut self) -> Result<bool> {
        let Some(pen) = self.tool.pen.as_mut() else { return Ok(false) };
        let Some(point) = pen.undone.pop() else { return Ok(false) };
        pen.points.push(point);
        self.tool.gesture = Gesture::None;
        self.pen_update()?;
        Ok(true)
    }

    /// Rebuild the pen's vector node from its anchors.
    pub(super) fn pen_update(&mut self) -> Result<()> {
        let Some(pen) = self.tool.pen.clone() else { return Ok(()) };
        self.tool.overlay.pen = pen.points.iter().map(|(p, _, _)| [p.x, p.y]).collect();
        self.tool.overlay.pen_handles = pen_handles(&pen.points);
        if pen.points.len() < 2 {
            // No node yet (or just one anchor): drop any drawing selection so the toolbar and the
            // per-point highlight don't show before there is geometry to select.
            if let Ok(ds) = self.ds_mut() {
                ds.vector_edit = None;
                ds.revision += 1;
            }
            return Ok(());
        }
        let page = self.page()?;
        let parent = pen.parent.unwrap_or(page);
        // CodeRabbit PR#11 outside-diff: a new Pen vector must use the toolbar's chosen colour and
        // weight; the strokes used to be hard-coded BLACK and never revisited.
        let pen_ink = self.tool.draw_style(ToolId::Pen).map(|s| (s.color(), s.weight(ToolId::Pen)));
        if pen.id.is_none() {
            self.begin_interaction("Pen")?;
        }
        let ds = self.ds_mut()?;
        let doc = ds.doc_mut();
        let pw = doc.world_transform(parent);
        let inv = pw.inverse();
        // Anchors and handles into the parent's space.
        let pts: Vec<(Point, Point, Point)> = pen.points.iter().map(|(p, i, o)| (inv * *p, inv * (*p + *i), inv * (*p + *o))).collect();
        let mut path = figcraft_geom::BezPath::new();
        for (k, (p, _, _)) in pts.iter().enumerate() {
            if k == 0 {
                path.move_to(*p);
            } else if let (Some(prev), Some(cur)) = (pts.get(k - 1), pts.get(k)) {
                path.curve_to(prev.2, cur.1, *p);
            }
        }
        if pen.closed
            && let (Some(last), Some(first)) = (pts.last(), pts.first())
        {
            path.curve_to(last.2, first.1, first.0);
            path.close_path();
        }
        use figcraft_geom::Shape;
        let bb = path.bounding_box();
        path.apply_affine(Affine::translate((-bb.x0, -bb.y0)));
        let mut net = VectorNetwork::from_path(&path);
        // Straight segments stay straight.
        for s in &mut net.segments {
            if (s.ta.0.abs() + s.ta.1.abs()) < 1e-9 && (s.tb.0.abs() + s.tb.1.abs()) < 1e-9 {
                s.ta = (0.0, 0.0);
                s.tb = (0.0, 0.0);
            }
        }
        if !pen.closed {
            net.regions.clear();
        }
        // Anchor k is vertex k of the open path.
        if let (Some(k), Some((last, _, out))) = (pen.join, pts.last()) {
            let from = pts.len() - 1;
            crate::vectoredit::ops::connect(&mut net, from, k, (out.x - last.x, out.y - last.y), (0.0, 0.0));
        }
        let id = match pen.id {
            Some(id) if doc.contains(id) => id,
            _ => {
                let id = doc.alloc_id();
                let mut n = Node::new(id, NodeKind::Vector { network: VectorNetwork::default() }, "Vector", 0.0, 0.0);
                n.fills.clear();
                let (ink, weight) = pen_ink.unwrap_or((figcraft_color::Color::BLACK, 2.0));
                n.strokes = vec![figcraft_color::Paint::solid(ink)];
                n.stroke.weight = weight;
                n.stroke.align = StrokeAlign::Center;
                doc.insert(n, parent, None)?;
                id
            }
        };
        let n = doc.get_mut(id)?;
        n.kind = NodeKind::Vector { network: net };
        n.width = bb.width();
        n.height = bb.height();
        n.transform = Affine::translate((bb.x0, bb.y0));
        if pen.closed && n.fills.is_empty() {
            n.fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::DEFAULT_SHAPE)];
        }
        ds.selection = vec![id];
        ds.revision += 1;
        if let Some(p) = self.tool.pen.as_mut() {
            p.id = Some(id);
        }
        // Drawing selection: the last placed point is selected (Figma: each placed point is
        // selected as you go, and you can keep placing the next). The main tool stays Pen, so
        // pointer routing still goes through `pen_down` and the path keeps growing.
        let last = pen.points.len() - 1;
        if let Ok(ds) = self.ds_mut() {
            let mut ve = ds.vector_edit.take().unwrap_or_else(|| crate::vectoredit::VectorEdit::new(id));
            ve.id = id;
            ve.tool = VTool::Pen;
            ve.vertices = vec![last];
            ve.anchor = Some(last);
            ve.pen_out = None;
            ds.vector_edit = Some(ve);
        }
        Ok(())
    }

    /// Edit `id` as a vector with `tool` (the pen going on from point `anchor`, or, without one,
    /// starting a new path in the same layer at the next click).
    fn edit_vector_with(&mut self, id: NodeId, tool: VTool, anchor: Option<usize>) -> Result<()> {
        self.set_tool(ToolId::Move);
        crate::vectoredit::begin(self, id)?;
        crate::vectoredit::set_tool(self, tool)?;
        crate::vectoredit::set_state(self, |e| {
            e.anchor = anchor;
            e.vertices = anchor.into_iter().collect();
        })
    }

    /// Escape while the pen draws: the path ends but its layer stays in edit mode with the pen, so
    /// the next path joins the same vector (another Escape leaves edit mode).
    pub fn pen_end(&mut self) -> Result<()> {
        self.end_pen_path(VTool::Pen)
    }

    /// Enter (or a double-click) while the pen draws: the path ends and its layer is edited with
    /// the Move tool, its points and handles selectable.
    pub fn pen_enter(&mut self) -> Result<()> {
        self.end_pen_path(VTool::Move)
    }

    /// End the pen's path and edit its layer with `tool` (nothing to edit before a segment).
    fn end_pen_path(&mut self, tool: VTool) -> Result<()> {
        let id = self.tool.pen.as_ref().and_then(|p| p.id);
        self.pen_finish()?;
        match id {
            Some(id) if self.doc()?.contains(id) => self.edit_vector_with(id, tool, None),
            _ => Ok(()),
        }
    }

    /// A double-click with the pen (#1430). While a path is drawn it ends there, as Enter does.
    /// Otherwise the double-click only started a path (its second press adds nothing): that is
    /// dropped, and the layer under the pointer is edited with the Move tool, as a double-click on
    /// it with the Move tool would.
    pub(super) fn pen_double_click(&mut self, ev: &PointerEvent) -> Result<()> {
        if self.tool.pen.as_ref().is_some_and(|p| p.id.is_some()) {
            return self.pen_enter();
        }
        self.pen_finish()?;
        let (doc, page) = {
            let ds = self.ds()?;
            (ds.doc.clone(), ds.page)
        };
        match self.hit(&doc, page, ev.p(), ev.tol() * 2.0).filter(|id| crate::vectoredit::enter_targets(&doc, &[*id]).is_some()) {
            Some(id) => self.edit_vector_with(id, VTool::Move, None),
            None => Ok(()),
        }
    }

    /// Finish the pen path (and keep the pen tool: double-click, Delete, a tool switch).
    pub fn pen_finish(&mut self) -> Result<()> {
        let pen = self.tool.pen.take();
        self.tool.overlay.pen.clear();
        self.tool.overlay.pen_handles.clear();
        self.tool.overlay.rubber = None;
        self.tool.overlay.rubber_handle = None;
        self.tool.gesture = Gesture::None;
        if pen.as_ref().is_some_and(|p| p.id.is_some()) {
            self.commit_interaction()?;
        }
        // A finished pen path leaves the drawing selection behind; `edit_vector_with` (Escape /
        // Enter) re-establishes a proper edit session, so only clear the Pen-drawing one here.
        if let Ok(ds) = self.ds_mut() {
            if ds.vector_edit.as_ref().is_some_and(|ve| ve.tool == VTool::Pen) {
                ds.vector_edit = None;
            }
        }
        Ok(())
    }

    /// A finished freehand stroke: the samples cleaned up, fitted to a vector curve, and the
    /// pressure that made it — one geometry for both the brush and the pen, so the two can never
    /// disagree about where the line went.
    fn freehand(&mut self, points: &[Point], pressure: &[Option<f32>], zoom: f64) -> Option<(figcraft_geom::BezPath, Vec<Point>, Vec<Option<f32>>)> {
        // A hand never teleports: a sample many times the stroke's own pace is a stray pointer
        // position (a captured move over a panel, a display change, the position the button came
        // up at). Fitting a curve through it puts a spike in the stroke — "the circle flies off".
        let (clean, clean_pressure) = despike(points, pressure);
        if clean.len() < 2 {
            return None;
        }
        // Prepare the samples once — deduped, then the tremor averaged out — and fit *and* measure
        // the pressure on that one set. Measuring the profile on the raw samples put it on a
        // different length from the fitted curve, so the taper slid along the stroke instead of
        // sitting where the hand pressed.
        //
        // Both distances are screen distances: a kernel and a tolerance in page units smoothed and
        // rounded the stroke differently at every zoom (four times coarser at 400%).
        let zoom = if zoom.is_finite() && zoom > 1e-6 { zoom } else { 1.0 };
        let sigma = self.smoothing_px();
        let (prepared, from) = figcraft_geom::prepare_samples(&clean, sigma / zoom);
        if prepared.len() < 2 {
            return None;
        }
        // Fit a curve through the samples. kurbo's `simplify_bezpath` only fits across joins whose
        // tangents are nearly parallel, so a hand-drawn stroke came out as one straight line per
        // sample — hundreds of points, visibly faceted. `fit_ready` fits the whole stroke with the
        // fewest cubics that stay within a screen pixel of it, and each segment carries its own
        // curvature handles.
        let fitted = figcraft_geom::fit_ready(&prepared, FIT_PX / zoom)?;
        let carried: Vec<Option<f32>> = from.iter().map(|i| clean_pressure.get(*i).copied().flatten()).collect();
        Some((fitted, prepared, carried))
    }

    /// A finished brush stroke (`ToolId::Brush`): the vector curve, painted with the brush preset
    /// and the pressure taper. Only the brush writes `stroke.brush` and `widthProfile`.
    pub(super) fn brush_finish(&mut self, points: &[Point], pressure: &[Option<f32>], zoom: f64) -> Result<()> {
        let Some((fitted, prepared, carried)) = self.freehand(points, pressure, zoom) else {
            return Ok(());
        };
        let page = self.page()?;
        let parent = frame_at(self.doc()?, page, points.first().copied().unwrap_or_default(), &[]).unwrap_or(page);
        let mut local = fitted.clone();
        local.apply_affine(self.doc()?.world_transform(parent).inverse());
        let style = self.tool.brush_style.clone();
        let weight = self.brush_weight();
        let color = match style.color {
            Some(c) => c,
            None => super::handles::pencil_color(self.doc()?, parent),
        };
        let mut props = json!({"fills": [], "strokes": [color.to_css_hex()], "strokeWeight": weight, "strokeCap": "round", "strokeJoin": "round", "strokeAlign": "center"});
        if let Some(profile) = brush_profile(&prepared, &carried, zoom) {
            props["widthProfile"] = json!(profile);
        }
        if let Some(b) = &style.preset {
            // Flow and opacity ride on the stroke's brush: they shape what the dabs do to each
            // other, so they belong to the stroke, not to the tool that drew it.
            let mut b = b.clone();
            b.flow = style.flow;
            b.opacity = style.opacity;
            props["brush"] = serde_json::to_value(b).unwrap_or_default();
        }
        self.execute(
            "node.create",
            &json!({"type": "vector", "path": local.to_svg(), "closed": false, "parent": parent, "name": "Vector", "props": props}),
        )?;
        Ok(())
    }

    /// A finished pen stroke (`ToolId::Pencil`): one width the whole way — a plain stroked vector
    /// curve. On a board it is board ink (erasable, and the highlighter style is see-through and
    /// multiplied); on a design page it is an ordinary vector stroke.
    pub(super) fn pencil_finish(&mut self, points: &[Point], pressure: &[Option<f32>], zoom: f64) -> Result<()> {
        let Some((fitted, _, _)) = self.freehand(points, pressure, zoom) else {
            return Ok(());
        };
        let page = self.page()?;
        let parent = frame_at(self.doc()?, page, points.first().copied().unwrap_or_default(), &[]).unwrap_or(page);
        let weight = self.pen_weight();
        let color = self.pen_color(parent);
        if crate::board::is_board(self.doc()?) {
            return self.ink_finish(&fitted.to_svg(), self.tool.pencil_kind, color, weight);
        }
        let mut local = fitted.clone();
        local.apply_affine(self.doc()?.world_transform(parent).inverse());
        let props = json!({"fills": [], "strokes": [color.to_css_hex()], "strokeWeight": weight, "strokeCap": "round", "strokeJoin": "round", "strokeAlign": "center"});
        self.execute(
            "node.create",
            &json!({"type": "vector", "path": local.to_svg(), "closed": false, "parent": parent, "name": "Vector", "props": props}),
        )?;
        Ok(())
    }

    /// How far, in screen pixels, the hand's tremor is smoothed before the stroke is fitted: the
    /// stabilizer's amount. Preview and committed stroke both take it from here, so what is under
    /// the pointer is what lands.
    pub(crate) fn smoothing_px(&self) -> f64 {
        let s = self.tool.draw_style(self.tool.tool).map_or(0.0, |d| d.smoothing.clamp(0.0, 1.0));
        SMOOTH_PX * (1.0 + 3.0 * s)
    }

    /// The weight a brush stroke draws with: the tool's own, else the brush's wide default (so an
    /// explicit thin brush still draws thin).
    pub(crate) fn brush_weight(&self) -> f64 {
        self.tool.brush_style.weight.unwrap_or(BRUSH_DEFAULT_WEIGHT)
    }

    /// The weight the pen draws with: the tool's own, else the canvas default (2 pt) or, on a
    /// board, the ink / highlighter default (4 / 20 pt).
    pub(crate) fn pen_weight(&self) -> f64 {
        let (board, _, _) = self.tool.pencil_kind.defaults();
        let fallback = if self.doc().is_ok_and(crate::board::is_board) { board } else { PENCIL_STROKE.1 };
        self.tool.pencil_style.weight.unwrap_or(fallback)
    }

    /// The colour the pen draws with in `parent`: on a board the ink or highlighter colour, on a
    /// page the tool's own (white over a dark background, #1741).
    pub(crate) fn pen_color(&self, parent: NodeId) -> figcraft_color::Color {
        let Ok(doc) = self.doc() else { return PENCIL_STROKE.0 };
        if crate::board::is_board(doc) {
            return match self.tool.pencil_kind {
                crate::tools::PenKind::Ink => self.tool.board.marker,
                crate::tools::PenKind::Highlighter => self.tool.board.highlighter,
            };
        }
        self.tool.pencil_style.color.unwrap_or_else(|| super::handles::pencil_color(doc, parent))
    }

    /// The colour the freehand stroke starting at `p` will have (the canvas previews it).
    pub(super) fn pen_color_at(&self, p: figcraft_geom::Point) -> Result<figcraft_color::Color> {
        let page = self.page()?;
        let doc = self.doc()?;
        let parent = frame_at(doc, page, p, &[]).unwrap_or(page);
        if self.tool.tool == ToolId::Brush {
            return Ok(self.tool.brush_style.color.unwrap_or_else(|| super::handles::pencil_color(doc, parent)));
        }
        Ok(self.pen_color(parent))
    }
}
