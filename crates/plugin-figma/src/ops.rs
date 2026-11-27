//! The commands a Figma operation turns into.
//!
//! A proxy write in JavaScript (`node.x = 40`, `parent.appendChild(child)`) becomes one of the
//! engine's settled commands. Two rules matter:
//!
//! - **Batch within one node.** `node.set` takes a `props` object, so a run of property writes on
//!   the same layer is one call (the bridge accumulates them and flushes at the end of the
//!   callback or before a read), never one call per property.
//! - **Report, don't guess.** An operation this layer has no command for returns [`OpError`]; the
//!   backend turns that into a JavaScript `Error` the plugin can catch, exactly like a Figma API
//!   that doesn't exist.
//!
//! Every command named here is one `figcraft-engine` registers (`node.create`, `node.set`,
//! `node.get`, `node.reparent`, `node.reorder`, `edit.delete`, `object.group`, `object.ungroup`,
//! `object.moveToPage`, `select.set`, `page.add`). The engine still applies its policy: a plugin
//! with `read` access is refused the writes, and the whole callback is one undo step.

use serde_json::{Map, Value, json};
use thiserror::Error;

/// One engine command and its parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub command: &'static str,
    pub params: Value,
}

impl Call {
    fn new(command: &'static str, params: Value) -> Self {
        Self { command, params }
    }
}

/// Why an operation can't be expressed as a command.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OpError {
    #[error("layer 0 is reserved for the document")]
    ZeroId,
    #[error("`{0}` isn't a layer kind Figma creates")]
    CreateKind(&'static str),
}

/// The node kinds `figma.create*` makes, in the engine's `node.create` spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateKind {
    Frame,
    Rectangle,
    Ellipse,
    Polygon,
    Star,
    Line,
    Vector,
    Text,
    Section,
    Component,
    Slice,
}

impl CreateKind {
    /// The `type` a `node.create` takes.
    pub fn create_type(self) -> &'static str {
        match self {
            Self::Frame => "frame",
            Self::Rectangle => "rectangle",
            Self::Ellipse => "ellipse",
            Self::Polygon => "polygon",
            Self::Star => "star",
            Self::Line => "line",
            Self::Vector => "vector",
            Self::Text => "text",
            Self::Section => "section",
            Self::Component => "component",
            Self::Slice => "slice",
        }
    }
}

/// What `figma.create*` sets at creation. Everything left as `None` takes the engine's default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateSpec {
    pub parent: Option<u64>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub name: Option<String>,
    /// A text layer's first content (`figma.createText()` then `characters = …`).
    pub characters: Option<String>,
}

/// `figma.createRectangle()` and friends: `node.create`, not selected (Figma leaves the document
/// selection alone; the plugin sets it explicitly).
pub fn create(kind: CreateKind, spec: &CreateSpec) -> Result<Call, OpError> {
    let mut params = Map::new();
    params.insert("type".into(), json!(kind.create_type()));
    params.insert("select".into(), json!(false));
    if let Some(parent) = spec.parent {
        params.insert("parent".into(), json!(check(parent)?));
    }
    for (key, value) in [("x", spec.x), ("y", spec.y), ("width", spec.width), ("height", spec.height)] {
        if let Some(value) = value.filter(|value| value.is_finite()) {
            params.insert(key.into(), json!(value));
        }
    }
    if let Some(name) = &spec.name {
        params.insert("name".into(), json!(name));
    }
    if let Some(characters) = &spec.characters {
        params.insert("characters".into(), json!(characters));
    }
    Ok(Call::new("node.create", Value::Object(params)))
}

/// `node.fills = …`: one `node.set` for the whole property batch.
pub fn set(id: u64, props: Map<String, Value>) -> Result<Call, OpError> {
    Ok(Call::new("node.set", json!({"id": check(id)?, "props": props})))
}

/// `figma.getNodeById(id)`: `node.get`, optionally narrowed to the fields the proxy is building.
pub fn get(id: u64, fields: Option<&[&str]>) -> Result<Call, OpError> {
    let id = check(id)?;
    Ok(match fields {
        Some(fields) => Call::new("node.get", json!({"id": id, "fields": fields})),
        None => Call::new("node.get", json!({"id": id})),
    })
}

/// `parent.appendChild(child)`: `node.reparent` (z-order is bottom-first on both sides).
pub fn append_child(parent: u64, child: u64, index: Option<usize>) -> Result<Call, OpError> {
    let mut params = json!({"ids": [check(child)?], "parent": check(parent)?});
    if let Some(index) = index
        && let Some(object) = params.as_object_mut()
    {
        object.insert("index".into(), json!(index));
    }
    Ok(Call::new("node.reparent", params))
}

/// `parent.insertChild(index, child)`: the same command with an explicit slot.
pub fn insert_child(parent: u64, child: u64, index: usize) -> Result<Call, OpError> {
    append_child(parent, child, Some(index))
}

/// `parent.children` reordered within the same parent: `node.reorder`.
pub fn reorder(id: u64, index: usize) -> Result<Call, OpError> {
    Ok(Call::new("node.reorder", json!({"id": check(id)?, "index": index})))
}

/// `node.remove()`: `edit.delete`.
pub fn remove(id: u64) -> Result<Call, OpError> {
    Ok(Call::new("edit.delete", json!({"ids": [check(id)?]})))
}

/// `figma.group(nodes, parent)`: `object.group` groups in place; the proxy reparents the new group
/// afterwards when the plugin named another parent.
pub fn group(ids: &[u64]) -> Result<Call, OpError> {
    let ids: Vec<u64> = ids.iter().copied().map(check).collect::<Result<_, _>>()?;
    Ok(Call::new("object.group", json!({"ids": ids})))
}

/// `figma.ungroup(node)`: `object.ungroup`.
pub fn ungroup(id: u64) -> Result<Call, OpError> {
    Ok(Call::new("object.ungroup", json!({"ids": [check(id)?]})))
}

/// `figma.currentPage.selection = nodes`: `select.set` (replace the selection).
pub fn select(ids: &[u64]) -> Result<Call, OpError> {
    let ids: Vec<u64> = ids.iter().copied().map(check).collect::<Result<_, _>>()?;
    Ok(Call::new("select.set", json!({"ids": ids})))
}

/// `figma.createPage()`: `page.add`.
pub fn add_page(name: Option<&str>) -> Call {
    Call::new("page.add", name.map_or_else(|| json!({}), |name| json!({"name": name})))
}

/// `node.moveToPage`: `object.moveToPage`.
pub fn move_to_page(ids: &[u64], page: u64) -> Result<Call, OpError> {
    let ids: Vec<u64> = ids.iter().copied().map(check).collect::<Result<_, _>>()?;
    Ok(Call::new("object.moveToPage", json!({"ids": ids, "page": check(page)?})))
}

fn check(id: u64) -> Result<u64, OpError> {
    if id == 0 { Err(OpError::ZeroId) } else { Ok(id) }
}
