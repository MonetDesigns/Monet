//! Brushes and pen pressure through the session: `stroke.brush`, ABR tip compositing and
//! outlining brush strokes, and the Pencil recording pressure (and its brush) into the stroke it
//! draws.

use serde_json::{Value, json};

use crate::Session;
use crate::ToolId;
use crate::params::id_of;
use crate::tools::{PenKind, PointerEvent, PointerKind, pressure_profile};
use figcraft_doc::{NodeId, NodeKind};
use figcraft_geom::Point;

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

/// A 200-long horizontal vector with a 10 px black stroke.
fn line(s: &mut Session) -> NodeId {
    let v = id(s.execute("vector.create", &json!({"path": "M0 0 L200 0", "x": 0, "y": 0})).unwrap());
    s.execute("node.set", &json!({"id": v, "props": {"strokes": ["#000000"], "strokeWeight": 10, "strokeAlign": "center"}})).unwrap();
    v
}

/// The alpha of the page's layers at page point (`x`, `y`), rendered at 1x without the page colour.
fn alpha(s: &Session, x: f64, y: f64) -> u8 {
    let doc = s.doc().unwrap();
    let view = figcraft_render::View { origin: (x - 1.0, y - 1.0), scale: 1.0, width: 3, height: 3 };
    let mut pix = figcraft_render::Pixmap::new(3, 3).unwrap();
    figcraft_render::render_nodes_into(doc, &doc.node(s.page().unwrap()).unwrap().children, &view, &mut pix);
    pix.data()[(3 + 1) * 4 + 3]
}

/// Install a tiny external `.abr` library (round sampled tips) for the renderer to resolve. The
/// names are the eight legacy ids, so documents saved before ABR support keep painting.
fn install_test_brushes() {
    use figcraft_abr::{AbrSample, BrushLibrary, LegacyBrush, LegacyTip, install, write_v12};
    let disk: Vec<u8> = (0..256)
        .map(|i| {
            let (x, y) = ((i % 16) as f64 - 7.5, (i / 16) as f64 - 7.5);
            if x * x + y * y <= 36.0 { 255 } else { 0 }
        })
        .collect();
    let brush = |name: &str| LegacyBrush {
        name: name.to_string(),
        spacing: 20,
        anti_alias: true,
        tip: LegacyTip::Sampled(AbrSample { id: String::new(), width: 16, height: 16, depth: 8, data: disk.clone() }),
    };
    let names = ["taper", "ink", "ribbon", "bristle", "dots", "stars", "confetti", "leaves"];
    let brushes: Vec<LegacyBrush> = names.iter().map(|n| brush(n)).collect();
    let bytes = write_v12(2, &brushes, false).unwrap();
    let mut lib = BrushLibrary::empty();
    assert_eq!(lib.add_bytes(&bytes, "monet-defaults").unwrap(), 8);
    install(lib);
}

#[test]
fn brushes_draw_along_the_stroke_with_external_abr_tips() {
    install_test_brushes();
    let mut s = Session::new();
    let v = line(&mut s);
    assert_eq!(alpha(&s, 15.0, 0.0), 255, "a plain stroke covers the line");
    assert!(s.execute("stroke.brush", &json!({"ids": [v], "preset": ""})).is_err(), "an empty brush id is rejected");

    // The document's spacing override: one 10 px tip every 20 units leaves a gap at 10.
    s.execute("stroke.brush", &json!({"ids": [v], "preset": "dots", "spacing": 2})).unwrap();
    assert_eq!(alpha(&s, 0.0, 0.0), 255, "an ABR tip paints a dab at the stroke's start");
    assert_eq!(alpha(&s, 10.0, 0.0), 0, "the gap between two dabs stays clear");
    let b = s.execute("node.get", &json!({"id": v})).unwrap()["brush"].clone();
    assert_eq!((b["preset"].as_str(), b["spacing"].as_f64()), (Some("dots"), Some(2.0)), "{b}");

    // A legacy id from a document saved before ABR support resolves to the factory preset.
    s.execute("stroke.brush", &json!({"ids": [v], "preset": "taper"})).unwrap();
    assert_eq!(alpha(&s, 0.0, 0.0), 255, "legacy ids still paint");

    // Outlining turns the stroke into a filled vector (the geometric stand-in), still visible.
    s.execute("vector.outlineStroke", &json!({"ids": [v]})).unwrap();
    let doc = s.doc().unwrap();
    let outlined = doc.nodes().find(|n| matches!(n.kind, NodeKind::Vector { .. }) && !n.fills.is_empty()).expect("an outlined vector");
    assert!(outlined.strokes.is_empty() || outlined.stroke.brush.is_none());
    assert_eq!(alpha(&s, 100.0, 0.0), 255, "the outlined stroke stays visible");

    // Removing the brush: a plain stroke again.
    let mut s = Session::new();
    let v = line(&mut s);
    s.execute("stroke.brush", &json!({"ids": [v], "preset": "dots", "spacing": 2})).unwrap();
    s.execute("stroke.brush", &json!({"ids": [v], "preset": null})).unwrap();
    assert_eq!(alpha(&s, 10.0, 0.0), 255, "a plain stroke covers the gap");
    // Exports draw it too (brush strokes are rasterised by the renderer, exported as a path).
    s.execute("stroke.brush", &json!({"ids": [v], "preset": "stars"})).unwrap();
    let doc = s.doc().unwrap();
    assert!(figcraft_svg::export_svg(doc, &[v], &figcraft_svg::SvgOptions::default()).unwrap().contains("<path"));
    assert!(figcraft_pdf::export_nodes_pdf(doc, &[v]).is_ok());
}

