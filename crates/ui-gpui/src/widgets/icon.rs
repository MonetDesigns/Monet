//! Monet icon set: every glyph is an external SVG asset under `assets/icons/`.
//!
//! Every glyph is a flat file under `assets/icons/`. The product mark stays under `brand/`. Every `stroke-width` in those files is the shared [`ICON_STROKE`];
//! `rescale_strokes` compensates at load time so the visual width survives arbitrary sizes.
//!
//! No geometry is generated in this module anymore. A missing glyph means a missing file, not a
//! silent inline fallback.

use std::borrow::Cow;

use gpui::{App, AssetSource, Bounds, Hsla, IntoElement, Pixels, Point, Result, SharedString, TransformationMatrix, Window, prelude::*, px, svg};

/// The one stroke width every vector icon uses, in points.
pub const ICON_STROKE: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Plus,
    Minus,
    Close,
    Check,
    ChevronDown,
    ChevronUp,
    ChevronLeft,
    ChevronRight,
    Search,
    Dot,
    Variable,
    /// `#` — a number field's mark (user-supplied 24-grid SVG).
    Number,
    /// Toggle — a boolean field's mark (user-supplied 24-grid SVG).
    Boolean,
    LinkOn,
    LinkOff,
    Section,
    Panel,
    Menu,
    Settings,
    Trash,
    /// Filled rectangle (the `Rectangle` node kind).
    Rectangle,
    /// Filled ellipse (the `Ellipse` node kind).
    Ellipse,
    /// Outlined rectangle (a `Frame` / `Group` node kind).
    Frame,
    /// The letter "T" (a `Text` node kind).
    Text,
    /// Filled diamond (a `Component` / `ComponentSet` node kind).
    Component,
    /// Outlined diamond (an `Instance` node kind).
    Instance,
    /// Outlined rectangle with a folded corner (the `Page` node kind).
    Page,
    /// Monet's own mark: the curled tail of the chameleon (the rail's main-menu button).
    Logo,
    /// A diamond, a circle, a square and a triangle: the Assets panel.
    Assets,
    /// Four squares: the variables editor (`ui-egui/src/icons/draw.rs::Variables`).
    Variables,
    /// A picture frame with a sun and a hill (the `Image` fill).
    Image,
    /// A 3×3 tile grid (the `Pattern` fill).
    ViewGrid,
    /// A broken link (unbind a variable).
    Detach,
    /// The six alignment glyphs (`align.left` … `align.bottom`) and the distribute one.
    AlignLeft,
    AlignHCenter,
    AlignRight,
    AlignTop,
    AlignVCenter,
    AlignBottom,
    DistributeH,
    /// Rotate 90°, flip horizontally / vertically, and the rotation field's angle mark.
    Rotate,
    FlipH,
    FlipV,
    /// "Ignore auto layout" (absolute position) — four corner brackets.
    AbsolutePosition,
    // ── The bottom toolbar's marks (`ui-egui/src/toolbar.rs`) ────────────────────────────────
    //
    // The tool glyphs, the mode switcher's four marks and the Actions bolt. Geometry is ported
    // from `ui-egui/src/icons/draw.rs`'s own 16-unit table, drawn here from this crate's filled
    // primitives (`tri` / `thick_line` / `disc` / `rect` / `poly_fill` / `stroke_path`).
    /// The arrow pointer (`ToolId::Move`).
    Move,
    /// An open hand (`ToolId::Hand`).
    Hand,
    /// A frame with a diagonal resize arrow (`ToolId::Scale`).
    Scale,
    /// A knife (`ToolId::Slice`).
    Slice,
    /// A diagonal line (`ToolId::Line`).
    Line,
    /// A line with a head (`ToolId::Arrow`).
    Arrow,
    /// A pentagon (`ToolId::Polygon`).
    Polygon,
    /// A five-point star (`ToolId::Star`).
    Star,
    /// A pen nib (`ToolId::Pen`).
    Pen,
    /// A pencil (`ToolId::Pencil`, the Draw mode mark).
    Pencil,
    /// Two overlapping rings with the lens filled (`ToolId::ShapeBuilder`).
    ShapeBuilder,
    /// A small T over the curve it follows (`ToolId::TextOnPath`).
    TextOnPath,
    /// A speech bubble (`ToolId::Comment`).
    Comment,
    /// A board sticky note (`ToolId::Sticky`).
    Board,
    /// A marker tip — the brush tool's glyph.
    Marker,
    /// An eraser (`ToolId::Eraser`).
    Eraser,
    /// Shapes (`ToolId::BoardShape`).
    Shapes,
    /// A connector (`ToolId::Connector`).
    Connector,
    /// A stamp (`ToolId::Stamp`).
    Stamp,
    /// The board templates picker.
    Templates,
    /// A table.
    Table,
    /// The Actions bolt.
    Actions,
    /// Three columns (the Design panel's button).
    Panels,
    /// An almond eye with its pupil (the view-only badge).
    Eye,
    /// A motion path with two keyframes (the Animation mode mark).
    Animation,
    /// Angle brackets (the Inspect mode mark).
    Code,
    /// Outlined rectangle with a nested outline (the `Group` node kind).
    Group,
    /// A triangle silhouette (the `Vector` node kind).
    Vector,
    /// Diamond inside an outline rectangle (the `ComponentSet` node kind).
    ComponentSet,
    /// A diamond with a centre dot (the `Icon` node kind).
    IconKind,
    /// Outlined rectangle with a centre dot (a frame that is a component slot).
    Slot,
    /// A padlock (the layer-lock toggle).
    Lock,
    /// An open padlock (the layer-unlock toggle).
    Unlock,
    /// An almond with a diagonal slash (the layer-hidden toggle).
    EyeOff,
    /// Two overlapping filled discs (boolean `Union`).
    Union,
    /// A filled disc behind an outlined one (boolean `Subtract`).
    Subtract,
    /// Two rings with their shared lens filled (boolean `Intersect`).
    Intersect,
    /// Two rings only (boolean `Exclude`).
    Exclude,
    // ── 标签条（上游 `ui-egui/src/tabs.rs`，P0-1）──────────────────────────────────────────
    /// Home tab: a house.
    Home,
    /// Light/dark theme toggle: the sun (shown in dark theme).
    Sun,
    /// Light/dark theme toggle: the moon (shown in light theme).
    Moon,
    /// A presentation tab's play triangle.
    Play,
    /// Frameless title bar: restore.
    WinRestore,
    /// A live session whose connection dropped.
    LiveDisconnected,
    LayoutH,
    LayoutV,
    LayoutGrid,
}

