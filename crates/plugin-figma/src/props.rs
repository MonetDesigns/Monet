//! The property table: what a Figma node property is called on the FigCraft side.
//!
//! The two APIs describe the same document with different vocabularies. Most names are shared
//! (`name`, `visible`, `opacity`, `fills`, `effects`, `cornerRadius`, `characters`), but several are
//! not, and a few Figma properties are assembled from more than one engine key:
//!
//! | Figma | FigCraft `node.set` |
//! |---|---|
//! | `layoutMode` | `layout` (`"NONE"` is `null`) |
//! | `primaryAxisSizingMode` / `counterAxisSizingMode` | `sizingH` / `sizingV`, by layout direction |
//! | `primaryAxisAlignItems` / `counterAxisAlignItems` | `mainAlign` / `crossAlign` |
//! | `itemSpacing` / `counterAxisSpacing` | `gap` / `crossGap` |
//! | `fontName` (`{family, style}`) | `fontFamily` + `fontStyle` |
//! | `topLeftRadius` … `bottomLeftRadius` | `cornerRadius` as `[tl, tr, br, bl]` |
//! | `layoutGrids` / `exportSettings` / `reactions` | `grids` / `exports` / `interactions` |
//! | `constraints` (`{horizontal, vertical}` strings) | `constraints {horizontal, vertical}` |
//!
//! [`PROPS`] is the table; [`Kind`] says how a value travels; [`convert_enum`] maps the Figma enum
//! constants whose spellings differ (and turns `layoutMode: "NONE"` into `null`). Anything not in
//! the table is unknown to this layer and must be reported as unimplemented, not guessed.

use serde_json::{Value, json};

/// How a property's value travels between the two APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A number written as is.
    Number,
    /// A boolean written as is.
    Boolean,
    /// A string written as is (`name`, `characters`, `description`).
    Text,
    /// One of a set of constants: see [`convert_enum`].
    Enum,
    /// A list of numbers written as is (`dashPattern`).
    Array,
    /// Paints (`fills`, `strokes`): Figma `Paint[]` against the engine's paint objects, converted by
    /// the bridge (colours are 0–1 RGBA in Figma, hex or objects on the engine side).
    Paints,
    /// Effects (`effects`) with the same conversion problem as paints.
    Effects,
    /// Several engine keys, or a value whose engine key depends on the node: the proxy assembles
    /// it. `engine` names the key when there is exactly one.
    Derived,
    /// Read-only in Figma: served from `node.get`, never written through `node.set`.
    ReadOnly,
}

/// One row of the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropDef {
    /// The Figma property name (`node.fills`, `layoutMode`).
    pub figma: &'static str,
    /// The `node.set` key, when the value goes to one key as is (or, for [`Kind::Derived`], when
    /// the composed value lands in one key).
    pub engine: Option<&'static str>,
    pub kind: Kind,
    /// What the proxy must do beyond renaming, and what still needs checking against Figma.
    pub note: &'static str,
}

const fn prop(figma: &'static str, engine: &'static str, kind: Kind, note: &'static str) -> PropDef {
    PropDef { figma, engine: Some(engine), kind, note }
}

const fn derived(figma: &'static str, engine: Option<&'static str>, note: &'static str) -> PropDef {
    PropDef { figma, engine, kind: Kind::Derived, note }
}

const fn read_only(figma: &'static str, note: &'static str) -> PropDef {
    PropDef { figma, engine: None, kind: Kind::ReadOnly, note }
}