#[test]
fn brush_commands_list_and_import_the_external_library() {
    install_test_brushes();
    let mut s = Session::new();
    let listed = s.execute("brush.list", &json!({})).unwrap();
    assert_eq!(listed["brushes"].as_array().map(Vec::len), Some(8), "{listed}");
    assert!(listed["brushes"][0]["id"].as_str().is_some(), "every brush has a stable ABR id");
    // Don't run `brush.reload` here: it rescans the real user directory and would replace the
    // in-memory test library while the rendering test above is using it (tests run in parallel).
    assert!(crate::cmd::find_command("brush.reload").is_some());
    assert!(s.execute("brush.import", &json!({})).is_err(), "import needs a path");
}

/// The three freehand tools are three tools, not one tool wearing two hats: the brush has its own
/// preset slot, the pen refuses one, and the old marker / highlighter names still land on the pen
/// in the right style.
#[test]
fn the_brush_owns_the_preset_and_the_old_names_are_pen_styles() {
    let mut s = Session::new();
    let set = |s: &mut Session, tool: &str| s.execute("tool.set", &json!({ "tool": tool })).unwrap()["tool"].clone();
    assert_eq!(set(&mut s, "brush"), json!("brush"));
    assert_eq!(s.tool.tool, ToolId::Brush);
    assert!(s.execute("tool.drawStyle", &json!({ "tool": "pencil", "preset": "ink" })).is_err(), "the pen has no brush preset");
    assert!(s.execute("tool.drawStyle", &json!({ "tool": "brush", "preset": "ink" })).is_ok());

    for (name, kind) in [("marker", PenKind::Ink), ("highlighter", PenKind::Highlighter), ("pencil", PenKind::Ink)] {
        assert_eq!(set(&mut s, name), json!("pencil"), "{name} is the pen");
        assert_eq!(s.tool.tool, ToolId::Pencil);
        if name != "pencil" {
            assert_eq!(s.tool.pencil_kind, kind, "{name} picks the style");
        }
    }
}

/// Flow and opacity are the paint program's split: flow is how much one dab carries (so a stroke
/// that doubles back over itself builds up), opacity is the ceiling over the whole stroke.
#[test]
fn flow_builds_up_and_opacity_caps_the_stroke() {
    install_test_brushes();
    let at_crossing = |extra: serde_json::Value, ink: &str| -> u8 {
        let mut s = Session::new();
        let v = id(s.execute("vector.create", &json!({"path": "M0 0 L60 0 L0 0", "x": 0, "y": 0})).unwrap());
        s.execute("node.set", &json!({"id": v, "props": {"strokes": [ink], "strokeWeight": 10, "strokeAlign": "center"}})).unwrap();
        // One 10 pt tip every 20 units: the way back lands dabs exactly on the way out's.
        let mut q = json!({"ids": [v], "preset": "dots", "spacing": 2.0});
        for (k, val) in extra.as_object().expect("an object").clone() {
            q[k] = val;
        }
        s.execute("stroke.brush", &q).unwrap();
        alpha(&s, 40.0, 0.0)
    };
    assert_eq!(at_crossing(json!({}), "#000000"), 255, "two full dabs paint solid");
    let flow = at_crossing(json!({"flow": 0.5}), "#000000");
    assert!(flow > 120 && flow < 240, "half flow over itself is denser than one pass but not solid: {flow}");
    // Opacity is the stroke colour's alpha now, not a brush field: #00000080 is 50 % ink.
    assert_eq!(at_crossing(json!({}), "#00000080"), 128, "the stroke colour's alpha caps the stroke");
}