impl Icon {
    /// Every icon, in enum order. Resource tables and tests derive from this list.
    pub const ALL: &'static [Icon] = &[
        Icon::Plus,
        Icon::Minus,
        Icon::Close,
        Icon::Check,
        Icon::ChevronDown,
        Icon::ChevronUp,
        Icon::ChevronLeft,
        Icon::ChevronRight,
        Icon::Search,
        Icon::Dot,
        Icon::Variable,
        Icon::Number,
        Icon::Boolean,
        Icon::LinkOn,
        Icon::LinkOff,
        Icon::Section,
        Icon::Panel,
        Icon::Menu,
        Icon::Settings,
        Icon::Trash,
        Icon::Rectangle,
        Icon::Ellipse,
        Icon::Frame,
        Icon::Text,
        Icon::Component,
        Icon::Instance,
        Icon::Page,
        Icon::Logo,
        Icon::Assets,
        Icon::Variables,
        Icon::Image,
        Icon::ViewGrid,
        Icon::Detach,
        Icon::AlignLeft,
        Icon::AlignHCenter,
        Icon::AlignRight,
        Icon::AlignTop,
        Icon::AlignVCenter,
        Icon::AlignBottom,
        Icon::DistributeH,
        Icon::Rotate,
        Icon::FlipH,
        Icon::FlipV,
        Icon::AbsolutePosition,
        Icon::Move,
        Icon::Hand,
        Icon::Scale,
        Icon::Slice,
        Icon::Line,
        Icon::Arrow,
        Icon::Polygon,
        Icon::Star,
        Icon::Pen,
        Icon::Pencil,
        Icon::ShapeBuilder,
        Icon::TextOnPath,
        Icon::Comment,
        Icon::Board,
        Icon::Marker,
        Icon::Eraser,
        Icon::Shapes,
        Icon::Connector,
        Icon::Stamp,
        Icon::Templates,
        Icon::Table,
        Icon::Actions,
        Icon::Panels,
        Icon::Eye,
        Icon::Animation,
        Icon::Code,
        Icon::Group,
        Icon::Vector,
        Icon::ComponentSet,
        Icon::IconKind,
        Icon::Slot,
        Icon::Lock,
        Icon::Unlock,
        Icon::EyeOff,
        Icon::Union,
        Icon::Subtract,
        Icon::Intersect,
        Icon::Exclude,
        Icon::Home,
        Icon::Sun,
        Icon::Moon,
        Icon::Play,
        Icon::WinRestore,
        Icon::LiveDisconnected,
        Icon::LayoutH,
        Icon::LayoutV,
        Icon::LayoutGrid,
    ];

    /// Stable file stem (kebab-case).
    pub fn asset_name(self) -> &'static str {
        match self {
            Icon::Plus => "plus",
            Icon::Minus => "minus",
            Icon::Close => "close",
            Icon::Check => "check",
            Icon::ChevronDown => "chevron-down",
            Icon::ChevronUp => "chevron-up",
            Icon::ChevronLeft => "chevron-left",
            Icon::ChevronRight => "chevron-right",
            Icon::Search => "search",
            Icon::Dot => "dot",
            Icon::Variable => "variable",
            Icon::Number => "number",
            Icon::Boolean => "boolean",
            Icon::Logo => "logo",
            Icon::Assets => "assets",
            Icon::Variables => "variables",
            Icon::LinkOn => "link-on",
            Icon::LinkOff => "link-off",
            Icon::Section => "section",
            Icon::Panel => "panel",
            Icon::Menu => "menu",
            Icon::Settings => "settings",
            Icon::Trash => "trash",
            Icon::Rectangle => "rectangle",
            Icon::Ellipse => "ellipse",
            Icon::Frame => "frame",
            Icon::Text => "text",
            Icon::Component => "component",
            Icon::Instance => "instance",
            Icon::Page => "page",
            Icon::Image => "image",
            Icon::ViewGrid => "view-grid",
            Icon::Detach => "detach",
            Icon::AlignLeft => "align-left",
            Icon::AlignHCenter => "align-h-center",
            Icon::AlignRight => "align-right",
            Icon::AlignTop => "align-top",
            Icon::AlignVCenter => "align-v-center",
            Icon::AlignBottom => "align-bottom",
            Icon::DistributeH => "distribute-h",
            Icon::Rotate => "rotate",
            Icon::FlipH => "flip-h",
            Icon::FlipV => "flip-v",
            Icon::AbsolutePosition => "absolute-position",
            Icon::Move => "move",
            Icon::Hand => "hand",
            Icon::Scale => "scale",
            Icon::Slice => "slice",
            Icon::Line => "line",
            Icon::Arrow => "arrow",
            Icon::Polygon => "polygon",
            Icon::Star => "star",
            Icon::Pen => "pen",
            Icon::Pencil => "pencil",
            Icon::ShapeBuilder => "shape-builder",
            Icon::TextOnPath => "text-on-path",
            Icon::Comment => "comment",
            Icon::Board => "board",
            Icon::Marker => "marker",
            Icon::Eraser => "eraser",
            Icon::Shapes => "shapes",
            Icon::Connector => "connector",
            Icon::Stamp => "stamp",
            Icon::Templates => "templates",
            Icon::Table => "table",
            Icon::Actions => "actions",
            Icon::Panels => "panels",
            Icon::Eye => "eye",
            Icon::Animation => "animation",
            Icon::Code => "code",
            Icon::Group => "group",
            Icon::Vector => "vector",
            Icon::ComponentSet => "component-set",
            Icon::IconKind => "icon-kind",
            Icon::Slot => "slot",
            Icon::Lock => "lock",
            Icon::Unlock => "unlock",
            Icon::EyeOff => "eye-off",
            Icon::Union => "union",
            Icon::Subtract => "subtract",
            Icon::Intersect => "intersect",
            Icon::Exclude => "exclude",
            Icon::Home => "home",
            Icon::Sun => "sun",
            Icon::Moon => "moon",
            Icon::Play => "play",
            Icon::WinRestore => "win-restore",
            Icon::LiveDisconnected => "live-disconnected",
            Icon::LayoutH => "layout-h",
            Icon::LayoutV => "layout-v",
            Icon::LayoutGrid => "layout-grid",
        }
    }

    /// Flat file stem below `assets/icons/`. The product mark lives under `brand/`.
    pub fn asset_rel(self) -> &'static str {
        match self {
            Icon::Plus => "ui/plus",
            Icon::Minus => "ui/minus",
            Icon::Close => "ui/close",
            Icon::Check => "ui/check",
            Icon::ChevronDown => "ui/chevron-down",
            Icon::ChevronUp => "ui/chevron-up",
            Icon::ChevronLeft => "ui/chevron-left",
            Icon::ChevronRight => "ui/chevron-right",
            Icon::Search => "ui/search",
            Icon::Dot => "ui/dot",
            Icon::Variable => "data/variable",
            Icon::Number => "data/number",
            Icon::Boolean => "data/boolean",
            Icon::Logo => "brand/monet-mono",
            Icon::Assets => "ui/assets",
            Icon::Variables => "data/variables",
            Icon::LinkOn => "ui/link-on",
            Icon::LinkOff => "ui/link-off",
            Icon::Section => "ui/section",
            Icon::Panel => "ui/panel",
            Icon::Menu => "ui/menu",
            Icon::Settings => "ui/settings",
            Icon::Trash => "ui/trash",
            Icon::Rectangle => "design/rectangle",
            Icon::Ellipse => "design/ellipse",
            Icon::Frame => "design/frame",
            Icon::Text => "design/text",
            Icon::Component => "design/component",
            Icon::Instance => "design/instance",
            Icon::Page => "design/page",
            Icon::Image => "design/image",
            Icon::ViewGrid => "ui/view-grid",
            Icon::Detach => "ui/detach",
            Icon::AlignLeft => "layout/align-left",
            Icon::AlignHCenter => "layout/align-h-center",
            Icon::AlignRight => "layout/align-right",
            Icon::AlignTop => "layout/align-top",
            Icon::AlignVCenter => "layout/align-v-center",
            Icon::AlignBottom => "layout/align-bottom",
            Icon::DistributeH => "layout/distribute-h",
            Icon::Rotate => "layout/rotate",
            Icon::FlipH => "layout/flip-h",
            Icon::FlipV => "layout/flip-v",
            Icon::AbsolutePosition => "design/absolute-position",
            Icon::Move => "tools/move",
            Icon::Hand => "tools/hand",
            Icon::Scale => "tools/scale",
            Icon::Slice => "design/slice",
            Icon::Line => "design/line",
            Icon::Arrow => "design/arrow",
            Icon::Polygon => "design/polygon",
            Icon::Star => "design/star",
            Icon::Pen => "design/pen",
            Icon::Pencil => "design/pencil",
            Icon::ShapeBuilder => "design/shape-builder",
            Icon::TextOnPath => "design/text-on-path",
            Icon::Comment => "design/comment",
            Icon::Board => "design/board",
            Icon::Marker => "design/marker",
            Icon::Eraser => "design/eraser",
            Icon::Shapes => "design/shapes",
            Icon::Connector => "design/connector",
            Icon::Stamp => "design/stamp",
            Icon::Templates => "design/templates",
            Icon::Table => "design/table",
            Icon::Actions => "design/actions",
            Icon::Panels => "ui/panels",
            Icon::Eye => "ui/eye",
            Icon::Animation => "design/animation",
            Icon::Code => "design/code",
            Icon::Group => "design/group",
            Icon::Vector => "design/vector",
            Icon::ComponentSet => "design/component-set",
            Icon::IconKind => "design/icon-kind",
            Icon::Slot => "design/slot",
            Icon::Lock => "ui/lock",
            Icon::Unlock => "ui/unlock",
            Icon::EyeOff => "ui/eye-off",
            Icon::Union => "design/union",
            Icon::Subtract => "design/subtract",
            Icon::Intersect => "design/intersect",
            Icon::Exclude => "design/exclude",
            Icon::Home => "ui/home",
            Icon::Sun => "ui/sun",
            Icon::Moon => "ui/moon",
            Icon::Play => "ui/play",
            Icon::WinRestore => "ui/win-restore",
            Icon::LiveDisconnected => "ui/live-disconnected",
            Icon::LayoutH => "layout/layout-h",
            Icon::LayoutV => "layout/layout-v",
            Icon::LayoutGrid => "layout/layout-grid",
        }
    }

    /// Asset path used by GPUI's `svg()` element.
    pub fn asset_path(self) -> SharedString {
        let rel = self.asset_rel();
        if rel.starts_with("brand/") {
            SharedString::from(format!("{rel}.svg"))
        } else {
            SharedString::from(format!("icons/{rel}.svg"))
        }
    }
}

