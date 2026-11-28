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
    Menu,
    Settings,
    /// 四圆点：Fill / Stroke / 样式 / 网格行的「打开样式编辑器」按钮（24 网格）。
    StyleSettings,
    Trash,
    /// Filled rectangle (the `Rectangle` node kind).
    Rectangle,
    /// Filled ellipse (the `Ellipse` node kind).
    Ellipse,
    /// Outlined rectangle (a `Frame` / `Group` node kind).
    Frame,
    /// The letter "T" (a `Text` node kind).
    Text,
    /// The bottom toolbar's dedicated text tool glyph.
    ToolText,
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
    /// Frameless title bar: minimise (Figma 16-grid outline).
    WinMinimize,
    /// Frameless title bar: maximise (Figma 16-grid outline).
    WinMaximize,
    /// Frameless title bar: restore (Figma 16-grid outline).
    WinRestore,
    /// Frameless title bar: close (Figma 16-grid outline).
    WinClose,
    /// A live session whose connection dropped.
    LiveDisconnected,
    LayoutH,
    LayoutV,
    LayoutGrid,
    // ── 右栏属性字段（此前走命名路径绕过枚举，统一收编）────────────────────────────────
    CornerIndividual,
    CornerTopLeft,
    CornerTopRight,
    CornerBottomLeft,
    CornerBottomRight,
    PaddingIndividual,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    StrokeSides,
    StrokeSideTop,
    StrokeSideRight,
    StrokeSideBottom,
    StrokeSideLeft,
    StrokeSideCheck,
    /// 对比度行左侧图标的背景圆（填真实背景色）。
    ContrastBackground,
    /// 对比度行左侧图标的前景弧（填当前选中色）。
    ContrastForeground,
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
        Icon::Menu,
        Icon::Settings,
        // 四圆点：Fill / Stroke / 样式 / 网格行的「打开样式编辑器」按钮（24 网格）。
        Icon::StyleSettings,
        Icon::Trash,
        Icon::Rectangle,
        Icon::Ellipse,
        Icon::Frame,
        Icon::Text,
        Icon::ToolText,
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
        Icon::WinMinimize,
        Icon::WinMaximize,
        Icon::WinRestore,
        Icon::WinClose,
        Icon::LiveDisconnected,
        Icon::LayoutH,
        Icon::LayoutV,
        Icon::LayoutGrid,
        Icon::CornerIndividual,
        Icon::CornerTopLeft,
        Icon::CornerTopRight,
        Icon::CornerBottomLeft,
        Icon::CornerBottomRight,
        Icon::PaddingIndividual,
        Icon::PaddingTop,
        Icon::PaddingRight,
        Icon::PaddingBottom,
        Icon::PaddingLeft,
        Icon::StrokeSides,
        Icon::StrokeSideTop,
        Icon::StrokeSideRight,
        Icon::StrokeSideBottom,
        Icon::StrokeSideLeft,
        Icon::StrokeSideCheck,
        Icon::ContrastBackground,
        Icon::ContrastForeground,
    ];

    /// Stable file stem (kebab-case).
    pub fn asset_name(self) -> &'static str {
        match self {
            Icon::Plus => "ui-plus",
            Icon::Minus => "ui-minus",
            Icon::Close => "ui-close",
            Icon::Check => "ui-check",
            Icon::ChevronDown => "ui-chevron-down",
            Icon::ChevronUp => "ui-chevron-up",
            Icon::ChevronLeft => "ui-chevron-left",
            Icon::ChevronRight => "ui-chevron-right",
            Icon::Search => "ui-search",
            Icon::Dot => "ui-dot",
            Icon::Variable => "data-variable",
            Icon::Number => "data-number",
            Icon::Boolean => "data-boolean",
            Icon::Logo => "logo",
            Icon::Assets => "ui-assets",
            Icon::Variables => "data-variables",
            Icon::LinkOn => "ui-link-on",
            Icon::LinkOff => "ui-link-off",
            Icon::Section => "ui-section",
            Icon::Menu => "ui-menu",
            Icon::Settings => "ui-sliders",
            Icon::StyleSettings => "ui-style-settings",
            Icon::Trash => "ui-trash",
            Icon::Rectangle => "shape-rectangle",
            Icon::Ellipse => "shape-ellipse",
            Icon::Frame => "shape-frame",
            Icon::Text => "shape-text",
            Icon::ToolText => "tool-text",
            Icon::Component => "shape-component",
            Icon::Instance => "shape-instance",
            Icon::Page => "shape-page",
            Icon::Image => "shape-image",
            Icon::ViewGrid => "ui-view-grid",
            Icon::Detach => "ui-detach",
            Icon::AlignLeft => "layout-align-left",
            Icon::AlignHCenter => "layout-align-h-center",
            Icon::AlignRight => "layout-align-right",
            Icon::AlignTop => "layout-align-top",
            Icon::AlignVCenter => "layout-align-v-center",
            Icon::AlignBottom => "layout-align-bottom",
            Icon::DistributeH => "layout-distribute-h",
            Icon::Rotate => "layout-rotate",
            Icon::FlipH => "layout-flip-h",
            Icon::FlipV => "layout-flip-v",
            Icon::AbsolutePosition => "shape-absolute-position",
            Icon::Move => "tool-move",
            Icon::Hand => "tool-hand",
            Icon::Scale => "tool-scale",
            Icon::Slice => "shape-slice",
            Icon::Line => "shape-line",
            Icon::Arrow => "shape-arrow",
            Icon::Polygon => "shape-polygon",
            Icon::Star => "shape-star",
            Icon::Pen => "tool-pen",
            Icon::Pencil => "tool-pencil",
            Icon::ShapeBuilder => "tool-shape-builder",
            Icon::TextOnPath => "tool-text-on-path",
            Icon::Comment => "tool-comment",
            Icon::Board => "shape-board",
            Icon::Marker => "tool-marker",
            Icon::Eraser => "tool-eraser",
            Icon::Shapes => "shape-shapes",
            Icon::Connector => "tool-connector",
            Icon::Stamp => "shape-stamp",
            Icon::Templates => "ui-templates",
            Icon::Table => "shape-table",
            Icon::Actions => "ui-actions",
            Icon::Panels => "ui-panels",
            Icon::Eye => "ui-eye",
            Icon::Animation => "ui-animation",
            Icon::Code => "ui-code",
            Icon::Group => "shape-group",
            Icon::Vector => "shape-vector",
            Icon::ComponentSet => "shape-component-set",
            Icon::IconKind => "shape-icon-kind",
            Icon::Slot => "shape-slot",
            Icon::Lock => "ui-lock",
            Icon::Unlock => "ui-unlock",
            Icon::EyeOff => "ui-eye-off",
            Icon::Union => "shape-union",
            Icon::Subtract => "shape-subtract",
            Icon::Intersect => "tool-shape-builder",
            Icon::Exclude => "shape-exclude",
            Icon::Home => "ui-home",
            Icon::Sun => "ui-sun",
            Icon::Moon => "ui-moon",
            Icon::Play => "ui-play",
            Icon::WinMinimize => "ui-win-minimize",
            Icon::WinMaximize => "ui-win-maximize",
            Icon::WinRestore => "ui-win-restore",
            Icon::WinClose => "ui-win-close",
            Icon::LiveDisconnected => "ui-live-disconnected",
            Icon::LayoutH => "layout-h",
            Icon::LayoutV => "layout-v",
            Icon::LayoutGrid => "layout-grid",
            Icon::CornerIndividual => "layout-corner-individual",
            Icon::CornerTopLeft => "layout-corner-tl",
            Icon::CornerTopRight => "layout-corner-tr",
            Icon::CornerBottomLeft => "layout-corner-bl",
            Icon::CornerBottomRight => "layout-corner-br",
            Icon::PaddingIndividual => "layout-padding-individual",
            Icon::PaddingTop => "layout-padding-top",
            Icon::PaddingRight => "layout-padding-right",
            Icon::PaddingBottom => "layout-padding-bottom",
            Icon::PaddingLeft => "layout-padding-left",
            Icon::StrokeSides => "property-stroke-sides",
            Icon::StrokeSideTop => "stroke-side-top",
            Icon::StrokeSideRight => "stroke-side-right",
            Icon::StrokeSideBottom => "stroke-side-bottom",
            Icon::StrokeSideLeft => "stroke-side-left",
            Icon::StrokeSideCheck => "stroke-side-check",
            Icon::ContrastBackground => "ui-contrast-bg",
            Icon::ContrastForeground => "ui-contrast-fg",
        }
    }

    /// Flat file stem below `assets/icons/`. The product mark lives under `brand/`.
    pub fn asset_rel(self) -> &'static str {
        match self {
            Icon::Plus => "ui-plus",
            Icon::Minus => "ui-minus",
            Icon::Close => "ui-close",
            Icon::Check => "ui-check",
            Icon::ChevronDown => "ui-chevron-down",
            Icon::ChevronUp => "ui-chevron-up",
            Icon::ChevronLeft => "ui-chevron-left",
            Icon::ChevronRight => "ui-chevron-right",
            Icon::Search => "ui-search",
            Icon::Dot => "ui-dot",
            Icon::Variable => "data-variable",
            Icon::Number => "data-number",
            Icon::Boolean => "data-boolean",
            Icon::Logo => "brand/monet-mono",
            Icon::Assets => "ui-assets",
            Icon::Variables => "data-variables",
            Icon::LinkOn => "ui-link-on",
            Icon::LinkOff => "ui-link-off",
            Icon::Section => "ui-section",
            Icon::Menu => "ui-menu",
            Icon::Settings => "ui-sliders",
            Icon::StyleSettings => "ui-style-settings",
            Icon::Trash => "ui-trash",
            Icon::Rectangle => "shape-rectangle",
            Icon::Ellipse => "shape-ellipse",
            Icon::Frame => "shape-frame",
            Icon::Text => "shape-text",
            Icon::ToolText => "tool-text",
            Icon::Component => "shape-component",
            Icon::Instance => "shape-instance",
            Icon::Page => "shape-page",
            Icon::Image => "shape-image",
            Icon::ViewGrid => "ui-view-grid",
            Icon::Detach => "ui-detach",
            Icon::AlignLeft => "layout-align-left",
            Icon::AlignHCenter => "layout-align-h-center",
            Icon::AlignRight => "layout-align-right",
            Icon::AlignTop => "layout-align-top",
            Icon::AlignVCenter => "layout-align-v-center",
            Icon::AlignBottom => "layout-align-bottom",
            Icon::DistributeH => "layout-distribute-h",
            Icon::Rotate => "layout-rotate",
            Icon::FlipH => "layout-flip-h",
            Icon::FlipV => "layout-flip-v",
            Icon::AbsolutePosition => "shape-absolute-position",
            Icon::Move => "tool-move",
            Icon::Hand => "tool-hand",
            Icon::Scale => "tool-scale",
            Icon::Slice => "shape-slice",
            Icon::Line => "shape-line",
            Icon::Arrow => "shape-arrow",
            Icon::Polygon => "shape-polygon",
            Icon::Star => "shape-star",
            Icon::Pen => "tool-pen",
            Icon::Pencil => "tool-pencil",
            Icon::ShapeBuilder => "tool-shape-builder",
            Icon::TextOnPath => "tool-text-on-path",
            Icon::Comment => "tool-comment",
            Icon::Board => "shape-board",
            Icon::Marker => "tool-marker",
            Icon::Eraser => "tool-eraser",
            Icon::Shapes => "shape-shapes",
            Icon::Connector => "tool-connector",
            Icon::Stamp => "shape-stamp",
            Icon::Templates => "ui-templates",
            Icon::Table => "shape-table",
            Icon::Actions => "ui-actions",
            Icon::Panels => "ui-panels",
            Icon::Eye => "ui-eye",
            Icon::Animation => "ui-animation",
            Icon::Code => "ui-code",
            Icon::Group => "shape-group",
            Icon::Vector => "shape-vector",
            Icon::ComponentSet => "shape-component-set",
            Icon::IconKind => "shape-icon-kind",
            Icon::Slot => "shape-slot",
            Icon::Lock => "ui-lock",
            Icon::Unlock => "ui-unlock",
            Icon::EyeOff => "ui-eye-off",
            Icon::Union => "shape-union",
            Icon::Subtract => "shape-subtract",
            Icon::Intersect => "tool-shape-builder",
            Icon::Exclude => "shape-exclude",
            Icon::Home => "ui-home",
            Icon::Sun => "ui-sun",
            Icon::Moon => "ui-moon",
            Icon::Play => "ui-play",
            Icon::WinMinimize => "ui-win-minimize",
            Icon::WinMaximize => "ui-win-maximize",
            Icon::WinRestore => "ui-win-restore",
            Icon::WinClose => "ui-win-close",
            Icon::LiveDisconnected => "ui-live-disconnected",
            Icon::LayoutH => "layout-h",
            Icon::LayoutV => "layout-v",
            Icon::LayoutGrid => "layout-grid",
            Icon::CornerIndividual => "layout-corner-individual",
            Icon::CornerTopLeft => "layout-corner-tl",
            Icon::CornerTopRight => "layout-corner-tr",
            Icon::CornerBottomLeft => "layout-corner-bl",
            Icon::CornerBottomRight => "layout-corner-br",
            Icon::PaddingIndividual => "layout-padding-individual",
            Icon::PaddingTop => "layout-padding-top",
            Icon::PaddingRight => "layout-padding-right",
            Icon::PaddingBottom => "layout-padding-bottom",
            Icon::PaddingLeft => "layout-padding-left",
            Icon::StrokeSides => "property-stroke-sides",
            Icon::StrokeSideTop => "stroke-side-top",
            Icon::StrokeSideRight => "stroke-side-right",
            Icon::StrokeSideBottom => "stroke-side-bottom",
            Icon::StrokeSideLeft => "stroke-side-left",
            Icon::StrokeSideCheck => "stroke-side-check",
            Icon::ContrastBackground => "ui-contrast-bg",
            Icon::ContrastForeground => "ui-contrast-fg",
        }
    }

    /// Asset path used by GPUI's `svg()` element.
    pub fn asset_path(self) -> SharedString {
        let rel = self.asset_rel();
        if rel.starts_with("brand/") { SharedString::from(format!("{rel}.svg")) } else { SharedString::from(format!("icons/{rel}.svg")) }
    }
}