/// A brush stroke lifts its tip at both ends, and a faster flick carries a longer, thinner tail:
/// the ramp grows with how far the pointer moved between samples.
#[test]
fn a_brush_stroke_tapers_at_the_ends() {
    let stroke = |step: f64| -> figcraft_doc::WidthProfile {
        let mut s = Session::new();
        s.execute("tool.set", &json!({"tool": "brush"})).unwrap();
        let mut x = 0.0;
        s.pointer(&PointerEvent::at(PointerKind::Down, x, 40.0, 1.0)).unwrap();
        while x < 240.0 {
            x += step;
            s.pointer(&PointerEvent::at(PointerKind::Move, x, 40.0, 1.0)).unwrap();
        }
        s.pointer(&PointerEvent::at(PointerKind::Up, x, 40.0, 1.0)).unwrap();
        let doc = s.doc().unwrap();
        let n = doc.nodes().find(|n| matches!(n.kind, NodeKind::Vector { .. })).expect("a brush stroke");
        n.stroke.profile().expect("the brush tapers").clone()
    };
    let slow = stroke(1.0);
    let fast = stroke(8.0);
    assert!(slow.scale_at(0.0) < 0.6, "the slow stroke lifts its tip: {:?}", slow.scale_at(0.0));
    assert!(slow.scale_at(0.5) > 0.98, "and is full width in the middle: {:?}", slow.scale_at(0.5));
    assert!(fast.scale_at(0.15) < slow.scale_at(0.15) - 0.1, "a flick tapers farther in: {:?} vs {:?}", fast.scale_at(0.15), slow.scale_at(0.15));
}

/// The stabilizer is input handling: the same shaky hand lands a smoother stroke with it up, and
/// it never reaches the document (no extra property, just a calmer curve).
#[test]
fn the_stabilizer_smooths_the_stroke() {
    let height = |smoothing: f64| -> f64 {
        let mut s = Session::new();
        s.execute("tool.set", &json!({"tool": "brush"})).unwrap();
        s.execute("tool.drawStyle", &json!({"tool": "brush", "smoothing": smoothing})).unwrap();
        let mut x = 0.0;
        s.pointer(&PointerEvent::at(PointerKind::Down, x, 40.0, 1.0)).unwrap();
        while x < 120.0 {
            x += 2.0;
            // A hand's tremor: a few points either side of the line.
            let y = 40.0 + if (x / 2.0) as i32 % 2 == 0 { 5.0 } else { -5.0 };
            s.pointer(&PointerEvent::at(PointerKind::Move, x, y, 1.0)).unwrap();
        }
        s.pointer(&PointerEvent::at(PointerKind::Up, x, 40.0, 1.0)).unwrap();
        let doc = s.doc().unwrap();
        doc.nodes().find(|n| matches!(n.kind, NodeKind::Vector { .. })).expect("a stroke").height
    };
    let (raw, steady) = (height(0.0), height(1.0));
    assert!(steady < raw * 0.7, "the stabilizer flattens the shake: {raw} vs {steady}");
}

#[test]
fn pressure_becomes_a_width_profile() {
    let pts: Vec<Point> = (0..=10).map(|i| Point::new(f64::from(i) * 10.0, 0.0)).collect();
    assert!(pressure_profile(&pts, &[None; 11]).is_none(), "a mouse: a plain stroke");
    let light_to_hard: Vec<Option<f32>> = (0..=10).map(|i| Some(i as f32 / 10.0)).collect();
    let prof = pressure_profile(&pts, &light_to_hard).unwrap();
    assert_eq!(prof.len(), 11);
    assert_eq!(prof.first(), Some(&[0.0, 0.1]), "no pressure still draws a hairline");
    assert_eq!(prof.get(5), Some(&[0.5, 1.0]), "medium pressure: the stroke weight");
    assert_eq!(prof.last(), Some(&[1.0, 2.0]));
    assert!(pressure_profile(&[Point::ZERO, Point::ZERO], &[Some(0.5), Some(0.5)]).is_none(), "no length");
}

/// The profile is keyed by *length along the stroke*, not by sample index: a fast stroke has few
/// samples where a slow one has many, and a profile keyed by index would put the pressure in the
/// wrong place. This stroke presses hard for its first 9 pt (9 of 19 samples — nearly half the
/// indices) and then runs 90 pt light: measured by length the hard part is 9% of the profile,
/// measured by index it would be 47%.
#[test]
fn pressure_is_placed_by_length_along_the_stroke() {
    let dense: Vec<f64> = (0..=9).map(f64::from).collect();
    let tail: Vec<f64> = (1..=10).map(|i| f64::from(i) * 10.0).collect();
    let xs: Vec<f64> = dense.iter().chain(tail.iter()).copied().collect();
    let pts: Vec<Point> = xs.iter().map(|x| Point::new(*x, 0.0)).collect();
    let pressure: Vec<Option<f32>> = xs.iter().map(|x| Some(if *x <= 9.0 { 1.0 } else { 0.0 })).collect();
    let prof = pressure_profile(&pts, &pressure).unwrap();
    assert_eq!(prof.first(), Some(&[0.0, 2.0]), "full pressure at the start");
    let light = prof.iter().find(|p| p[1] < 0.5).expect("the stroke ends light");
    assert!(light[0] < 0.25, "light within the first quarter of the stroke, not at half: {prof:?}");
    assert_eq!(prof.last(), Some(&[1.0, 0.1]), "no pressure at the end");
}