/// One icon's bytes. `rel` is the grouped path without `.svg`.
static ICON_FILES: &[(&str, &[u8])] = &[
    ("data/boolean", include_bytes!("../../assets/icons/data/boolean.svg")),
    ("data/number", include_bytes!("../../assets/icons/data/number.svg")),
    ("data/variable", include_bytes!("../../assets/icons/data/variable.svg")),
    ("data/variables", include_bytes!("../../assets/icons/data/variables.svg")),
    ("design/absolute-position", include_bytes!("../../assets/icons/design/absolute-position.svg")),
    ("design/actions", include_bytes!("../../assets/icons/design/actions.svg")),
    ("design/animation", include_bytes!("../../assets/icons/design/animation.svg")),
    ("design/arrow", include_bytes!("../../assets/icons/design/arrow.svg")),
    ("design/board", include_bytes!("../../assets/icons/design/board.svg")),
    ("design/code", include_bytes!("../../assets/icons/design/code.svg")),
    ("design/comment", include_bytes!("../../assets/icons/design/comment.svg")),
    ("design/component", include_bytes!("../../assets/icons/design/component.svg")),
    ("design/component-set", include_bytes!("../../assets/icons/design/component-set.svg")),
    ("design/connector", include_bytes!("../../assets/icons/design/connector.svg")),
    ("design/ellipse", include_bytes!("../../assets/icons/design/ellipse.svg")),
    ("design/eraser", include_bytes!("../../assets/icons/design/eraser.svg")),
    ("design/exclude", include_bytes!("../../assets/icons/design/exclude.svg")),
    ("design/frame", include_bytes!("../../assets/icons/design/frame.svg")),
    ("design/group", include_bytes!("../../assets/icons/design/group.svg")),
    ("design/icon-kind", include_bytes!("../../assets/icons/design/icon-kind.svg")),
    ("design/image", include_bytes!("../../assets/icons/design/image.svg")),
    ("design/instance", include_bytes!("../../assets/icons/design/instance.svg")),
    ("design/intersect", include_bytes!("../../assets/icons/design/intersect.svg")),
    ("design/line", include_bytes!("../../assets/icons/design/line.svg")),
    ("design/marker", include_bytes!("../../assets/icons/design/marker.svg")),
    ("design/page", include_bytes!("../../assets/icons/design/page.svg")),
    ("design/pen", include_bytes!("../../assets/icons/design/pen.svg")),
    ("design/pencil", include_bytes!("../../assets/icons/design/pencil.svg")),
    ("design/polygon", include_bytes!("../../assets/icons/design/polygon.svg")),
    ("design/rectangle", include_bytes!("../../assets/icons/design/rectangle.svg")),
    ("design/shape-builder", include_bytes!("../../assets/icons/design/shape-builder.svg")),
    ("design/shapes", include_bytes!("../../assets/icons/design/shapes.svg")),
    ("design/slice", include_bytes!("../../assets/icons/design/slice.svg")),
    ("design/slot", include_bytes!("../../assets/icons/design/slot.svg")),
    ("design/stamp", include_bytes!("../../assets/icons/design/stamp.svg")),
    ("design/star", include_bytes!("../../assets/icons/design/star.svg")),
    ("design/subtract", include_bytes!("../../assets/icons/design/subtract.svg")),
    ("design/table", include_bytes!("../../assets/icons/design/table.svg")),
    ("design/templates", include_bytes!("../../assets/icons/design/templates.svg")),
    ("design/text", include_bytes!("../../assets/icons/design/text.svg")),
    ("design/text-on-path", include_bytes!("../../assets/icons/design/text-on-path.svg")),
    ("design/union", include_bytes!("../../assets/icons/design/union.svg")),
    ("design/vector", include_bytes!("../../assets/icons/design/vector.svg")),
    ("layout/align-bottom", include_bytes!("../../assets/icons/layout/align-bottom.svg")),
    ("layout/align-h-center", include_bytes!("../../assets/icons/layout/align-h-center.svg")),
    ("layout/align-left", include_bytes!("../../assets/icons/layout/align-left.svg")),
    ("layout/align-right", include_bytes!("../../assets/icons/layout/align-right.svg")),
    ("layout/align-top", include_bytes!("../../assets/icons/layout/align-top.svg")),
    ("layout/align-v-center", include_bytes!("../../assets/icons/layout/align-v-center.svg")),
    ("layout/distribute-h", include_bytes!("../../assets/icons/layout/distribute-h.svg")),
    ("layout/flip-h", include_bytes!("../../assets/icons/layout/flip-h.svg")),
    ("layout/flip-v", include_bytes!("../../assets/icons/layout/flip-v.svg")),
    ("layout/layout-grid", include_bytes!("../../assets/icons/layout/layout-grid.svg")),
    ("layout/layout-h", include_bytes!("../../assets/icons/layout/layout-h.svg")),
    ("layout/layout-v", include_bytes!("../../assets/icons/layout/layout-v.svg")),
    ("layout/rotate", include_bytes!("../../assets/icons/layout/rotate.svg")),
    ("tools/hand", include_bytes!("../../assets/icons/tools/hand.svg")),
    ("tools/move", include_bytes!("../../assets/icons/tools/move.svg")),
    ("tools/scale", include_bytes!("../../assets/icons/tools/scale.svg")),
    ("ui/assets", include_bytes!("../../assets/icons/ui/assets.svg")),
    ("ui/check", include_bytes!("../../assets/icons/ui/check.svg")),
    ("ui/chevron-down", include_bytes!("../../assets/icons/ui/chevron-down.svg")),
    ("ui/chevron-left", include_bytes!("../../assets/icons/ui/chevron-left.svg")),
    ("ui/chevron-right", include_bytes!("../../assets/icons/ui/chevron-right.svg")),
    ("ui/chevron-up", include_bytes!("../../assets/icons/ui/chevron-up.svg")),
    ("ui/close", include_bytes!("../../assets/icons/ui/close.svg")),
    ("ui/detach", include_bytes!("../../assets/icons/ui/detach.svg")),
    ("ui/dot", include_bytes!("../../assets/icons/ui/dot.svg")),
    ("ui/eye", include_bytes!("../../assets/icons/ui/eye.svg")),
    ("ui/eye-off", include_bytes!("../../assets/icons/ui/eye-off.svg")),
    ("ui/home", include_bytes!("../../assets/icons/ui/home.svg")),
    ("ui/link-off", include_bytes!("../../assets/icons/ui/link-off.svg")),
    ("ui/link-on", include_bytes!("../../assets/icons/ui/link-on.svg")),
    ("ui/live-disconnected", include_bytes!("../../assets/icons/ui/live-disconnected.svg")),
    ("ui/lock", include_bytes!("../../assets/icons/ui/lock.svg")),
    ("ui/menu", include_bytes!("../../assets/icons/ui/menu.svg")),
    ("ui/minus", include_bytes!("../../assets/icons/ui/minus.svg")),
    ("ui/moon", include_bytes!("../../assets/icons/ui/moon.svg")),
    ("ui/panel", include_bytes!("../../assets/icons/ui/panel.svg")),
    ("ui/panels", include_bytes!("../../assets/icons/ui/panels.svg")),
    ("ui/play", include_bytes!("../../assets/icons/ui/play.svg")),
    ("ui/plus", include_bytes!("../../assets/icons/ui/plus.svg")),
    ("ui/search", include_bytes!("../../assets/icons/ui/search.svg")),
    ("ui/section", include_bytes!("../../assets/icons/ui/section.svg")),
    ("ui/settings", include_bytes!("../../assets/icons/ui/settings.svg")),
    ("ui/sun", include_bytes!("../../assets/icons/ui/sun.svg")),
    ("ui/trash", include_bytes!("../../assets/icons/ui/trash.svg")),
    ("ui/unlock", include_bytes!("../../assets/icons/ui/unlock.svg")),
    ("ui/view-grid", include_bytes!("../../assets/icons/ui/view-grid.svg")),
    ("ui/win-restore", include_bytes!("../../assets/icons/ui/win-restore.svg")),
    ("brand/monet-mono", include_bytes!("../../assets/brand/monet-mono.svg")),
];

