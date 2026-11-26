//! The external prototype device catalogue. Monet ships no devices compiled into the engine:
//! every family, variant, colour and bezel artwork is read from `devices.json` in the user's
//! devices directory, and each variant's body is a real PNG in a family subfolder. The screen
//! rectangle inside each PNG comes from the source artwork's `Screen` layer.
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
    /// Bezel PNGs by orientation (`portrait` / `landscape` / `frame`) then colour name.
    /// Paths are relative to the devices directory (e.g. `iphone-17/iPhone_17_-_Black_-_Portrait.png`).
    pub frame: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
    /// Screen size in logical points, upright.
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
    /// The screen rectangle inside each orientation's PNG: `[x, y, w, h]` in PNG pixels.
    #[serde(default, alias = "screenRects")]
    pub screen_rects: std::collections::BTreeMap<String, [f64; 4]>,
    /// Screen-window corner radius per orientation, in PNG pixels (measured from the alpha).
    #[serde(default, alias = "screenRadius")]
    pub screen_radii: std::collections::BTreeMap<String, f64>,
    /// Body outer-outline corner radius per orientation, in PNG pixels. Content spilling into
    /// the transparent corners outside this outline must be masked to the stage background.
    #[serde(default, alias = "outerRadius")]
    pub outer_radii: std::collections::BTreeMap<String, f64>,
}

impl DeviceVariant {
    /// The bezel PNG for an orientation and colour name, with a same-orientation / first-colour
    /// fallback so any (rotation, colour) pair resolves to a real file.
    pub fn frame_for(&self, orientation: &str, color: &str) -> Option<&str> {
        let pick = |o: &str| -> Option<&str> {
            let m = self.frame.get(o)?;
            m.get(color).or_else(|| m.values().next()).map(|s| s.as_str())
        };
        pick(orientation)
            .or_else(|| pick("portrait"))
            .or_else(|| pick("landscape"))
            .or_else(|| pick("frame"))
            .or_else(|| self.frame.values().find_map(|m| m.values().next().map(|s| s.as_str())))
    }

    /// The screen rectangle for an orientation, falling back to any orientation.
    pub fn screen_rect(&self, orientation: &str) -> Option<[f64; 4]> {
        self.screen_rects.get(orientation).copied().or_else(|| self.screen_rects.values().next().copied())
    }

    /// Corner radii for an orientation, falling back to any orientation.
    pub fn radii(&self, orientation: &str) -> (f64, f64) {
        let pick = |m: &std::collections::BTreeMap<String, f64>| m.get(orientation).copied().or_else(|| m.values().next().copied()).unwrap_or(0.0);
        (pick(&self.screen_radii), pick(&self.outer_radii))
    }
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
        if let Some(found) = variant.filter(|v| !v.is_empty()).and_then(|v| self.variant(v)) {
            return Some(found);
        }
        if let Some(f) = self.family(device) {
            return f.variants.first();
        }
        self.variant(device)
    }

    /// The absolute path of a variant's bezel PNG within `dir`.
    pub fn frame_path(&self, dir: &Path, variant: &DeviceVariant, orientation: &str, color: &str) -> Option<PathBuf> {
        variant.frame_for(orientation, color).map(|rel| dir.join(rel))
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

/// The installed catalogue (cloned; prefer [`with_catalog`] on hot paths to avoid the copy).
pub fn catalog() -> DeviceCatalog {
    global().read().map(|g| g.catalog.clone()).unwrap_or_default()
}

/// Borrow the installed catalogue for `f` without cloning it — the per-frame presenter calls this.
/// Returns `None` when the lock is poisoned.
pub fn with_catalog<R>(f: impl FnOnce(&DeviceCatalog) -> R) -> Option<R> {
    global().read().ok().map(|g| f(&g.catalog))
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
            for m in v.frame.values() {
                if let Some(rel) = m.values().next()
                    && !dir.join(rel).is_file()
                {
                    warnings.push(format!("{}: frame not found: {}", v.name, rel));
                }
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
            for m in v.frame.values() {
                for rel in m.values() {
                    copy_if_changed(Path::new(rel))?;
                }
            }
        }
    }
    load_dir(&dir)
}

/// Rescan after the user edited the external files.
pub fn reload() -> Result<usize, String> {
    load_dir(&devices_dir())
}

/// The pixel size `[w, h]` of an installed bezel PNG: reads only the 24-byte signature + IHDR,
/// never the whole (multi-MB) file. Cached per path so a per-frame caller is cheap.
pub fn frame_size(rel: &str) -> Option<[f64; 2]> {
    use std::io::Read;
    use std::sync::Mutex;
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, Option<[f64; 2]>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Some(hit) = cache.lock().ok().and_then(|m| m.get(rel).copied()) {
        return hit;
    }
    let size = (|| {
        let mut f = std::fs::File::open(devices_dir().join(rel)).ok()?;
        let mut head = [0u8; 24];
        f.read_exact(&mut head).ok()?;
        if &head[..8] != b"\x89PNG\r\n\x1a\n" {
            return None;
        }
        let w = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
        let h = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
        Some([w as f64, h as f64])
    })();
    if let Ok(mut m) = cache.lock() {
        m.insert(rel.to_string(), size);
    }
    size
}

/// Read the PNG bytes of a variant's bezel from the installed directory.
pub fn frame_bytes(variant: &DeviceVariant, orientation: &str, color: &str) -> Option<Vec<u8>> {
    variant.frame_for(orientation, color).and_then(|rel| std::fs::read(devices_dir().join(rel)).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r##"{
        "version": 2,
        "families": [
            {"id":"iphone-17","name":"iPhone 17","variants":[
                {"name":"iPhone 17","frame":{"portrait":{"Black":"x/1.png"}},"width":402,"height":874,"rotates":true,"kind":"phone","colors":[{"name":"Black","rgb":[56,57,61]}],"screenRects":{"portrait":[72,69,1206,2622]}},
                {"name":"iPhone 17 Pro","frame":{"portrait":{"Silver":"x/2.png"}},"width":402,"height":874,"rotates":true,"kind":"phone","colors":[{"name":"Silver","rgb":[215,215,213]}]}
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
    fn bezel_png_and_screen_rect_resolve() {
        let c = DeviceCatalog::from_json(JSON.as_bytes()).unwrap();
        let v = c.variant("iPhone 17").unwrap();
        assert_eq!(v.frame_for("portrait", "Black"), Some("x/1.png"));
        assert_eq!(v.frame_for("landscape", "Black"), Some("x/1.png"), "falls back across orientations");
        assert_eq!(v.screen_rect("portrait"), Some([72.0, 69.0, 1206.0, 2622.0]));
    }

    #[test]
    fn bundled_catalogue_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets").join("devices").join(MANIFEST_FILE);
        let bytes = std::fs::read(&path).unwrap();
        let c = DeviceCatalog::from_json(&bytes).unwrap();
        assert_eq!(c.families.len(), 13);
        assert_eq!(c.variant_names().len(), 25);
        assert!(!c.resolve("iPhone 17", Some("iPhone 17 Pro")).unwrap().colors.is_empty());
    }
}