/// Every Figma property this layer maps.
pub const PROPS: &[PropDef] = &[
    // Identity and visibility.
    prop("name", "name", Kind::Text, ""),
    prop("visible", "visible", Kind::Boolean, ""),
    prop("locked", "locked", Kind::Boolean, ""),
    prop("opacity", "opacity", Kind::Number, "0–1 on both sides"),
    prop("blendMode", "blend", Kind::Enum, "the engine normalises names, so Figma's constants pass through; `PASS_THROUGH` is accepted"),
    // Geometry. Figma's x/y are relative to the direct parent (groups included); the engine's are
    // relative to the parent frame, groups not counting: the proxy converts when a parent is a group.
    prop("x", "x", Kind::Number, "relative to the parent: Figma counts groups, the engine does not"),
    prop("y", "y", Kind::Number, "relative to the parent: Figma counts groups, the engine does not"),
    prop("width", "width", Kind::Number, "a resize: text auto-resize and layout limits apply"),
    prop("height", "height", Kind::Number, "a resize: text auto-resize and layout limits apply"),
    prop("rotation", "rotation", Kind::Number, "Figma: counter-clockwise degrees around the top-left; check the engine's sign"),
    prop("minWidth", "minWidth", Kind::Number, "Figma's default is null (no limit); the engine removes a limit with null"),
    prop("maxWidth", "maxWidth", Kind::Number, "Figma's default is null (no limit)"),
    prop("minHeight", "minHeight", Kind::Number, ""),
    prop("maxHeight", "maxHeight", Kind::Number, ""),
    derived(
        "constraints",
        Some("constraints"),
        "Figma: `{horizontal, vertical}` of MIN/MAX/CENTER/STRETCH; one object on both sides, values to check",
    ),
    // Appearance.
    prop("fills", "fills", Kind::Paints, "Figma RGBA is 0–1; gradients use `gradientTransform` matrices"),
    prop("strokes", "strokes", Kind::Paints, "as `fills`"),
    prop("strokeWeight", "strokeWeight", Kind::Number, "a number, or `figma.mixed` across the sides of a variable-width stroke"),
    prop("strokeAlign", "strokeAlign", Kind::Enum, ""),
    prop("strokeCap", "strokeCap", Kind::Enum, ""),
    prop("strokeJoin", "strokeJoin", Kind::Enum, ""),
    prop("dashPattern", "dashes", Kind::Array, "`number[]`; the engine calls it `dashes`"),
    prop("effects", "effects", Kind::Effects, "shadow/blur kinds and units to check"),
    prop("cornerRadius", "cornerRadius", Kind::Number, "a number, or `figma.mixed` when the corners differ"),
    derived("topLeftRadius", Some("cornerRadius"), "compose `cornerRadius: [tl, tr, br, bl]` in Figma's corner order"),
    derived("topRightRadius", Some("cornerRadius"), "as `topLeftRadius`"),
    derived("bottomRightRadius", Some("cornerRadius"), "as `topLeftRadius`"),
    derived("bottomLeftRadius", Some("cornerRadius"), "as `topLeftRadius`"),
    prop("clipsContent", "clip", Kind::Boolean, "the engine calls it `clip`"),
    prop("isMask", "isMask", Kind::Boolean, ""),
    // Auto layout. `layoutMode`, the sizing modes and the spacing names differ; gaps fold into one
    // key per axis on the engine side.
    prop("layoutMode", "layout", Kind::Enum, "`NONE` is `null`; `GRID` maps to the engine's grid layout"),
    derived("primaryAxisSizingMode", None, "`sizingH` in a horizontal layout, `sizingV` in a vertical one: FIXED / HUG / FILL"),
    derived("counterAxisSizingMode", None, "the other axis: `sizingV` in a horizontal layout, `sizingH` in a vertical one"),
    prop("primaryAxisAlignItems", "mainAlign", Kind::Enum, "SPACE_BETWEEN → the engine's auto gap"),
    prop("counterAxisAlignItems", "crossAlign", Kind::Enum, "BASELINE exists on both sides for horizontal layouts"),
    prop("itemSpacing", "gap", Kind::Number, "the engine calls it `gap`"),
    prop("counterAxisSpacing", "crossGap", Kind::Number, "`null` when the counter axis packs"),
    prop("paddingLeft", "paddingLeft", Kind::Number, ""),
    prop("paddingTop", "paddingTop", Kind::Number, ""),
    prop("paddingRight", "paddingRight", Kind::Number, ""),
    prop("paddingBottom", "paddingBottom", Kind::Number, ""),
    prop("layoutGrids", "grids", Kind::Derived, "grid kinds (columns/rows/grid) to check"),
    // Text.
    prop("characters", "characters", Kind::Text, "needs the font loaded first (`figma.loadFontAsync`)"),
    derived("fontName", None, "`{family, style}` → `fontFamily` + `fontStyle`; must be loaded first"),
    prop("fontSize", "fontSize", Kind::Number, ""),
    prop("textAlignHorizontal", "textAlign", Kind::Enum, "the engine calls it `textAlign`"),
    prop("textAlignVertical", "textAlignVertical", Kind::Enum, ""),
    prop("letterSpacing", "letterSpacing", Kind::Number, "a number (px) or `{unit: \"PERCENT\", value}`"),
    prop("lineHeight", "lineHeight", Kind::Number, "a number (px), `{unit: \"PERCENT\"}` or `\"AUTO\"`"),
    prop("textAutoResize", "textAutoResize", Kind::Enum, ""),
    prop("textCase", "textCase", Kind::Enum, ""),
    prop("textDecoration", "textDecoration", Kind::Enum, ""),
    prop("paragraphSpacing", "paragraphSpacing", Kind::Number, ""),
    prop("paragraphIndent", "paragraphIndent", Kind::Number, ""),
    // Data, descriptions and prototype links.
    prop("pluginData", "pluginData", Kind::Derived, "`setPluginData(key, value)`: key ≤ 64 characters, value bounded per node"),
    derived("sharedPluginData", None, "namespaced data: no engine equivalent yet, report as unsupported instead of writing `pluginData`"),
    prop("description", "description", Kind::Text, ""),
    prop("reactions", "interactions", Kind::Derived, "prototype actions: the engine's `interactions` list"),
    prop("exportSettings", "exports", Kind::Derived, "`ExportSettings[]` against the engine's `exports`"),
    // Read-only: served from `node.get`, or from the node graph itself.
    read_only("id", "the string form of the numeric id, see `ids`"),
    read_only("type", "`FRAME`, `TEXT`, …: the engine's node kinds"),
    read_only("parent", "a live reference: `node.get` alone is a snapshot"),
    read_only("children", "a live list, in z-order"),
    read_only("removed", "true once the node is deleted: later writes throw"),
    read_only("absoluteBoundingBox", "needs a layout pass before it is read"),
    read_only("absoluteTransform", "a 2×3 matrix; recompute after a move"),
    read_only("componentPropertyDefinitions", "only on a component or a component set"),
    read_only("variantProperties", "only on a component set or a variant"),
    read_only("mainComponent", "only on an instance"),
    read_only("fillStyleId", "a style id: `S:n` here, a GUID in Figma; treat as opaque"),
    read_only("strokeStyleId", "as `fillStyleId`"),
    read_only("effectStyleId", "as `fillStyleId`"),
    read_only("gridStyleId", "as `fillStyleId`"),
    read_only("textStyleId", "as `fillStyleId`"),
];

