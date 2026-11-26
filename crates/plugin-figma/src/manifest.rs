//! `manifest.json`: the Figma plugin manifest, parsed and checked.
//!
//! A Figma plugin folder is `manifest.json` plus the scripts it names: `main` (JavaScript, with
//! `__html__` inlined from `ui` when `showUI` is called) and optionally `ui` (HTML). The fields
//! that matter here are the ones the runtime has to honour: the API version, the editor scopes,
//! whether pages load on demand, and which hosts the plugin may reach.
//!
//! Structural problems are refused with a [`ManifestError`]; things FigCraft cannot run yet (the
//! widget API, board-only plugins, Developer Mode, Slides) are not errors: [`Manifest::blockers`]
//! reports them so Plugins ▸ Manage plugins can say exactly what is missing.

use serde::{Deserialize, Deserializer, Serialize};
use url::Url;

/// The file a plugin folder names as its entry.
pub const MANIFEST_FILE: &str = "manifest.json";
/// The prefix FigCraft gives a Figma plugin's id (`figma:<id>`), so ids never collide with ours.
pub const PLUGIN_KEY_PREFIX: &str = "figma:";
/// The only plugin API major version this layer targets. Figma publishes `"1.0.0"`.
pub const API_MAJOR: u64 = 1;
/// The longest `main` or `ui` path accepted.
pub const MAX_ASSET_PATH: usize = 240;
/// The longest plugin name accepted (Figma's own limit is 255).
pub const MAX_NAME: usize = 255;

/// Why a manifest was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("the manifest isn't JSON: {0}")]
    Json(String),
    #[error("`{field}` {why}")]
    Invalid { field: &'static str, why: String },
}

fn invalid(field: &'static str, why: impl Into<String>) -> ManifestError {
    ManifestError::Invalid { field, why: why.into() }
}

/// Where a plugin may run, from `editorType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditorType {
    Figma,
    FigJam,
    Dev,
    Slides,
    /// A value from a newer Figma than this one: kept, and reported as unsupported.
    #[serde(other)]
    Other,
}

/// Which editor scopes a manifest asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Design,
    Board,
    DeveloperMode,
    Slides,
}

impl EditorType {
    fn scope(self) -> Scope {
        match self {
            Self::Figma => Scope::Design,
            Self::FigJam => Scope::Board,
            Self::Dev => Scope::DeveloperMode,
            Self::Slides => Scope::Slides,
            Self::Other => Scope::Slides,
        }
    }
}

/// What FigCraft does not provide yet, for a manifest that asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    /// `containsWidget` / `widgetApi`: Figma's widget API is a different runtime (hooks + JSX).
    Widget,
    /// `editorType` includes `figjam`: boards are read-only for plugins here.
    Board,
    /// `editorType` is `dev`: the Inspect / codegen plugin modes.
    DeveloperMode,
    /// `editorType` includes `slides`.
    Slides,
}

impl Blocker {
    /// A sentence for the user, naming what the plugin needs.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Widget => "this plugin is a Figma widget (the widget API runs on hooks and JSX)",
            Self::Board => "this plugin runs in FigJam, which plugins can't edit here yet",
            Self::DeveloperMode => "this plugin runs in Dev Mode / Developer Mode, which isn't available yet",
            Self::Slides => "this plugin runs in Figma Slides, which isn't available yet",
        }
    }
}

/// `documentAccess`: whether pages load on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DocumentAccess {
    /// The field is missing: Figma loads every page before the plugin runs (legacy).
    #[default]
    Legacy,
    /// New plugins: pages load on demand, and the synchronous getters throw.
    DynamicPage,
}

impl<'de> Deserialize<'de> for DocumentAccess {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "dynamic-page" => Ok(Self::DynamicPage),
            "legacy" => Ok(Self::Legacy),
            other => Err(serde::de::Error::custom(format!("`documentAccess` is `dynamic-page` or missing, got `{other}`"))),
        }
    }
}

/// `networkAccess`: which hosts a plugin may reach (`fetch`, `XMLHttpRequest`, `WebSocket`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkAccess {
    /// Hosts, each optionally with a scheme, a port and a path prefix (`https://api.example.com`,
    /// `*.example.com`, `example.com/v1/`). `["none"]` means none, `["*"]` means anything.
    #[serde(default, rename = "allowedDomains")]
    pub allowed_domains: Vec<String>,
    /// Why the plugin needs the network, for the install confirmation.
    #[serde(default)]
    pub reasoning: Option<String>,
    /// Hosts allowed only while the plugin is imported from a manifest (development).
    #[serde(default, rename = "devAllowedDomains")]
    pub dev_allowed_domains: Vec<String>,
}

