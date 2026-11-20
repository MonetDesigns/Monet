//! 图层树字形：`assets/icons/` 下的外置 SVG（扁平，`layer-` 前缀），编译期内嵌。
//! 上游 `icons_ui3::paint_layer` 用 16×16 viewBox 画这些字；GPUI 的 SvgRenderer 只取
//! alpha，颜色由 `.text_color()` 给，所以 SVG 里的白色描边/填充正好能被统一着色。

use gpui::{Hsla, IntoElement, SharedString, prelude::*, px, svg};
use std::sync::OnceLock;

/// Layer glyphs and shape-tool marks share the very same stroke constant.
pub use crate::widgets::icon::ICON_STROKE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerIcon {
    Frame,
    LayoutH,
    LayoutV,
    LayoutGrid,
    Text,
    Absolute,
    Mask,
    EllipseMask,
    Component,
    Instance,
    Group,
}

impl LayerIcon {
    pub fn path(self) -> SharedString {
        SharedString::from(format!("icons/{}.svg", ASSETS[self as usize].0))
    }
}

/// (file stem, bytes). Order MUST match the enum above.
pub static ASSETS: &[(&str, &[u8])] = &[
    ("layer-frame", include_bytes!("../../assets/icons/layer-frame.svg")),
    ("layer-layout-horizontal", include_bytes!("../../assets/icons/layer-layout-horizontal.svg")),
    ("layer-layout-vertical", include_bytes!("../../assets/icons/layer-layout-vertical.svg")),
    ("layer-layout-grid", include_bytes!("../../assets/icons/layout-grid.svg")),
    ("layer-text", include_bytes!("../../assets/icons/shape-text.svg")),
    ("layer-absolute", include_bytes!("../../assets/icons/layer-absolute.svg")),
    ("layer-mask", include_bytes!("../../assets/icons/layer-mask.svg")),
    ("layer-ellipse-mask", include_bytes!("../../assets/icons/layer-ellipse-mask.svg")),
    ("layer-component", include_bytes!("../../assets/icons/layer-component.svg")),
    ("layer-instance", include_bytes!("../../assets/icons/layer-instance.svg")),
    ("layer-group", include_bytes!("../../assets/icons/layer-group.svg")),
];

/// The SVG's own viewBox width: the vector occupies the space it actually has, rather than
/// being forced into a caller-supplied box (which would scale its stroke with it).
pub fn natural_size(icon: LayerIcon) -> f32 {
    let text = String::from_utf8_lossy(ASSETS[icon as usize].1);
    let Some(i) = text.find("viewBox=\"") else { return 16.0 };
    let after = &text[i + "viewBox=\"".len()..];
    after.split_whitespace().nth(2).and_then(|v| v.trim_end_matches('\"').parse().ok()).unwrap_or(16.0)
}

/// Replace every embedded `stroke-width` with [`ICON_STROKE`], once, at load time.
fn normalised() -> &'static Vec<&'static [u8]> {
    static CACHE: OnceLock<Vec<&'static [u8]>> = OnceLock::new();
    CACHE.get_or_init(|| {
        ASSETS
            .iter()
            .map(|(_, bytes)| {
                Box::leak(crate::widgets::icon::normalise_stroke(bytes).into_boxed_slice()) as &'static [u8]
            })
            .collect()
    })
}

/// Draw one layer glyph in a single ink colour, at its **natural** size.
pub fn paint(icon: LayerIcon, color: gpui::Rgba) -> impl IntoElement {
    let s = natural_size(icon);
    svg().path(icon.path()).w(px(s)).h(px(s)).flex_none().text_color(Hsla::from(color))
}

/// AssetSource hook: map `layer-<stem>.svg` onto the embedded bytes, with one shared stroke.
pub fn load(path: &str) -> Option<&'static [u8]> {
    let stem = path.strip_prefix("icons/")?.strip_suffix(".svg")?;
    let index = ASSETS.iter().position(|(name, _)| *name == stem)?;
    Some(normalised()[index])
}

pub fn paths() -> Vec<SharedString> {
    ASSETS.iter().map(|(name, _)| SharedString::from(format!("icons/{name}.svg"))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    /// No glyph may carry its own stroke width: the painter and the SVGs share one constant.
    #[test]
    fn every_stroke_uses_the_shared_constant() {
        for (stem, _) in ASSETS {
            let bytes = load(&format!("icons/{stem}.svg")).unwrap();
            let text = std::str::from_utf8(bytes).unwrap();
            let needle = "stroke-width=\"";
            let mut rest = text;
            while let Some(i) = rest.find(needle) {
                let after = &rest[i + needle.len()..];
                let end = after.find('\"').unwrap();
                assert_eq!(&after[..end], format!("{}", ICON_STROKE), "{stem} has its own stroke width");
                rest = &after[end..];
            }
        }
        assert_eq!(natural_size(LayerIcon::Text), 16.0);
    }

    #[test]
    fn every_glyph_has_an_asset() {
        assert_eq!(ASSETS.len(), 11);
        for (stem, bytes) in ASSETS {
            assert!(!bytes.is_empty(), "{stem} is empty");
            let text = std::str::from_utf8(bytes).unwrap();
            assert!(text.contains("<svg"), "{stem} is not an SVG");
            // GPUI renders SVGs through usvg; a parse failure would silently draw nothing.
            assert!(usvg::Tree::from_data(bytes, &usvg::Options::default()).is_ok(), "{stem} does not parse");
        }
    }
}