/// The row for a Figma property name.
pub fn by_figma(name: &str) -> Option<&'static PropDef> {
    PROPS.iter().find(|prop| prop.figma == name)
}

/// The `node.set` key a Figma property writes, when it writes exactly one.
pub fn engine_key(name: &str) -> Option<&'static str> {
    by_figma(name).filter(|prop| prop.kind != Kind::ReadOnly).and_then(|prop| prop.engine)
}

/// Whether a value goes to one engine key as is (after [`convert_enum`] for enums).
pub fn is_direct(name: &str) -> bool {
    by_figma(name)
        .is_some_and(|prop| matches!(prop.kind, Kind::Number | Kind::Boolean | Kind::Text | Kind::Enum | Kind::Array | Kind::Paints | Kind::Effects))
}

/// Whether a Figma property is read-only.
pub fn is_read_only(name: &str) -> bool {
    by_figma(name).is_some_and(|prop| prop.kind == Kind::ReadOnly)
}

/// The `node.get` key a Figma property reads. Read-only properties such as `type` live under
/// their own name even though they can never be written; `engine_key` deliberately excludes them
/// so a `node.set` cannot reach a read-only field.
pub fn read_key(name: &str) -> Option<&'static str> {
    match name {
        "type" => Some("type"),
        _ => engine_key(name),
    }
}