/// A pen reports its pressure collapsing as it leaves the tablet. That last reading is not a
/// gesture: a stroke must not end in a needle the hand never drew.
#[test]
fn the_lift_does_not_end_the_stroke_in_a_needle() {
    let pts: Vec<Point> = (0..=10).map(|i| Point::new(f64::from(i) * 10.0, 0.0)).collect();
    let mut pressure: Vec<Option<f32>> = (0..=10).map(|_| Some(0.5)).collect();
    pressure[10] = Some(0.0);
    let prof = pressure_profile(&pts, &pressure).unwrap();
    assert!(prof.last().is_some_and(|p| p[1] > 0.9), "the stroke ends at the pen's last pressure: {prof:?}");
}

/// A pencil without a brush is one width the whole way: swelling under pressure is the brush's
/// job, and the brush is its own tool.
#[test]
fn a_pencil_without_a_brush_ignores_pressure() {
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    let ev = |kind, x: f64, p: f32| PointerEvent { pressure: Some(p), ..PointerEvent::at(kind, x, 20.0, 1.0) };
    s.pointer(&ev(PointerKind::Down, 0.0, 0.1)).unwrap();
    for i in 1..=20 {
        s.pointer(&ev(PointerKind::Move, f64::from(i) * 10.0, 0.1 + 0.04 * i as f32)).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, 200.0, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    let n = doc.nodes().find(|n| matches!(n.kind, NodeKind::Vector { .. })).expect("a pencil stroke");
    assert!(n.stroke.profile().is_none(), "a pencil stroke keeps one width");
    assert!(n.stroke.brush.is_none());
}

/// The hand's tremor is a distance on the *screen*, so a stroke drawn at 400% must come out the
/// same as the same gesture at 100% — a page-unit kernel made the zoomed-in one four times as
/// smooth (and a page-unit fit tolerance four times as coarse), which is what made the line feel
/// different at every zoom.
#[test]
fn the_same_hand_at_any_zoom_makes_the_same_stroke() {
    // The stroke's width, in screen points, drawn at `zoom`: the page coordinates are the screen
    // ones over the zoom.
    let drawn_at = |zoom: f64| -> f64 {
        let mut s = Session::new();
        s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
        let ev = |kind, x: f64| PointerEvent::at(kind, x / zoom, (60.0 + (x / 26.0).sin() * 12.0) / zoom, zoom);
        s.pointer(&ev(PointerKind::Down, 0.0)).unwrap();
        let mut x = 0.0;
        while x < 240.0 {
            x += 1.5;
            s.pointer(&ev(PointerKind::Move, x)).unwrap();
        }
        s.pointer(&ev(PointerKind::Up, x)).unwrap();
        let doc = s.doc().unwrap();
        let n = doc.nodes().find(|n| matches!(n.kind, NodeKind::Vector { .. })).expect("a stroke");
        n.width * zoom
    };
    let (one, four) = (drawn_at(1.0), drawn_at(4.0));
    let (lo, hi) = (one.min(four), one.max(four));
    assert!(hi - lo <= 0.01 * hi, "the same stroke at 100% and 400%: {one} vs {four}");
}

/// Pressure has to reach the pixels, not just the document: the same stroke drawn light and hard
/// must paint different widths. This is the whole chain — pressure → `widthProfile` → the dab size
/// of every ABR stamp (`pathops::brush::stamps`) → the composited mask.
#[test]
fn pressure_reaches_the_painted_width() {
    install_test_brushes();
    let stroke = |pressure: f32| -> u8 {
        let mut s = Session::new();
        s.execute("tool.set", &json!({"tool": "brush"})).unwrap();
        s.execute("tool.brushPreset", &json!({"preset": "taper"})).unwrap();
        let ev = |kind, x: f64| PointerEvent { pressure: Some(pressure), ..PointerEvent::at(kind, x, 0.0, 1.0) };
        s.pointer(&ev(PointerKind::Down, 0.0)).unwrap();
        for i in 1..=10 {
            s.pointer(&ev(PointerKind::Move, f64::from(i) * 10.0)).unwrap();
        }
        s.pointer(&ev(PointerKind::Up, 100.0)).unwrap();
        // 3 pt off the centre line: inside a hard stroke's 8 pt tip, outside a light one's.
        alpha(&s, 50.0, 3.0)
    };
    assert_eq!(stroke(1.0), 255, "hard pressure paints wide");
    assert_eq!(stroke(0.2), 0, "light pressure paints narrow");
}

#[test]
fn the_brush_records_pressure_and_the_pen_does_not() {
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "brush"})).unwrap();
    assert!(s.execute("tool.brushPreset", &json!({"preset": ""})).is_err(), "an empty brush id is rejected");
    s.execute("tool.brushPreset", &json!({"preset": "taper"})).unwrap();
    let ev = |kind, x: f64, p: f32| PointerEvent { pressure: Some(p), ..PointerEvent::at(kind, x, 50.0 + (x / 20.0).sin() * 10.0, 1.0) };
    s.pointer(&ev(PointerKind::Down, 0.0, 0.2)).unwrap();
    for i in 1..=40 {
        s.pointer(&ev(PointerKind::Move, f64::from(i) * 5.0, 0.2 + 0.02 * i as f32)).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, 200.0, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    let n = doc.nodes().find(|n| matches!(n.kind, NodeKind::Vector { .. })).expect("a brush stroke");
    let prof = n.stroke.profile().expect("pressure recorded");
    // Light at the start, hard three quarters of the way in (the pen presses as it goes), and the
    // tip lifting again at the very end: pressure times the end taper.
    assert!(prof.scale_at(0.0) < 0.5, "light at the start: {prof:?}");
    assert!(prof.scale_at(0.75) > 1.5, "hard by three quarters: {prof:?}");
    assert!(prof.scale_at(1.0) < prof.scale_at(0.75), "the end lifts: {prof:?}");
    assert_eq!(n.stroke.brush.as_ref().map(|b| b.preset.as_str()), Some("taper"));
    // The same hand with the pen: one width, no brush, and the pen keeps its own style.
    s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    let before = s.doc().unwrap().nodes().count();
    s.pointer(&PointerEvent::at(PointerKind::Down, 0.0, 200.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 50.0, 220.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 100.0, 200.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 100.0, 200.0, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(doc.nodes().count(), before + 1);
    let last = doc.nodes().filter(|n| matches!(n.kind, NodeKind::Vector { .. })).max_by_key(|n| n.id).unwrap();
    assert!(last.stroke.profile().is_none() && last.stroke.brush.is_none(), "the pen has no brush and no taper");
}

/// The Pencil draws black on a light page and white on a dark page or inside a dark frame, the
/// rule new text follows (#1741; Figma: "in black, unless you're sketching on a dark canvas or
/// frame"), and the stroke in progress previews in that colour.
#[test]
fn the_pencil_draws_white_on_a_dark_background() {
    use figcraft_color::Color;
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    // A stroke from (x, y); its colour, and the preview colour the overlay carried while drawing.
    let stroke = |s: &mut Session, x: f64, y: f64| {
        s.pointer(&PointerEvent::at(PointerKind::Down, x, y, 1.0)).unwrap();
        s.pointer(&PointerEvent::at(PointerKind::Move, x + 40.0, y + 10.0, 1.0)).unwrap();
        let preview = s.tool.overlay.pen_color;
        s.pointer(&PointerEvent::at(PointerKind::Move, x + 80.0, y, 1.0)).unwrap();
        s.pointer(&PointerEvent::at(PointerKind::Up, x + 80.0, y, 1.0)).unwrap();
        assert!(s.tool.overlay.pen_color.is_none(), "no preview colour after the stroke");
        let doc = s.doc().unwrap();
        let last = doc.nodes().filter(|n| matches!(n.kind, NodeKind::Vector { .. })).max_by_key(|n| n.id).unwrap();
        (last.strokes.first().and_then(|p| p.solid_color()).unwrap(), preview)
    };
    assert_eq!(stroke(&mut s, 0.0, 0.0), (Color::BLACK, Some(Color::BLACK)), "a light page");
    s.execute("page.setBackground", &json!({"color": "#1E1E1E"})).unwrap();
    assert_eq!(stroke(&mut s, 0.0, 100.0), (Color::WHITE, Some(Color::WHITE)), "a dark page");
    // A dark frame on a light page, and a light frame on a dark page.
    s.execute("page.setBackground", &json!({"color": "#F5F5F5"})).unwrap();
    s.execute("node.create", &json!({"type": "frame", "x": 300, "y": 300, "width": 200, "height": 200, "props": {"fills": ["#202020"]}})).unwrap();
    assert_eq!(stroke(&mut s, 320.0, 320.0), (Color::WHITE, Some(Color::WHITE)), "inside a dark frame");
    s.execute("page.setBackground", &json!({"color": "#1E1E1E"})).unwrap();
    s.execute("node.create", &json!({"type": "frame", "x": 600, "y": 300, "width": 200, "height": 200, "props": {"fills": ["#FFFFFF"]}})).unwrap();
    assert_eq!(stroke(&mut s, 620.0, 320.0), (Color::BLACK, Some(Color::BLACK)), "a light frame on a dark page");
    // A frame without a fill, or with a see-through one, is looked through to what is behind it.
    s.execute("node.create", &json!({"type": "frame", "x": 900, "y": 300, "width": 200, "height": 200, "props": {"fills": []}})).unwrap();
    assert_eq!(stroke(&mut s, 920.0, 320.0), (Color::WHITE, Some(Color::WHITE)), "a frame without a fill on a dark page");
    s.execute("node.create", &json!({"type": "frame", "x": 1200, "y": 300, "width": 200, "height": 200, "props": {"fills": ["#FFFFFF40"]}})).unwrap();
    assert_eq!(stroke(&mut s, 1220.0, 320.0), (Color::WHITE, Some(Color::WHITE)), "a mostly transparent light frame on a dark page");
    // The threshold is Color::is_dark: white reads better than black from #757575 down.
    for (bg, want) in [("#767676", Color::BLACK), ("#757575", Color::WHITE)] {
        s.execute("page.setBackground", &json!({"color": bg})).unwrap();
        assert_eq!(stroke(&mut s, 0.0, 600.0).0, want, "on {bg}");
    }
}

/// The canvas draws through `render_view` (banded on desktop), not just `render_nodes_into`.
/// A brush stroke must still paint pixels there — this is the "nothing I draw shows up" path.
#[test]
fn brush_strokes_render_in_a_full_page_view() {
    install_test_brushes();
    let mut s = Session::new();
    let v = line(&mut s);
    s.execute("stroke.brush", &json!({"ids": [v], "preset": "dots", "spacing": 2})).unwrap();
    let doc = s.doc().unwrap();
    // 800×400 is over the 300k banding threshold: this exercises the same path the canvas uses.
    let view = figcraft_render::View { origin: (0.0, -100.0), scale: 1.0, width: 800, height: 400 };
    let pix = figcraft_render::render_view(doc, s.page().unwrap(), &view, &figcraft_render::Options::default()).unwrap();
    let painted = pix.data().chunks_exact(4).filter(|p| p[3] > 0).count();
    assert!(painted > 0, "a brush stroke painted nothing in a full-page view");
}

/// Draw with the pointer exactly like the canvas does (the brush tool + an ABR tip), then render
/// the page: this is the "I draw and nothing shows up" path end to end.
#[test]
fn a_brush_stroke_shows_up_on_the_canvas() {
    install_test_brushes();
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "brush"})).unwrap();
    s.execute("tool.brushPreset", &json!({"preset": "dots"})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 10.0, 100.0, 1.0)).unwrap();
    for i in 1..=20 {
        s.pointer(&PointerEvent::at(PointerKind::Move, 10.0 + f64::from(i) * 5.0, 100.0, 1.0)).unwrap();
    }
    s.pointer(&PointerEvent::at(PointerKind::Up, 110.0, 100.0, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    let view = figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 200 };
    let pix = figcraft_render::render_view(doc, s.page().unwrap(), &view, &figcraft_render::Options::default()).unwrap();
    let painted = pix.data().chunks_exact(4).filter(|p| p[3] > 0).count();
    assert!(painted > 0, "a brush stroke painted nothing on the canvas");
}