fn file_bytes(rel: &str) -> Option<&'static [u8]> {
    ICON_FILES.iter().find(|(name, _)| *name == rel).map(|(_, bytes)| *bytes)
}

/// The SVG's own viewBox width.
fn view_box_size(bytes: &[u8]) -> f32 {
    let text = String::from_utf8_lossy(bytes);
    let Some(i) = text.find("viewBox=\"") else { return 16.0 };
    let after = &text[i + "viewBox=\"".len()..];
    after.split_whitespace().nth(2).and_then(|v| v.trim_end_matches('"').parse().ok()).unwrap_or(16.0)
}

/// Replace every `stroke-width` with [`ICON_STROKE`], keeping any other paint attributes.
pub(crate) fn normalise_stroke(bytes: &[u8]) -> Vec<u8> {
    let src = String::from_utf8_lossy(bytes);
    let width = format!("{}", ICON_STROKE);
    let needle = "stroke-width=\"";
    let mut out = String::with_capacity(src.len());
    let mut rest: &str = &src;
    while let Some(i) = rest.find(needle) {
        out.push_str(&rest[..i + needle.len()]);
        let after = &rest[i + needle.len()..];
        let end = after.find('"').unwrap_or(after.len());
        out.push_str(&width);
        rest = &after[end..];
    }
    out.push_str(rest);
    out.into_bytes()
}

