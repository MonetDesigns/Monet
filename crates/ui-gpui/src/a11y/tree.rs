//! Build an AccessKit `TreeUpdate` from the control registry.
//!
//! The node mapping and `repair_tree_update` are copied from gpui-pre 0.3.12
//! `src/window/a11y.rs` (Apache-2.0), minus the stack machinery we do not need:
//! our region containers are known up front, so nodes are emitted as leaves
//! under their region. The state mapping follows the gpui-kit accessibility
//! guide (role + name + state + action, one stable node id per control).

use super::registry::{Control, Registry, ROOT_NODE_ID, States};
use accesskit::{Action, Node, NodeId, Rect, Role, Toggled, Tree, TreeId, TreeUpdate};
use crate::a11y::Region;
use gpui::FocusHandle;
use std::collections::HashSet;

/// Convert a GPUI bounds (logical points) into AccessKit physical pixels.
fn bounds(b: gpui::Bounds<gpui::Pixels>, scale: f32) -> Rect {
    let x0 = f64::from(f32::from(b.origin.x)) * f64::from(scale);
    let y0 = f64::from(f32::from(b.origin.y)) * f64::from(scale);
    Rect {
        x0,
        y0,
        x1: x0 + f64::from(f32::from(b.size.width)) * f64::from(scale),
        y1: y0 + f64::from(f32::from(b.size.height)) * f64::from(scale),
    }
}

fn add_states(node: &mut Node, states: &States) {
    if !states.label.is_empty() {
        node.set_label(states.label.clone());
    }
    if !states.description.is_empty() {
        node.set_description(states.description.clone());
    }
    if let Some(on) = states.toggled {
        node.set_toggled(if on { Toggled::True } else { Toggled::False });
    }
    if states.selected {
        node.set_selected(true);
    }
    if let Some(open) = states.expanded {
        node.set_expanded(open);
    }
    if let Some(value) = &states.value {
        node.set_value(value.clone());
    }
    if let Some(v) = states.numeric {
        node.set_numeric_value(v);
    }
    if let Some((min, max)) = states.range {
        node.set_min_numeric_value(min);
        node.set_max_numeric_value(max);
    }
    if states.disabled {
        node.set_disabled();
    }
}

fn control_node(control: &Control, scale: f32) -> Node {
    let mut node = Node::new(control.role);
    node.set_bounds(bounds(control.bounds, scale));
    add_states(&mut node, &control.states);
    for action in &control.actions {
        node.add_action(*action);
    }
    node
}

/// Build the whole window tree. `focused` is GPUI's current `FocusHandle`;
/// when it belongs to a registered control that node becomes the tree focus.
pub(crate) fn build_update(
    registry: &Registry,
    title: &str,
    focused: Option<&FocusHandle>,
    scale: f32,
) -> TreeUpdate {
    let mut nodes: Vec<(NodeId, Node)> = Vec::new();
    let mut root = Node::new(Role::Window);
    if !title.is_empty() {
        root.set_label(title.to_string());
    }

    let mut tree_focus = ROOT_NODE_ID;
    let mut seen: HashSet<NodeId> = HashSet::new();

    for region in Region::ALL {
        let controls: Vec<&Control> = registry.previous.iter().filter(|c| c.region == region).collect();
        if controls.is_empty() {
            continue;
        }
        let group_id = super::registry::node_id(&format!("region:{}", region.label()));
        if !seen.insert(group_id) {
            continue;
        }
        let mut group = Node::new(Role::Group);
        group.set_label(region.label().to_string());
        let mut children = Vec::new();
        for control in controls {
            if !seen.insert(control.node_id) {
                continue;
            }
            children.push(control.node_id);
            if focused.is_some_and(|f| control.focus.as_ref() == Some(f)) {
                tree_focus = control.node_id;
            }
            nodes.push((control.node_id, control_node(control, scale)));
        }
        group.set_children(children);
        root.push_child(group_id);
        nodes.push((group_id, group));
    }

    nodes.push((ROOT_NODE_ID, root));
    let update = TreeUpdate {
        nodes,
        tree: Some(Tree::new(ROOT_NODE_ID)),
        tree_id: TreeId::ROOT,
        focus: tree_focus,
    };
    repair_tree_update(update)
}

/// AccessKit panics on invalid updates; gpui-pre guards against the two
/// invariants that can break here (focus not in tree, dangling child ids).
fn repair_tree_update(mut update: TreeUpdate) -> TreeUpdate {
    let ids: HashSet<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
    if !ids.contains(&update.focus) {
        log::warn!("a11y: focused node {:?} is not in the tree; falling back to root", update.focus);
        update.focus = ROOT_NODE_ID;
    }
    for (id, node) in &mut update.nodes {
        let children = node.children();
        if children.iter().any(|child| !ids.contains(child)) {
            log::warn!("a11y: node {:?} references a child missing from the tree; stripping", id);
            node.set_children(children.iter().copied().filter(|child| ids.contains(child)).collect::<Vec<_>>());
        }
    }
    update
}

/// The actions a plain clickable control reports, mirroring gpui-kit's
/// "on_click is also the AccessKit Click action" rule.
pub(crate) const CLICKABLE: [Action; 2] = [Action::Click, Action::Focus];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a11y::registry::{Control, States};
    use gpui::{Bounds, point, px, size};

    fn control(key: &str, region: Region, role: Role, label: &str) -> Control {
        Control {
            node_id: super::super::registry::node_id(key),
            key: key.to_string(),
            region,
            role,
            bounds: Bounds { origin: point(px(1.0), px(2.0)), size: size(px(10.0), px(20.0)) },
            states: States { label: label.to_string(), ..States::default() },
            focus: None,
            command: None,
            actions: super::CLICKABLE.to_vec(),
        }
    }

    #[test]
    fn tree_has_window_root_region_group_and_named_control() {
        let mut reg = Registry::default();
        let c = control("tool:move", Region::Toolbar, Role::Button, "Move");
        reg.previous.push(c.clone());
        let update = build_update(&reg, "Monet", None, 2.0);
        let ids: Vec<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ROOT_NODE_ID));
        assert!(ids.contains(&c.node_id));
        let root = update.nodes.iter().find(|(id, _)| *id == ROOT_NODE_ID).unwrap().1.clone();
        assert_eq!(root.role(), Role::Window);
        assert_eq!(root.label(), Some("Monet"));
        assert!(!root.children().is_empty());
        let button = update.nodes.iter().find(|(id, _)| *id == c.node_id).unwrap().1.clone();
        assert_eq!(button.role(), Role::Button);
        assert_eq!(button.label(), Some("Move"));
        // bounds are physical pixels: logical (1,2) at scale 2 -> (2,4).
        assert_eq!(button.bounds().unwrap().x0, 2.0);
        assert_eq!(button.bounds().unwrap().y0, 4.0);
    }

    #[test]
    fn duplicate_keys_are_dropped_and_children_stay_valid() {
        let mut reg = Registry::default();
        let a = control("layer:1", Region::Layers, Role::TreeItem, "Frame");
        reg.previous.push(a.clone());
        reg.previous.push(a.clone());
        let update = build_update(&reg, "", None, 1.0);
        let ids: Vec<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids.iter().filter(|id| **id == a.node_id).count(), 1);
        for (_, node) in &update.nodes {
            assert!(node.children().iter().all(|child| ids.contains(child)));
        }
    }
}