/// The canvas pans to fractional origins and zooms; dabs must land there too.
#[test]
fn brush_strokes_render_at_fractional_origins_and_zoom() {
    install_test_brushes();
    let mut s = Session::new();
    let v = line(&mut s);
    s.execute("stroke.brush", &json!({"ids": [v], "preset": "dots", "spacing": 2})).unwrap();
    let doc = s.doc().unwrap();
    for (origin, scale) in [((0.37, -99.6), 1.0), ((0.0, -50.0), 1.5), ((12.3, -10.7), 0.75)] {
        let view = figcraft_render::View { origin, scale, width: 800, height: 400 };
        let pix = figcraft_render::render_view_single(doc, s.page().unwrap(), &view, &figcraft_render::Options::default()).unwrap();
        let painted = pix.data().chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(painted > 0, "no pixels at origin {origin:?} scale {scale}");
    }
}

/// The path of the last layer the pencil drew on the page, in its own coordinates.
fn pencil_path(s: &Session) -> figcraft_geom::BezPath {
    let doc = s.doc().unwrap();
    let page = s.page().unwrap();
    let id = *doc.node(page).unwrap().children.last().unwrap();
    match &doc.node(id).unwrap().kind {
        NodeKind::Vector { network } => network.stroke_path(),
        other => panic!("the pencil drew a {other:?}, not a vector"),
    }
}