/// The authored viewBox stays 16 or 24 units. At render size `size`, scale the authored stroke
/// inversely so the visual width remains exactly [`ICON_STROKE`].
fn rescale_strokes(bytes: &[u8], grid: f32, size: f32) -> Vec<u8> {
    let src = String::from_utf8_lossy(bytes);
    let width = ICON_STROKE * grid / size.max(0.001);
    let needle = "stroke-width=\"";
    let mut out = String::with_capacity(src.len());
    let mut rest: &str = &src;
    while let Some(i) = rest.find(needle) {
        out.push_str(&rest[..i + needle.len()]);
        let after = &rest[i + needle.len()..];
        let end = after.find('"').unwrap_or(after.len());
        if after[..end].parse::<f32>().is_ok() {
            out.push_str(&format!("{}", width));
        } else {
            out.push_str(&after[..end]);
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out.into_bytes()
}

pub fn natural_size(icon: Icon) -> f32 {
    file_bytes(icon.asset_rel()).map(view_box_size).unwrap_or(16.0)
}

pub fn is_24_grid(icon: Icon) -> bool {
    natural_size(icon) >= 24.0
}

/// The six shape-tool marks are authored on the 24-unit grid and drawn at their natural size.
pub fn is_shape_tool(icon: Icon) -> bool {
    matches!(icon, Icon::Rectangle | Icon::Line | Icon::Arrow | Icon::Ellipse | Icon::Polygon | Icon::Star)
}

fn sized_icon_path(icon: Icon, size: f32) -> SharedString {
    SharedString::from(format!("icons/{}@{size}.svg", icon.asset_rel()))
}

pub fn paint(icon: Icon, color: impl Into<gpui::Rgba>, size: f32) -> impl IntoElement {
    let rgba: gpui::Rgba = color.into();
    let path = sized_icon_path(icon, size);
    svg().path(path).w(px(size)).h(px(size)).flex_none().text_color(Hsla::from(rgba))
}

pub fn paint_tool_at(icon: Icon, color: impl Into<gpui::Rgba>, size: f32) -> impl IntoElement {
    paint(icon, color, size)
}

pub fn paint_tool(icon: Icon, color: impl Into<gpui::Rgba>) -> impl IntoElement {
    paint(icon, color, natural_size(icon))
}

/// Paint `icon` centred in `bounds`, scaled to the shorter side, through `Window::paint_svg`.
pub(crate) fn draw_centred(window: &mut Window, bounds: Bounds<Pixels>, icon: Icon, color: gpui::Rgba, cx: &App) {
    let w: f32 = bounds.size.width.into();
    let h: f32 = bounds.size.height.into();
    let side = w.min(h).max(1.0);
    let ox = f32::from(bounds.origin.x) + (w - side) / 2.0;
    let oy = f32::from(bounds.origin.y) + (h - side) / 2.0;
    let square = Bounds { origin: Point::new(px(ox), px(oy)), size: gpui::size(px(side), px(side)) };
    let _ = window.paint_svg(square, icon.asset_path(), TransformationMatrix::unit(), Hsla::from(color), cx);
}

/// Register with `Application::with_assets`. `icons/<group>/<name>[@size].svg`, `brand/…` and the
/// layer-tree glyphs all resolve here.
pub struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(rest) = path.strip_prefix("icons/").and_then(|p| p.strip_suffix(".svg")) {
            let (rel, size) = match rest.rsplit_once('@') {
                Some((rel, sz)) => (rel, sz.parse::<f32>().ok()),
                None => (rest, None),
            };
            if let Some(bytes) = file_bytes(rel) {
                return Ok(Some(match size {
                    Some(size) => Cow::Owned(rescale_strokes(bytes, view_box_size(bytes), size)),
                    None => Cow::Borrowed(bytes),
                }));
            }
        }
        if path == "brand/monet-mono.svg" {
            return Ok(file_bytes("brand/monet-mono").map(Cow::Borrowed));
        }
        if let Some(bytes) = crate::widgets::layer_icons::load(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        Ok(None)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        if path.is_empty() || path == "icons" || path == "icons/" {
            Ok(Icon::ALL.iter().map(|i| i.asset_path()).collect())
        } else {
            Ok(Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_icon_has_an_external_svg_with_the_shared_stroke() {
        assert_eq!(Icon::ALL.len(), 91, "Icon 变体数变了：请同步 ALL / asset_name / 本断言");
        let mut names = HashSet::new();
        for &icon in Icon::ALL {
            let rel = icon.asset_rel();
            assert!(names.insert(rel), "duplicate asset {rel}");
            let bytes = file_bytes(rel).unwrap_or_else(|| panic!("missing {rel}.svg"));
            let text = std::str::from_utf8(bytes).expect("svg is utf8");
            assert!(text.starts_with("<svg "), "{rel}: {text}");
            assert!(text.trim_end().ends_with("</svg>"), "{rel}");
            assert!(!text.contains("NaN") && !text.contains("inf"), "{rel}");
            for cap in text.split("stroke-width=\"").skip(1) {
                let width = cap.split('"').next().unwrap_or("");
                assert_eq!(width, format!("{}", ICON_STROKE), "{rel} stroke width {width}");
            }
        }
    }

    #[test]
    fn asset_source_resolves_every_icon_path() {
        let assets = IconAssets;
        for &icon in Icon::ALL {
            let path = icon.asset_path();
            let got = assets.load(&path).unwrap().unwrap_or_else(|| panic!("load {path}"));
            assert_eq!(&*got, file_bytes(icon.asset_rel()).unwrap(), "{path}");
            let sized = sized_icon_path(icon, 16.0);
            assert!(assets.load(&sized).unwrap().is_some(), "{sized}");
        }
        assert!(assets.load("icons/not-a-real-icon.svg").unwrap().is_none());
        assert_eq!(assets.list("icons").unwrap().len(), Icon::ALL.len());
    }

    #[test]
    fn every_icon_svg_parses_with_the_shared_renderer() {
        for &icon in Icon::ALL {
            let bytes = file_bytes(icon.asset_rel()).unwrap();
            let parsed = usvg::Tree::from_data(bytes, &usvg::Options::default());
            assert!(parsed.is_ok(), "{:?}: {:?}", icon, parsed.err());
        }
    }

    #[test]
    fn rescale_binds_every_stroke_to_one_width() {
        let src = br#"<svg><path stroke-width="2.4"/><path stroke-width="4.8"/></svg>"#;
        let out = String::from_utf8(rescale_strokes(src, 16.0, 32.0)).unwrap();
        assert_eq!(out.matches("stroke-width=\"0.5\"").count(), 2, "{out}");
    }

    #[test]
    fn shape_tool_marks_keep_their_natural_24_grid() {
        for icon in [Icon::Rectangle, Icon::Line, Icon::Arrow, Icon::Ellipse, Icon::Polygon, Icon::Star] {
            assert!(is_shape_tool(icon));
            assert_eq!(natural_size(icon), 24.0, "{icon:?}");
        }
    }
}
