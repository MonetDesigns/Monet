//! 旋转文本（标尺左侧数字）。
//!
//! GPUI 0.2.2 的文字管线（`ShapedLine` / `Window::paint_glyph`）把变换矩阵写死成
//! `TransformationMatrix::unit()`，**没有** egui `TextShape::with_angle_and_anchor(-π/2)`
//! 的对应能力。整个框架里唯一带变换的公开绘制路径是 SVG：`Svg::with_transformation`
//! → `Window::paint_svg(bounds, path, transformation, color, cx)` → `MonochromeSprite`。
//! `gpui-kit` 的 `Icon::rotate()` / `.transform(Transformation::rotate(..))` 走的也是这条路，
//! 而且 kit 明确写了「只有 SVG 绘图会变换，布局/命中盒不转」——正好是标尺标签需要的语义。
//!
//! 于是把短标签动态做成一张内联 SVG（`<text>` 由 usvg 转成轮廓，渲染器已
//! `load_system_fonts()`），再用 `with_transformation` 绕自身中心旋转 -90°。
//! SVG 被渲染成 alpha mask、颜色由 `.text_color()` 注入，所以 SVG 里只用黑色。
//!
//! 资源路径把文字、字号、框尺寸全部编进去（hex），`AssetSource::load` 现场生成字节，
//! 不需要全局可变表，也不会与冻结的 `IconAssets` 冲突：非 `text:` 前缀原样委托。

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// 前缀。完整格式：`text:{w}x{h}@{pt}:{hex}.svg`。
const PREFIX: &str = "text:";

/// 注册到 `Application::with_assets` 的资源源：动态文本 SVG + 原有图标。
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // 外置设备外框：`device:<relative path>` 从用户的 devices 目录读机身 PNG（外置文件，
        // 和笔刷 / WGSL 一样不在二进制里）。
        if let Some(rel) = path.strip_prefix("device:") {
            let bytes = figcraft_devices::devices_dir().join(rel);
            return Ok(std::fs::read(bytes).ok().map(Cow::Owned));
        }
        if let Some(spec) = path.strip_prefix(PREFIX) {
            return Ok(svg_bytes(spec).map(|svg| Cow::Owned(svg.into_bytes())));
        }
        // 排版对话框字形（照 参考实现 typ-*.svg 1:1 移植），编译期内联、不碰冻结的 Icon 枚举。
        // 前缀共享扁平的 `icons/`，所以查不到时必须继续往下走，不能直接返回 None——
        // 否则所有主图标都会被这里拦掉、整片消失。
        if let Some(name) = path.strip_prefix(crate::widgets::typ_icons::ASSET_PREFIX).and_then(|s| s.strip_suffix(".svg")) {
            if let Some(svg) = crate::widgets::typ_icons::TypIcon::load(name) {
                return Ok(Some(Cow::Borrowed(svg.as_bytes())));
            }
        }
        // 取色器四条工具栏的 24 pt 图标（参考编辑器 DOM 原始 SVG，见 `crate::picker_icons`）。
        if let Some(name) = path.strip_prefix(crate::picker_icons::ASSET_PREFIX).and_then(|s| s.strip_suffix(".svg")) {
            if let Some(svg) = crate::picker_icons::PickerIcon::load(name) {
                return Ok(Some(Cow::Borrowed(svg.as_bytes())));
            }
        }
        crate::widgets::icon::IconAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all: Vec<SharedString> = crate::widgets::icon::IconAssets.list(path)?;
        all.extend(crate::widgets::typ_icons::TypIcon::list());
        all.extend(crate::picker_icons::PickerIcon::list());
        Ok(all)
    }
}

/// 一个旋转标签的 SVG 资源路径。`w`/`h` 是**旋转前**的水平文字框，
/// 必须与 `Svg` 元素的布局尺寸一致，否则 `Transformation::rotate` 的绕点会偏。
pub fn text_key(text: &str, pt: f32, w: f32, h: f32) -> SharedString {
    text_key_weighted(text, pt, w, h, 305)
}

