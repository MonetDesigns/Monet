//! The board grid's settings: kept with the preferences, every value held within its range, and a
//! settings file edited by hand never costing the other preferences.

use serde_json::json;

use crate::board_grid::{BoardGrid, GridStyle, dot_fade, dot_step, line_spacing};
use crate::{Prefs, Session};

#[test]
fn board_grid_sets_and_reads_back_the_preferences() {
    let mut s = Session::new();
    let r = s.execute("board.grid", &json!({})).unwrap();
    assert_eq!(r, json!({"style": "dots", "spacing": 16.0, "major": 5, "size": 1.0, "contrast": 1.0}));
    let r = s.execute("board.grid", &json!({"style": "graph", "spacing": 8, "major": 10, "contrast": 2.5})).unwrap();
    assert_eq!(r["style"], "graph");
    assert_eq!(s.prefs.board_grid, BoardGrid { style: GridStyle::Graph, spacing: 8.0, major: 10, size: 1.0, contrast: 2.5 });
    // Out of range: the nearest end.
    let r = s.execute("board.grid", &json!({"spacing": 1e9, "major": 1, "size": 0, "contrast": 99})).unwrap();
    assert_eq!(r, json!({"style": "graph", "spacing": 512.0, "major": 2, "size": 0.5, "contrast": 4.0}));
    // Refused, and nothing changes.
    let before = s.prefs.board_grid.clone();
    for p in [json!({"style": "hexagons"}), json!({"style": 3}), json!({"spacing": "wide"}), json!({"contrast": [1]})] {
        assert!(s.execute("board.grid", &p).is_err(), "{p}");
    }
    assert_eq!(s.prefs.board_grid, before);
    // Off, then back to the default.
    assert_eq!(s.execute("board.grid", &json!({"style": "None"})).unwrap()["style"], "none");
    s.execute("board.grid", &json!({"reset": true})).unwrap();
    assert_eq!(s.prefs.board_grid, BoardGrid::default());
}

#[test]
fn a_saved_board_grid_survives_hand_edits() {
    let p = Prefs { board_grid: BoardGrid { style: GridStyle::Lines, spacing: 32.0, major: 4, size: 2.0, contrast: 1.5 }, ..Prefs::default() };
    let back: Prefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(back.board_grid, p.board_grid);
    // Unreadable: the default grid, and the other preferences stay.
    let back: Prefs = serde_json::from_value(json!({"nudge_small": 3.0, "board_grid": {"style": "zigzag"}})).unwrap();
    assert_eq!((back.nudge_small, back.board_grid), (3.0, BoardGrid::default()));
    let back: Prefs = serde_json::from_value(json!({"board_grid": "dots"})).unwrap();
    assert_eq!(back.board_grid, BoardGrid::default());
    // Out of range: clamped; missing fields: their defaults.
    let back: Prefs = serde_json::from_value(json!({"board_grid": {"style": "graph", "spacing": -5, "major": 400}})).unwrap();
    assert_eq!(back.board_grid, BoardGrid { style: GridStyle::Graph, spacing: 4.0, major: 12, ..BoardGrid::default() });
}

#[test]
fn the_dot_grid_steps_by_doubles_and_fades_in() {
    // 100 % and above: 16 page units between dots; every halving of the zoom doubles it.
    assert_eq!(dot_step(1.0, 16.0), 16.0);
    assert_eq!(dot_step(8.0, 16.0), 16.0);
    assert_eq!(dot_step(0.5, 16.0), 16.0);
    assert_eq!(dot_step(0.49, 16.0), 32.0);
    assert_eq!(dot_step(0.24, 16.0), 64.0);
    assert_eq!(dot_step(0.12, 16.0), 128.0);
    assert_eq!(dot_step(0.06, 16.0), 256.0);
    assert_eq!(dot_step(0.03, 16.0), 512.0);
    assert_eq!(dot_step(0.001, 16.0), 512.0);
    // On screen the gap stays between 8 and 16 pixels below 100 %.
    for z in [0.4, 0.2, 0.1, 0.05, 0.025] {
        let gap = dot_step(z, 16.0) * z;
        assert!((8.0..16.0).contains(&gap), "{z}: {gap}");
    }
    // The base is the preference; a bad one falls back to 16.
    assert_eq!(dot_step(1.0, 24.0), 24.0);
    assert_eq!(dot_step(1.0, f64::NAN), 16.0);
    assert_eq!(dot_step(1.0, 0.0), 16.0);
    // Nothing below 50 %, everything from 100 %, half way between.
    assert_eq!(dot_fade(0.5), 0.0);
    assert_eq!(dot_fade(0.25), 0.0);
    assert_eq!(dot_fade(1.0), 1.0);
    assert_eq!(dot_fade(64.0), 1.0);
    assert!((dot_fade(0.75) - 0.5).abs() < 1e-6);
    assert_eq!(dot_fade(f64::NAN), 0.0);
}

#[test]
fn lines_double_until_they_are_ten_pixels_apart() {
    assert_eq!(line_spacing(1.0, 16.0, 2, 10.0), Some((16.0, 0.6)));
    assert_eq!(line_spacing(0.25, 16.0, 2, 10.0).map(|s| s.0), Some(64.0));
    assert_eq!(line_spacing(4.0, 16.0, 5, 10.0), Some((16.0, 1.0)));
    assert_eq!(line_spacing(0.0, 16.0, 2, 10.0), None);
    assert_eq!(line_spacing(f64::NAN, 16.0, 2, 10.0), None);
}