/// Elements of `path`: `(lines, curves)`.
fn shape_of(path: &figcraft_geom::BezPath) -> (usize, usize) {
    let (mut lines, mut curves) = (0, 0);
    for el in path.elements() {
        match el {
            figcraft_geom::kurbo::PathEl::LineTo(_) => lines += 1,
            figcraft_geom::kurbo::PathEl::CurveTo(..) | figcraft_geom::kurbo::PathEl::QuadTo(..) => curves += 1,
            _ => {}
        }
    }
    (lines, curves)
}

/// Draws a circle of radius 80 at (300, 300) through the pointer, `stray` being an optional
/// pointer position that jumps far off it mid-stroke.
fn draw_circle(s: &mut Session, stray: Option<(f64, f64)>) {
    let (cx, cy, r) = (300.0, 300.0, 80.0);
    s.pointer(&PointerEvent::at(PointerKind::Down, cx + r, cy, 1.0)).unwrap();
    for i in 1..=200 {
        let t = i as f64 / 200.0 * std::f64::consts::TAU * 0.95;
        if i == 100
            && let Some((x, y)) = stray
        {
            s.pointer(&PointerEvent::at(PointerKind::Move, x, y, 1.0)).unwrap();
        }
        s.pointer(&PointerEvent::at(PointerKind::Move, cx + r * t.cos(), cy + r * t.sin(), 1.0)).unwrap();
    }
    s.pointer(&PointerEvent::at(PointerKind::Up, cx + r, cy + 1.0, 1.0)).unwrap();
}