/// [`text_key`] with the label's CSS font weight: the rotated SVG must use the same face as the
/// horizontal GPUI labels (MiSans 400 / 600), or the name/size text visibly changes family.
pub fn text_key_weighted(text: &str, pt: f32, w: f32, h: f32, weight: u16) -> SharedString {
    let mut hex = String::with_capacity(text.len() * 2);
    for b in text.as_bytes() {
        hex.push_str(&format!("{b:02x}"));
    }
    SharedString::from(format!("{PREFIX}{w:.2}x{h:.2}@{pt:.2}w{weight}:{hex}.svg"))
}

fn svg_bytes(spec: &str) -> Option<String> {
    let spec = spec.strip_suffix(".svg").unwrap_or(spec);
    let (geom, hex) = spec.rsplit_once(':')?;
    let (wh, pt) = geom.rsplit_once('@')?;
    let (w, h) = wh.split_once('x')?;
    let w: f32 = w.parse().ok()?;
    let h: f32 = h.parse().ok()?;
    let (pt, weight): (f32, u16) = match pt.rsplit_once('w') {
        Some((p, weight)) => (p.parse().ok()?, weight.parse().ok()?),
        None => (pt.parse().ok()?, 400),
    };
    if !(w > 0.0 && h > 0.0 && pt > 0.0) {
        return None;
    }
    let mut raw = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    if bytes.len() % 2 != 0 {
        return None;
    }
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        raw.push((hi * 16 + lo) as u8);
    }
    // `text_key` stores the UTF-8 bytes (a frame name can be Chinese, and the size label uses ×).
    // Decode as a whole UTF-8 string — byte-by-byte `char::from_u32` turns "×" into "Ã—".
    let text = String::from_utf8(raw).ok()?;
    let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    // 与画布 horizontal label 的排法对齐：水平居中、基线落在行高的 ~0.78 处
    // （`text_size()` 的行高是 1.25×字号），整块绕中心旋转 -90° 后正好贴在左标尺。
    // 字体必须与页面其它文字一致：根节点用 MiSans，SVG 不能退回 Segoe UI。
    Some(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><text x="{cx}" y="{baseline}" text-anchor="middle" font-family="MiSans, Segoe UI, Arial, sans-serif" font-size="{pt}" font-weight="{weight}" fill="black">{escaped}</text></svg>"##,
        w = w,
        h = h,
        cx = w / 2.0,
        baseline = h * 0.78,
        pt = pt,
    ))
}

#[cfg(test)]
mod tests {
    use super::{svg_bytes, text_key, text_key_weighted};

    /// 主图标、typ、picker 共用扁平的 `icons/` 前缀：typ/picker 查不到时必须继续下沉到
    /// `IconAssets`，否则主图标会被前缀分支吞掉、整片消失（2026-11-20 回归）。
    #[test]
    fn every_icon_family_resolves_through_the_shared_prefix() {
        use gpui::AssetSource;
        let assets = super::AppAssets;
        for path in ["icons/ui-plus.svg", "icons/shape-rectangle.svg", "icons/typ-align-left.svg", "icons/picker-solid.svg"] {
            assert!(assets.load(path).unwrap().is_some(), "{path} 加载失败");
        }
    }

    #[test]
    fn a_key_round_trips_into_an_svg_with_the_same_text() {
        let key = text_key("-150", 9.0, 20.0, 11.25);
        let svg = svg_bytes(key.strip_prefix("text:").unwrap()).unwrap();
        assert!(svg.contains(">-150</text>"), "{svg}");
        assert!(svg.contains("width=\"20\""), "{svg}");
        assert!(svg.contains("font-size=\"9\""), "{svg}");
        assert!(svg.contains("font-family=\"MiSans"), "{svg}");
        assert!(svg.contains("font-weight=\"305\""), "{svg}");
        assert!(svg.contains("text-anchor=\"middle\""), "{svg}");
    }

    #[test]
    fn the_weighted_key_keeps_the_labels_weight() {
        let key = text_key_weighted("480 Hug × 180 Hug", 11.0, 120.0, 13.75, 330);
        let svg = svg_bytes(key.strip_prefix("text:").unwrap()).unwrap();
        assert!(svg.contains("font-weight=\"330\""), "{svg}");
        assert!(svg.contains("480 Hug × 180 Hug"), "non-ASCII text was mangled: {svg}");
    }

    #[test]
    fn malformed_keys_do_not_panic() {
        assert!(svg_bytes("garbage").is_none());
        assert!(svg_bytes("20x11@9:zz.svg").is_none());
    }
}
