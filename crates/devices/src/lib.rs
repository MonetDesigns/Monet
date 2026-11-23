//! The external prototype device catalogue. Monet ships no devices compiled into the engine:
//! every family, variant, colour and frame outline is read from `devices.json` in the user's
//! devices directory, and each variant's bezel artwork is a separate SVG file in a subfolder.
//!
//! The directory is `MONET_DEVICES_DIR` when set, otherwise `<APPDATA>/Monet/devices` on Windows
//! and `<HOME>/.local/share/monet/devices` elsewhere. The desktop shell installs the factory
//! manifest and its SVG frames on first run (see [`bootstrap`]); after that it is an ordinary
//! folder the user can edit. Rotation (portrait / landscape) and the preview background are fixed
//! settings in the document; everything else here is catalogue data.

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

pub const MANIFEST_FILE: &str = "devices.json";
/// The device used when none is chosen (presents the frame at its own size).
pub const NONE_DEVICE: &str = "None";
/// The device whose screen is a custom size (no body drawn around it).
pub const CUSTOM_DEVICE: &str = "Custom size";

/// A colour a device variant comes in (its body; the glass around the screen stays black).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceColor {
    pub name: String,
    /// The body's RGB bytes (the glass around the screen stays black).
    pub rgb: [u8; 3],
}

/// One concrete device (a variant of a family), held upright.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceVariant {
    pub name: String,
    /// Path to the bezel SVG, relative to the devices directory (e.g. `iphone-17/iphone-17.svg`).
    pub frame: String,
    pub width: f64,
    pub height: f64,
    /// Phones and tablets turn; laptops and desktops don't.
    #[serde(default)]
    pub rotates: bool,
    /// `phone` / `tablet` / `laptop` / `desktop` — the geometry band the frame uses.
    #[serde(default = "default_kind")]
    pub kind: String,
    /// The colours it comes in, the default first.
    #[serde(default)]
    pub colors: Vec<DeviceColor>,
}

fn default_kind() -> String {
    "phone".into()
}

/// A family shown as one row in the device dropdown; its variants are the flyout's second level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceFamily {
    pub id: String,
    /// The name shown in the top-level dropdown (e.g. `iPhone 17`).
    pub name: String,
    pub variants: Vec<DeviceVariant>,
}

/// The whole catalogue from `devices.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceCatalog {
    #[serde(default = "default_version")]
    pub version: u32,
    pub families: Vec<DeviceFamily>,
}

fn default_version() -> u32 {
    1
}

impl DeviceCatalog {
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| format!("{MANIFEST_FILE}: {e}"))
    }

    /// The family whose id or name matches, case-insensitively.
    pub fn family(&self, id_or_name: &str) -> Option<&DeviceFamily> {
        self.families.iter().find(|f| f.id.eq_ignore_ascii_case(id_or_name) || f.name.eq_ignore_ascii_case(id_or_name))
    }

    /// The variant by exact (case-insensitive) name, anywhere in the catalogue.
    pub fn variant(&self, name: &str) -> Option<&DeviceVariant> {
        self.families.iter().flat_map(|f| f.variants.iter()).find(|v| v.name.eq_ignore_ascii_case(name))
    }

    /// The variant's owning family.
    pub fn family_of(&self, variant: &str) -> Option<&DeviceFamily> {
        self.families.iter().find(|f| f.variants.iter().any(|v| v.name.eq_ignore_ascii_case(variant)))
    }

    /// All families' names, for the top-level dropdown (plus "None" / "Custom size" where used).
    pub fn family_names(&self) -> Vec<String> {
        self.families.iter().map(|f| f.name.clone()).collect()
    }

    /// All variant names, kept for documents saved before families existed.
    pub fn variant_names(&self) -> Vec<String> {
        self.families.iter().flat_map(|f| f.variants.iter().map(|v| v.name.clone())).collect()
    }

    /// The default variant of a family (its first), used when a family is chosen with no variant.
    pub fn default_variant(&self, family: &str) -> Option<&DeviceVariant> {
        self.family(family)?.variants.first()
    }

    /// Resolve a document's `device` + `variant` pair. Old documents store a variant name in
    /// `device`; those keep working by finding its family. `None` / `Custom size` are not here.
    pub fn resolve(&self, device: &str, variant: Option<&str>) -> Option<&DeviceVariant> {
        if let Some(v) = variant.filter(|v| !v.is_empty()) {
            if let Some(found) = self.variant(v) {
                return Some(found);
            }
        }
        if let Some(f) = self.family(device) {
            return f.variants.first();
        }
        self.variant(device)
    }

    /// The absolute path of a variant's frame SVG within `dir`.
    pub fn frame_path(&self, dir: &Path, variant: &DeviceVariant) -> PathBuf {
        dir.join(&variant.frame)
    }
}

/// An installed catalogue plus any load warnings.
#[derive(Clone, Debug, Default)]
pub struct DeviceLibrary {
    pub catalog: DeviceCatalog,
    pub warnings: Vec<String>,
}

static GLOBAL: OnceLock<RwLock<DeviceLibrary>> = OnceLock::new();

pub fn global() -> &'static RwLock<DeviceLibrary> {
    GLOBAL.get_or_init(|| RwLock::new(DeviceLibrary::default()))
}

pub fn install(lib: DeviceLibrary) {
    if let Ok(mut g) = global().write() {
        *g = lib;
    }
}

