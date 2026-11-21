//! Component value-table (matrix) commands.
//!
//! SOURCE NOTE (2026-11-21): ported from `brava-group/lizard` main,
//! `crates/engine/src/cmd/component.rs` (the `MATRIX_FIELDS` + `matrix_*` block).
//! `storytold/figcraft` main has NO component_matrix / component.matrix* commands (verified
//! against its live main source). WHEN figcraft gains equivalents, re-sync from figcraft
//! and remove this note.

use std::collections::BTreeMap;

use figcraft_doc::{ComponentAxis, ComponentCell, ComponentField, ComponentMatrix, Document, Node, NodeId, NodeKind};
use serde_json::{Value, json};

use crate::components::{inside_instance, prop_default_value, variant_options, variant_set};
use crate::params::{req_id, string};
use crate::{EngineError, Result, Session, bad};

pub(crate) use matrix_impl::*;

mod matrix_impl {
use super::*;

    const MATRIX_FIELDS: &[&str] = &[
        "fill",
        "fills",
        "stroke",
        "strokes",
        "strokeWeight",
        "cornerRadius",
        "opacity",
        "visible",
        "width",
        "height",
        "padding",
        "paddingTop",
        "paddingRight",
        "paddingBottom",
        "paddingLeft",
        "gap",
        "crossGap",
        "sizingH",
        "sizingV",
        "fontSize",
        "fontFamily",
        "fontStyle",
        "lineHeight",
        "letterSpacing",
        "characters",
        "effects",
    ];

