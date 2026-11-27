#![allow(clippy::unwrap_used)]
// A test that can't unwrap can't assert: the workspace lints let tests do it (AGENTS.md › Never crash).

use figcraft_plugin_figma::ops::{
    CreateKind, CreateSpec, OpError, add_page, append_child, create, get, group, insert_child, move_to_page, remove, reorder, select, set, ungroup,
};
use serde_json::{Map, json};

#[test]
fn creating_a_layer_names_the_engine_kind_and_leaves_the_selection_alone() {
    let call = create(
        CreateKind::Frame,
        &CreateSpec {
            parent: Some(5),
            x: Some(10.0),
            y: Some(20.0),
            width: Some(100.0),
            height: Some(50.0),
            name: Some("Card".into()),
            characters: None,
        },
    )
    .unwrap();
    assert_eq!(call.command, "node.create");
    assert_eq!(
        call.params,
        json!({"type": "frame", "select": false, "parent": 5, "x": 10.0, "y": 20.0, "width": 100.0, "height": 50.0, "name": "Card"})
    );
}

#[test]
fn a_text_layer_carries_its_first_content() {
    let call = create(CreateKind::Text, &CreateSpec { characters: Some("Hello".into()), ..CreateSpec::default() }).unwrap();
    assert_eq!(call.params, json!({"type": "text", "select": false, "characters": "Hello"}));
    assert_eq!(CreateKind::Component.create_type(), "component");
    assert_eq!(CreateKind::Section.create_type(), "section");
}

#[test]
fn one_property_batch_is_one_call() {
    let mut props = Map::new();
    props.insert("x".into(), json!(1.0));
    props.insert("fills".into(), json!([{"type": "SOLID", "color": {"r": 1.0, "g": 0.0, "b": 0.0}}]));
    let call = set(7, props).unwrap();
    assert_eq!(call.command, "node.set");
    assert_eq!(call.params["id"], 7);
    assert!(call.params["props"].get("fills").is_some());
}

#[test]
fn children_move_with_a_parent_and_an_index() {
    let call = append_child(1, 2, None).unwrap();
    assert_eq!(call.command, "node.reparent");
    assert_eq!(call.params, json!({"ids": [2], "parent": 1}));
    let call = insert_child(1, 2, 0).unwrap();
    assert_eq!(call.params, json!({"ids": [2], "parent": 1, "index": 0}));
    let call = reorder(2, 3).unwrap();
    assert_eq!(call.command, "node.reorder");
    assert_eq!(call.params, json!({"id": 2, "index": 3}));
}

#[test]
fn removals_groups_and_selections_use_the_settled_commands() {
    assert_eq!(remove(3).unwrap().command, "edit.delete");
    assert_eq!(remove(3).unwrap().params, json!({"ids": [3]}));
    assert_eq!(group(&[3, 4]).unwrap().params, json!({"ids": [3, 4]}));
    assert_eq!(ungroup(5).unwrap().command, "object.ungroup");
    assert_eq!(select(&[3, 4]).unwrap().command, "select.set");
    assert_eq!(select(&[]).unwrap().params, json!({"ids": []}));
    assert_eq!(add_page(Some("Icons")).params, json!({"name": "Icons"}));
    assert_eq!(add_page(None).params, json!({}));
    assert_eq!(move_to_page(&[3], 9).unwrap().command, "object.moveToPage");
    assert_eq!(move_to_page(&[3], 9).unwrap().params, json!({"ids": [3], "page": 9}));
}

#[test]
fn reads_narrow_to_the_fields_the_proxy_needs() {
    let call = get(9, Some(&["name", "type"])).unwrap();
    assert_eq!(call.command, "node.get");
    assert_eq!(call.params, json!({"id": 9, "fields": ["name", "type"]}));
    let call = get(9, None).unwrap();
    assert_eq!(call.params, json!({"id": 9}));
}

#[test]
fn the_document_root_is_not_a_layer() {
    assert_eq!(set(0, Map::new()), Err(OpError::ZeroId));
    assert_eq!(append_child(0, 1, None), Err(OpError::ZeroId));
    assert_eq!(select(&[0]), Err(OpError::ZeroId));
    assert_eq!(get(0, None), Err(OpError::ZeroId));
}
