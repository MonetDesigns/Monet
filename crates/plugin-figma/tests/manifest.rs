#![allow(clippy::unwrap_used)]
// A test that can't unwrap can't assert: the workspace lints let tests do it (AGENTS.md › Never crash).

use figcraft_plugin_figma::manifest::{Blocker, DocumentAccess, Manifest, ManifestError, Scope};

const ICON_GRID: &str = r#"{
  "name": "Icon grid",
  "id": "1234567890123456789",
  "api": "1.0.0",
  "main": "dist/code.js",
  "ui": "dist/ui.html",
  "editorType": ["figma"],
  "documentAccess": "dynamic-page",
  "networkAccess": {
    "allowedDomains": ["https://api.example.com", "*.cdn.example.com", "example.org/v1/"],
    "devAllowedDomains": ["http://localhost:3000"],
    "reasoning": "Fetches the icon set."
  },
  "parameters": [{"name": "Set", "key": "set", "description": "Which set"}],
  "menu": [{"command": "grid", "name": "Grid"}]
}"#;

fn icon_grid() -> Manifest {
    Manifest::parse(ICON_GRID).unwrap()
}

#[test]
fn reads_a_new_style_manifest() {
    let manifest = icon_grid();
    assert_eq!(manifest.plugin_key(), "figma:1234567890123456789");
    assert_eq!(manifest.scopes(), vec![Scope::Design]);
    assert_eq!(manifest.blockers(), Vec::<Blocker>::new());
    assert!(manifest.runs_in_design());
    assert_eq!(manifest.document_access, DocumentAccess::DynamicPage);
    assert_eq!(manifest.ui.as_deref(), Some("dist/ui.html"));
    assert_eq!(manifest.parameters[0].key, "set");
    assert_eq!(manifest.menu[0].command, "grid");
}

#[test]
fn network_access_matches_hosts_schemes_paths_and_development() {
    let network = icon_grid().network_access;
    assert!(network.allows("https://api.example.com/v2/icons?q=1", false));
    assert!(!network.allows("http://api.example.com/v2/icons", false), "scheme matters");
    assert!(!network.allows("https://api.example.com.evil.com/", false));
    assert!(!network.allows("https://evil.com/", false));
    assert!(network.allows("https://icons.cdn.example.com/a.svg", false), "subdomains of a wildcard");
    assert!(!network.allows("https://cdn.example.com/a.svg", false), "the wildcard is strict");
    assert!(network.allows("https://example.org/v1/tree.json", false));
    assert!(!network.allows("https://example.org/v2/tree.json", false));
    assert!(network.allows("http://localhost:3000/dev", true));
    assert!(!network.allows("http://localhost:3000/dev", false), "development only");
    assert!(!network.allows("file:///etc/passwd", true));
    assert!(!network.allows("data:text/html,x", true));
}

#[test]
fn none_and_star_are_honoured() {
    let none = Manifest::parse(
        &ICON_GRID
            .replace(r#"["https://api.example.com", "*.cdn.example.com", "example.org/v1/"]"#, r#"["none"]"#)
            .replace(r#""devAllowedDomains": ["http://localhost:3000"],"#, ""),
    )
    .unwrap();
    assert!(!none.network_access.allows("https://anything.example/", false));
    assert!(none.network_access.is_empty());

    let star = Manifest::parse(&ICON_GRID.replace(r#"["https://api.example.com", "*.cdn.example.com", "example.org/v1/"]"#, r#"["*"]"#)).unwrap();
    assert!(star.network_access.allows("https://anything.example/", false));
}

#[test]
fn a_widget_is_reported_not_half_supported() {
    let text = ICON_GRID
        .replace(r#""ui": "dist/ui.html","#, "")
        .replace(r#""documentAccess": "dynamic-page","#, r#""documentAccess": "dynamic-page", "containsWidget": true, "widgetApi": "1.0.0","#)
        .replace(r#""menu": [{"command": "grid", "name": "Grid"}]"#, r#""menu": []"#);
    let manifest = Manifest::parse(&text).unwrap();
    assert_eq!(manifest.blockers(), vec![Blocker::Widget]);
    assert!(!manifest.runs_in_design());
    assert!(!manifest.blockers()[0].reason().is_empty());
}

#[test]
fn figjam_is_a_blocker_but_design_still_runs() {
    let text = ICON_GRID.replace(r#""editorType": ["figma"]"#, r#""editorType": ["figma", "figjam"]"#);
    let manifest = Manifest::parse(&text).unwrap();
    assert!(manifest.runs_in_design());
    assert_eq!(manifest.blockers(), vec![Blocker::Board]);
}

#[test]
fn a_dev_only_plugin_is_refused_by_the_scope() {
    let text = ICON_GRID.replace(r#""editorType": ["figma"]"#, r#""editorType": ["dev"]"#);
    let manifest = Manifest::parse(&text).unwrap();
    assert!(!manifest.runs_in_design());
    assert_eq!(manifest.blockers(), vec![Blocker::DeveloperMode]);
}

#[test]
fn refuses_an_api_this_layer_does_not_target() {
    let text = ICON_GRID.replace(r#""api": "1.0.0""#, r#""api": "2.0.0""#);
    assert!(matches!(Manifest::parse(&text), Err(ManifestError::Invalid { field: "api", .. })));
    let text = ICON_GRID.replace(r#""api": "1.0.0""#, r#""api": "1.0.0-beta""#);
    assert!(matches!(Manifest::parse(&text), Err(ManifestError::Invalid { field: "api", .. })));
}

#[test]
fn accepts_a_numeric_or_partial_api() {
    let text = ICON_GRID.replace(r#""api": "1.0.0""#, r#""api": 1"#);
    assert_eq!(Manifest::parse(&text).unwrap().api, "1");
    let text = ICON_GRID.replace(r#""api": "1.0.0""#, r#""api": "1.4""#);
    assert!(Manifest::parse(&text).is_ok());
}

#[test]
fn refuses_paths_that_leave_the_folder() {
    for main in ["../code.js", "/code.js", "C:/code.js", "dist\\\\code.js", "./code.js", "code.txt"] {
        let text = ICON_GRID.replace(r#""main": "dist/code.js""#, &format!(r#""main": "{main}""#));
        assert!(matches!(Manifest::parse(&text), Err(ManifestError::Invalid { field: "main", .. })), "`{main}` was accepted");
    }
    let text = ICON_GRID.replace(r#""ui": "dist/ui.html""#, r#""ui": "dist/ui.htm""#);
    assert!(matches!(Manifest::parse(&text), Err(ManifestError::Invalid { field: "ui", .. })));
}

#[test]
fn refuses_a_manifest_without_a_scope_or_with_bad_ids() {
    let text = ICON_GRID.replace(r#""editorType": ["figma"]"#, r#""editorType": []"#);
    assert!(matches!(Manifest::parse(&text), Err(ManifestError::Invalid { field: "editorType", .. })));
    let text = ICON_GRID.replace(r#""id": "1234567890123456789""#, r#""id": "abc-123""#);
    assert!(matches!(Manifest::parse(&text), Err(ManifestError::Invalid { field: "id", .. })));
    let text = ICON_GRID.replace(r#""name": "Icon grid""#, r#""name": """#);
    assert!(matches!(Manifest::parse(&text), Err(ManifestError::Invalid { field: "name", .. })));
}

#[test]
fn a_missing_document_access_is_the_legacy_mode() {
    let text = ICON_GRID.replace(r#""documentAccess": "dynamic-page","#, "");
    let manifest = Manifest::parse(&text).unwrap();
    assert_eq!(manifest.document_access, DocumentAccess::Legacy);
}
