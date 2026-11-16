//! Brushes and pen pressure through the session: `stroke.brush`, ABR tip compositing and
//! outlining brush strokes, and the Pencil recording pressure (and its brush) into the stroke it
//! draws.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, pressure_profile};
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

#[test]
fn the_pencil_records_pressure_and_its_brush() {
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    assert!(s.execute("tool.pencilBrush", &json!({"preset": ""})).is_err(), "an empty brush id is rejected");
    s.execute("tool.pencilBrush", &json!({"preset": "taper"})).unwrap();
    let ev = |kind, x: f64, p: f32| PointerEvent { pressure: Some(p), ..PointerEvent::at(kind, x, 50.0 + (x / 20.0).sin() * 10.0, 1.0) };
    s.pointer(&ev(PointerKind::Down, 0.0, 0.2)).unwrap();
    for i in 1..=40 {
        s.pointer(&ev(PointerKind::Move, f64::from(i) * 5.0, 0.2 + 0.02 * i as f32)).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, 200.0, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    let n = doc.nodes().find(|n| matches!(n.kind, NodeKind::Vector { .. })).expect("a pencil stroke");
    let prof = n.stroke.profile().expect("pressure recorded");
    assert!(prof.scale_at(0.0) < 0.5 && prof.scale_at(1.0) > 1.9, "light to hard: {prof:?}");
    assert_eq!(n.stroke.brush.as_ref().map(|b| b.preset.as_str()), Some("taper"));
    // A mouse (no pressure) draws a plain uniform stroke.
    s.execute("tool.pencilBrush", &json!({"preset": null})).unwrap();
    s.execute("edit.selectAll", &json!({})).ok();
    let before = s.doc().unwrap().nodes().count();
    s.pointer(&PointerEvent::at(PointerKind::Down, 0.0, 200.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 50.0, 220.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 100.0, 200.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 100.0, 200.0, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(doc.nodes().count(), before + 1);
    let last = doc.nodes().filter(|n| matches!(n.kind, NodeKind::Vector { .. })).max_by_key(|n| n.id).unwrap();
    assert!(last.stroke.profile().is_none() && last.stroke.brush.is_none());
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

/// Draw with the pointer exactly like the canvas does (Pencil + an ABR brush), then render the
/// page: this is the "I draw and nothing shows up" path end to end.
#[test]
fn a_pencil_brush_stroke_shows_up_on_the_canvas() {
    install_test_brushes();
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    s.execute("tool.pencilBrush", &json!({"preset": "dots"})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 10.0, 100.0, 1.0)).unwrap();
    for i in 1..=20 {
        s.pointer(&PointerEvent::at(PointerKind::Move, 10.0 + f64::from(i) * 5.0, 100.0, 1.0)).unwrap();
    }
    s.pointer(&PointerEvent::at(PointerKind::Up, 110.0, 100.0, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    let view = figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 200 };
    let pix = figcraft_render::render_view(doc, s.page().unwrap(), &view, &figcraft_render::Options::default()).unwrap();
    let painted = pix.data().chunks_exact(4).filter(|p| p[3] > 0).count();
    assert!(painted > 0, "a pencil brush stroke painted nothing on the canvas");
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