pub fn catalog() -> DeviceCatalog {
    global().read().map(|g| g.catalog.clone()).unwrap_or_default()
}

/// Where the external device catalogue lives.
pub fn devices_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("MONET_DEVICES_DIR") {
        return PathBuf::from(p);
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("Monet").join("devices");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local").join("share").join("monet").join("devices");
    }
    PathBuf::from("devices")
}

/// Rescan the devices directory and install the result. Returns the number of variants.
pub fn load_dir(dir: &Path) -> Result<usize, String> {
    let manifest = dir.join(MANIFEST_FILE);
    let bytes = std::fs::read(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let cat = DeviceCatalog::from_json(&bytes)?;
    let mut warnings = Vec::new();
    for f in &cat.families {
        for v in &f.variants {
            if !dir.join(&v.frame).is_file() {
                warnings.push(format!("{}: frame not found: {}", v.name, v.frame));
            }
        }
    }
    let n = cat.families.iter().map(|f| f.variants.len()).sum();
    for w in &warnings {
        eprintln!("monet devices: {w}");
    }
    install(DeviceLibrary { catalog: cat, warnings });
    Ok(n)
}

/// Locate the bundled factory directory (an asset beside the executable, or the repository's
/// `assets/devices` in a development build). Never compiled in.
fn bundled_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MONET_ASSETS_DIR") {
        let d = PathBuf::from(p).join("devices");
        if d.join(MANIFEST_FILE).is_file() {
            return Some(d);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let d = parent.join("assets").join("devices");
        if d.join(MANIFEST_FILE).is_file() {
            return Some(d);
        }
        // Development: exe sits in target/{profile}/examples.
        let mut cur = parent.to_path_buf();
        for _ in 0..4 {
            cur.pop();
            let d = cur.join("assets").join("devices");
            if d.join(MANIFEST_FILE).is_file() {
                return Some(d);
            }
        }
    }
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets").join("devices");
    if d.join(MANIFEST_FILE).is_file() {
        return Some(d);
    }
    None
}

/// Copy the factory catalogue into the user's devices directory where it differs, then load it.
/// The manifest and every frame are ordinary external files; the factory copy is kept in step.
pub fn bootstrap() -> Result<usize, String> {
    let dir = devices_dir();
    let src = bundled_dir().ok_or_else(|| "bundled assets/devices not found".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let copy_if_changed = |rel: &Path| -> Result<(), String> {
        let from = src.join(rel);
        let to = dir.join(rel);
        let bytes = std::fs::read(&from).map_err(|e| format!("{}: {e}", from.display()))?;
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        if std::fs::read(&to).ok().as_deref() != Some(bytes.as_slice()) {
            std::fs::write(&to, &bytes).map_err(|e| format!("{}: {e}", to.display()))?;
        }
        Ok(())
    };
    copy_if_changed(Path::new(MANIFEST_FILE))?;
    let cat = DeviceCatalog::from_json(&std::fs::read(src.join(MANIFEST_FILE)).unwrap())?;
    for f in &cat.families {
        for v in &f.variants {
            copy_if_changed(Path::new(&v.frame))?;
        }
    }
    load_dir(&dir)
}

/// Rescan after the user edited the external files.
pub fn reload() -> Result<usize, String> {
    load_dir(&devices_dir())
}

/// Read the SVG bytes of a variant's frame from the installed directory.
pub fn frame_bytes(variant: &DeviceVariant) -> Option<Vec<u8>> {
    std::fs::read(devices_dir().join(&variant.frame)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r##"{
        "version": 1,
        "families": [
            {"id":"iphone-17","name":"iPhone 17","variants":[
                {"name":"iPhone 17","frame":"iphone-17/iphone-17.svg","width":402,"height":874,"rotates":true,"kind":"phone","colors":[{"name":"Black","rgb":[56,57,61]}]},
                {"name":"iPhone 17 Pro","frame":"iphone-17/iphone-17-pro.svg","width":402,"height":874,"rotates":true,"kind":"phone","colors":[{"name":"Silver","rgb":[215,215,213]}]}
            ]}
        ]
    }"##;

    #[test]
    fn families_group_variants_and_resolve_by_name() {
        let c = DeviceCatalog::from_json(JSON.as_bytes()).unwrap();
        assert_eq!(c.family_names(), vec!["iPhone 17"]);
        assert_eq!(c.variant_names(), vec!["iPhone 17", "iPhone 17 Pro"]);
        assert_eq!(c.resolve("iPhone 17", Some("iPhone 17 Pro")).unwrap().name, "iPhone 17 Pro");
        // An old document storing the variant name in `device` still resolves.
        assert_eq!(c.resolve("iPhone 17 Pro", None).unwrap().name, "iPhone 17 Pro");
        assert_eq!(c.family_of("iPhone 17 Pro").unwrap().name, "iPhone 17");
    }

    #[test]
    fn bundled_catalogue_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets").join("devices").join(MANIFEST_FILE);
        let bytes = std::fs::read(&path).unwrap();
        let c = DeviceCatalog::from_json(&bytes).unwrap();
        assert_eq!(c.families.len(), 9);
        assert_eq!(c.variant_names().len(), 19);
        assert_eq!(c.resolve("iPhone 17", Some("iPhone 17 Pro")).unwrap().colors[0].name, "Black");
    }
}