/// A hand-drawn circle must be a few smooth curves, not one straight line per pointer sample
/// (kurbo's fit left 299 lines and no curve at all for 300 samples).
#[test]
fn a_pencil_circle_is_a_few_curves() {
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    draw_circle(&mut s, None);
    let path = pencil_path(&s);
    let (lines, curves) = shape_of(&path);
    assert_eq!(lines, 0, "a drawn circle came out as {lines} straight segments");
    assert!((2..=12).contains(&curves), "expected a few cubics, got {curves}");
    let b = figcraft_geom::Shape::bounding_box(&path);
    assert!((b.width() - 160.0).abs() < 6.0 && (b.height() - 152.0).abs() < 8.0, "the stroke left its circle: {b:?}");
}

/// A pointer sample that jumps far off the stroke (a captured move over a panel, a display
/// change) must not drag the curve with it.
#[test]
fn a_stray_sample_does_not_throw_the_stroke_off() {
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    draw_circle(&mut s, Some((900.0, 900.0)));
    let path = pencil_path(&s);
    let b = figcraft_geom::Shape::bounding_box(&path);
    assert!(b.width() < 200.0 && b.height() < 200.0, "a stray sample stretched the stroke to {b:?}");
    let (_, curves) = shape_of(&path);
    assert!(curves >= 2, "the stroke was dropped instead of fitted: {curves} curves");
}

/// The UI preset menu uses `tool.drawStyle {tool:"brush", preset:<abr id>}` (not the older
/// `tool.brushPreset`). Reproduce that exact path end to end: choosing a preset and drawing
/// must paint with that tip, not silently fall back to nothing.
#[test]
fn ui_drawstyle_preset_paints_the_same_as_brushpreset() {
    install_test_brushes();
    fn painted(setup: impl Fn(&mut Session)) -> usize {
        let mut s = Session::new();
        s.execute("tool.set", &json!({"tool": "brush"})).unwrap();
        setup(&mut s);
        s.pointer(&PointerEvent::at(PointerKind::Down, 10.0, 100.0, 1.0)).unwrap();
        for i in 1..=20 {
            s.pointer(&PointerEvent::at(PointerKind::Move, 10.0 + f64::from(i) * 5.0, 100.0, 1.0)).unwrap();
        }
        s.pointer(&PointerEvent::at(PointerKind::Up, 110.0, 100.0, 1.0)).unwrap();
        let doc = s.doc().unwrap();
        let view = figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 200 };
        let pix = figcraft_render::render_view(doc, s.page().unwrap(), &view, &figcraft_render::Options::default()).unwrap();
        pix.data().chunks_exact(4).filter(|p| p[3] > 0).count()
    }
    let via_old = painted(|s| {
        s.execute("tool.brushPreset", &json!({"preset": "dots"})).unwrap();
    });
    let via_ui = painted(|s| {
        s.execute("tool.drawStyle", &json!({"tool": "brush", "preset": "dots"})).unwrap();
    });
    // The UI hands `infos()[].id`, which is `<group>/<index>/<name>` — not a legacy short name.
    let full_id = figcraft_abr::infos().first().map(|b| b.id.clone()).expect("test library has brushes");
    let via_full_id = painted(|s| {
        s.execute("tool.drawStyle", &json!({"tool": "brush", "preset": full_id})).unwrap();
    });
    assert!(via_old > 0, "brushPreset paints");
    assert!(via_ui > 0, "UI tool.drawStyle preset must paint, got 0 pixels");
    assert!(via_full_id > 0, "UI passes the full ABR id; it must paint, got 0 pixels");
}