/// One icon's bytes. `rel` is the grouped path without `.svg`.
static ICON_FILES: &[(&str, &[u8])] = &[
    ("data-boolean", include_bytes!("../../assets/icons/data-boolean.svg")),
    ("data-number", include_bytes!("../../assets/icons/data-number.svg")),
    ("data-variable", include_bytes!("../../assets/icons/data-variable.svg")),
    ("data-variables", include_bytes!("../../assets/icons/data-variables.svg")),
    ("layout-align-bottom", include_bytes!("../../assets/icons/layout-align-bottom.svg")),
    ("layout-align-h-center", include_bytes!("../../assets/icons/layout-align-h-center.svg")),
    ("layout-align-left", include_bytes!("../../assets/icons/layout-align-left.svg")),
    ("layout-align-right", include_bytes!("../../assets/icons/layout-align-right.svg")),
    ("layout-align-top", include_bytes!("../../assets/icons/layout-align-top.svg")),
    ("layout-align-v-center", include_bytes!("../../assets/icons/layout-align-v-center.svg")),
    ("layout-distribute-h", include_bytes!("../../assets/icons/layout-distribute-h.svg")),
    ("layout-flip-h", include_bytes!("../../assets/icons/layout-flip-h.svg")),
    ("layout-flip-v", include_bytes!("../../assets/icons/layout-flip-v.svg")),
    ("layout-grid", include_bytes!("../../assets/icons/layout-grid.svg")),
    ("layout-h", include_bytes!("../../assets/icons/layout-h.svg")),
    ("layout-rotate", include_bytes!("../../assets/icons/layout-rotate.svg")),
    ("layout-v", include_bytes!("../../assets/icons/layout-v.svg")),
    ("brand/monet-mono", include_bytes!("../../assets/brand/monet-mono.svg")),
    ("shape-absolute-position", include_bytes!("../../assets/icons/shape-absolute-position.svg")),
    ("shape-arrow", include_bytes!("../../assets/icons/shape-arrow.svg")),
    ("shape-board", include_bytes!("../../assets/icons/shape-board.svg")),
    ("shape-component", include_bytes!("../../assets/icons/shape-component.svg")),
    ("shape-component-set", include_bytes!("../../assets/icons/shape-component-set.svg")),
    ("shape-ellipse", include_bytes!("../../assets/icons/shape-ellipse.svg")),
    ("shape-exclude", include_bytes!("../../assets/icons/shape-exclude.svg")),
    ("shape-frame", include_bytes!("../../assets/icons/shape-frame.svg")),
    ("shape-group", include_bytes!("../../assets/icons/shape-group.svg")),
    ("shape-icon-kind", include_bytes!("../../assets/icons/shape-icon-kind.svg")),
    ("shape-image", include_bytes!("../../assets/icons/shape-image.svg")),
    ("shape-instance", include_bytes!("../../assets/icons/shape-instance.svg")),
    ("shape-line", include_bytes!("../../assets/icons/shape-line.svg")),
    ("shape-page", include_bytes!("../../assets/icons/shape-page.svg")),
    ("shape-polygon", include_bytes!("../../assets/icons/shape-polygon.svg")),
    ("shape-rectangle", include_bytes!("../../assets/icons/shape-rectangle.svg")),
    ("shape-shapes", include_bytes!("../../assets/icons/shape-shapes.svg")),
    ("shape-slice", include_bytes!("../../assets/icons/shape-slice.svg")),
    ("shape-slot", include_bytes!("../../assets/icons/shape-slot.svg")),
    ("shape-stamp", include_bytes!("../../assets/icons/shape-stamp.svg")),
    ("shape-star", include_bytes!("../../assets/icons/shape-star.svg")),
    ("shape-subtract", include_bytes!("../../assets/icons/shape-subtract.svg")),
    ("shape-table", include_bytes!("../../assets/icons/shape-table.svg")),
    ("shape-text", include_bytes!("../../assets/icons/shape-text.svg")),
    ("shape-union", include_bytes!("../../assets/icons/shape-union.svg")),
    ("shape-vector", include_bytes!("../../assets/icons/shape-vector.svg")),
    ("property-stroke-sides", include_bytes!("../../assets/icons/property-stroke-sides.svg")),
    ("stroke-side-check", include_bytes!("../../assets/icons/stroke-side-check.svg")),
    ("stroke-side-top", include_bytes!("../../assets/icons/stroke-side-top.svg")),
    ("stroke-side-right", include_bytes!("../../assets/icons/stroke-side-right.svg")),
    ("stroke-side-bottom", include_bytes!("../../assets/icons/stroke-side-bottom.svg")),
    ("stroke-side-left", include_bytes!("../../assets/icons/stroke-side-left.svg")),
    ("tool-comment", include_bytes!("../../assets/icons/tool-comment.svg")),
    ("tool-connector", include_bytes!("../../assets/icons/tool-connector.svg")),
    ("tool-eraser", include_bytes!("../../assets/icons/tool-eraser.svg")),
    ("tool-hand", include_bytes!("../../assets/icons/tool-hand.svg")),
    ("tool-marker", include_bytes!("../../assets/icons/tool-marker.svg")),
    ("tool-move", include_bytes!("../../assets/icons/tool-move.svg")),
    ("tool-pen", include_bytes!("../../assets/icons/tool-pen.svg")),
    ("tool-pencil", include_bytes!("../../assets/icons/tool-pencil.svg")),
    ("tool-scale", include_bytes!("../../assets/icons/tool-scale.svg")),
    ("tool-shape-builder", include_bytes!("../../assets/icons/tool-shape-builder.svg")),
    ("tool-text-on-path", include_bytes!("../../assets/icons/tool-text-on-path.svg")),
    ("tool-text", include_bytes!("../../assets/icons/tool-text.svg")),
    ("ui-actions", include_bytes!("../../assets/icons/ui-actions.svg")),
    ("ui-animation", include_bytes!("../../assets/icons/ui-animation.svg")),
    ("ui-assets", include_bytes!("../../assets/icons/ui-assets.svg")),
    ("ui-check", include_bytes!("../../assets/icons/ui-check.svg")),
    ("ui-chevron-down", include_bytes!("../../assets/icons/ui-chevron-down.svg")),
    ("ui-chevron-left", include_bytes!("../../assets/icons/ui-chevron-left.svg")),
    ("ui-chevron-right", include_bytes!("../../assets/icons/ui-chevron-right.svg")),
    ("ui-chevron-up", include_bytes!("../../assets/icons/ui-chevron-up.svg")),
    ("ui-close", include_bytes!("../../assets/icons/ui-close.svg")),
    ("ui-code", include_bytes!("../../assets/icons/ui-code.svg")),
    ("ui-detach", include_bytes!("../../assets/icons/ui-detach.svg")),
    ("ui-dot", include_bytes!("../../assets/icons/ui-dot.svg")),
    ("ui-eye", include_bytes!("../../assets/icons/ui-eye.svg")),
    ("ui-eye-off", include_bytes!("../../assets/icons/ui-eye-off.svg")),
    ("ui-home", include_bytes!("../../assets/icons/ui-home.svg")),
    ("ui-link-off", include_bytes!("../../assets/icons/ui-link-off.svg")),
    ("ui-link-on", include_bytes!("../../assets/icons/ui-link-on.svg")),
    ("ui-live-disconnected", include_bytes!("../../assets/icons/ui-live-disconnected.svg")),
    ("ui-lock", include_bytes!("../../assets/icons/ui-lock.svg")),
    ("ui-menu", include_bytes!("../../assets/icons/ui-menu.svg")),
    ("ui-minus", include_bytes!("../../assets/icons/ui-minus.svg")),
    ("ui-moon", include_bytes!("../../assets/icons/ui-moon.svg")),
    ("ui-panels", include_bytes!("../../assets/icons/ui-panels.svg")),
    ("ui-play", include_bytes!("../../assets/icons/ui-play.svg")),
    ("ui-plus", include_bytes!("../../assets/icons/ui-plus.svg")),
    ("ui-search", include_bytes!("../../assets/icons/ui-search.svg")),
    ("ui-section", include_bytes!("../../assets/icons/ui-section.svg")),
    ("ui-sliders", include_bytes!("../../assets/icons/ui-sliders.svg")),
    ("ui-style-settings", include_bytes!("../../assets/icons/ui-style-settings.svg")),
    ("ui-sun", include_bytes!("../../assets/icons/ui-sun.svg")),
    ("ui-templates", include_bytes!("../../assets/icons/ui-templates.svg")),
    ("ui-trash", include_bytes!("../../assets/icons/ui-trash.svg")),
    ("ui-unlock", include_bytes!("../../assets/icons/ui-unlock.svg")),
    ("ui-view-grid", include_bytes!("../../assets/icons/ui-view-grid.svg")),
    ("ui-win-minimize", include_bytes!("../../assets/icons/ui-win-minimize.svg")),
    ("ui-win-maximize", include_bytes!("../../assets/icons/ui-win-maximize.svg")),
    ("ui-win-restore", include_bytes!("../../assets/icons/ui-win-restore.svg")),
    ("ui-win-close", include_bytes!("../../assets/icons/ui-win-close.svg")),
    ("layout-corner-bl", include_bytes!("../../assets/icons/layout-corner-bl.svg")),
    ("layout-corner-br", include_bytes!("../../assets/icons/layout-corner-br.svg")),
    ("layout-corner-individual", include_bytes!("../../assets/icons/layout-corner-individual.svg")),
    ("layout-corner-tl", include_bytes!("../../assets/icons/layout-corner-tl.svg")),
    ("layout-corner-tr", include_bytes!("../../assets/icons/layout-corner-tr.svg")),
    ("layout-padding-bottom", include_bytes!("../../assets/icons/layout-padding-bottom.svg")),
    ("layout-padding-individual", include_bytes!("../../assets/icons/layout-padding-individual.svg")),
    ("layout-padding-left", include_bytes!("../../assets/icons/layout-padding-left.svg")),
    ("layout-padding-right", include_bytes!("../../assets/icons/layout-padding-right.svg")),
    ("layout-padding-top", include_bytes!("../../assets/icons/layout-padding-top.svg")),
    ("ui-contrast-bg", include_bytes!("../../assets/icons/ui-contrast-bg.svg")),
    ("ui-contrast-fg", include_bytes!("../../assets/icons/ui-contrast-fg.svg")),
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

/// 图标按钮 / 输入框插槽 / 分段控件统一用它取**原生 glyph 尺寸**：按各自 SVG 的 viewBox
/// （16 或 24）画，不压成 16/12（2026-11-20 用户明令旧标准已废）。24 槽里画 24 网格
/// 1:1，16 网格仍画 16，线宽由 `rescale_strokes` 保持不变。
pub fn default_glyph(icon: Icon) -> f32 {
    natural_size(icon)
}

/// 槽位尺寸 → 该图标在这里该画多大。
///
/// 规则：24pt 槽画原生尺寸；其余槽位（16 / 32）就把图标缩放到槽大小，线宽由
/// `rescale_strokes` 反比补偿，视觉上始终是 `ICON_STROKE`。
/// 将来若某个图标要出专门的 16 / 32 变体，在 `ICON_FILES` 里登记
/// `<stem>@<size>` 资源并在这里优先命中即可，调用处不需改动。
pub fn slot_glyph(icon: Icon, slot: f32) -> f32 {
    let dedicated = format!("{}@{}", icon.asset_rel(), slot as i32);
    if file_bytes(&dedicated).is_some() {
        return slot;
    }
    if slot == 24.0 { natural_size(icon) } else { slot }
}

/// 按槽位画图标，颜色缺省取 `tokens().icon`。这是业务代码唯一需要记的入口：
/// 传 `Icon` 和槽位大小即可，尺寸换算、线宽补偿、默认墨色全在通用层。
pub fn paint_in_slot(icon: Icon, slot: f32, color: Option<gpui::Rgba>) -> impl IntoElement {
    let rgba = color.unwrap_or_else(|| crate::widgets::rgba(crate::widgets::tokens().icon));
    paint(icon, rgba, slot_glyph(icon, slot))
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
        if path.is_empty() || path == "icons" || path == "icons/" { Ok(Icon::ALL.iter().map(|i| i.asset_path()).collect()) } else { Ok(Vec::new()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_icon_has_an_external_svg_with_the_shared_stroke() {
        assert_eq!(Icon::ALL.len(), 113, "Icon 变体数变了：请同步 ALL / asset_name / asset_rel / 本断言");
        // 同一图形可由两个逻辑图标共用一个物理文件（见 assets/icons/README.md）。
        // 只允许明确登记的共用，避免把「漏删的重复文件」混进来。
        const SHARED_ASSETS: &[&str] = &["tool-shape-builder"];
        let mut names = HashSet::new();
        for &icon in Icon::ALL {
            let rel = icon.asset_rel();
            assert!(
                names.insert(rel) || SHARED_ASSETS.contains(&rel),
                "duplicate asset {rel}"
            );
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
    fn slot_glyph_scales_to_the_slot_and_keeps_native_at_24() {
        // 24 槽取原生（无论资源是 16 还是 24 网格）；16 / 32 槽直接按槽位画，
        // 线宽由 rescale_strokes 补偿，视觉恒为 1pt。
        assert_eq!(slot_glyph(Icon::Plus, 24.0), natural_size(Icon::Plus));
        assert_eq!(slot_glyph(Icon::Plus, 16.0), 16.0);
        assert_eq!(slot_glyph(Icon::Plus, 32.0), 32.0);
        assert_eq!(slot_glyph(Icon::CornerTopLeft, 24.0), 24.0);
        assert_eq!(slot_glyph(Icon::CornerTopLeft, 16.0), 16.0);
    }

    #[test]
    fn every_registered_asset_has_an_enum_variant() {
        // 规范：只往 ICON_FILES 加资源、不进枚举 = 图标不存在。这条断言把老路彻底堵死。
        let mapped: HashSet<&str> = Icon::ALL.iter().map(|i| i.asset_rel()).collect();
        for (rel, _) in ICON_FILES {
            assert!(mapped.contains(rel), "{rel} 只注册了资源、没有 Icon 枚举变体");
        }
    }

    #[test]
    fn the_icon_usage_contract_is_documented() {
        let doc = include_str!("../../assets/icons/README.md");
        for clause in [
            "## 使用流程（强制，唯一合法路径）",
            "没有枚举变体的图标等于不存在",
            "icon::paint_in_slot",
            "slot_glyph",
            "rescale_strokes",
            "❌ `svg().path(\"icons/…\")`",
        ] {
            assert!(doc.contains(clause), "图标使用规范缺少条款：{clause}");
        }
    }

    #[test]
    fn layout_property_icons_fill_their_twenty_four_point_slot() {
        for rel in [
            "layout-corner-tl",
            "layout-corner-tr",
            "layout-corner-bl",
            "layout-corner-individual",
            "layout-padding-top",
            "layout-padding-individual",
            "property-stroke-sides",
            "stroke-side-check",
            "ui-code",
        ] {
            let bytes = file_bytes(rel).unwrap_or_else(|| panic!("missing {rel}.svg"));
            assert_eq!(view_box_size(bytes), 24.0, "{rel} must be drawn at its 24 pt slot size");
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

    /// 右栏圆角 / 间距 / sides 下拉引用的 16 网格图标虽无 `Icon` 枚举变体（冻结文件不能加），
    /// 但都已注册进 `ICON_FILES`，必须能从 `icons/<name>.svg` 解析出来 —— 否则字段 lead /
    /// 菜单行画出的是空白方块。
    #[test]
    fn layout_glyphs_resolve_through_the_shared_prefix() {
        let assets = IconAssets;
        for name in [
            "layout-corner-individual",
            "layout-corner-tl",
            "layout-corner-tr",
            "layout-corner-bl",
            "layout-corner-br",
            "layout-padding-individual",
            "layout-padding-top",
            "layout-padding-right",
            "layout-padding-bottom",
            "layout-padding-left",
        ] {
            assert!(file_bytes(name).is_some(), "{name} 没进 ICON_FILES");
            let path = format!("icons/{name}.svg");
            assert!(assets.load(&path).unwrap().is_some(), "{path} 无法解析");
        }
    }

    #[test]
    fn every_icon_svg_parses_with_the_shared_renderer() {
        for &icon in Icon::ALL {
            let bytes = file_bytes(icon.asset_rel()).unwrap();
            let parsed = usvg::Tree::from_data(bytes, &usvg::Options::default());
            assert!(parsed.is_ok(), "{:?}: {:?}", icon, parsed.err());
        }
    }

    /// 图标路径不得用 `transform="scale(...)"` 缩放：它会连 stroke-width 一起缩，
    /// 破坏 ICON_STROKE 的视觉线宽（ui-home 曾被 0.667 缩放 + mask 裁切成 0.33pt）。
    /// 需要不同网格就改 SVG 视图尺寸，用 `rescale_strokes` 按公式补偿。
    #[test]
    fn no_icon_scales_its_path_transform() {
        for &icon in Icon::ALL {
            let rel = icon.asset_rel();
            let text = std::str::from_utf8(file_bytes(rel).unwrap()).unwrap();
            assert!(!text.contains("transform=\"scale("), "{rel}: 路径 transform scale 会缩放 stroke-width，破坏统一线宽");
        }
    }

    /// SVG 遮罩按亮度取 alpha：白 = 显示，黑 = 全透明。GPUI 只取 SVG 的 alpha、颜色由
    /// `text_color` 注入，所以 mask 里的 `fill` **不是颜色而是可见性开关**，绝不能被归一化
    /// 成 `#000`。2026-11-20 图标整理把 5 个 mask 的 `white` 改成 `#000`，图标整个消失。
    ///
    /// 合法写法只有两种：mask 自身 `fill="white"`，或 mask 内有一个 `fill="white"` 的白底
    /// rect（Figma 的 inside-stroke 导出，黑 path 在白底上挖洞）。两种都没有 = 该 mask 全遮。
    #[test]
    fn every_icon_mask_has_an_opaque_white_region() {
        for &icon in Icon::ALL {
            let rel = icon.asset_rel();
            let text = std::str::from_utf8(file_bytes(rel).unwrap()).unwrap();
            let mut rest = text;
            while let Some(at) = rest.find("<mask") {
                let tag_end = rest[at..].find('>').map(|i| at + i).unwrap_or(rest.len());
                let tag = &rest[at..tag_end];
                // mask 内容到 </mask>；取不到就按到 svg 尾。
                let close = rest[tag_end..].find("</mask>").map(|i| tag_end + i);
                let inner = close.map(|i| &rest[tag_end..i]).unwrap_or("");
                let mask_is_white = tag.contains("fill=\"white\"") || tag.contains("fill=\"#fff\"") || tag.contains("fill=\"#ffffff\"");
                let has_white_rect = inner.contains("fill=\"white\"") || inner.contains("fill=\"#fff\"") || inner.contains("fill=\"#ffffff\"");
                assert!(mask_is_white || has_white_rect, "{rel}: mask 既非 white 也无白底 rect → 全透明，被它遮住的描边看不见（PR#9 归一化 bug）");
                match close {
                    Some(i) => rest = &rest[i + "</mask>".len()..],
                    None => break,
                }
            }
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