/// Translate an engine enum value back to the Figma spelling. `None` means the value has no
/// mapping (or the property is not an enum), so the bridge leaves it untouched rather than
/// inventing a constant.
pub fn enum_to_figma(figma_prop: &str, value: &Value) -> Option<Value> {
    let value = match figma_prop {
        "layoutMode" => {
            if value.is_null() {
                json!("NONE")
            } else {
                match value.as_str()? {
                    "horizontal" => json!("HORIZONTAL"),
                    "vertical" => json!("VERTICAL"),
                    "grid" => json!("GRID"),
                    _ => return None,
                }
            }
        }
        "strokeAlign" => match value.as_str()? {
            "inside" => json!("INSIDE"),
            "center" => json!("CENTER"),
            "outside" => json!("OUTSIDE"),
            _ => return None,
        },
        "strokeCap" => match value.as_str()? {
            "none" => json!("NONE"),
            "round" => json!("ROUND"),
            "square" => json!("SQUARE"),
            _ => return None,
        },
        "strokeJoin" => match value.as_str()? {
            "miter" => json!("MITER"),
            "round" => json!("ROUND"),
            "bevel" => json!("BEVEL"),
            _ => return None,
        },
        "textAlignHorizontal" => match value.as_str()? {
            "left" => json!("LEFT"),
            "center" => json!("CENTER"),
            "right" => json!("RIGHT"),
            "justified" => json!("JUSTIFIED"),
            _ => return None,
        },
        "textAlignVertical" => match value.as_str()? {
            "top" => json!("TOP"),
            "center" => json!("CENTER"),
            "bottom" => json!("BOTTOM"),
            _ => return None,
        },
        "textAutoResize" => match value.as_str()? {
            "none" => json!("NONE"),
            "widthAndHeight" => json!("WIDTH_AND_HEIGHT"),
            "height" => json!("HEIGHT"),
            "truncate" => json!("TRUNCATE"),
            _ => return None,
        },
        "textCase" => match value.as_str()? {
            "original" => json!("ORIGINAL"),
            "upper" => json!("UPPER"),
            "lower" => json!("LOWER"),
            "title" => json!("TITLE"),
            "smallCaps" => json!("SMALL_CAPS"),
            _ => return None,
        },
        "primaryAxisAlignItems" => match value.as_str()? {
            "min" => json!("MIN"),
            "center" => json!("CENTER"),
            "max" => json!("MAX"),
            "spaceBetween" => json!("SPACE_BETWEEN"),
            _ => return None,
        },
        "counterAxisAlignItems" => match value.as_str()? {
            "min" => json!("MIN"),
            "center" => json!("CENTER"),
            "max" => json!("MAX"),
            "baseline" => json!("BASELINE"),
            _ => return None,
        },
        // The engine keeps Figma's blend constants, so they round-trip unchanged.
        "blendMode" => value.clone(),
        _ => return None,
    };
    Some(value)
}

