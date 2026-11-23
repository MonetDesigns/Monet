//! Bridge between the external device catalogue and the document's prototype settings.
//!
//! The catalogue (`figcraft-devices`, loaded from the user's `devices/` directory) is the single
//! source of device families, variants and colours. This module resolves a page's `device` +
//! `variant` pair to the concrete [`Device`] the presenter draws, and falls back to the built-in
//! defaults when the catalogue is empty (headless sessions, tests before bootstrap).

use figcraft_devices::{DeviceColor as ExtColor, DeviceVariant, NONE_DEVICE, catalog};
use figcraft_doc::proto::{Device, DeviceColor, PrototypeSettings};

/// Convert an external variant into the document's owned device shape.
pub fn to_device(v: &DeviceVariant) -> Device {
    Device {
        name: v.name.clone(),
        width: v.width,
        height: v.height,
        rotates: v.rotates,
        colors: v.colors.iter().map(|c| to_color(c)).collect(),
    }
}

fn to_color(c: &ExtColor) -> DeviceColor {
    DeviceColor { name: c.name.clone(), rgb: c.rgb }
}

/// The effective device for `ps`, held upright. `None` / `Custom size` come from the document;
/// everything else comes from the external catalogue (falling back to the built-ins).
pub fn resolve_device(ps: &PrototypeSettings) -> Option<Device> {
    if ps.device == NONE_DEVICE {
        return None;
    }
    if ps.device == PrototypeSettings::CUSTOM_DEVICE {
        let ([w, h], [dw, dh]) = (ps.custom_size, PrototypeSettings::CUSTOM_DEFAULT);
        let (width, height) = (PrototypeSettings::custom_side(w).unwrap_or(dw), PrototypeSettings::custom_side(h).unwrap_or(dh));
        return Some(Device {
            name: PrototypeSettings::CUSTOM_DEVICE.into(),
            width,
            height,
            rotates: false,
            colors: Vec::new(),
        });
    }
    let cat = catalog();
    if !cat.families.is_empty() {
        if let Some(v) = cat.resolve(&ps.device, Some(&ps.variant)) {
            return Some(to_device(v));
        }
    }
    // Fallback: the built-in defaults, with the family's first variant for a family name.
    let built = PrototypeSettings::fallback_devices();
    built.into_iter().find(|d| d.name == ps.device || d.name == ps.variant).or_else(|| {
        let cat = catalog();
        cat.family(&ps.device).and_then(|f| f.variants.first()).map(to_device)
    })
}

/// The effective device as presented (swapped when turned to landscape).
pub fn presented_device(ps: &PrototypeSettings) -> Option<Device> {
    let d = resolve_device(ps)?;
    let landscape = ps.rotation == figcraft_doc::proto::DeviceRotation::Landscape && d.rotates;
    Some(if landscape { Device { width: d.height, height: d.width, ..d } } else { d })
}

/// The chosen colour of the effective device.
pub fn device_color(ps: &PrototypeSettings) -> Option<DeviceColor> {
    let colors = resolve_device(ps)?.colors;
    colors.iter().find(|c| c.name == ps.color).or(colors.first()).cloned()
}

/// The external bezel SVG path for the effective variant (relative to the devices directory),
/// or `None` for None / Custom / a device that only exists in the fallback set.
pub fn frame_path_of(ps: &PrototypeSettings) -> Option<String> {
    if ps.device == NONE_DEVICE || ps.device == PrototypeSettings::CUSTOM_DEVICE {
        return None;
    }
    let cat = catalog();
    if cat.families.is_empty() {
        return None;
    }
    cat.resolve(&ps.device, Some(&ps.variant)).map(|v| v.frame.clone())
}

/// Family names for the top-level dropdown (plus the fixed None / Custom entries elsewhere).
pub fn family_names() -> Vec<String> {
    let cat = catalog();
    if cat.families.is_empty() {
        return PrototypeSettings::fallback_devices().into_iter().map(|d| d.name).collect();
    }
    cat.family_names()
}

/// Variant names inside a family, for the second-level dropdown. Falls back to the family itself.
pub fn variant_names(family: &str) -> Vec<String> {
    let cat = catalog();
    if let Some(f) = cat.family(family) {
        return f.variants.iter().map(|v| v.name.clone()).collect();
    }
    vec![family.to_string()]
}

/// A variant's colours by name.
pub fn colors_for(family: &str, variant: &str) -> Vec<(String, [u8; 3])> {
    let cat = catalog();
    if let Some(v) = cat.variant(variant).or_else(|| cat.default_variant(family)) {
        return v.colors.iter().map(|c| (c.name.clone(), c.rgb)).collect();
    }
    PrototypeSettings::fallback_devices()
        .into_iter()
        .find(|d| d.name == family || d.name == variant)
        .map(|d| d.colors.into_iter().map(|c| (c.name, c.rgb)).collect())
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
            r##"{"version":1,"families":[{"id":"x","name":"X","variants":[
                {"name":"X one","frame":"x/1.svg","width":100,"height":200,"rotates":true,"kind":"phone","colors":[{"name":"Black","rgb":[1,2,3]}]},
                {"name":"X pro","frame":"x/2.svg","width":100,"height":200,"rotates":true,"kind":"phone","colors":[{"name":"White","rgb":[4,5,6]}]}
            ]}]}"##.as_bytes(),
        ).unwrap();
        figcraft_devices::install(figcraft_devices::DeviceLibrary { catalog: cat, warnings: Vec::new() });
        let ps = PrototypeSettings { device: "X".into(), variant: "X pro".into(), ..Default::default() };
        let d = resolve_device(&ps).unwrap();
        assert_eq!(d.name, "X pro");
        assert_eq!(d.colors[0].name, "White");
        assert_eq!(variant_names("X"), vec!["X one", "X pro"]);
        // Restore empty catalogue for other tests.
        figcraft_devices::install(figcraft_devices::DeviceLibrary::default());
    }
}