impl NetworkAccess {
    /// Whether `url` may be reached. `developer` is true for a plugin imported from a manifest,
    /// where `devAllowedDomains` applies too. Off-scheme (`file:`, `data:`) never passes.
    pub fn allows(&self, url: &str, developer: bool) -> bool {
        if !matches!(parse_scheme(url), Some("http" | "https" | "ws" | "wss")) {
            return false;
        }
        let Ok(parsed) = Url::parse(url) else { return false };
        let mut entries = self.allowed_domains.iter();
        let dev = self.dev_allowed_domains.iter().filter(|_| developer);
        entries.by_ref().chain(dev).any(|entry| allows_entry(entry, &parsed))
    }

    /// Whether the manifest asks for any network at all.
    pub fn is_empty(&self) -> bool {
        self.allowed_domains.iter().all(|d| d == "none") && self.dev_allowed_domains.is_empty()
    }
}

fn parse_scheme(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    (!scheme.is_empty() && rest.starts_with(|c: char| c.is_ascii_alphanumeric())).then_some(scheme)
}

fn allows_entry(entry: &str, url: &Url) -> bool {
    let entry = entry.trim();
    if entry.is_empty() || entry == "none" {
        return false;
    }
    if entry == "*" {
        return true;
    }
    let (scheme, rest) = match entry.split_once("://") {
        Some((scheme, rest)) => (Some(scheme.to_ascii_lowercase()), rest),
        None => (None, entry),
    };
    if let Some(scheme) = &scheme
        && !url.scheme().eq_ignore_ascii_case(scheme)
    {
        return false;
    }
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return false;
    }
    let (host, port) = split_port(authority);
    if let Some(port) = port
        && url.port_or_known_default() != Some(port)
    {
        return false;
    }
    let host = host.to_ascii_lowercase();
    let some_host = url.host_str();
    let host_matches = if host == "*" {
        true
    } else if let Some(suffix) = host.strip_prefix("*.") {
        // Strict: `*.example.com` matches `a.example.com`, not `example.com` itself.
        some_host.is_some_and(|h| {
            let h = h.to_ascii_lowercase();
            h.len() > suffix.len() && h.ends_with(&format!(".{suffix}"))
        })
    } else {
        some_host.is_some_and(|h| h.eq_ignore_ascii_case(&host))
    };
    if !host_matches {
        return false;
    }
    path.is_empty() || url.path().starts_with(path)
}

fn split_port(authority: &str) -> (&str, Option<u16>) {
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => (host, port.parse().ok()),
        _ => (authority, None),
    }
}

/// A parameter a quick-action plugin accepts (`figma.parameters`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub key: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "allowCustomValue")]
    pub allow_custom_value: bool,
}

/// An item of the plugin's context menu.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuItem {
    pub command: String,
    pub name: String,
    #[serde(default)]
    pub separator: bool,
}

/// A relaunch button (`figma.setRelaunchData`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelaunchButton {
    pub command: String,
    pub name: String,
}

/// A capability a plugin declares (Dev Mode, codegen, VS Code…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    Inspect,
    Codegen,
    Vscode,
    Textreview,
    Auth,
    #[serde(other)]
    Other,
}

/// Figma's `manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    /// Figma's numeric plugin id, as a string (`"1234567890123456789"`).
    pub id: String,
    /// `"1.0.0"`: checked, and kept as written.
    #[serde(deserialize_with = "deserialize_api")]
    pub api: String,
    /// The JavaScript bundle, relative to the manifest (`"dist/code.js"`).
    pub main: String,
    /// The HTML frame `figma.showUI(__html__)` shows, relative to the manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<String>,
    #[serde(default, rename = "editorType")]
    pub editor_type: Vec<EditorType>,
    #[serde(default, rename = "documentAccess")]
    pub document_access: DocumentAccess,
    #[serde(default, rename = "networkAccess")]
    pub network_access: NetworkAccess,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub menu: Vec<MenuItem>,
    #[serde(default, rename = "relaunchButtons")]
    pub relaunch_buttons: Vec<RelaunchButton>,
    #[serde(default, rename = "containsWidget")]
    pub contains_widget: bool,
    #[serde(default, rename = "widgetApi")]
    pub widget_api: Option<String>,
    #[serde(default, rename = "enableProposedApi")]
    pub enable_proposed_api: bool,
}