/// End-to-end through the UI path: the toolbar's Flow / Opacity / Smoothing steppers issue
/// `tool.drawStyle {tool:"brush", <key>: v}`; the next stroke must reflect each one.
/// Rendered with `render_nodes_into` (no page colour) so alpha is the paint's own coverage —
/// with the page background under it every pixel would be opaque regardless.
#[test]
fn ui_flow_opacity_smoothing_steppers_take_effect() {
    install_test_brushes();
    // Draw a horizontal stroke through a fixed point and return (alpha at the centre, profile len).
    let run = |style: serde_json::Value| -> (u8, usize) {
        let mut s = Session::new();
        s.execute("tool.set", &json!({"tool": "brush"})).unwrap();
        s.execute("tool.drawStyle", &json!({"tool": "brush", "preset": "dots"})).unwrap();
        for (k, v) in style.as_object().expect("object").clone() {
            s.execute("tool.drawStyle", &json!({"tool": "brush", k: v})).unwrap();
        }
        s.pointer(&PointerEvent::at(PointerKind::Down, 20.0, 100.0, 1.0)).unwrap();
        for i in 1..=20 {
            s.pointer(&PointerEvent::at(PointerKind::Move, 20.0 + f64::from(i) * 4.0, 100.0, 1.0)).unwrap();
        }
        s.pointer(&PointerEvent::at(PointerKind::Up, 100.0, 100.0, 1.0)).unwrap();
        let doc = s.doc().unwrap();
        let view = figcraft_render::View { origin: (50.0, 99.0), scale: 1.0, width: 3, height: 3 };
        let mut pix = figcraft_render::Pixmap::new(3, 3).unwrap();
        figcraft_render::render_nodes_into(doc, &doc.node(s.page().unwrap()).unwrap().children, &view, &mut pix);
        let a = pix.data()[(3 + 1) * 4 + 3];
        let pts = doc.nodes().find_map(|n| n.stroke.profile().map(|pr| pr.points.len())).unwrap_or(0);
        (a, pts)
    };

    let (base, _) = run(json!({}));
    let (op_half, _) = run(json!({"opacity": 0.5}));
    let (flow_half, _) = run(json!({"flow": 0.5}));

    assert!(base > 200, "default brush paints solid at the centre, got {base}");
    assert!(op_half > 40 && op_half < 200, "opacity 0.5 must cap the stroke near half, got {op_half}");
    assert!(flow_half > 0 && flow_half < base, "flow 0.5 must lay down less ink per dab than 1.0, got {flow_half} vs {base}");

    // Smoothing reshapes the written width profile (the stabilizer smooths the sampled points).
    // Compare the actual profile *points*, not their count: smoothing moves samples, it does not
    // add or remove them.
    let profile_of = |style: serde_json::Value| -> String {
        let mut s = Session::new();
        s.execute("tool.set", &json!({"tool": "brush"})).unwrap();
        s.execute("tool.drawStyle", &json!({"tool": "brush", "preset": "dots"})).unwrap();
        for (k, v) in style.as_object().expect("object").clone() {
            s.execute("tool.drawStyle", &json!({"tool": "brush", k: v})).unwrap();
        }
        s.pointer(&PointerEvent::at(PointerKind::Down, 20.0, 100.0, 1.0)).unwrap();
        // A wobbly path so smoothing has something to smooth.
        for i in 1..=20 {
            let y = 100.0 + if i % 2 == 0 { 1.6 } else { -1.6 };
            s.pointer(&PointerEvent::at(PointerKind::Move, 20.0 + f64::from(i) * 4.0, y, 1.0)).unwrap();
        }
        s.pointer(&PointerEvent::at(PointerKind::Up, 100.0, 100.0, 1.0)).unwrap();
        let doc = s.doc().unwrap();
        doc.nodes().find_map(|n| n.stroke.profile().map(|pr| format!("{:?}", pr.points))).unwrap_or_default()
    };
    let p_none = profile_of(json!({"smoothing": 0.0}));
    let p_max = profile_of(json!({"smoothing": 1.0}));
    assert!(!p_none.is_empty(), "a brush stroke writes a width profile");
    assert_ne!(p_none, p_max, "smoothing 0 vs 1 must change the written width profile");
}

/// 描边高级设置里每个下拉值都必须是引擎认的值（#823）：下拉项变多了，点了要真能生效。
#[test]
fn every_stroke_dropdown_value_is_accepted() {
    let mut s = Session::new();
    let rect = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap());
    s.execute("stroke.add", &json!({"ids": [rect]})).unwrap();
    for cap in ["none", "round", "square", "lineArrow", "triangleArrow", "reversedTriangle", "circleFilled", "diamondFilled"] {
        assert!(s.execute("node.set", &json!({"id": rect, "props": {"startCap": cap, "endCap": cap}})).is_ok(), "cap {cap} must be accepted");
    }
    for join in ["miter", "bevel", "round"] {
        assert!(s.execute("node.set", &json!({"id": rect, "props": {"strokeJoin": join}})).is_ok(), "join {join} must be accepted");
    }
    for preset in ["uniform", "wedge", "taper", "eye", "rounded", "pinch"] {
        assert!(s.execute("stroke.widthProfile", &json!({"ids": [rect], "preset": preset})).is_ok(), "profile {preset} must be accepted");
    }
}
