//! The board canvas's background grid, kept with the preferences: dots (the default), lines, or
//! graph paper (a strong line every few cells over fainter ones, like Japanese 方眼紙 section
//! paper), how far apart, how large and how strong its marks are, or no grid at all. The canvas
//! thins it out as it zooms out, so it never turns into a wash of marks.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::{boolean, get, num};
use crate::{Result, Session, bad, spec};

/// What the grid draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GridStyle {
    #[default]
    Dots,
    Lines,
    /// Lines with every `major`th one strong (section paper).
    Graph,
    None,
}

impl GridStyle {
    pub const ALL: [GridStyle; 4] = [GridStyle::Dots, GridStyle::Lines, GridStyle::Graph, GridStyle::None];

    pub fn id(self) -> &'static str {
        match self {
            GridStyle::Dots => "dots",
            GridStyle::Lines => "lines",
            GridStyle::Graph => "graph",
            GridStyle::None => "none",
        }
    }

    pub fn from_id(id: &str) -> Option<GridStyle> {
        GridStyle::ALL.into_iter().find(|s| s.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// The spacing's range (page units between neighbouring marks at 100 %).
pub const SPACING: (f64, f64) = (4.0, 512.0);
/// Graph paper's cells per strong line.
pub const MAJOR: (u32, u32) = (2, 12);
/// The marks' size and strength, relative to the usual (1).
pub const SIZE: (f64, f64) = (0.5, 4.0);
pub const CONTRAST: (f64, f64) = (0.25, 4.0);

/// The board grid's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BoardGrid {
    pub style: GridStyle,
    /// Page units between neighbouring marks at 100 % (zoomed out, every second, fourth… stays).
    pub spacing: f64,
    /// Graph paper: a strong line every `major` cells.
    pub major: u32,
    /// How large the marks are: 1 is about a point and a half across for dots, a point for lines.
    pub size: f64,
    /// How strong the marks are against the page: 1 is the usual, subtle grid.
    pub contrast: f64,
}

impl Default for BoardGrid {
    fn default() -> Self {
        Self { style: GridStyle::Dots, spacing: 16.0, major: 5, size: 1.0, contrast: 1.0 }
    }
}

/// `v` kept within `range`; `d` when it is not a finite number.
fn within(v: f64, (lo, hi): (f64, f64), d: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { d }
}

impl BoardGrid {
    /// Every value back within its range (a settings file edited by hand).
    pub fn sanitized(mut self) -> Self {
        let d = Self::default();
        self.spacing = within(self.spacing, SPACING, d.spacing);
        self.major = self.major.clamp(MAJOR.0, MAJOR.1);
        self.size = within(self.size, SIZE, d.size);
        self.contrast = within(self.contrast, CONTRAST, d.contrast);
        self
    }

    pub fn to_json(&self) -> Value {
        json!({"style": self.style.id(), "spacing": self.spacing, "major": self.major, "size": self.size, "contrast": self.contrast})
    }
}

/// Reads the saved grid: anything unreadable is the default grid and values out of range are
/// clamped, so a settings file edited by hand never costs the other preferences.
pub(crate) fn read<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<BoardGrid, D::Error> {
    Ok(serde_json::from_value::<BoardGrid>(Value::deserialize(d)?).unwrap_or_default().sanitized())
}

const CMD: &str = "board.grid";

/// A number param of `board.grid`: `None` when omitted, an error when not a finite number.
fn number(p: &Value, key: &str) -> Result<Option<f64>> {
    match get(p, key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => num(p, key).filter(|v| v.is_finite()).map(Some).ok_or_else(|| bad(CMD, format!("`{key}` must be a number"))),
    }
}

fn grid_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let mut g = if boolean(p, "reset") == Some(true) { BoardGrid::default() } else { s.prefs.board_grid.clone() };
    if let Some(v) = get(p, "style").filter(|v| !v.is_null()) {
        let id = v.as_str().unwrap_or_default();
        g.style = GridStyle::from_id(id).ok_or_else(|| bad(CMD, "`style` is dots, lines, graph or none"))?;
    }
    if let Some(v) = number(p, "spacing")? {
        g.spacing = v;
    }
    if let Some(v) = number(p, "major")? {
        g.major = v.round().clamp(f64::from(MAJOR.0), f64::from(MAJOR.1)) as u32;
    }
    if let Some(v) = number(p, "size")? {
        g.size = v;
    }
    if let Some(v) = number(p, "contrast")? {
        g.contrast = v;
    }
    s.prefs.board_grid = g.sanitized();
    Ok(s.prefs.board_grid.to_json())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "board.grid",
        "Board grid",
        "{style?: dots|lines|graph|none (graph: section paper, a strong line every `major` cells), spacing? (4–512 page units at 100 %), major? (2–12), size? (0.5–4, 1 the usual), contrast? (0.25–4, 1 the usual), reset? (back to the default first); omit all: just read it} → {style, spacing, major, size, contrast}  the board canvas's background grid, kept with the preferences; zoomed out it thins out",
        false,
        grid_cmd
    )]
}

/// The dot grid's spacing on the page at `zoom` (page units): `base` × {32,16,8,4,2,1}, chosen by
/// the zoom so the marks stay 8–16 screen pixels apart below 100 %, and at `base` above it.
pub fn dot_step(zoom: f64, base: f64) -> f64 {
    let k = if zoom < 0.03125 {
        32.0
    } else if zoom < 0.0625 {
        16.0
    } else if zoom < 0.125 {
        8.0
    } else if zoom < 0.25 {
        4.0
    } else if zoom < 0.5 {
        2.0
    } else {
        1.0
    };
    if base.is_finite() && base > 0.0 { base * k } else { 16.0 * k }
}

/// How strongly the dot grid shows at `zoom`: nothing below 50 %, all of it from 100 % (smoothstep
/// in between), so the dots read as one grid whichever wheel step they come from.
pub fn dot_fade(zoom: f64) -> f32 {
    if !zoom.is_finite() {
        return 0.0;
    }
    let t = ((zoom - 0.5) / 0.5).clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

/// The line grid's spacing at `zoom`: `base` × `major`ⁿ until the lines are `min_gap` screen
/// pixels apart, and how strongly the marks the next step drops still show (0 to 1). `None` when
/// no finite spacing gets there.
pub fn line_spacing(zoom: f64, base: f64, major: u32, min_gap: f64) -> Option<(f64, f32)> {
    let ok = |v: f64| v.is_finite() && v > 0.0;
    if !(ok(zoom) && ok(base) && ok(min_gap)) {
        return None;
    }
    let factor = f64::from(major.max(2));
    let mut step = base;
    // Bounded: past about a thousand doublings the step is no longer finite, and there is no grid.
    for _ in 0..1100 {
        let gap = step * zoom;
        if !gap.is_finite() {
            return None;
        }
        if gap >= min_gap {
            return Some((step, ((gap - min_gap) / min_gap).clamp(0.0, 1.0) as f32));
        }
        step *= factor;
    }
    None
}
