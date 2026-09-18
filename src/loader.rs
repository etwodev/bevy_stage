//! The load-time hook that bakes parsed metadata into the glTF asset.
//!
//! Bevy 0.19's [`GltfExtensionHandler`] runs while the glTF is being loaded,
//! with mutable access to the entity being built inside the asset's world. That
//! lets the plugin parse each node's `extras` **once per asset load** rather
//! than once per spawn, and sidesteps a sharp edge in the alternative: Bevy
//! inserts [`GltfExtras`] on both node entities *and*
//! their primitive children, so a post-spawn query would see each tagged mesh
//! twice and double-spawn whatever the tag maps to.

use std::sync::Arc;

use bevy::gltf::extensions::{ErasedGltfExtensionHandler, GltfExtensionHandler};
use bevy::prelude::*;
use bevy::{asset::LoadContext, ecs::world::EntityWorldMut};
use gltf::Node;

use crate::tag::parse::build_stage_tags;
use crate::persist::StageNodeId;
use crate::tag::registry::KnownTagNames;
use crate::tag::value::normalize_key;

/// Parses artist-authored metadata as the glTF loads and stores the result on
/// the node's entity inside the loaded asset.
#[derive(Clone)]
pub struct StageGltfHandler {
    /// The `extras` property that names a node's tags.
    discriminator: Arc<str>,
    /// The names the game has registered, used to decide whether a node's name
    /// alone is worth recording.
    known_names: KnownTagNames,
}

impl StageGltfHandler {
    /// Build a handler reading tags from the given discriminator property.
    pub fn new(discriminator: impl Into<Arc<str>>, known_names: KnownTagNames) -> Self {
        Self {
            discriminator: discriminator.into(),
            known_names,
        }
    }

    /// Whether this node's name matches a registered tag.
    fn name_is_registered(&self, base_name: &str) -> bool {
        self.known_names
            .read()
            .map(|names| names.contains(&normalize_key(base_name)))
            .unwrap_or(false)
    }
}

impl GltfExtensionHandler for StageGltfHandler {
    fn dyn_clone(&self) -> Box<dyn ErasedGltfExtensionHandler> {
        Box::new(self.clone())
    }

    fn on_gltf_node(
        &mut self,
        load_context: &mut LoadContext<'_>,
        gltf_node: &Node,
        entity: &mut EntityWorldMut,
    ) {
        // Unnamed nodes still get a synthetic name from Bevy, but for tagging
        // purposes an unnamed node simply has no name channel.
        let name = gltf_node.name().unwrap_or_default();
        let extras = gltf_node.extras().as_ref().map(|raw| raw.get());

        let (tags, error) = build_stage_tags(name, extras, &self.discriminator);

        if let Some(error) = error {
            // Name the asset and the node, because "invalid JSON" on its own
            // sends an artist hunting through a whole level file.
            warn!(
                "bevy_stage: ignoring metadata on node `{}` (index {}) of `{}`: {}",
                name,
                gltf_node.index(),
                load_context.path(),
                error,
            );
        }

        // Most nodes in a level are plain geometry. Recording metadata for them
        // would cost memory and resolution time for nothing, so a node is only
        // worth baking if it carries `extras` or its name matches a tag the
        // game registered.
        if !tags.is_empty() || self.name_is_registered(&tags.base_name) {
            entity.insert(tags);
            // The glTF node index is a stable name for this node across
            // reloads — sturdier than the node's own name, which artists
            // rename freely. Only baked for nodes that carry metadata, so a
            // level of plain geometry costs nothing.
            entity.insert(StageNodeId(gltf_node.index() as u32));
        }
    }
}
