//! Bridge between the external device catalogue and the document's prototype settings.
//!
//! The catalogue (`figcraft-devices`, loaded from the user's `devices/` directory) is the single
//! source of device families, variants and colours. This module resolves a page's `device` +
//! `variant` pair to the concrete [`Device`] the presenter draws, and falls back to the built-in
//! defaults when the catalogue is empty (headless sessions, tests before bootstrap).
//!
//! The presenter calls these every frame, so every function borrows the catalogue through
//! [`figcraft_devices::with_catalog`] instead of cloning it.

use figcraft_devices::{DeviceVariant, NONE_DEVICE, with_catalog};
use figcraft_doc::proto::{Device, DeviceColor, PrototypeSettings};
use figcraft_i18n::tr;

/// Convert an external variant into the document's owned device shape.
fn to_device(v: &DeviceVariant) -> Device {
    Device {
        name: v.name.clone(),
        width: v.width,
        height: v.height,
        rotates: v.rotates,
        colors: v.colors.iter().map(|c| DeviceColor { name: c.name.clone(), rgb: c.rgb }).collect(),
    }
}

fn is_special(device: &str) -> bool {
    device == NONE_DEVICE || device == PrototypeSettings::CUSTOM_DEVICE
}

/// The custom-size pseudo device, from the document's own fields.
fn custom_device(ps: &PrototypeSettings) -> Device {
    let ([w, h], [dw, dh]) = (ps.custom_size, PrototypeSettings::CUSTOM_DEFAULT);
    let (width, height) = (PrototypeSettings::custom_side(w).unwrap_or(dw), PrototypeSettings::custom_side(h).unwrap_or(dh));
    Device { name: PrototypeSettings::CUSTOM_DEVICE.into(), width, height, rotates: false, colors: Vec::new() }
}

/// The effective device for `ps`, held upright. `None` / `Custom size` come from the document;
/// everything else comes from the external catalogue (falling back to the built-ins).
pub fn resolve_device(ps: &PrototypeSettings) -> Option<Device> {
    if ps.device == NONE_DEVICE {
        return None;
    }
    if ps.device == PrototypeSettings::CUSTOM_DEVICE {
        return Some(custom_device(ps));
    }
    // Prefer the external catalogue; fall back to the built-in defaults for a name it lacks.
    with_catalog(|cat| cat.resolve(&ps.device, Some(&ps.variant)).map(to_device))
        .flatten()
        .or_else(|| {
            PrototypeSettings::fallback_devices()
                .into_iter()
                .find(|d| d.name == ps.device || d.name == ps.variant)
        })
}

/// The effective device as presented (screen swapped when turned to landscape).
pub fn presented_device(ps: &PrototypeSettings) -> Option<Device> {
    let d = resolve_device(ps)?;
    let landscape = ps.rotation == figcraft_doc::proto::DeviceRotation::Landscape && d.rotates;
    Some(if landscape { Device { width: d.height, height: d.width, ..d } } else { d })
}

/// The chosen colour of the effective device (the catalogue's first unless one is picked).
pub fn device_color(ps: &PrototypeSettings) -> Option<DeviceColor> {
    let colors = resolve_device(ps)?.colors;
    colors.iter().find(|c| c.name == ps.color).or(colors.first()).cloned()
}

/// Everything the presenter needs to draw a device bezel, from the external catalogue.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameInfo {
    /// Resource key for `img()` (`device:<relative path>`), resolving the body PNG.
    pub image: String,
    /// The screen rectangle inside the PNG: `[x, y, w, h]` in PNG pixels.
    pub screen_rect: [f64; 4],
    /// The PNG's own pixel size, so the screen rect maps into the placed body.
    pub png_size: [f64; 2],
    /// `[screen window radius, body outline radius]` in PNG pixels, measured from the alpha.
    pub radii: [f64; 2],
    /// Placement relative to the snapped screen rect, filled by layout: `[dx, dy, png_w, png_h,
    /// radius_window]` all in window pixels. `device_body` / the mask use these exact values so
    /// the artwork and the content share one snapped geometry (no sub-pixel seam).
    pub placed: Option<[f32; 5]>,
}

/// A size label like `402 × 874` for a variant (its logical screen points).
pub fn size_label(variant: &str) -> String {
    let from_cat = with_catalog(|cat| cat.variant(variant).map(|v| (v.width, v.height))).flatten();
    let (w, h) = from_cat
        .or_else(|| PrototypeSettings::fallback_devices().into_iter().find(|d| d.name == variant).map(|d| (d.width, d.height)))
        .unwrap_or((0.0, 0.0));
    if w <= 0.0 {
        return String::new();
    }
    format!("{} × {}", w.round() as i64, h.round() as i64)
}