    pub(crate) fn matrix_main<'a>(cmd: &str, doc: &'a Document, id: NodeId) -> Result<&'a Node> {
        let main = doc.node(id).ok_or_else(|| bad(cmd, format!("no node {id}")))?;
        if !matches!(main.kind, NodeKind::Component { .. }) || inside_instance(doc, id) {
            return Err(bad(cmd, "id must be a main component"));
        }
        if doc.is_remote(id) {
            return Err(EngineError::Disabled(cmd.into(), crate::library::READ_ONLY.into()));
        }
        Ok(main)
    }

    pub(crate) fn matrix_set_axis(s: &mut Session, p: &Value) -> Result<Value> {
        let cmd = "component.matrixSetAxis";
        let id = req_id(cmd, p, "id")?;
        let name =
            p["name"].as_str().map(str::trim).filter(|n| !n.is_empty() && n.len() <= 80).ok_or_else(|| bad(cmd, "name must have 1–80 characters"))?;
        let raw = p["options"].as_array().ok_or_else(|| bad(cmd, "options must be an array"))?;
        let mut options = Vec::<String>::new();
        for value in raw {
            let option = value
                .as_str()
                .map(str::trim)
                .filter(|v| !v.is_empty() && v.len() <= 80)
                .ok_or_else(|| bad(cmd, "each option must have 1–80 characters"))?;
            if options.iter().any(|v| v == option) {
                return Err(bad(cmd, "options must be unique"));
            }
            options.push(option.to_string());
        }
        if options.is_empty() || options.len() > 32 {
            return Err(bad(cmd, "an axis needs 1–32 options"));
        }
        let default = p["default"].as_str().unwrap_or(&options[0]);
        if !options.iter().any(|v| v == default) {
            return Err(bad(cmd, "default must be one of the options"));
        }
        let doc = s.doc()?;
        let main = matrix_main(cmd, doc, id)?;
        let old = main.component_matrix.axes.iter().find(|a| a.name == name);
        if old.is_none()
            && (matches!(&main.kind, NodeKind::Component { props, .. } if props.contains_key(name))
                || variant_set(doc, id).is_some_and(|set| variant_options(doc, set).iter().any(|(key, _)| key == name)))
        {
            return Err(bad(cmd, "axis name is already used by a component property"));
        }
        if main.component_matrix.axes.len() >= 12 && old.is_none() {
            return Err(bad(cmd, "a component can have at most 12 axes"));
        }
        let old_options = old.map(|a| a.options.clone()).unwrap_or_default();
        let doc = s.doc_mut()?;
        let matrix = &mut doc.get_mut(id)?.component_matrix;
        if let Some(axis) = matrix.axes.iter_mut().find(|a| a.name == name) {
            axis.options = options.clone();
            axis.default = default.to_string();
        } else {
            matrix.axes.push(ComponentAxis { name: name.to_string(), options: options.clone(), default: default.to_string() });
        }
        if !old_options.is_empty() {
            for field in &mut matrix.fields {
                field.cells.retain(|cell| cell.when.get(name).is_none_or(|v| options.contains(v)));
            }
        }
        let instances: Vec<NodeId> = doc
            .nodes()
            .filter_map(|n| match &n.kind {
                NodeKind::Instance { component, props }
                    if *component == id && props.get(name).is_some_and(|v| v.as_str().is_none_or(|v| !options.iter().any(|o| o == v))) =>
                {
                    Some(n.id)
                }
                _ => None,
            })
            .collect();
        for instance in instances {
            if let NodeKind::Instance { props, .. } = &mut doc.get_mut(instance)?.kind {
                props.remove(name);
            }
        }
        Ok(json!({"id": id, "name": name, "options": options, "default": default}))
    }

    pub(crate) fn matrix_delete_axis(s: &mut Session, p: &Value) -> Result<Value> {
        let cmd = "component.matrixDeleteAxis";
        let id = req_id(cmd, p, "id")?;
        let name = p["name"].as_str().ok_or_else(|| bad(cmd, "name required"))?;
        matrix_main(cmd, s.doc()?, id)?;
        let doc = s.doc_mut()?;
        let matrix = &mut doc.get_mut(id)?.component_matrix;
        let before = matrix.axes.len();
        matrix.axes.retain(|axis| axis.name != name);
        if matrix.axes.len() == before {
            return Err(bad(cmd, "axis not found"));
        }
        for field in &mut matrix.fields {
            field.cells.retain(|cell| !cell.when.contains_key(name));
        }
        matrix.fields.retain(|field| !field.cells.is_empty());
        let instances: Vec<NodeId> = doc
            .nodes()
            .filter_map(|n| match &n.kind {
                NodeKind::Instance { component, props } if *component == id && props.contains_key(name) => Some(n.id),
                _ => None,
            })
            .collect();
        for instance in instances {
            if let NodeKind::Instance { props, .. } = &mut doc.get_mut(instance)?.kind {
                props.remove(name);
            }
        }
        Ok(json!({"id": id, "deleted": name}))
    }

    pub(crate) fn matrix_rename_axis(s: &mut Session, p: &Value) -> Result<Value> {
        let cmd = "component.matrixRenameAxis";
        let id = req_id(cmd, p, "id")?;
        let name = p["name"].as_str().ok_or_else(|| bad(cmd, "name required"))?;
        let new_name = p["newName"]
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.len() <= 80)
            .ok_or_else(|| bad(cmd, "newName must have 1–80 characters"))?;
        let doc = s.doc()?;
        let main = matrix_main(cmd, doc, id)?;
        if !main.component_matrix.axes.iter().any(|axis| axis.name == name) {
            return Err(bad(cmd, "axis not found"));
        }
        if name == new_name {
            return Ok(json!({"id": id, "name": name}));
        }
        if main.component_matrix.axes.iter().any(|axis| axis.name == new_name)
            || matches!(&main.kind, NodeKind::Component { props, .. } if props.contains_key(new_name))
            || variant_set(doc, id).is_some_and(|set| variant_options(doc, set).iter().any(|(key, _)| key == new_name))
        {
            return Err(bad(cmd, "new name is already used by a component property"));
        }
        let doc = s.doc_mut()?;
        let matrix = &mut doc.get_mut(id)?.component_matrix;
        if let Some(axis) = matrix.axes.iter_mut().find(|axis| axis.name == name) {
            axis.name = new_name.to_string();
        }
        for field in &mut matrix.fields {
            for cell in &mut field.cells {
                if let Some(value) = cell.when.remove(name) {
                    cell.when.insert(new_name.to_string(), value);
                }
            }
        }
        let instances: Vec<NodeId> = doc
            .nodes()
            .filter_map(|node| match &node.kind {
                NodeKind::Instance { component, props } if *component == id && props.contains_key(name) => Some(node.id),
                _ => None,
            })
            .collect();
        for instance in instances {
            if let NodeKind::Instance { props, .. } = &mut doc.get_mut(instance)?.kind
                && let Some(value) = props.remove(name)
            {
                props.insert(new_name.to_string(), value);
            }
        }
        Ok(json!({"id": id, "name": new_name}))
    }

    pub(crate) fn matrix_rename_option(s: &mut Session, p: &Value) -> Result<Value> {
        let cmd = "component.matrixRenameOption";
        let id = req_id(cmd, p, "id")?;
        let axis_name = p["axis"].as_str().ok_or_else(|| bad(cmd, "axis required"))?;
        let option = p["option"].as_str().ok_or_else(|| bad(cmd, "option required"))?;
        let new_name = p["newName"]
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.len() <= 80)
            .ok_or_else(|| bad(cmd, "newName must have 1–80 characters"))?;
        let doc = s.doc()?;
        let main = matrix_main(cmd, doc, id)?;
        let axis = main.component_matrix.axes.iter().find(|axis| axis.name == axis_name).ok_or_else(|| bad(cmd, "axis not found"))?;
        if !axis.options.iter().any(|value| value == option) {
            return Err(bad(cmd, "option not found"));
        }
        if option == new_name {
            return Ok(json!({"id": id, "axis": axis_name, "option": option}));
        }
        if axis.options.iter().any(|value| value == new_name) {
            return Err(bad(cmd, "option name is already used"));
        }
        let doc = s.doc_mut()?;
        let matrix = &mut doc.get_mut(id)?.component_matrix;
        if let Some(axis) = matrix.axes.iter_mut().find(|axis| axis.name == axis_name) {
            for value in &mut axis.options {
                if value == option {
                    *value = new_name.to_string();
                }
            }
            if axis.default == option {
                axis.default = new_name.to_string();
            }
        }
        for field in &mut matrix.fields {
            for cell in &mut field.cells {
                if cell.when.get(axis_name).is_some_and(|value| value == option) {
                    cell.when.insert(axis_name.to_string(), new_name.to_string());
                }
            }
        }
        let instances: Vec<NodeId> = doc
            .nodes()
            .filter_map(|node| match &node.kind {
                NodeKind::Instance { component, props }
                    if *component == id && props.get(axis_name).is_some_and(|value| value.as_str() == Some(option)) =>
                {
                    Some(node.id)
                }
                _ => None,
            })
            .collect();
        for instance in instances {
            if let NodeKind::Instance { props, .. } = &mut doc.get_mut(instance)?.kind {
                props.insert(axis_name.to_string(), json!(new_name));
            }
        }
        Ok(json!({"id": id, "axis": axis_name, "option": new_name}))
    }

    pub(crate) fn matrix_when(cmd: &str, p: &Value, matrix: &ComponentMatrix) -> Result<BTreeMap<String, String>> {
        let object = p["when"].as_object().ok_or_else(|| bad(cmd, "when must be an object (use {} for a fallback)"))?;
        let mut when = BTreeMap::new();
        for (name, value) in object {
            let axis = matrix.axes.iter().find(|axis| &axis.name == name).ok_or_else(|| bad(cmd, format!("unknown axis `{name}`")))?;
            let option = value.as_str().ok_or_else(|| bad(cmd, format!("`{name}` must be a string")))?;
            if !axis.options.iter().any(|v| v == option) {
                return Err(bad(cmd, format!("unknown option `{option}` for `{name}`")));
            }
            when.insert(name.clone(), option.to_string());
        }
        Ok(when)
    }

    pub(crate) fn matrix_target(cmd: &str, doc: &Document, p: &Value) -> Result<(NodeId, NodeId, String)> {
        let id = req_id(cmd, p, "id")?;
        matrix_main(cmd, doc, id)?;
        let layer = req_id(cmd, p, "layer")?;
        if layer != id && !doc.is_ancestor(id, layer) {
            return Err(bad(cmd, "layer must belong to the main component"));
        }
        if inside_instance(doc, layer) {
            return Err(bad(cmd, "bind a field on the nested instance itself, not on its expanded layers"));
        }
        let field = p["field"]
            .as_str()
            .filter(|f| MATRIX_FIELDS.contains(f))
            .ok_or_else(|| bad(cmd, format!("field must be one of {}", MATRIX_FIELDS.join(", "))))?;
        Ok((id, layer, field.to_string()))
    }

    pub(crate) fn matrix_set_cell(s: &mut Session, p: &Value) -> Result<Value> {
        let cmd = "component.matrixSetCell";
        let doc = s.doc()?;
        let (id, layer, field) = matrix_target(cmd, doc, p)?;
        let matrix = &doc.node(id).ok_or_else(|| bad(cmd, "main missing"))?.component_matrix;
        let when = matrix_when(cmd, p, matrix)?;
        let value = p.get("value").cloned().ok_or_else(|| bad(cmd, "value required"))?;
        let other = matrix.fields.iter().find(|f| f.layer == layer && f.field == field);
        let total_cells: usize = matrix.fields.iter().map(|field| field.cells.len()).sum();
        if total_cells >= 512 && !other.is_some_and(|field| field.cells.iter().any(|cell| cell.when == when)) {
            return Err(bad(cmd, "a component can have at most 512 cells"));
        }
        if other.is_none() && matrix.fields.len() >= 128 {
            return Err(bad(cmd, "a component can bind at most 128 fields"));
        }
        if other.is_some_and(|f| f.cells.len() >= 256 && !f.cells.iter().any(|c| c.when == when)) {
            return Err(bad(cmd, "a field can have at most 256 cells"));
        }
        if let Some(other) = other {
            for cell in &other.cells {
                if cell.when == when {
                    continue;
                }
                let overlaps = cell.when.iter().all(|(k, v)| when.get(k).is_none_or(|x| x == v))
                    && when.iter().all(|(k, v)| cell.when.get(k).is_none_or(|x| x == v));
                if overlaps && cell.when.len() == when.len() {
                    return Err(bad(cmd, "equally specific cells overlap; add an axis to one condition"));
                }
            }
        }
        // Validate the value through the same parser used by node.set without changing the document.
        let mut probe = doc.clone();
        crate::cmd::props::apply(cmd, &mut probe, layer, &serde_json::Map::from_iter([(field.clone(), value.clone())]))?;
        let matrix = &mut s.doc_mut()?.get_mut(id)?.component_matrix;
        let target = if let Some(target) = matrix.fields.iter_mut().find(|f| f.layer == layer && f.field == field) {
            target
        } else {
            matrix.fields.push(ComponentField { layer, field: field.clone(), cells: Vec::new() });
            matrix.fields.last_mut().ok_or_else(|| bad(cmd, "could not create field"))?
        };
        if let Some(cell) = target.cells.iter_mut().find(|c| c.when == when) {
            cell.value = value.clone();
        } else {
            target.cells.push(ComponentCell { when: when.clone(), value: value.clone() });
        }
        Ok(json!({"id": id, "layer": layer, "field": field, "when": when, "value": value}))
    }

    pub(crate) fn matrix_delete_cell(s: &mut Session, p: &Value) -> Result<Value> {
        let cmd = "component.matrixDeleteCell";
        let (id, layer, field) = matrix_target(cmd, s.doc()?, p)?;
        let when = matrix_when(cmd, p, &s.doc()?.node(id).ok_or_else(|| bad(cmd, "main missing"))?.component_matrix)?;
        let matrix = &mut s.doc_mut()?.get_mut(id)?.component_matrix;
        let Some(target) = matrix.fields.iter_mut().find(|f| f.layer == layer && f.field == field) else {
            return Err(bad(cmd, "field not bound"));
        };
        let before = target.cells.len();
        target.cells.retain(|c| c.when != when);
        if target.cells.len() == before {
            return Err(bad(cmd, "cell not found"));
        }
        matrix.fields.retain(|f| !f.cells.is_empty());
        Ok(json!({"id": id, "layer": layer, "field": field, "when": when}))
    }

    pub(crate) fn matrix_describe(s: &mut Session, p: &Value) -> Result<Value> {
        let cmd = "component.matrixDescribe";
        let id = req_id(cmd, p, "id")?;
        let doc = s.doc()?;
        let node = doc.node(id).ok_or_else(|| bad(cmd, "node not found"))?;
        let (main, selected) = match &node.kind {
            NodeKind::Instance { component, props } => (*component, props.clone()),
            NodeKind::Component { .. } => (id, BTreeMap::new()),
            _ => return Err(bad(cmd, "id must be a component or instance")),
        };
        let matrix = &doc.node(main).ok_or_else(|| bad(cmd, "main missing"))?.component_matrix;
        let choices = matrix.choices(&selected);
        let resolved: Vec<Value> = matrix
            .fields
            .iter()
            .filter_map(|f| matrix.resolve(f, &choices).map(|value| json!({"layer": f.layer, "field": f.field, "value": value})))
            .collect();
        Ok(json!({"main": main, "axes": matrix.axes, "fields": matrix.fields, "choices": choices, "resolved": resolved}))
    }
}
