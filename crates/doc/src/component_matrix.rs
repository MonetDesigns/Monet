//! Local, composable choices for one main component.
//!
//! SOURCE NOTE (2026-11-21): ported from `brava-group/lizard` main,
//! `crates/doc/src/component_matrix.rs` (github.com/brava-group/lizard). This is a LIZARD-ONLY
//! engine extension — `storytold/figcraft` main has no `ComponentMatrix` / `component_matrix`
//! field on `Node` and no `component.matrix*` commands (verified against its live main source,
//! not a local copy). Kept here so Monet's component panel can offer lizard's value-table
//! behaviour. WHEN figcraft gains an equivalent, re-sync from figcraft and drop this note.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::NodeId;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComponentMatrix {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub axes: Vec<ComponentAxis>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<ComponentField>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentAxis {
    pub name: String,
    pub options: Vec<String>,
    pub default: String,
}

/// A field on the main component or one of its layers. The most specific matching cell wins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentField {
    pub layer: NodeId,
    pub field: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<ComponentCell>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentCell {
    /// Empty means a fallback; a partial selection applies to every other axis value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub when: BTreeMap<String, String>,
    pub value: Value,
}

impl ComponentMatrix {
    /// Whether the node has no value table (the common case; omitted when serializing).
    pub fn is_empty(&self) -> bool {
        self.axes.is_empty() && self.fields.is_empty()
    }

    pub fn choices(&self, selected: &BTreeMap<String, Value>) -> BTreeMap<String, String> {
        self.axes
            .iter()
            .map(|axis| {
                let value = selected.get(&axis.name).and_then(Value::as_str).filter(|v| axis.options.iter().any(|o| o == *v));
                (axis.name.clone(), value.unwrap_or(&axis.default).to_string())
            })
            .collect()
    }

    pub fn resolve<'a>(&'a self, field: &'a ComponentField, choices: &BTreeMap<String, String>) -> Option<&'a Value> {
        field
            .cells
            .iter()
            .filter(|cell| cell.when.iter().all(|(axis, value)| choices.get(axis) == Some(value)))
            .max_by_key(|cell| cell.when.len())
            .map(|cell| &cell.value)
    }
}
