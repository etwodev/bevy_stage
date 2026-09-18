//! Reading a level's structure from the glTF asset, before anything is spawned.
//!
//! Placement needs one number: where a named anchor sits relative to its own
//! stage root. Getting that by spawning the stage and reading the result back
//! would mean the stage exists in the wrong place for a frame or two and has to
//! be hidden until it settles. Reading it from the asset instead means a stage
//! is correctly positioned on the very first frame it is visible.

use bevy::gltf::{Gltf, GltfNode};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;

use crate::tag::parse::{parse_extras, parse_node_name};
use crate::tag::value::normalize_key;

/// The tag that marks a named connection point.
pub const ANCHOR_TAG: &str = "anchor";

/// Find a named anchor's transform relative to its stage root.
///
/// An anchor matches when the node is tagged `anchor` with an `id` property
/// equal to `anchor_id`, or — for artists who would rather just name the empty
/// — when the node's own name matches `anchor_id`.
pub fn find_anchor(
    gltf: &Gltf,
    nodes: &Assets<GltfNode>,
    anchor_id: &str,
    discriminator: &str,
) -> Option<GlobalTransform> {
    let wanted = normalize_key(anchor_id);

    for root in scene_roots(gltf, nodes) {
        if let Some(found) = search(nodes, root, GlobalTransform::IDENTITY, &wanted, discriminator)
        {
            return Some(found);
        }
    }
    None
}

/// Every anchor in the level, by id, relative to the stage root.
pub fn find_all_anchors(
    gltf: &Gltf,
    nodes: &Assets<GltfNode>,
    discriminator: &str,
) -> Vec<(String, GlobalTransform)> {
    let mut found = Vec::new();
    for root in scene_roots(gltf, nodes) {
        collect(nodes, root, GlobalTransform::IDENTITY, discriminator, &mut found);
    }
    found
}

/// The nodes that are not a child of any other node.
///
/// `Gltf::nodes` is a flat list, so the roots have to be found by elimination:
/// anything claimed as somebody's child is not a root.
fn scene_roots<'a>(gltf: &'a Gltf, nodes: &Assets<GltfNode>) -> Vec<&'a Handle<GltfNode>> {
    let mut claimed: HashSet<AssetId<GltfNode>> = HashSet::new();
    for handle in &gltf.nodes {
        if let Some(node) = nodes.get(handle) {
            for child in &node.children {
                claimed.insert(child.id());
            }
        }
    }
    gltf.nodes
        .iter()
        .filter(|handle| !claimed.contains(&handle.id()))
        .collect()
}

/// The anchor id a node declares, if any.
fn anchor_id_of(node: &GltfNode, discriminator: &str) -> Option<String> {
    let (base_name, name_params) = parse_node_name(&node.name);

    if let Some(extras) = &node.extras
        && let Ok((_, tags)) = parse_extras(&extras.value, discriminator)
    {
        for tag in &tags {
            if normalize_key(&tag.key) == normalize_key(ANCHOR_TAG) {
                // An explicit `id` wins; otherwise the node's name names it.
                return Some(
                    tag.params
                        .get_str("id")
                        .map(str::to_string)
                        .unwrap_or_else(|| base_name.clone()),
                );
            }
        }
    }

    // A node named like the anchor, tagged by name alone.
    if normalize_key(&base_name) == normalize_key(ANCHOR_TAG) {
        return name_params.get_str("id").map(str::to_string);
    }
    None
}

fn search(
    nodes: &Assets<GltfNode>,
    handle: &Handle<GltfNode>,
    parent: GlobalTransform,
    wanted: &str,
    discriminator: &str,
) -> Option<GlobalTransform> {
    let node = nodes.get(handle)?;
    let here = parent * GlobalTransform::from(node.transform);

    if let Some(id) = anchor_id_of(node, discriminator)
        && normalize_key(&id) == wanted
    {
        return Some(here);
    }
    // Allow naming the empty after the anchor directly.
    let (base_name, _) = parse_node_name(&node.name);
    if normalize_key(&base_name) == wanted {
        return Some(here);
    }

    for child in &node.children {
        if let Some(found) = search(nodes, child, here, wanted, discriminator) {
            return Some(found);
        }
    }
    None
}

fn collect(
    nodes: &Assets<GltfNode>,
    handle: &Handle<GltfNode>,
    parent: GlobalTransform,
    discriminator: &str,
    out: &mut Vec<(String, GlobalTransform)>,
) {
    let Some(node) = nodes.get(handle) else {
        return;
    };
    let here = parent * GlobalTransform::from(node.transform);

    if let Some(id) = anchor_id_of(node, discriminator) {
        out.push((id, here));
    }
    for child in &node.children {
        collect(nodes, child, here, discriminator, out);
    }
}