/// A size label for a family: each distinct variant size, joined (`402 × 874 / 440 × 956`).
pub fn family_size_label(family: &str) -> String {
    let labels = with_catalog(|cat| {
        let Some(f) = cat.family(family) else { return Vec::new() };
        let mut out: Vec<String> = Vec::new();
        for v in &f.variants {
            let s = format!("{} × {}", v.width.round() as i64, v.height.round() as i64);
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    });
    match labels {
        Some(l) if !l.is_empty() => l.join(" / "),
        _ => size_label(family),
    }
}

/// Resolve the effective bezel and screen rectangle for `ps` at a rotation and colour.
/// `landscape` picks the landscape artwork; the screen's logical size is still carried by
/// [`presented_device`]. `None` for None / Custom / a device that only exists in the fallback set.
pub fn frame_info(ps: &PrototypeSettings, landscape: bool, color: &str) -> Option<FrameInfo> {
    if is_special(&ps.device) {
        return None;
    }
    // Resolve inside the borrow, then read the file size outside it.
    let (rel, rect, radii) = with_catalog(|cat| {
        let v = cat.resolve(&ps.device, Some(&ps.variant))?;
        let orientation = if v.rotates { if landscape { "landscape" } else { "portrait" } } else { "frame" };
        let rel = v.frame_for(orientation, color)?.to_string();
        let rect = v.screen_rect(orientation)?;
        let (sr, or_) = v.radii(orientation);
        Some((rel, rect, [sr, or_]))
    })
    .flatten()?;
    // The manifest stores the screen rect but not the body size; read the PNG's IHDR (cached).
    // Without it, fall back to a rect-centred guess — honest but only a last resort.
    let png_size = figcraft_devices::frame_size(&rel).unwrap_or([rect[0] * 2.0 + rect[2], rect[1] * 2.0 + rect[3]]);
    Some(FrameInfo { image: format!("device:{rel}"), screen_rect: rect, png_size, radii, placed: None })
}

/// Family names for the top-level dropdown.
pub fn family_names() -> Vec<String> {
    with_catalog(|cat| {
        if cat.families.is_empty() {
            PrototypeSettings::fallback_devices().into_iter().map(|d| d.name).collect()
        } else {
            cat.family_names()
        }
    })
    .unwrap_or_else(|| PrototypeSettings::fallback_devices().into_iter().map(|d| d.name).collect())
}

/// Variant names inside a family, for the second-level dropdown. Falls back to the family itself.
pub fn variant_names(family: &str) -> Vec<String> {
    with_catalog(|cat| cat.family(family).map(|f| f.variants.iter().map(|v| v.name.clone()).collect()))
        .flatten()
        .unwrap_or_else(|| vec![family.to_string()])
}

/// A variant's colours by name (the catalogue's, else the built-in device's).
pub fn colors_for(family: &str, variant: &str) -> Vec<(String, [u8; 3])> {
    let from_cat = with_catalog(|cat| {
        cat.variant(variant)
            .or_else(|| cat.default_variant(family))
            .map(|v| v.colors.iter().map(|c| (c.name.clone(), c.rgb)).collect::<Vec<_>>())
    })
    .flatten();
    from_cat
        .filter(|v| !v.is_empty())
        .or_else(|| {
            PrototypeSettings::fallback_devices()
                .into_iter()
                .find(|d| d.name == family || d.name == variant)
                .map(|d| d.colors.into_iter().map(|c| (c.name, c.rgb)).collect())
        })
        .unwrap_or_default()
}

/// Every catalogue variant as a frame preset `(category, name, width, height)`. The Frame
/// tool uses this so its device sizes come from the same catalogue as the prototype picker,
/// instead of the frozen `FRAME_PRESETS` list.
pub fn frame_presets() -> Vec<(String, String, f64, f64)> {
    with_catalog(|cat| {
        let mut out: Vec<(String, String, f64, f64)> = Vec::new();
        for f in &cat.families {
            let category = match f.variants.first().map(|v| v.kind.as_str()) {
                Some("phone") => "Phone",
                Some("tablet") => "Tablet",
                Some("laptop") | Some("desktop") => "Desktop",
                _ => "Phone",
            };
            for v in &f.variants {
                out.push((category.to_string(), v.name.clone(), v.width, v.height));
            }
        }
        out
    })
    .unwrap_or_default()
}

/// Whether the effective device turns (phones and tablets).
pub fn rotates(ps: &PrototypeSettings) -> bool {
    resolve_device(ps).is_some_and(|d| d.rotates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_and_variant_resolve_when_catalogue_installed() {
        let cat = figcraft_devices::DeviceCatalog::from_json(
            r##"{"version":2,"families":[{"id":"x","name":"X","variants":[
                {"name":"X one","frame":{"portrait":{"Black":"x/1.png"}},"width":100,"height":200,"rotates":true,"kind":"phone","colors":[{"name":"Black","rgb":[1,2,3]}]},
                {"name":"X pro","frame":{"portrait":{"White":"x/2.png"}},"width":120,"height":240,"rotates":true,"kind":"phone","colors":[{"name":"White","rgb":[4,5,6]}]}
            ]}]}"##.as_bytes(),
        ).unwrap();
        figcraft_devices::install(figcraft_devices::DeviceLibrary { catalog: cat, warnings: Vec::new() });
        let ps = PrototypeSettings { device: "X".into(), variant: "X pro".into(), ..Default::default() };
        let d = resolve_device(&ps).unwrap();
        assert_eq!(d.name, "X pro");
        assert_eq!((d.width, d.height), (120.0, 240.0));
        assert_eq!(d.colors[0].name, "White");
        assert_eq!(variant_names("X"), vec!["X one", "X pro"]);
        assert_eq!(size_label("X pro"), "120 × 240");
        assert_eq!(family_size_label("X"), "100 × 200 / 120 × 240");
        // Restore empty catalogue for other tests.
        figcraft_devices::install(figcraft_devices::DeviceLibrary::default());
    }
}
