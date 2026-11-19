// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
//
// 2026-11-08 用户确认 P0-1 落地蓝图（上一轮交付的对比报告，本轮「继续」）后授权本次解冻：
// 第 3 步要求给标签条补 Home / Sun / Moon / Play / Win* / LiveDisconnected 共 8 个变体，
// 并按上游 #1729 的 20/32/12/26pt 图标规范使用。本次**只新增枚举变体与其绘制分支**，
// 三条不变量、既有 70 个变体的绘制与所有控件行为一律未动。
//
// 2026-11-08 二次解冻（用户指令原文）：
//   「还有你当前这个 SVG 的渲染东西有 bug，我建议你也去抄一下 kit」
//   前置约束：「注意我是让你抄它的，不是让你把整个库搬过来」
//   → 按外部组件库 `gpui-kit` 的 Icon 公开契约（IconName → SVG 资源路径 → `.text_color()`）
//     在**本地**实现，不引入 gpui-kit / gpui_component 依赖。
// ============================================================================
//! Monet's icon set, rendered as real SVG assets through GPUI's SVG pipeline.
//!
//! GPUI 0.2.1 **does** have an SVG element (`svg()` / `Svg`) and `Window::paint_svg()`; the old
//! header claimed otherwise. The previous implementation hand-built every glyph out of filled
//! triangles via `PathBuilder::fill()`: circles were 16/22-segment polygons, diagonal strokes
//! were two triangles with visible seams, and `PathBuilder` curve fills are broken on 0.2.1
//! (see `canvas/painter.rs`). That is the rendering bug this module fixes.
//!
//! Contract copied from `gpui-kit`'s `Icon` (implementation, not the crate):
//! - `IconName`-like enum → a stable asset path `icons/<kebab-name>.svg`;
//! - the bytes come from an `AssetSource` registered with `Application::with_assets()`
//!   (kit's `.data(include_bytes!(...))` / `.path(...)` equivalence);
//! - colour is applied with `.text_color()` (kit's `.text_color(cx.theme().red)`), i.e. GPUI
//!   renders the SVG into an alpha mask and tints it with the given `Hsla`;
//! - size is `.with_size(px(n))` over a 16x16 viewBox (here `.w().h().flex_none()`).
//!
//! The geometry table below is ported from `ui-egui/src/icons/draw.rs` on a 16x16 grid. It is
//! unchanged: the helpers now emit SVG primitives in grid units instead of pixel triangles.
//! `ox/oy/s` are kept (0, 0, 16) so every match arm stays byte-for-byte the same.

use std::{borrow::Cow, sync::OnceLock};

use gpui::{App, AssetSource, Bounds, Hsla, IntoElement, Pixels, Point, Result, SharedString, TransformationMatrix, Window, prelude::*, px, svg};

/// The one stroke width every vector icon uses, in points. Forcing an SVG into a caller-supplied
/// box used to scale each glyph's own stroke differently (a 24-unit mark squeezed into 18 pt drew
/// 0.75 pt, a 16-unit one grew to 1.125 pt). Layer glyphs and shape-tool marks both read this.
pub const ICON_STROKE: f32 = 1.0;

/// Every icon Monet's UI can show. The names mirror `crates/ui-egui/src/icons.rs` so the two
/// front-ends share one vocabulary; glyphs are drawn for the common ones and fall back otherwise.
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
    /// A frame outline with four inward arrowheads — "fit to contents" (`object.resizeToFit`).
    FitContents,
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
    Angle,
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
    /// A highlighter tip (the pen's highlighter style's mark).
    Highlighter,
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
    /// Frameless title bar: minimise.
    WinMinimize,
    /// Frameless title bar: maximise.
    WinMaximize,
    /// Frameless title bar: restore.
    WinRestore,
    /// A live session whose connection dropped.
    LiveDisconnected,
    LayoutH,
    LayoutV,
    LayoutGrid,
}

impl Icon {
    /// 全部图标，按枚举声明顺序（kit `IconName` 的等价物）。
    /// 图标资源表、`AssetSource::list()`、单测都以这张表为准。
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
        Icon::FitContents,
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
        Icon::Angle,
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
        Icon::Highlighter,
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
        Icon::LiveDisconnected,
        Icon::LayoutH,
        Icon::LayoutV,
        Icon::LayoutGrid,
    ];

    /// 资源名（kebab-case，对应 `icons/<name>.svg`）。
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
            Icon::FitContents => "fit-contents",
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
            Icon::Angle => "angle",
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
            Icon::Highlighter => "highlighter",
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
            Icon::WinMinimize => "win-minimize",
            Icon::WinMaximize => "win-maximize",
            Icon::WinRestore => "win-restore",
            Icon::LiveDisconnected => "live-disconnected",
            Icon::LayoutH => "layout-h",
            Icon::LayoutV => "layout-v",
            Icon::LayoutGrid => "layout-grid",
        }
    }

    /// kit `Icon::empty().path("icons/my-brand-logo.svg")` 的等价物：图标 → 资源路径。
    pub fn asset_path(self) -> SharedString {
        let asset = &table()[self as usize];
        debug_assert_eq!(asset.icon, self, "Icon 判别值与资源表顺序不一致");
        asset.path.clone()
    }
}

/// 一个图标的静态资源（路径 + 生成好的 SVG 字节）。
struct IconAsset {
    icon: Icon,
    name: &'static str,
    path: SharedString,
    svg: Vec<u8>,
}

/// The six marks created with the shape tools: the design file's own 24x24 SVGs. They live on a
/// separate `tools/` path so the 94 generated glyphs (also used by Home, the right panel, …) are
/// untouched, and they are drawn at their natural size rather than forced into an 18 pt box.
static TOOL_ASSETS: &[(&str, &[u8])] = &[
    ("rectangle", include_bytes!("../../assets/tools/rectangle.svg")),
    ("line", include_bytes!("../../assets/tools/line.svg")),
    ("arrow", include_bytes!("../../assets/tools/arrow.svg")),
    ("ellipse", include_bytes!("../../assets/tools/ellipse.svg")),
    ("polygon", include_bytes!("../../assets/tools/polygon.svg")),
    ("star", include_bytes!("../../assets/tools/star.svg")),
];

fn tool_stem(icon: Icon) -> Option<&'static str> {
    Some(match icon {
        Icon::Rectangle => "rectangle",
        Icon::Line => "line",
        Icon::Arrow => "arrow",
        Icon::Ellipse => "ellipse",
        Icon::Polygon => "polygon",
        Icon::Star => "star",
        _ => return None,
    })
}

pub fn is_shape_tool(icon: Icon) -> bool {
    tool_stem(icon).is_some()
}

fn tool_bytes(stem: &str) -> Option<&'static [u8]> {
    TOOL_ASSETS.iter().find(|(name, _)| *name == stem).map(|(_, bytes)| *bytes)
}

/// Bind every `stroke-width` in an SVG to [`ICON_STROKE`] at load time.
pub(crate) fn normalise_stroke(bytes: &[u8]) -> Vec<u8> {
    let src = String::from_utf8_lossy(bytes);
    let needle = "stroke-width=\"";
    let width = format!("{}", ICON_STROKE);
    let mut out = String::with_capacity(src.len());
    let mut rest: &str = &src;
    while let Some(i) = rest.find(needle) {
        out.push_str(&rest[..i + needle.len()]);
        let after = &rest[i + needle.len()..];
        let end = after.find('\"').unwrap_or(after.len());
        out.push_str(&width);
        rest = &after[end..];
    }
    out.push_str(rest);
    out.into_bytes()
}

/// An SVG's own viewBox width: the vector occupies exactly the space it has.
fn view_box_size(bytes: &[u8]) -> f32 {
    let text = String::from_utf8_lossy(bytes);
    let Some(i) = text.find("viewBox=\"") else { return 16.0 };
    let after = &text[i + "viewBox=\"".len()..];
    after.split_whitespace().nth(2).and_then(|v| v.trim_end_matches('\"').parse().ok()).unwrap_or(16.0)
}

pub fn natural_size(icon: Icon) -> f32 {
    tool_stem(icon)
        .and_then(tool_bytes)
        .map(view_box_size)
        .unwrap_or_else(|| view_box_size(&table()[icon as usize].svg))
}

/// Draw a shape-tool mark at `size` pt. Geometry scales with the box; the stroke is
/// compensated at load time so its visual width stays exactly [`ICON_STROKE`].
pub fn paint_tool_at(icon: Icon, color: impl Into<gpui::Rgba>, size: f32) -> impl IntoElement {
    let rgba: gpui::Rgba = color.into();
    let path = tool_stem(icon)
        .map(|stem| SharedString::from(format!("tools/{stem}@{size}.svg")))
        .unwrap_or_else(|| sized_icon_path(icon, size));
    svg().path(path).w(px(size)).h(px(size)).flex_none().text_color(Hsla::from(rgba))
}

/// Draw a shape-tool mark at its natural 24 pt size.
pub fn paint_tool(icon: Icon, color: impl Into<gpui::Rgba>) -> impl IntoElement {
    paint_tool_at(icon, color, natural_size(icon))
}