impl Manifest {
    /// Parse and check a `manifest.json`'s text.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_str(text).map_err(|e| ManifestError::Json(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Refuse a manifest whose paths or versions can't be honoured.
    pub fn validate(&self) -> Result<(), ManifestError> {
        let name_len = self.name.chars().count();
        if name_len == 0 || name_len > MAX_NAME || self.name.chars().any(char::is_control) {
            return Err(invalid("name", format!("must be 1–{MAX_NAME} characters without control characters")));
        }
        if self.id.is_empty() || self.id.len() > 64 || !self.id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid("id", "must be Figma's numeric plugin id (digits only)"));
        }
        if api_major(&self.api) != Some(API_MAJOR) {
            return Err(invalid("api", format!("must be the 1.x plugin API, got `{}`", self.api)));
        }
        relative_asset("main", &self.main, ".js")?;
        if let Some(ui) = &self.ui {
            relative_asset("ui", ui, ".html")?;
            if ui == &self.main {
                return Err(invalid("ui", "can't be the same file as `main`"));
            }
        }
        if self.editor_type.is_empty() {
            return Err(invalid("editorType", "is required (`[\"figma\"]` for a design plugin)"));
        }
        if self.parameters.len() > 64 {
            return Err(invalid("parameters", "takes at most 64 parameters"));
        }
        for parameter in &self.parameters {
            if parameter.name.is_empty() || parameter.key.is_empty() {
                return Err(invalid("parameters", "needs a `name` and a `key` on every parameter"));
            }
        }
        Ok(())
    }

    /// The id FigCraft keeps this plugin under (`figma:<id>`).
    pub fn plugin_key(&self) -> String {
        format!("{PLUGIN_KEY_PREFIX}{}", self.id)
    }

    /// The scopes the manifest asks for.
    pub fn scopes(&self) -> Vec<Scope> {
        let mut scopes: Vec<Scope> = self.editor_type.iter().map(|e| e.scope()).collect();
        scopes.dedup();
        scopes
    }

    /// What FigCraft doesn't provide for this manifest, each with a reason for the user.
    pub fn blockers(&self) -> Vec<Blocker> {
        let mut blockers = Vec::new();
        if self.contains_widget || self.widget_api.is_some() {
            blockers.push(Blocker::Widget);
        }
        for scope in self.scopes() {
            let blocker = match scope {
                Scope::Design => None,
                Scope::Board => Some(Blocker::Board),
                Scope::DeveloperMode => Some(Blocker::DeveloperMode),
                Scope::Slides => Some(Blocker::Slides),
            };
            if let Some(blocker) = blocker
                && !blockers.contains(&blocker)
            {
                blockers.push(blocker);
            }
        }
        blockers
    }

    /// Whether the plugin runs as a design plugin (the scope this layer implements first).
    pub fn runs_in_design(&self) -> bool {
        self.scopes().contains(&Scope::Design) && !self.contains_widget
    }
}

/// The major version of `"1.0.0"` (or `"1"`, `"1.2"`), when every part is a number.
fn api_major(api: &str) -> Option<u64> {
    let mut parts = api.trim().split('.');
    let major = parts.next()?;
    if major.is_empty() || !major.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let major: u64 = major.parse().ok()?;
    let rest_ok = parts.all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
    rest_ok.then_some(major)
}

/// `api` as Figma writes it (a string), or a number in a hand-written manifest.
fn deserialize_api<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Api {
        Text(String),
        Number(f64),
    }
    match Api::deserialize(deserializer)? {
        Api::Text(text) => Ok(text),
        Api::Number(number) if number.is_finite() && number >= 0.0 && number.fract() == 0.0 => Ok(format!("{number:.0}")),
        Api::Number(number) => Err(serde::de::Error::custom(format!("`api` must be a version like \"1.0.0\", got {number}"))),
    }
}

/// A `main`/`ui` path: relative, inside the folder, with the expected extension.
fn relative_asset(field: &'static str, value: &str, extension: &str) -> Result<(), ManifestError> {
    let bad_path = value.is_empty()
        || value.len() > MAX_ASSET_PATH
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains(':')
        || value.contains('\0')
        || value.split('/').any(|segment| segment.is_empty() || segment == "." || segment == "..");
    if bad_path {
        return Err(invalid(field, "must be a relative path inside the plugin folder (no `..`, no absolute path, `/` as the separator)"));
    }
    if !value.to_ascii_lowercase().ends_with(extension) {
        return Err(invalid(field, format!("must end in `{extension}`")));
    }
    Ok(())
}