/// The engine value for a Figma enum constant, when the spelling differs.
///
/// `None` means this table doesn't map that property or value; the proxy must not guess. Unknown
/// values are how a plugin written against a newer Figma fails loudly instead of writing something
/// else.
pub fn convert_enum(figma_prop: &str, value: &str) -> Option<Value> {
    let value = match figma_prop {
        "layoutMode" => match value {
            "NONE" => Value::Null,
            "HORIZONTAL" => json!("horizontal"),
            "VERTICAL" => json!("vertical"),
            "GRID" => json!("grid"),
            _ => return None,
        },
        "strokeAlign" => match value {
            "INSIDE" => json!("inside"),
            "CENTER" => json!("center"),
            "OUTSIDE" => json!("outside"),
            _ => return None,
        },
        "strokeCap" => match value {
            "NONE" => json!("none"),
            "ROUND" => json!("round"),
            "SQUARE" => json!("square"),
            _ => return None,
        },
        "strokeJoin" => match value {
            "MITER" => json!("miter"),
            "ROUND" => json!("round"),
            "BEVEL" => json!("bevel"),
            _ => return None,
        },
        "textAlignHorizontal" => match value {
            "LEFT" => json!("left"),
            "CENTER" => json!("center"),
            "RIGHT" => json!("right"),
            "JUSTIFIED" => json!("justified"),
            _ => return None,
        },
        "textAlignVertical" => match value {
            "TOP" => json!("top"),
            "CENTER" => json!("center"),
            "BOTTOM" => json!("bottom"),
            _ => return None,
        },
        "textAutoResize" => match value {
            "NONE" => json!("none"),
            "WIDTH_AND_HEIGHT" => json!("widthAndHeight"),
            "HEIGHT" => json!("height"),
            "TRUNCATE" => json!("truncate"),
            _ => return None,
        },
        "textCase" => match value {
            "ORIGINAL" => json!("original"),
            "UPPER" => json!("upper"),
            "LOWER" => json!("lower"),
            "TITLE" => json!("title"),
            "SMALL_CAPS" => json!("smallCaps"),
            _ => return None,
        },
        "primaryAxisAlignItems" => match value {
            "MIN" => json!("min"),
            "CENTER" => json!("center"),
            "MAX" => json!("max"),
            "SPACE_BETWEEN" => json!("spaceBetween"),
            _ => return None,
        },
        "counterAxisAlignItems" => match value {
            "MIN" => json!("min"),
            "CENTER" => json!("center"),
            "MAX" => json!("max"),
            "BASELINE" => json!("baseline"),
            _ => return None,
        },
        // The engine normalises blend names, so Figma's constants already fit.
        "blendMode" => json!(value),
        _ => return None,
    };
    Some(value)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{Kind, PROPS, by_figma, convert_enum, engine_key, enum_to_figma, is_direct, is_read_only, read_key};

    #[test]
    fn every_figma_name_is_unique() {
        let mut names: Vec<&str> = PROPS.iter().map(|prop| prop.figma).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), before, "a Figma property is listed twice");
    }

    #[test]
    fn the_familiar_names_map_straight_through() {
        for name in ["name", "visible", "opacity", "fills", "strokes", "effects", "characters", "cornerRadius"] {
            assert_eq!(engine_key(name), Some(name));
            assert!(is_direct(name));
        }
    }

    #[test]
    fn the_renamed_properties_are_the_expected_ones() {
        assert_eq!(engine_key("layoutMode"), Some("layout"));
        assert_eq!(engine_key("clipsContent"), Some("clip"));
        assert_eq!(engine_key("textAlignHorizontal"), Some("textAlign"));
        assert_eq!(engine_key("dashPattern"), Some("dashes"));
        assert_eq!(engine_key("itemSpacing"), Some("gap"));
        assert_eq!(engine_key("counterAxisSpacing"), Some("crossGap"));
        assert_eq!(engine_key("layoutGrids"), Some("grids"));
        assert_eq!(engine_key("exportSettings"), Some("exports"));
        assert_eq!(engine_key("reactions"), Some("interactions"));
    }

    #[test]
    fn composed_properties_are_not_direct_writes() {
        for name in ["fontName", "primaryAxisSizingMode", "counterAxisSizingMode", "topLeftRadius", "sharedPluginData"] {
            assert!(!is_direct(name), "{name} must be composed by the proxy");
        }
        assert_eq!(by_figma("topLeftRadius").and_then(|p| p.engine), Some("cornerRadius"));
        assert_eq!(by_figma("fontName").and_then(|p| p.engine), None);
        assert_eq!(by_figma("topLeftRadius").map(|p| p.kind), Some(Kind::Derived));
    }

    #[test]
    fn read_only_type_is_readable_but_not_writable() {
        assert_eq!(engine_key("type"), None, "writes must stay rejected");
        assert_eq!(read_key("type"), Some("type"));
        assert!(is_read_only("type"));
    }

    #[test]
    fn engine_enums_round_trip_to_figma() {
        assert_eq!(enum_to_figma("layoutMode", &Value::Null), Some(json!("NONE")));
        assert_eq!(enum_to_figma("layoutMode", &json!("horizontal")), Some(json!("HORIZONTAL")));
        assert_eq!(enum_to_figma("layoutMode", &json!("vertical")), Some(json!("VERTICAL")));
        assert_eq!(enum_to_figma("primaryAxisAlignItems", &json!("spaceBetween")), Some(json!("SPACE_BETWEEN")));
        assert_eq!(enum_to_figma("textAutoResize", &json!("widthAndHeight")), Some(json!("WIDTH_AND_HEIGHT")));
        assert_eq!(enum_to_figma("blendMode", &json!("MULTIPLY")), Some(json!("MULTIPLY")));
        assert_eq!(enum_to_figma("layoutMode", &json!("sideways")), None, "unknown values must not be guessed");
    }

    #[test]
    fn the_node_graph_properties_are_read_only() {
        for name in ["id", "type", "parent", "children", "removed", "absoluteBoundingBox"] {
            assert!(is_read_only(name));
            assert_eq!(engine_key(name), None);
        }
    }

    #[test]
    fn enum_constants_take_the_engine_spelling() {
        assert_eq!(convert_enum("layoutMode", "NONE"), Some(json!(null)));
        assert_eq!(convert_enum("layoutMode", "HORIZONTAL"), Some(json!("horizontal")));
        assert_eq!(convert_enum("primaryAxisAlignItems", "SPACE_BETWEEN"), Some(json!("spaceBetween")));
        assert_eq!(convert_enum("counterAxisAlignItems", "BASELINE"), Some(json!("baseline")));
        assert_eq!(convert_enum("textAutoResize", "WIDTH_AND_HEIGHT"), Some(json!("widthAndHeight")));
        assert_eq!(convert_enum("textCase", "SMALL_CAPS"), Some(json!("smallCaps")));
        assert_eq!(convert_enum("strokeCap", "SQUARE"), Some(json!("square")));
        assert_eq!(convert_enum("textAlignHorizontal", "JUSTIFIED"), Some(json!("justified")));
        assert_eq!(convert_enum("blendMode", "PASS_THROUGH"), Some(json!("PASS_THROUGH")));
        assert_eq!(convert_enum("layoutMode", "SOMETHING_NEW"), None);
        assert_eq!(convert_enum("name", "x"), None, "not an enum");
    }
}