/// Figma 的四个 24×24 描边图标（用户 2026-11-15）：加号 / 减号 / 叉号 / 下拉小箭头。
/// **网格和路径坐标保持原样**，只有显示尺寸会等比缩放；`__STROKE__` 由
/// [`ICON_STROKE`] 按缩放比反算注入，缩放后视觉线宽恒为 1 pt。
const FIGMA_UI_ICONS: [(Icon, &str); 28] = [
    (Icon::Plus, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M12 6.5V12M12 12V17.5M12 12H6.5M12 12H17.5" stroke="black" stroke-width="__STROKE__" stroke-linecap="round"/></svg>"#),
    (Icon::Minus, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M6.5 12H17.5" stroke="black" stroke-width="__STROKE__" stroke-linecap="round"/></svg>"#),
    (Icon::Close, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M6.50003 17.5L13.6109 10.3891L17.5 6.50006M6.5 6.5L17.5 17.5" stroke="black" stroke-width="__STROKE__" stroke-linecap="round"/></svg>"#),
    (Icon::ChevronDown, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M8.99994 11L12 14L14.9999 11" stroke="black" stroke-width="__STROKE__" stroke-linecap="round" stroke-linejoin="round"/></svg>"#),
    // 其余三个方向都是同一个小箭头**旋转**出来的（用户 2026-11-15）：
    // 向下箭头顺时针 90° = 向左；逆时针 90° = 向右；180° = 向上。路径坐标不动。
    (Icon::ChevronLeft, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><g transform="rotate(90 12 12)"><path d="M8.99994 11L12 14L14.9999 11" stroke="black" stroke-width="__STROKE__" stroke-linecap="round" stroke-linejoin="round"/></g></svg>"#),
    (Icon::ChevronRight, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><g transform="rotate(-90 12 12)"><path d="M8.99994 11L12 14L14.9999 11" stroke="black" stroke-width="__STROKE__" stroke-linecap="round" stroke-linejoin="round"/></g></svg>"#),
    (Icon::ChevronUp, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><g transform="rotate(180 12 12)"><path d="M8.99994 11L12 14L14.9999 11" stroke="black" stroke-width="__STROKE__" stroke-linecap="round" stroke-linejoin="round"/></g></svg>"#),
    // 对齐图标（用户 2026-11-19 提供，24 网格）：引导线 30% 不透明、色块 90%。
    // 左 / 水平居中 / 右来自压缩包；上 / 垂直居中 / 下由它们顺时针旋转 90° 得到（路径坐标不动）。
    (Icon::AlignLeft, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M6.5 6.5V17.5" stroke="black" stroke-opacity="0.3" stroke-width="__STROKE__" stroke-linecap="round"/><rect x="8" y="9" width="10" height="2" rx="0.5" fill="black" fill-opacity="0.9"/><rect x="8" y="13" width="6" height="2" rx="0.5" fill="black" fill-opacity="0.9"/></svg>"#),
    (Icon::AlignHCenter, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M12 6.5V17.5" stroke="black" stroke-opacity="0.3" stroke-width="__STROKE__" stroke-linecap="round"/><rect x="7" y="9" width="10" height="2" rx="0.5" fill="black" fill-opacity="0.9"/><rect x="9" y="13" width="6" height="2" rx="0.5" fill="black" fill-opacity="0.9"/></svg>"#),
    (Icon::AlignRight, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M17.5 6.5V17.5" stroke="black" stroke-opacity="0.3" stroke-width="__STROKE__" stroke-linecap="round"/><rect x="6" y="9" width="10" height="2" rx="0.5" fill="black" fill-opacity="0.9"/><rect x="10" y="13" width="6" height="2" rx="0.5" fill="black" fill-opacity="0.9"/></svg>"#),
    (Icon::AlignTop, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><g transform="rotate(90 12 12)"><path d="M6.5 6.5V17.5" stroke="black" stroke-opacity="0.3" stroke-width="__STROKE__" stroke-linecap="round"/><rect x="8" y="9" width="10" height="2" rx="0.5" fill="black" fill-opacity="0.9"/><rect x="8" y="13" width="6" height="2" rx="0.5" fill="black" fill-opacity="0.9"/></g></svg>"#),
    (Icon::AlignVCenter, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><g transform="rotate(90 12 12)"><path d="M12 6.5V17.5" stroke="black" stroke-opacity="0.3" stroke-width="__STROKE__" stroke-linecap="round"/><rect x="7" y="9" width="10" height="2" rx="0.5" fill="black" fill-opacity="0.9"/><rect x="9" y="13" width="6" height="2" rx="0.5" fill="black" fill-opacity="0.9"/></g></svg>"#),
    (Icon::AlignBottom, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><g transform="rotate(90 12 12)"><path d="M17.5 6.5V17.5" stroke="black" stroke-opacity="0.3" stroke-width="__STROKE__" stroke-linecap="round"/><rect x="6" y="9" width="10" height="2" rx="0.5" fill="black" fill-opacity="0.9"/><rect x="10" y="13" width="6" height="2" rx="0.5" fill="black" fill-opacity="0.9"/></g></svg>"#),
    // 镜像（FlipH / FlipV）：用户 2026-11-19 提供的一套，垂直轴为基准，水平轴由它顺时针旋转 90° 得到。
    (Icon::FlipH, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M12 6.5V17.5" stroke="black" stroke-opacity="0.9" stroke-width="__STROKE__" stroke-linecap="round"/><path d="M10 7.5H7.5C6.94772 7.5 6.5 7.94772 6.5 8.5V15.5C6.5 16.0523 6.94772 16.5 7.5 16.5H10" stroke="black" stroke-opacity="0.9" stroke-width="__STROKE__" stroke-linecap="round"/><path d="M14 7.5H16C16.8284 7.5 17.5 8.17157 17.5 9V15C17.5 15.8284 16.8284 16.5 16 16.5H14" stroke="black" stroke-opacity="0.3" stroke-width="__STROKE__" stroke-linecap="round"/></svg>"#),
    (Icon::FlipV, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><g transform="rotate(90 12 12)"><path d="M12 6.5V17.5" stroke="black" stroke-opacity="0.9" stroke-width="__STROKE__" stroke-linecap="round"/><path d="M10 7.5H7.5C6.94772 7.5 6.5 7.94772 6.5 8.5V15.5C6.5 16.0523 6.94772 16.5 7.5 16.5H10" stroke="black" stroke-opacity="0.9" stroke-width="__STROKE__" stroke-linecap="round"/><path d="M14 7.5H16C16.8284 7.5 17.5 8.17157 17.5 9V15C17.5 15.8284 16.8284 16.5 16 16.5H14" stroke="black" stroke-opacity="0.3" stroke-width="__STROKE__" stroke-linecap="round"/></g></svg>"#),
    // 用户 2026-11-19 提供的 6 个 24 网格线框图标（逐项对应，不再乱套）：
    // Frame-4 → Frame（画框），Frame → Number（#），Frame-2 → Boolean，
    // Frame 163 → Text，Frame-1 → Pen，Frame-3 → Page。
    (Icon::Frame, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M6 8H18M6 16H18M8 6V18M16 18V6" stroke="black" stroke-width="__STROKE__" stroke-linecap="round"/></svg>"#),
    (Icon::Number, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M10.5 6.5L9.5 17.5" stroke="black" stroke-width="__STROKE__" stroke-linecap="round"/><path d="M14.5 6.5L13.5 17.5" stroke="black" stroke-width="__STROKE__" stroke-linecap="round"/><path d="M6.5 10H17.5M6.5 14H17.5" stroke="black" stroke-width="__STROKE__" stroke-linecap="round"/></svg>"#),
    (Icon::Boolean, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><mask id="stroke0_274_459" fill="white"><path d="M9 16H15C17.2091 16 19 14.2091 19 12C19 9.79086 17.2091 8 15 8H9C6.79086 8 5 9.79086 5 12C5 14.2091 6.79086 16 9 16Z"/></mask><path d="M15 16H9C6.79086 16 5 14.2091 5 12C5 9.79086 6.79086 8 9 8H15C17.2091 8 19 9.79086 19 12C19 14.2091 17.2091 16 15 16Z" stroke="black" stroke-width="__STROKE__" mask="url(#stroke0_274_459)"/><circle cx="9.5" cy="12" r="2" stroke="black" stroke-width="__STROKE__"/></svg>"#),
    (Icon::Text, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M6.5 9V6.5H12M17.5 9V6.5H12M12 6.5V17.5M12 17.5H9.5M12 17.5H14.5" stroke="black" stroke-width="__STROKE__" stroke-linecap="round" stroke-linejoin="round"/></svg>"#),
    (Icon::Pen, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M13.5 17.5L14.6464 18.6464C14.8417 18.8417 15.1583 18.8417 15.3536 18.6464L18.6464 15.3536C18.8417 15.1583 18.8417 14.8417 18.6464 14.6464L17.5 13.5M13.5 17.5L17.5 13.5M13.5 17.5L11.0205 17.0041C9.48501 16.697 8.26993 15.5222 7.91128 13.9979L6.62759 8.54225C6.54827 8.20514 6.64902 7.85098 6.8939 7.6061L7.25 7.25M17.5 13.5L17.0041 11.0205C16.697 9.48501 15.5222 8.26993 13.9979 7.91128L8.54225 6.62759C8.20514 6.54827 7.85098 6.64902 7.6061 6.8939L7.25 7.25M7.25 7.25L11 11" stroke="black" stroke-width="__STROKE__" stroke-linejoin="round"/><circle cx="12" cy="12" r="1.5" stroke="black" stroke-width="__STROKE__"/></svg>"#),
    (Icon::Page, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M8.5 6H12.793C12.925 6 13.052 6.053 13.146 6.146L16.854 9.854C16.947 9.947 17 10.075 17 10.208V16.5C17 17.328 16.328 18 15.5 18H8.5C7.672 18 7 17.328 7 16.5V7.5C7 6.672 7.672 6 8.5 6Z" stroke="black" stroke-width="__STROKE__" stroke-linejoin="round"/><path d="M13 7V10H16" stroke="black" stroke-width="__STROKE__" stroke-linejoin="round"/></svg>"#),
    // 自动布局方向（用户 2026-11-19 的第二份包）：横向 = Frame-2 的胶囊，竖向由它顺时针
    // 旋转 90° 得到，网格 = Frame-4 的四个角标。
    (Icon::LayoutH, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M9 16H15C17.2091 16 19 14.2091 19 12C19 9.79086 17.2091 8 15 8H9C6.79086 8 5 9.79086 5 12C5 14.2091 6.79086 16 9 16Z" stroke="black" stroke-width="__STROKE__"/><circle cx="9.5" cy="12" r="2" stroke="black" stroke-width="__STROKE__"/></svg>"#),
    (Icon::LayoutV, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><g transform="rotate(90 12 12)"><path d="M9 16H15C17.2091 16 19 14.2091 19 12C19 9.79086 17.2091 8 15 8H9C6.79086 8 5 9.79086 5 12C5 14.2091 6.79086 16 9 16Z" stroke="black" stroke-width="__STROKE__"/><circle cx="9.5" cy="12" r="2" stroke="black" stroke-width="__STROKE__"/></g></svg>"#),
    (Icon::LayoutGrid, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M6 8H18M6 16H18M8 6V18M16 18V6" stroke-linecap="round" stroke="black" stroke-width="__STROKE__"/></svg>"#),
    // Move = 主菜单用的商标（monet-mono）纯线框版，几何不动，线宽走 ICON_STROKE。
    (Icon::Move, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M6.58595 6.58595C7.14047 6.03143 7.96787 5.8517 8.70216 6.12697L16.7022 9.12697C17.5218 9.43433 18.0467 10.2394 17.9971 11.1133C17.9473 11.9873 17.3346 12.728 16.4854 12.9404L13.6494 13.6494L12.9404 16.4854C12.728 17.3346 11.9873 17.9473 11.1133 17.9971C10.2394 18.0467 9.43433 17.5218 9.12697 16.7022L6.12697 8.70216C5.8517 7.96787 6.03143 7.14047 6.58595 6.58595Z" stroke="black" stroke-width="__STROKE__" stroke-linejoin="round"/></svg>"#),
    // 标题栏用产品自带的减号 / 方框，不另造 Win* 图标。
    (Icon::Play, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><g transform="rotate(90 12 12)"><path d="M10.9173 6.87673L5.58153 16.1253C5.10076 16.9586 5.70219 17.9999 6.66427 17.9999H17.3357C18.2978 17.9999 18.8992 16.9586 18.4185 16.1253L13.0827 6.87672C12.6017 6.04293 11.3983 6.04293 10.9173 6.87673Z" stroke="black" stroke-width="__STROKE__" stroke-linejoin="round"/></g></svg>"#),
    (Icon::Rectangle, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><rect x="6.5" y="6.5" width="11" height="11" rx="1" stroke="black" stroke-width="__STROKE__"/></svg>"#),
    (Icon::WinRestore, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><rect x="8.5" y="5.5" width="9" height="9" fill="none" stroke="black" stroke-width="__STROKE__"/><path d="M15.5 15.5V18.5H6.5V9.5H9.5" fill="none" stroke="black" stroke-width="__STROKE__" stroke-linejoin="round"/></svg>"#),
];

/// `size` 是最终显示的点数：24 网格等比缩到 `size` 时把描边反向放大 `24/size`，
/// 于是缩放后的线宽仍精确等于 [`ICON_STROKE`]。
fn figma_ui_icon(icon: Icon, size: f32) -> Option<Vec<u8>> {
    let (_, src) = FIGMA_UI_ICONS.iter().find(|(i, _)| *i == icon)?;
    let width = ICON_STROKE * 24.0 / size.max(0.001);
    let mut out = src.replace("__STROKE__", &format!("{}", width));
    // 用户 2026-11-19 的两组 24 网格线框图标图稿只占中间 6.5..17.5，按 16 pt 画出来比旧的
    // 16 网格字形小一圈。渲染色时把图稿按中心放大，viewBox 不变（描边同比变粗到约 1.4 pt，
    // 接近旧图标的观感）。
    let k = art_scale(icon);
    if k != 1.0
        && out.starts_with("<svg ")
        && let (Some(open), Some(close)) = (out.find('>'), out.rfind("</svg>"))
    {
        let inner = &out[open + 1..close];
        out = format!("{}<g transform=\"translate(12 12) scale({k}) translate(-12 -12)\">{inner}</g></svg>", &out[..=open]);
    }
    Some(out.into_bytes())
}

/// 图稿放大倍数：只有用户提供的线框图标（对齐 / 镜像 / 节点类型 / 自动布局方向）需要，
/// 加号、箭头这些原有图标保持原样。
fn art_scale(icon: Icon) -> f32 {
    match icon {
        Icon::AlignLeft
        | Icon::AlignHCenter
        | Icon::AlignRight
        | Icon::AlignTop
        | Icon::AlignVCenter
        | Icon::AlignBottom
        | Icon::FlipH
        | Icon::FlipV
        | Icon::Frame
        | Icon::Number
        | Icon::Boolean
        | Icon::Text
        | Icon::Pen
        | Icon::Page
        | Icon::LayoutH
        | Icon::LayoutV
        | Icon::LayoutGrid => 1.4,
        _ => 1.0,
    }
}

/// 每个图标按显示尺寸各有一个资源路径（`icons/<name>@<size>.svg`）。
/// 资源加载时按网格反算 stroke-width，所以任意尺寸下视觉线宽都等于 ICON_STROKE。
fn sized_icon_path(icon: Icon, size: f32) -> SharedString {
    SharedString::from(format!("icons/{}@{}.svg", icon.asset_name(), size))
}

/// 把 SVG 里所有 `stroke-width` 统一改成 ICON_STROKE 按 `grid / size` 反算后的值：
/// 无论原资源写的是 1、1.6 还是 2.4，等比缩放后视觉线宽都精确等于 ICON_STROKE。
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

/// 只在首次访问时把 79 个图标编译成 SVG；之后 `AssetSource::load()` 零拷贝返回。
static TABLE: OnceLock<Vec<IconAsset>> = OnceLock::new();

fn table() -> &'static Vec<IconAsset> {
    TABLE.get_or_init(|| {
        Icon::ALL
            .iter()
            .map(|&icon| {
                // The product mark lives under `brand/`, every other glyph under `icons/`.
                if icon == Icon::Logo {
                    return IconAsset {
                        icon,
                        name: "monet-mono",
                        path: SharedString::from("brand/monet-mono.svg"),
                        svg: include_bytes!("../../assets/brand/monet-mono.svg").to_vec(),
                    };
                }
                let name = icon.asset_name();
                let path = SharedString::from(format!("icons/{name}.svg"));
                let svg = figma_ui_icon(icon, 24.0).unwrap_or_else(|| svg_canvas(icon).to_svg().into_bytes());
                IconAsset { icon, name, path, svg }
            })
            .collect()
    })
}

/// 本地 `AssetSource`：把 `icons/<name>.svg` 映射到内存里的 SVG 字节。
///
/// 这就是 kit 里 `.data(include_bytes!("search.svg"))` / `.path("icons/...")` 背后的机制。
/// GPUI 的默认资源源是 `()`（什么都不加载），所以**必须**在 `Application` 上注册本类型，
/// 否则 `svg()` 会静默不画。项目里的 `img(ImageSource::Render(...))` 不经过 `AssetSource`，
/// 不受影响。
pub struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // `icons/<name>@<size>.svg`：所有图标（不只是 Figma 那几个）按尺寸反算线宽。
        if let Some(rest) = path.strip_prefix("icons/").and_then(|p| p.strip_suffix(".svg")) {
            if let Some((name, sz)) = rest.rsplit_once('@') {
                if let Ok(size) = sz.parse::<f32>() {
                    if let Some((icon, _)) = FIGMA_UI_ICONS.iter().find(|(i, _)| i.asset_name() == name) {
                        return Ok(figma_ui_icon(*icon, size).map(Cow::Owned));
                    }
                    if let Some(icon) = Icon::ALL.iter().copied().find(|i| i.asset_name() == name) {
                        if let Some(stem) = tool_stem(icon) {
                            return Ok(tool_bytes(stem).map(|b| Cow::Owned(rescale_strokes(b, 24.0, size))));
                        }
                        let grid = if icon == Icon::Logo { 24.0 } else { 16.0 };
                        return Ok(table().get(icon as usize).map(|a| Cow::Owned(rescale_strokes(&a.svg, grid, size))));
                    }
                }
            }
        }
        // The shape-tool marks keep their own 24x24 viewBox and one shared stroke width.
        if let Some(rest) = path.strip_prefix("tools/").and_then(|p| p.strip_suffix(".svg")) {
            let (stem, size) = match rest.rsplit_once('@') {
                Some((s, sz)) => (s, sz.parse::<f32>().ok().unwrap_or(24.0)),
                None => (rest, 24.0),
            };
            return Ok(tool_bytes(stem).map(|bytes| Cow::Owned(rescale_strokes(bytes, 24.0, size))));
        }
        // lizard's layer-tree glyphs are the original 16x16 SVGs, served verbatim.
        if let Some(bytes) = crate::widgets::layer_icons::load(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        // The two halves of the colour mark, tinted separately by the caller.
        match path {
            "brand/monet-dots.svg" => {
                return Ok(Some(Cow::Borrowed(include_bytes!("../../assets/brand/monet-dots.svg").as_slice())));
            }
            "brand/monet-path.svg" => {
                return Ok(Some(Cow::Borrowed(include_bytes!("../../assets/brand/monet-path.svg").as_slice())));
            }
            _ => {}
        }
        // Resolve by full path so `icons/*.svg` and `brand/*.svg` share one table.
        Ok(table().iter().find(|a| a.path.as_ref() == path).map(|a| Cow::Borrowed(a.svg.as_slice())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        if path.is_empty() || path == "icons" || path == "icons/" { Ok(table().iter().map(|a| a.path.clone()).collect()) } else { Ok(Vec::new()) }
    }
}

/// 16x16 viewBox 的矢量图元收集器。坐标都是设计网格单位（0..16）。
#[derive(Default)]
struct SvgCanvas {
    shapes: Vec<String>,
}

/// 用户 2026-11-16 提供的 24×24 房子轮廓路径（原样使用，坐标 24 网格）。
const HOME_PATH: &str = "M7 17L7.00001 11.4142C7.00001 11.149 7.10536 10.8946 7.2929 10.7071L11.2929 6.70711C11.6834 6.31658 12.3166 6.31658 12.7071 6.70711L16.7071 10.7071C16.8947 10.8946 17 11.149 17 11.4142V17C17 17.5523 16.5523 18 16 18H14C13.4477 18 13 17.5523 13 17V14H11V17C11 17.5523 10.5523 18 10 18H8C7.44772 18 7 17.5523 7 17Z";

impl SvgCanvas {
    /// 直接塞入一段现成的 SVG 片段（外部给定路径 / mask 时用，避免再拆成图元）。
    fn raw(&mut self, s: &str) {
        self.shapes.push(s.to_string());
    }

    fn polygon(&mut self, pts: &[(f32, f32)]) {
        if pts.len() < 3 {
            return;
        }
        let d =
            pts.iter().enumerate().map(|(i, (x, y))| format!("{}{:.3} {:.3}", if i == 0 { "M" } else { "L" }, x, y)).collect::<Vec<_>>().join(" ");
        self.shapes.push(format!("<path d=\"{d} Z\"/>"));
    }

    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
        self.polyline(&[(x0, y0), (x1, y1)], false, w);
    }

    fn polyline(&mut self, pts: &[(f32, f32)], closed: bool, w: f32) {
        if pts.len() < 2 {
            return;
        }
        let mut d = String::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            d.push_str(&format!("{}{:.3} {:.3} ", if i == 0 { "M" } else { "L" }, x, y));
        }
        if closed {
            d.push('Z');
        }
        // Round caps/joins replace the two-triangle strokes: no seams, no polygonal corners.
        self.shapes.push(format!(
            "<path d=\"{d}\" fill=\"none\" stroke=\"#000\" stroke-width=\"{w:.3}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>"
        ));
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32) {
        self.shapes.push(format!("<circle cx=\"{cx:.3}\" cy=\"{cy:.3}\" r=\"{:.3}\"/>", r.max(0.0)));
    }

    fn ring(&mut self, cx: f32, cy: f32, r: f32, w: f32) {
        self.shapes
            .push(format!("<circle cx=\"{cx:.3}\" cy=\"{cy:.3}\" r=\"{:.3}\" fill=\"none\" stroke=\"#000\" stroke-width=\"{w:.3}\"/>", r.max(0.0)));
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let (x, w) = ordered(x0, x1);
        let (y, h) = ordered(y0, y1);
        self.shapes.push(format!("<rect x=\"{x:.3}\" y=\"{y:.3}\" width=\"{w:.3}\" height=\"{h:.3}\"/>"));
    }

    fn outline_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
        let (x, wd) = ordered(x0, x1);
        let (y, h) = ordered(y0, y1);
        self.shapes.push(format!(
            "<rect x=\"{x:.3}\" y=\"{y:.3}\" width=\"{wd:.3}\" height=\"{h:.3}\" fill=\"none\" stroke=\"#000\" stroke-width=\"{w:.3}\"/>"
        ));
    }

    fn to_svg(self) -> String {
        // GPUI's SvgRenderer keeps only the alpha channel, so the ink colour here is irrelevant;
        // `.text_color()` / `paint_svg(color)` supplies the actual tint.
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\" fill=\"#000\" stroke=\"none\">{}</svg>",
            self.shapes.join("")
        )
    }
}

fn ordered(a: f32, b: f32) -> (f32, f32) {
    if a <= b { (a, b - a) } else { (b, a - b) }
}

/// 把一个图标的几何表翻译成 SVG 图元。`ox/oy/s` 固定为 (0, 0, 16)：设计网格直接就是
/// viewBox 单位，下面 380 行 match 分支与 ui-egui 图标表保持一致。
fn svg_canvas(icon: Icon) -> SvgCanvas {
    let ox = 0.0_f32;
    let oy = 0.0_f32;
    let s = 16.0_f32;
    let mut p = SvgCanvas::default();
    match icon {
        Icon::Plus => {
            thick_line(&mut p, ox, oy, s, 8.0, 3.0, 8.0, 13.0, 2.0);
            thick_line(&mut p, ox, oy, s, 3.0, 8.0, 13.0, 8.0, 2.0);
        }
        Icon::Minus => {
            thick_line(&mut p, ox, oy, s, 3.0, 8.0, 13.0, 8.0, 2.0);
        }
        Icon::Close => {
            thick_line(&mut p, ox, oy, s, 4.0, 4.0, 12.0, 12.0, 2.0);
            thick_line(&mut p, ox, oy, s, 12.0, 4.0, 4.0, 12.0, 2.0);
        }
        Icon::Check => {
            thick_line(&mut p, ox, oy, s, 3.5, 8.5, 6.5, 11.5, 2.0);
            thick_line(&mut p, ox, oy, s, 6.5, 11.5, 12.5, 4.5, 2.0);
        }
        Icon::ChevronDown => {
            thick_line(&mut p, ox, oy, s, 4.0, 6.0, 8.0, 10.0, 2.0);
            thick_line(&mut p, ox, oy, s, 8.0, 10.0, 12.0, 6.0, 2.0);
        }
        Icon::ChevronUp => {
            thick_line(&mut p, ox, oy, s, 4.0, 10.0, 8.0, 6.0, 2.0);
            thick_line(&mut p, ox, oy, s, 8.0, 6.0, 12.0, 10.0, 2.0);
        }
        Icon::ChevronLeft => {
            thick_line(&mut p, ox, oy, s, 10.0, 4.0, 6.0, 8.0, 2.0);
            thick_line(&mut p, ox, oy, s, 6.0, 8.0, 10.0, 12.0, 2.0);
        }
        Icon::ChevronRight => {
            thick_line(&mut p, ox, oy, s, 6.0, 4.0, 10.0, 8.0, 2.0);
            thick_line(&mut p, ox, oy, s, 10.0, 8.0, 6.0, 12.0, 2.0);
        }
        Icon::Search => {
            disc(&mut p, ox, oy, s, 7.0, 7.0, 4.5);
            thick_line(&mut p, ox, oy, s, 10.5, 10.5, 13.5, 13.5, 2.0);
        }
        Icon::Dot => {
            disc(&mut p, ox, oy, s, 8.0, 8.0, 3.5);
        }
        // A hexagon with a dot: an original mark for "bind a value" (`draw.rs::Variable`).
        Icon::Variable => {
            let pts: Vec<(f32, f32)> = (0..6)
                .map(|i| {
                    let a = std::f32::consts::FRAC_PI_3 * i as f32 + std::f32::consts::FRAC_PI_6;
                    (8.0 + 6.0 * a.cos(), 8.0 + 6.0 * a.sin())
                })
                .collect();
            stroke_path(&mut p, ox, oy, s, &pts, true, 1.4);
            disc(&mut p, ox, oy, s, 8.0, 8.0, 1.7);
        }
        // Real glyphs come from FIGMA_UI_ICONS (user-supplied 24-grid SVGs).
        Icon::Number | Icon::Boolean => {}
        Icon::LinkOn | Icon::LinkOff => {
            let capsule = |a: (f32, f32), b: (f32, f32), r: f32| -> Vec<(f32, f32)> {
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let len = (dx * dx + dy * dy).sqrt().max(0.001);
                let (nx, ny) = (-dy / len * r, dx / len * r);
                let mut v = arc_pts(a.0, a.1, r, std::f32::consts::FRAC_PI_2, 3.0 * std::f32::consts::FRAC_PI_2, 8);
                v.iter_mut().for_each(|q| *q = (q.0 + nx, q.1 + ny));
                let mut back = arc_pts(b.0, b.1, r, 3.0 * std::f32::consts::FRAC_PI_2, 5.0 * std::f32::consts::FRAC_PI_2, 8);
                back.iter_mut().for_each(|q| *q = (q.0 - nx, q.1 - ny));
                v.extend(back);
                v
            };
            let (l, r) = if matches!(icon, Icon::LinkOn) {
                (((3.5, 8.0), (6.5, 8.0), 2.2), ((9.5, 8.0), (12.5, 8.0), 2.2))
            } else {
                (((3.5, 12.5), (5.5, 10.5), 2.2), ((10.5, 5.5), (12.5, 3.5), 2.2))
            };
            for (a, b, r) in [l, r] {
                let pts = capsule(a, b, r);
                stroke_path(&mut p, ox, oy, s, &pts, true, 1.4);
            }
            if matches!(icon, Icon::LinkOff) {
                thick_line(&mut p, ox, oy, s, 9.5, 12.0, 9.5, 14.0, 1.4);
                thick_line(&mut p, ox, oy, s, 12.0, 9.5, 14.0, 9.5, 1.4);
                thick_line(&mut p, ox, oy, s, 2.0, 6.5, 4.0, 6.5, 1.4);
                thick_line(&mut p, ox, oy, s, 6.5, 2.0, 6.5, 4.0, 1.4);
            }
        }
        Icon::Menu => {
            for y in [3.5, 7.5, 11.5] {
                thick_line(&mut p, ox, oy, s, 2.0, y, 14.0, y, 1.5);
            }
        }
        Icon::Panel | Icon::Settings | Icon::Trash => {
            disc(&mut p, ox, oy, s, 8.0, 8.0, 3.5);
        }
        Icon::Logo => {
            // Monet's mark: the chameleon's tail, one bold stroke of three half turns
            // (`draw.rs::Logo`).
            use std::f32::consts::FRAC_PI_2;
            let mut tail = vec![(13.75, 13.0)];
            tail.extend(arc_pts(7.25, 8.0, 5.0, FRAC_PI_2, 3.0 * FRAC_PI_2, 16));
            tail.extend(arc_pts(7.25, 6.5, 3.5, 3.0 * FRAC_PI_2, 5.0 * FRAC_PI_2, 16));
            tail.extend(arc_pts(7.25, 8.25, 1.75, FRAC_PI_2, 3.0 * FRAC_PI_2, 16));
            stroke_path(&mut p, ox, oy, s, &tail, false, 1.7);
            disc(&mut p, ox, oy, s, 13.75, 13.0, 0.85);
            disc(&mut p, ox, oy, s, 7.25, 6.5, 0.85);
        }
        Icon::Assets => {
            // A diamond, a circle, a square and a triangle (`draw.rs::Assets`).
            stroke_path(&mut p, ox, oy, s, &[(5.0, 2.0), (8.0, 5.0), (5.0, 8.0), (2.0, 5.0)], true, 1.4);
            ring(&mut p, ox, oy, s, 11.0, 5.0, 2.5, 1.4, 16);
            stroke_path(&mut p, ox, oy, s, &[(2.5, 9.5), (7.5, 9.5), (7.5, 13.5), (2.5, 13.5)], true, 1.4);
            stroke_path(&mut p, ox, oy, s, &[(11.0, 9.0), (13.5, 13.5), (8.5, 13.5)], true, 1.4);
        }
        Icon::Variables => {
            // Four squares rather than dots: the editor's own mark (`draw.rs::Variables`).
            for (x, y) in [(5.0, 5.0), (11.0, 5.0), (5.0, 11.0), (11.0, 11.0)] {
                rect(&mut p, ox, oy, s, x - 2.0, y - 2.0, x + 2.0, y + 2.0);
            }
        }
        Icon::Section => {
            // A dashed-outline section with its solid title strip (`ui-egui`: a rect at 2.5,5.5 →
            // 13.5,13.5 under a filled 2,2 → 9,6 tab).
            outline_rect(&mut p, ox, oy, s, 2.5, 5.5, 13.5, 13.5, 1.4);
            rect(&mut p, ox, oy, s, 2.0, 2.0, 9.0, 6.0);
        }
        Icon::Rectangle => {
            rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0);
        }
        Icon::Ellipse => {
            disc(&mut p, ox, oy, s, 8.0, 8.0, 5.5);
        }
        Icon::Frame => {
            outline_rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0, 2.0);
        }
        Icon::FitContents => {
            // A frame outline with four inward-pointing arrowheads (shrink the frame to
            // wrap its children). Mirrors egui `Icon::FitContents` → `object.resizeToFit`.
            outline_rect(&mut p, ox, oy, s, 2.5, 2.5, 13.5, 13.5, 1.3);
            tri(&mut p, ox, oy, s, 2.5, 5.5, 5.5, 2.5, 5.5, 5.5); // top-left → centre
            tri(&mut p, ox, oy, s, 13.5, 5.5, 10.5, 2.5, 10.5, 5.5); // top-right → centre
            tri(&mut p, ox, oy, s, 2.5, 10.5, 5.5, 13.5, 5.5, 10.5); // bottom-left → centre
            tri(&mut p, ox, oy, s, 13.5, 10.5, 10.5, 13.5, 10.5, 10.5); // bottom-right → centre
        }
        Icon::Text => {
            thick_line(&mut p, ox, oy, s, 8.0, 3.0, 8.0, 13.0, 2.0);
            thick_line(&mut p, ox, oy, s, 4.0, 3.0, 12.0, 3.0, 2.0);
        }
        Icon::Component => {
            diamond(&mut p, ox, oy, s, 8.0, 8.0, 5.5);
        }
        Icon::Instance => {
            outline_diamond(&mut p, ox, oy, s, 8.0, 8.0, 5.5, 2.0);
        }
        Icon::Page => {
            outline_rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0, 2.0);
            thick_line(&mut p, ox, oy, s, 9.0, 3.0, 13.0, 7.0, 2.0);
        }
        Icon::Image => {
            outline_rect(&mut p, ox, oy, s, 2.5, 3.0, 13.5, 13.0, 1.5);
            disc(&mut p, ox, oy, s, 6.0, 6.5, 1.5);
            tri(&mut p, ox, oy, s, 3.5, 12.0, 7.5, 7.0, 11.5, 12.0);
            tri(&mut p, ox, oy, s, 9.0, 12.0, 11.5, 8.5, 13.0, 12.0);
        }
        Icon::Detach => {
            // Two halves of a link, pulled apart: the upper-left and lower-right halves.
            thick_line(&mut p, ox, oy, s, 3.0, 9.0, 6.5, 5.5, 1.8);
            thick_line(&mut p, ox, oy, s, 6.5, 12.0, 3.0, 15.0, 1.8);
            thick_line(&mut p, ox, oy, s, 13.0, 7.0, 9.5, 10.5, 1.8);
            thick_line(&mut p, ox, oy, s, 9.5, 4.0, 13.0, 1.0, 1.8);
        }
        Icon::AlignLeft => {
            thick_line(&mut p, ox, oy, s, 3.0, 2.0, 3.0, 14.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 5.0, 11.0, 5.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 10.0, 9.0, 10.0, 1.6);
        }
        Icon::AlignHCenter => {
            thick_line(&mut p, ox, oy, s, 8.0, 2.0, 8.0, 14.0, 1.6);
            thick_line(&mut p, ox, oy, s, 2.0, 5.0, 14.0, 5.0, 1.6);
            thick_line(&mut p, ox, oy, s, 4.5, 10.0, 11.5, 10.0, 1.6);
        }
        Icon::AlignRight => {
            thick_line(&mut p, ox, oy, s, 13.0, 2.0, 13.0, 14.0, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 5.0, 13.0, 5.0, 1.6);
            thick_line(&mut p, ox, oy, s, 7.0, 10.0, 13.0, 10.0, 1.6);
        }
        Icon::AlignTop => {
            thick_line(&mut p, ox, oy, s, 2.0, 3.0, 14.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 3.0, 5.0, 11.0, 1.6);
            thick_line(&mut p, ox, oy, s, 10.0, 3.0, 10.0, 9.0, 1.6);
        }
        Icon::AlignVCenter => {
            thick_line(&mut p, ox, oy, s, 2.0, 8.0, 14.0, 8.0, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 2.0, 5.0, 14.0, 1.6);
            thick_line(&mut p, ox, oy, s, 10.0, 4.5, 10.0, 11.5, 1.6);
        }
        Icon::AlignBottom => {
            thick_line(&mut p, ox, oy, s, 2.0, 13.0, 14.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 5.0, 5.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 10.0, 7.0, 10.0, 13.0, 1.6);
        }
        Icon::DistributeH => {
            thick_line(&mut p, ox, oy, s, 3.5, 3.0, 3.5, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 8.0, 3.0, 8.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 12.5, 3.0, 12.5, 13.0, 1.6);
        }
        Icon::Rotate => {
            thick_line(&mut p, ox, oy, s, 4.0, 11.0, 4.0, 6.5, 1.6);
            thick_line(&mut p, ox, oy, s, 4.0, 6.5, 6.5, 4.0, 1.6);
            thick_line(&mut p, ox, oy, s, 6.5, 4.0, 10.5, 4.0, 1.6);
            thick_line(&mut p, ox, oy, s, 10.5, 4.0, 12.0, 5.5, 1.6);
            thick_line(&mut p, ox, oy, s, 10.2, 2.2, 12.4, 5.0, 1.6);
            thick_line(&mut p, ox, oy, s, 12.4, 5.0, 9.4, 5.6, 1.6);
        }
        Icon::FlipH => {
            thick_line(&mut p, ox, oy, s, 8.0, 2.0, 8.0, 14.0, 1.6);
            thick_line(&mut p, ox, oy, s, 6.5, 5.0, 3.0, 8.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 8.0, 6.5, 11.0, 1.6);
            thick_line(&mut p, ox, oy, s, 6.5, 11.0, 6.5, 5.0, 1.6);
            thick_line(&mut p, ox, oy, s, 9.5, 5.0, 13.0, 8.0, 1.6);
            thick_line(&mut p, ox, oy, s, 13.0, 8.0, 9.5, 11.0, 1.6);
            thick_line(&mut p, ox, oy, s, 9.5, 11.0, 9.5, 5.0, 1.6);
        }
        Icon::FlipV => {
            thick_line(&mut p, ox, oy, s, 2.0, 8.0, 14.0, 8.0, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 6.5, 8.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 8.0, 3.0, 11.0, 6.5, 1.6);
            thick_line(&mut p, ox, oy, s, 11.0, 6.5, 5.0, 6.5, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 9.5, 8.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 8.0, 13.0, 11.0, 9.5, 1.6);
            thick_line(&mut p, ox, oy, s, 11.0, 9.5, 5.0, 9.5, 1.6);
        }
        Icon::Angle => {
            thick_line(&mut p, ox, oy, s, 3.0, 13.0, 13.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 13.0, 12.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 10.0, 6.0, 13.0, 1.6);
        }
        Icon::AbsolutePosition => {
            thick_line(&mut p, ox, oy, s, 3.0, 7.0, 3.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 3.0, 7.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 9.0, 3.0, 13.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 13.0, 3.0, 13.0, 7.0, 1.6);
            thick_line(&mut p, ox, oy, s, 13.0, 9.0, 13.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 13.0, 13.0, 9.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 7.0, 13.0, 3.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 13.0, 3.0, 9.0, 1.6);
        }
        Icon::ViewGrid => {
            for r in 0..3 {
                for c in 0..3 {
                    let x0 = 3.0 + c as f32 * 3.6;
                    let y0 = 3.0 + r as f32 * 3.6;
                    rect(&mut p, ox, oy, s, x0, y0, x0 + 2.8, y0 + 2.8);
                }
            }
        }
        // ── The bottom toolbar's marks ────────────────────────────────────────────────────────
        Icon::Move => {
            // The pointer: one silhouette, the tail separate from the head.
            poly_fill(&mut p, ox, oy, s, &[(4.0, 2.0), (4.0, 13.6), (6.9, 10.9), (8.8, 14.8), (10.6, 14.0), (8.7, 10.2), (12.6, 10.2)]);
        }
        Icon::Hand => {
            // Four fingers over a palm, then the thumb: the original's arcs become short
            // segments (a filled path builder has no stroked curves).
            for (x, top) in [(4.5, 5.0), (6.5, 3.5), (8.5, 4.0), (10.5, 5.5)] {
                thick_line(&mut p, ox, oy, s, x, 8.0, x, top + 1.0, 1.5);
                arc(&mut p, ox, oy, s, x + 0.75, top + 1.0, 0.75, std::f32::consts::PI, std::f32::consts::TAU, 1.5, 4);
                thick_line(&mut p, ox, oy, s, x + 1.5, top + 1.0, x + 1.5, 8.0, 1.5);
            }
            stroke_path(
                &mut p,
                ox,
                oy,
                s,
                &[(12.5, 8.0), (12.5, 10.0), (11.5, 12.0), (9.5, 13.0), (7.0, 12.4), (3.0, 9.4), (3.4, 8.6), (4.5, 8.5)],
                false,
                1.5,
            );
        }
        Icon::Scale => {
            outline_rect(&mut p, ox, oy, s, 2.5, 7.5, 8.5, 13.5, 1.5);
            thick_line(&mut p, ox, oy, s, 6.0, 10.0, 13.0, 3.0, 1.5);
            stroke_path(&mut p, ox, oy, s, &[(9.5, 2.5), (13.5, 2.5), (13.5, 6.5)], false, 1.5);
        }
        Icon::Slice => {
            stroke_path(&mut p, ox, oy, s, &[(13.5, 2.5), (5.0, 8.0), (8.0, 11.0)], true, 1.4);
            thick_line(&mut p, ox, oy, s, 2.8, 13.2, 5.9, 10.1, 2.6);
        }
        Icon::Line => thick_line(&mut p, ox, oy, s, 2.5, 13.5, 13.5, 2.5, 1.6),
        Icon::Arrow => {
            thick_line(&mut p, ox, oy, s, 2.5, 13.5, 13.0, 3.0, 1.6);
            stroke_path(&mut p, ox, oy, s, &[(7.5, 2.5), (13.5, 2.5), (13.5, 8.5)], false, 1.6);
        }
        Icon::Polygon => stroke_path(&mut p, ox, oy, s, &[(8.0, 2.5), (13.5, 12.5), (2.5, 12.5)], true, 1.6),
        Icon::Star => {
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 5.0;
                    let r = if i % 2 == 0 { 6.0 } else { 2.7 };
                    (8.0 + r * a.cos(), 8.6 + r * a.sin())
                })
                .collect();
            stroke_path(&mut p, ox, oy, s, &pts, true, 1.5);
        }
        Icon::Pen => {
            stroke_path(&mut p, ox, oy, s, &[(8.0, 2.0), (12.5, 8.5), (10.5, 13.5), (5.5, 13.5), (3.5, 8.5)], true, 1.5);
            thick_line(&mut p, ox, oy, s, 8.0, 2.0, 8.0, 7.0, 1.5);
            disc(&mut p, ox, oy, s, 8.0, 8.8, 1.2);
        }
        Icon::Pencil => {
            stroke_path(&mut p, ox, oy, s, &[(2.5, 13.5), (3.2, 10.3), (10.5, 3.0), (13.0, 5.5), (5.7, 12.8)], true, 1.4);
            thick_line(&mut p, ox, oy, s, 9.0, 4.5, 11.5, 7.0, 1.4);
        }
        Icon::ShapeBuilder => {
            // Two overlapping rings with the lens they share filled.
            let (r, d) = (4.5f32, 2.0f32);
            let a = (d / r).acos();
            ring(&mut p, ox, oy, s, 8.0 - d, 8.0, r, 1.4, 24);
            ring(&mut p, ox, oy, s, 8.0 + d, 8.0, r, 1.4, 24);
            let mut lens: Vec<(f32, f32)> = arc_pts(8.0 - d, 8.0, r, -a, a, 12);
            lens.extend(arc_pts(8.0 + d, 8.0, r, std::f32::consts::PI - a, std::f32::consts::PI + a, 12));
            poly_fill(&mut p, ox, oy, s, &lens);
        }
        Icon::TextOnPath => {
            thick_line(&mut p, ox, oy, s, 4.5, 3.0, 11.5, 3.0, 1.5);
            thick_line(&mut p, ox, oy, s, 8.0, 3.0, 8.0, 8.5, 1.5);
            stroke_path(&mut p, ox, oy, s, &[(2.0, 13.5), (5.0, 11.5), (8.0, 10.8), (11.0, 11.5), (14.0, 13.5)], false, 1.4);
        }
        Icon::Comment => {
            // A speech bubble: a ring with the tail at the bottom left.
            ring(&mut p, ox, oy, s, 8.5, 7.5, 5.0, 1.5, 28);
            stroke_path(&mut p, ox, oy, s, &[(6.0, 12.2), (2.5, 13.5), (4.4, 10.6)], true, 1.2);
        }
        Icon::Board => {
            stroke_path(&mut p, ox, oy, s, &[(2.5, 2.5), (13.5, 2.5), (13.5, 9.5), (9.5, 13.5), (2.5, 13.5)], true, 1.5);
            poly_fill(&mut p, ox, oy, s, &[(9.5, 13.5), (9.5, 9.5), (13.5, 9.5)]);
            thick_line(&mut p, ox, oy, s, 5.0, 6.5, 11.0, 6.5, 1.5);
            thick_line(&mut p, ox, oy, s, 5.0, 9.5, 8.0, 9.5, 1.5);
        }
        Icon::Marker => {
            stroke_path(&mut p, ox, oy, s, &[(3.0, 13.0), (3.0, 10.5), (10.0, 3.5), (12.5, 6.0), (5.5, 13.0)], true, 1.5);
            thick_line(&mut p, ox, oy, s, 8.5, 5.0, 11.0, 7.5, 1.5);
            poly_fill(&mut p, ox, oy, s, &[(2.0, 14.0), (2.0, 11.5), (4.5, 14.0)]);
        }
        Icon::Highlighter => {
            stroke_path(&mut p, ox, oy, s, &[(4.0, 10.0), (10.5, 3.5), (13.5, 6.5), (7.0, 13.0)], true, 1.5);
            stroke_path(&mut p, ox, oy, s, &[(4.0, 10.0), (2.5, 12.5), (4.5, 12.5), (7.0, 13.0)], false, 1.5);
            poly_fill(&mut p, ox, oy, s, &[(1.0, 14.0), (15.0, 14.0), (15.0, 15.5), (1.0, 15.5)]);
        }
        Icon::Eraser => {
            stroke_path(&mut p, ox, oy, s, &[(2.5, 10.0), (9.5, 3.0), (13.5, 7.0), (7.0, 13.5), (6.0, 13.5)], true, 1.5);
            thick_line(&mut p, ox, oy, s, 5.5, 7.0, 9.5, 11.0, 1.5);
            thick_line(&mut p, ox, oy, s, 7.0, 13.5, 14.0, 13.5, 1.5);
        }
        Icon::Shapes => {
            outline_rect(&mut p, ox, oy, s, 1.5, 5.5, 9.5, 13.5, 1.5);
            ring(&mut p, ox, oy, s, 10.5, 5.5, 4.0, 1.5, 28);
        }
        Icon::Connector => {
            outline_rect(&mut p, ox, oy, s, 1.5, 1.5, 6.5, 6.5, 1.5);
            outline_rect(&mut p, ox, oy, s, 9.5, 9.5, 14.5, 14.5, 1.5);
            stroke_path(&mut p, ox, oy, s, &[(6.5, 4.0), (12.0, 4.0), (12.0, 9.0)], false, 1.5);
            poly_fill(&mut p, ox, oy, s, &[(12.0, 9.0), (10.8, 6.6), (13.2, 6.6)]);
        }
        Icon::Stamp => {
            ring(&mut p, ox, oy, s, 8.0, 8.0, 6.0, 1.5, 28);
            stroke_path(&mut p, ox, oy, s, &[(5.0, 8.3), (7.2, 10.5), (11.0, 6.0)], false, 1.5);
        }
        Icon::Templates => {
            outline_rect(&mut p, ox, oy, s, 1.5, 2.5, 14.5, 13.5, 1.5);
            poly_fill(&mut p, ox, oy, s, &[(3.5, 4.5), (7.5, 4.5), (7.5, 8.5), (3.5, 8.5)]);
            thick_line(&mut p, ox, oy, s, 9.5, 5.5, 12.5, 5.5, 1.5);
            thick_line(&mut p, ox, oy, s, 9.5, 7.5, 12.5, 7.5, 1.5);
            thick_line(&mut p, ox, oy, s, 3.5, 11.0, 12.5, 11.0, 1.5);
        }
        Icon::Table => {
            outline_rect(&mut p, ox, oy, s, 1.5, 2.5, 14.5, 13.5, 1.5);
            thick_line(&mut p, ox, oy, s, 1.5, 6.5, 14.5, 6.5, 1.5);
            thick_line(&mut p, ox, oy, s, 1.5, 10.0, 14.5, 10.0, 1.5);
            thick_line(&mut p, ox, oy, s, 6.0, 2.5, 6.0, 13.5, 1.5);
            thick_line(&mut p, ox, oy, s, 10.0, 2.5, 10.0, 13.5, 1.5);
        }
        Icon::Actions => {
            poly_fill(&mut p, ox, oy, s, &[(9.0, 2.0), (3.5, 9.0), (7.5, 9.0), (6.5, 14.0), (12.5, 7.0), (8.5, 7.0)]);
        }
        Icon::Panels => {
            outline_rect(&mut p, ox, oy, s, 2.5, 3.5, 13.5, 12.5, 1.5);
            thick_line(&mut p, ox, oy, s, 5.5, 4.0, 5.5, 12.0, 1.5);
            thick_line(&mut p, ox, oy, s, 10.5, 4.0, 10.5, 12.0, 1.5);
        }
        Icon::Eye => {
            // An almond (two arcs meeting at the corners) around the pupil.
            let (r, cy, half) = (7.0f32, 7.5f32, 5.5f32);
            let a = (half / r).asin();
            let d = r * a.cos();
            let mut e: Vec<(f32, f32)> = arc_pts(8.0, cy + d, r, -std::f32::consts::FRAC_PI_2 - a, -std::f32::consts::FRAC_PI_2 + a, 10);
            e.extend(arc_pts(8.0, cy - d, r, std::f32::consts::FRAC_PI_2 - a, std::f32::consts::FRAC_PI_2 + a, 10));
            stroke_path(&mut p, ox, oy, s, &e, true, 1.3);
            ring(&mut p, ox, oy, s, 8.0, cy, 2.0, 1.2, 16);
        }
        Icon::Animation => {
            stroke_path(&mut p, ox, oy, s, &[(2.0, 13.0), (4.5, 9.0), (8.0, 6.0), (11.5, 4.0), (14.0, 3.5)], false, 1.4);
            poly_fill(&mut p, ox, oy, s, &[(4.5, 6.5), (7.0, 9.0), (4.5, 11.5), (2.0, 9.0)]);
            poly_fill(&mut p, ox, oy, s, &[(11.5, 1.5), (14.0, 4.0), (11.5, 6.5), (9.0, 4.0)]);
        }
        Icon::Code => {
            stroke_path(&mut p, ox, oy, s, &[(5.5, 4.0), (2.0, 8.0), (5.5, 12.0)], false, 1.5);
            stroke_path(&mut p, ox, oy, s, &[(10.5, 4.0), (14.0, 8.0), (10.5, 12.0)], false, 1.5);
            thick_line(&mut p, ox, oy, s, 9.0, 3.0, 7.0, 13.0, 1.4);
        }
        Icon::Group => {
            // A nested outline: the classic "group" mark (container of containers).
            outline_rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0, 1.5);
            outline_rect(&mut p, ox, oy, s, 5.5, 5.5, 11.5, 11.5, 1.2);
        }
        Icon::Vector => {
            poly_fill(&mut p, ox, oy, s, &[(8.0, 2.0), (14.0, 13.5), (2.0, 13.5)]);
        }
        Icon::ComponentSet => {
            outline_rect(&mut p, ox, oy, s, 2.5, 2.5, 13.5, 13.5, 1.3);
            diamond(&mut p, ox, oy, s, 8.0, 8.0, 4.5);
        }
        Icon::IconKind => {
            diamond(&mut p, ox, oy, s, 8.0, 8.0, 4.5);
            disc(&mut p, ox, oy, s, 8.0, 8.0, 1.5);
        }
        Icon::Slot => {
            outline_rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0, 1.5);
            disc(&mut p, ox, oy, s, 8.0, 8.0, 1.6);
        }
        Icon::Lock => {
            outline_rect(&mut p, ox, oy, s, 5.0, 7.0, 11.0, 13.0, 1.4);
            arc(&mut p, ox, oy, s, 8.0, 5.5, 2.3, std::f32::consts::PI, std::f32::consts::TAU, 1.3, 10);
        }
        Icon::Unlock => {
            outline_rect(&mut p, ox, oy, s, 5.0, 7.0, 11.0, 13.0, 1.4);
            arc(&mut p, ox, oy, s, 9.2, 6.0, 2.3, std::f32::consts::PI, std::f32::consts::TAU, 1.3, 10);
        }
        Icon::EyeOff => {
            let (r, cy, half) = (7.0f32, 7.5f32, 5.5f32);
            let a = (half / r).asin();
            let d = r * a.cos();
            let mut e: Vec<(f32, f32)> = arc_pts(8.0, cy + d, r, -std::f32::consts::FRAC_PI_2 - a, -std::f32::consts::FRAC_PI_2 + a, 10);
            e.extend(arc_pts(8.0, cy - d, r, std::f32::consts::FRAC_PI_2 - a, std::f32::consts::FRAC_PI_2 + a, 10));
            stroke_path(&mut p, ox, oy, s, &e, true, 1.3);
            ring(&mut p, ox, oy, s, 8.0, cy, 2.0, 1.2, 16);
            thick_line(&mut p, ox, oy, s, 2.5, 13.5, 13.5, 2.5, 1.6);
        }
        Icon::Union => {
            disc(&mut p, ox, oy, s, 6.0, 8.0, 4.0);
            disc(&mut p, ox, oy, s, 10.0, 8.0, 4.0);
        }
        Icon::Subtract => {
            disc(&mut p, ox, oy, s, 6.0, 8.0, 4.0);
            ring(&mut p, ox, oy, s, 10.0, 8.0, 4.0, 1.3, 18);
        }
        Icon::Intersect => {
            let (r, d) = (4.5f32, 2.0f32);
            let a = (d / r).acos();
            ring(&mut p, ox, oy, s, 8.0 - d, 8.0, r, 1.4, 24);
            ring(&mut p, ox, oy, s, 8.0 + d, 8.0, r, 1.4, 24);
            let mut lens: Vec<(f32, f32)> = arc_pts(8.0 - d, 8.0, r, -a, a, 12);
            lens.extend(arc_pts(8.0 + d, 8.0, r, std::f32::consts::PI - a, std::f32::consts::PI + a, 12));
            poly_fill(&mut p, ox, oy, s, &lens);
        }
        Icon::Exclude => {
            let r = 4.0f32;
            ring(&mut p, ox, oy, s, 6.0, 8.0, r, 1.4, 20);
            ring(&mut p, ox, oy, s, 10.0, 8.0, r, 1.4, 20);
        }
        // ── 标签条（P0-1）：与上面同一套 16 单位网格与原语 ────────────────────────────────
        Icon::Home => {
            // 用户 2026-11-16 给的 24×24 房子 SVG，原样嵌入（不再手画）：
            // `stroke-width=2` + 同形状 mask 是 Figma 图标的标准写法 —— mask 把描边落在
            // 轮廓外侧的一半裁掉，等效 1px 内描边。坐标按 24 → 16 网格缩放 2/3。
            // `stroke-width` 按放大倍数退回去：标题栏用 32pt 渲染（此前 20pt），
            // 线宽保持与 20pt 时一致（用户 2026-11-16：等比放大，线宽不变）。
            // 有效线宽 = sw / 2（mask 裁掉外半） × size / 24 ⇒ 20pt: 0.833，32pt: 0.833。
            p.raw(&format!(
                "<defs><mask id=\"home-mask\" fill=\"white\"><path d=\"{HOME_PATH}\"/></mask></defs>\
                 <g transform=\"scale(0.666667)\"><path d=\"{HOME_PATH}\" stroke-width=\"1.25\" stroke=\"#fff\" fill=\"none\" mask=\"url(#home-mask)\"/></g>"
            ));
        }
        Icon::Sun => {
            disc(&mut p, ox, oy, s, 8.0, 8.0, 3.4);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let (c, sn) = (a.cos(), a.sin());
                thick_line(&mut p, ox, oy, s, 8.0 + c * 5.2, 8.0 + sn * 5.2, 8.0 + c * 7.0, 8.0 + sn * 7.0, 1.5);
            }
        }
        Icon::Moon => {
            // Outer crescent + the short inner edge; a subtractive shape made of two arcs.
            arc(&mut p, ox, oy, s, 8.0, 8.0, 5.4, 0.42 * std::f32::consts::PI, 1.58 * std::f32::consts::PI, 2.0, 22);
            arc(&mut p, ox, oy, s, 11.2, 8.0, 4.4, 0.60 * std::f32::consts::PI, 1.40 * std::f32::consts::PI, 1.4, 16);
        }
        Icon::Play => {
            poly_fill(&mut p, ox, oy, s, &[(4.5, 2.5), (4.5, 13.5), (13.5, 8.0)]);
        }
        Icon::WinMinimize => {
            thick_line(&mut p, ox, oy, s, 3.0, 8.0, 13.0, 8.0, 1.6);
        }
        Icon::WinMaximize => {
            outline_rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0, 1.6);
        }
        Icon::WinRestore => {
            outline_rect(&mut p, ox, oy, s, 3.0, 5.5, 10.5, 13.0, 1.5);
            thick_line(&mut p, ox, oy, s, 5.5, 5.5, 5.5, 3.0, 1.5);
            thick_line(&mut p, ox, oy, s, 5.5, 3.0, 13.0, 3.0, 1.5);
            thick_line(&mut p, ox, oy, s, 13.0, 3.0, 13.0, 10.5, 1.5);
            thick_line(&mut p, ox, oy, s, 10.5, 10.5, 13.0, 10.5, 1.5);
        }
        Icon::LiveDisconnected => {
            ring(&mut p, ox, oy, s, 8.0, 8.0, 5.0, 1.6, 22);
            thick_line(&mut p, ox, oy, s, 4.0, 12.0, 12.0, 4.0, 2.0);
        }
        Icon::LayoutH => {
            // egui `draw`: two filled columns with a 1 pt corner radius.
            rect(&mut p, ox, oy, s, 2.0, 3.0, 7.0, 13.0);
            rect(&mut p, ox, oy, s, 9.0, 3.0, 14.0, 13.0);
        }
        Icon::LayoutV => {
            rect(&mut p, ox, oy, s, 3.0, 2.0, 13.0, 7.0);
            rect(&mut p, ox, oy, s, 3.0, 9.0, 13.0, 14.0);
        }
        Icon::LayoutGrid => {
            // egui `draw_grid_layoutgrid`: nine 3x3 cells on the 16-unit grid.
            for a in [2.5, 6.5, 10.5] {
                for b in [2.5, 6.5, 10.5] {
                    outline_rect(&mut p, ox, oy, s, a, b, a + 3.0, b + 3.0, 1.0);
                }
            }
        }
    }
    p
}

/// Paint `icon` centred in `bounds`, scaled to the shorter side. The entry point for callers that
/// draw directly into a `canvas` paint closure (a swatch's image/pattern glyph), which is why it
/// goes through `Window::paint_svg` instead of the `svg()` element.
pub(crate) fn draw_centred(window: &mut Window, bounds: Bounds<Pixels>, icon: Icon, color: gpui::Rgba, cx: &App) {
    let w: f32 = bounds.size.width.into();
    let h: f32 = bounds.size.height.into();
    let side = w.min(h).max(1.0);
    let ox = f32::from(bounds.origin.x) + (w - side) / 2.0;
    let oy = f32::from(bounds.origin.y) + (h - side) / 2.0;
    let square = Bounds { origin: Point::new(px(ox), px(oy)), size: gpui::size(px(side), px(side)) };
    let _ = window.paint_svg(square, icon.asset_path(), TransformationMatrix::unit(), Hsla::from(color), cx);
}

// ── 绘制原语：签名与旧版一致，输出从「像素三角形」换成「SVG 图元」──────────────────
fn tri(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32) {
    let k = s / 16.0;
    p.polygon(&[(ox + x0 * k, oy + y0 * k), (ox + x1 * k, oy + y1 * k), (ox + x2 * k, oy + y2 * k)]);
}

fn thick_line(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
    let k = s / 16.0;
    p.line(ox + x0 * k, oy + y0 * k, ox + x1 * k, oy + y1 * k, w * k);
}

fn disc(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32) {
    let k = s / 16.0;
    p.disc(ox + cx * k, oy + cy * k, r * k);
}

fn rect(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32) {
    let k = s / 16.0;
    p.rect(ox + x0 * k, oy + y0 * k, ox + x1 * k, oy + y1 * k);
}

fn outline_rect(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
    let k = s / 16.0;
    p.outline_rect(ox + x0 * k, oy + y0 * k, ox + x1 * k, oy + y1 * k, w * k);
}

fn diamond(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32) {
    let k = s / 16.0;
    let (x, y) = (ox + cx * k, oy + cy * k);
    let rr = r * k;
    p.polygon(&[(x, y - rr), (x + rr, y), (x, y + rr), (x - rr, y)]);
}

fn outline_diamond(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32, w: f32) {
    let k = s / 16.0;
    let (x, y) = (ox + cx * k, oy + cy * k);
    let rr = r * k;
    p.polyline(&[(x, y - rr), (x + rr, y), (x, y + rr), (x - rr, y)], true, w * k);
}

fn poly_fill(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, pts: &[(f32, f32)]) {
    let k = s / 16.0;
    let v: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (ox + x * k, oy + y * k)).collect();
    p.polygon(&v);
}

fn stroke_path(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, pts: &[(f32, f32)], closed: bool, w: f32) {
    let k = s / 16.0;
    let v: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (ox + x * k, oy + y * k)).collect();
    p.polyline(&v, closed, w * k);
}

/// The points along an arc (grid units), `n` segments from `a0` to `a1` radians.
fn arc_pts(cx: f32, cy: f32, r: f32, a0: f32, a1: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * (i as f32 / n as f32);
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn arc(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32, a0: f32, a1: f32, w: f32, n: usize) {
    let pts = arc_pts(cx, cy, r, a0, a1, n);
    stroke_path(p, ox, oy, s, &pts, false, w);
}

/// A true SVG circle replaces the 22-segment polygonal ring.
fn ring(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32, w: f32, _n: usize) {
    let k = s / 16.0;
    p.ring(ox + cx * k, oy + cy * k, r * k, w * k);
}

/// Draw `icon` in `color` at `size` px, as an element.
///
/// kit 契约：`Icon::new(name).with_size(px(20.)).text_color(theme.red)` →
/// `svg().path("icons/<name>.svg").w().h().flex_none().text_color(...)`.
/// 调用点（`icon::paint(icon, rgba(t.icon), 14.0)`）一行都不用改。
pub fn paint(icon: Icon, color: impl Into<gpui::Rgba>, size: f32) -> impl IntoElement {
    let rgba: gpui::Rgba = color.into();
    let path = sized_icon_path(icon, size);
    svg().path(path).w(px(size)).h(px(size)).flex_none().text_color(Hsla::from(rgba))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_icon_emits_a_well_formed_svg() {
        let t = table();
        assert_eq!(t.len(), Icon::ALL.len());
        assert_eq!(t.len(), 96, "Icon 变体数变了：请同步 ALL / asset_name / 本断言");
        let mut names = HashSet::new();
        for a in t {
            assert!(names.insert(a.name), "duplicate asset name {}", a.name);
            let s = std::str::from_utf8(&a.svg).expect("svg is utf8");
            assert!(s.starts_with("<svg "), "{}: {}", a.name, s);
            assert!(s.trim_end().ends_with("</svg>"), "{}", a.name);
            // 产品标记与 24 网格线框图标走 24-unit grid，其余生成字形是 16。
            let grid24 = a.icon == Icon::Logo || FIGMA_UI_ICONS.iter().any(|(i, _)| *i == a.icon);
            let grid = if grid24 { "0 0 24 24" } else { "0 0 16 16" };
            assert!(s.contains(&format!("viewBox=\"{grid}\"")), "{}", a.name);
            assert!(!s.contains("NaN") && !s.contains("inf"), "{}: {}", a.name, s);
            assert!(s.contains("<path") || s.contains("<circle") || s.contains("<rect"), "{} has no shapes: {}", a.name, s);
        }
    }

    #[test]
    fn asset_source_resolves_every_icon_path() {
        let assets = IconAssets;
        for a in table() {
            assert_eq!(a.icon.asset_path(), a.path);
            let got = assets.load(&a.path).unwrap().expect("load");
            assert_eq!(&*got, a.svg.as_slice(), "{}", a.name);
        }
        assert!(assets.load("icons/not-a-real-icon.svg").unwrap().is_none());
        assert!(assets.load("other/x.svg").unwrap().is_none());
        assert_eq!(assets.list("icons").unwrap().len(), Icon::ALL.len());
        assert!(assets.list("other").unwrap().is_empty());
    }

    /// GPUI 的 `SvgRenderer` 用 usvg 0.45.1 把 SVG 渲成 alpha mask；解析失败时 `paint_svg`
    /// 只 `log_err()` 然后静默不画。这里用同一个解析器把 79 个图标全部过一遍。
    #[test]
    fn every_icon_svg_parses_with_gpui_s_usvg() {
        for a in table() {
            let parsed = usvg::Tree::from_data(&a.svg, &usvg::Options::default());
            assert!(parsed.is_ok(), "{}: {:?}", a.name, parsed.err());
            let tree = parsed.unwrap();
            let side = if a.icon == Icon::Logo || FIGMA_UI_ICONS.iter().any(|(i, _)| *i == a.icon) { 24.0 } else { 16.0 };
            assert_eq!(tree.size().width(), side, "{} viewBox width", a.name);
            assert_eq!(tree.size().height(), side, "{} viewBox height", a.name);
        }
    }

    /// The six tool marks keep their own 24 pt size and all use the one stroke constant.
    #[test]
    fn shape_tool_marks_share_one_stroke_and_their_own_size() {
        for icon in [Icon::Rectangle, Icon::Line, Icon::Arrow, Icon::Ellipse, Icon::Polygon, Icon::Star] {
            let stem = tool_stem(icon).unwrap();
            let bytes = normalise_stroke(tool_bytes(stem).unwrap());
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.contains(&format!("stroke-width=\"{}\"", ICON_STROKE)), "{stem}");
            assert_eq!(view_box_size(&bytes), 24.0, "{stem}");
            assert_eq!(natural_size(icon), 24.0, "{stem}");
            assert!(usvg::Tree::from_data(&bytes, &usvg::Options::default()).is_ok(), "{stem}");
            let got = IconAssets.load(&format!("tools/{stem}.svg")).unwrap().unwrap();
            assert_eq!(&*got, bytes.as_slice(), "{stem} resolves");
        }
    }

    #[test]
    fn rescale_binds_every_stroke_to_one_width() {
        let src = br#"<svg><path stroke-width="2.4"/><path stroke-width="4.8"/></svg>"#;
        let out = String::from_utf8(rescale_strokes(src, 16.0, 32.0)).unwrap();
        assert_eq!(out.matches("stroke-width=\"0.5\"").count(), 2, "{out}");
    }

    #[test]
    fn figma_ui_icons_keep_their_grid_and_stroke_width() {
        for (icon, _) in FIGMA_UI_ICONS {
            let full = String::from_utf8(figma_ui_icon(icon, 24.0).unwrap()).unwrap();
            assert!(full.contains("viewBox=\"0 0 24 24\""), "{icon:?} grid changed");
            assert!(full.contains(&format!("stroke-width=\"{}\"", ICON_STROKE)), "{icon:?} stroke not bound");
            assert!(!full.contains("__STROKE__"), "{icon:?} placeholder left");
            assert!(usvg::Tree::from_data(full.as_bytes(), &usvg::Options::default()).is_ok(), "{icon:?} invalid svg");
            // 缩到 12pt 时描边先放大两倍，等比缩放回去后视觉线宽仍是 ICON_STROKE。
            let half = String::from_utf8(figma_ui_icon(icon, 12.0).unwrap()).unwrap();
            assert!(half.contains(&format!("stroke-width=\"{}\"", ICON_STROKE * 2.0)), "{icon:?} stroke scaled");
        }
    }

    #[test]
    fn common_glyphs_use_real_svg_primitives() {
        let svg = |i: Icon| String::from_utf8(svg_canvas(i).to_svg().into_bytes()).unwrap();
        assert!(svg(Icon::WinMaximize).contains("<rect"), "maximise is a real outline rect");
        let ring = svg(Icon::LiveDisconnected);
        assert!(ring.contains("<circle") && ring.contains("fill=\"none\""), "ring is a circle, not 22 triangles");
        assert!(!svg(Icon::Plus).contains("<circle"), "plus stays two strokes");
    }
}
