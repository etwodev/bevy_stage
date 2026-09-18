//! The plugin entry point.

use bevy::gltf::extensions::GltfExtensionHandlers;
use bevy::prelude::*;
use serde::de::DeserializeOwned;

use crate::loader::StageGltfHandler;
use crate::stage::anchor::{AnchorAlignment, StageAnchor};
use crate::persist::{
    capture_unloading_stages, restore_spawned_stages, PersistedComponents, StageDeltas,
    StageNodeId,
};
use crate::stream::{cull_inline_sectors, stream_external_sectors, StageSector};
use crate::view::{cull_distant_stages, wire_lod_groups, LodConfig, StageViewConfig};
use crate::stage::transition::{
    attach_portal_state, detect_portal_crossings, link_return_portals, preload_through_portals,
    unload_distant_stages, StagePortal,
};
use crate::stage::manager::{
    begin_stage_loads, finish_stage_spawns, process_stage_unloads, spawn_loaded_stages, StageId,
    StageManager, StageRoot, StageStatus,
};
use crate::tag::{
    component::{StageTag, StageTags, TagSource},
    parse::DEFAULT_DISCRIMINATOR,
    resolve::{enqueue_tagged_nodes, resolve_queued_tags, StageTagQueue, UnknownTags},
    value::{TagParams, TagValue},
    StageTagRegistry,
};

/// How the plugin reads and resolves tags.
#[derive(Resource, Clone, Debug)]
pub struct StageTagConfig {
    /// The `extras` property naming a node's tags.
    pub discriminator: String,
    /// How many nodes to resolve per frame, or `0` for no limit.
    ///
    /// Defaults to no limit: for a normal level the resolution pass is cheap,
    /// and a tag appearing a frame late is more surprising than a short hitch.
    /// Raise the limit off zero when streaming large levels.
    pub resolve_budget: usize,
}

impl Default for StageTagConfig {
    fn default() -> Self {
        Self {
            discriminator: DEFAULT_DISCRIMINATOR.to_string(),
            resolve_budget: 0,
        }
    }
}

/// Ordering for the plugin's work within a frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum StageSystems {
    /// Stages are loaded, placed and unloaded.
    ///
    /// Runs in `PreUpdate` so that a stage's scene is handed to Bevy before the
    /// `SpawnScene` schedule runs later in the same frame, rather than waiting
    /// for the next one.
    Manage,
    /// Newly spawned tagged nodes are collected.
    Collect,
    /// Queued nodes are resolved against the registry.
    Resolve,
    /// Stages whose scenes have finished spawning are promoted to ready.
    Settle,
    /// LOD groups are wired as levels spawn.
    View,
    /// Remembered state is put back on levels that have just spawned.
    Restore,
    /// Portals preload, hand over and unload.
    ///
    /// Runs in `PostUpdate` after transform propagation, because every decision
    /// here is about where things are in the world this frame.
    Stream,
}

/// Loads glTF levels and applies artist-authored tags.
#[derive(Default)]
pub struct StagePlugin {
    /// Tag reading and resolution settings.
    pub config: StageTagConfig,
}

impl Plugin for StagePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<StageTags>()
            .register_type::<StageTag>()
            .register_type::<TagSource>()
            .register_type::<TagParams>()
            .register_type::<TagValue>()
            .insert_resource(self.config.clone())
            .register_type::<StageRoot>()
            .register_type::<StageId>()
            .register_type::<StageStatus>()
            .register_type::<StageAnchor>()
            .register_type::<AnchorAlignment>()
            .register_type::<StagePortal>()
            .register_type::<StageSector>()
            .init_resource::<StageTagRegistry>()
            .init_resource::<StageTagQueue>()
            .init_resource::<UnknownTags>()
            .init_resource::<StageManager>()
            .init_resource::<LodConfig>()
            .init_resource::<StageViewConfig>()
            .init_resource::<PersistedComponents>()
            .init_resource::<StageDeltas>()
            .register_type::<StageNodeId>()
            .register_type::<LodConfig>()
            .register_type::<StageViewConfig>()
            .configure_sets(PreUpdate, StageSystems::Manage)
            .configure_sets(
                PostUpdate,
                StageSystems::Stream.after(bevy::transform::TransformSystems::Propagate),
            )
            .configure_sets(
                Update,
                (
                    StageSystems::Collect,
                    StageSystems::Resolve,
                    StageSystems::Settle,
                    StageSystems::View,
                    StageSystems::Restore,
                )
                    .chain(),
            )
            .add_systems(
                PreUpdate,
                (
                    // Capture first: unloading is the last moment the state
                    // still exists to be read.
                    capture_unloading_stages,
                    process_stage_unloads,
                    begin_stage_loads,
                    spawn_loaded_stages,
                )
                    .chain()
                    .in_set(StageSystems::Manage),
            )
            .add_systems(
                Update,
                (
                    enqueue_tagged_nodes.in_set(StageSystems::Collect),
                    resolve_queued_tags.in_set(StageSystems::Resolve),
                    finish_stage_spawns.in_set(StageSystems::Settle),
                    wire_lod_groups.in_set(StageSystems::View),
                    // After tags have been applied, so remembered values win
                    // over whatever the level file says.
                    restore_spawned_stages.in_set(StageSystems::Restore),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    attach_portal_state,
                    link_return_portals,
                    preload_through_portals,
                    detect_portal_crossings,
                    unload_distant_stages,
                    stream_external_sectors,
                    cull_inline_sectors,
                    cull_distant_stages,
                )
                    .chain()
                    .in_set(StageSystems::Stream),
            );

        // The plugin claims `anchor` for itself, on the same registry games use
        // for their own tags. Nothing about it is privileged.
        {
            let mut registry = app.world_mut().resource_mut::<StageTagRegistry>();
            registry.register::<StageAnchor>(crate::stage::scan::ANCHOR_TAG);
            registry.register::<StagePortal>("portal");
            registry.register::<StageSector>("sector");
        }

        // Parse metadata as the glTF loads rather than after it spawns. The
        // resource is shared with the loader through an `Arc`, and the loader
        // is only built in `GltfPlugin::finish`, so pushing here is seen by it
        // regardless of plugin order.
        app.init_resource::<GltfExtensionHandlers>();
        let known_names = app.world().resource::<StageTagRegistry>().known_names();
        let handlers = app.world().resource::<GltfExtensionHandlers>().0.clone();
        handlers.write_blocking().push(Box::new(StageGltfHandler::new(
            self.config.discriminator.as_str(),
            known_names,
        )));
    }
}

/// Registering tags on the [`App`].
pub trait StageAppExt {
    /// Register a tag that inserts `T`, built from the tag's parameters.
    ///
    /// The component lands on the entity the glTF loader created for the node,
    /// so the node's transform, name and parenting come along with it.
    fn register_stage_tag<T>(&mut self, key: &str) -> &mut Self
    where
        T: Component + DeserializeOwned;

    /// Register a tag that carries no component of its own, for games that
    /// only want the [`StageTagFound`](crate::tag::resolve::StageTagFound) event.
    fn register_stage_marker(&mut self, key: &str) -> &mut Self;

    /// Remember `T` across unload and reload.
    ///
    /// Only nodes that carry a tag have a stable identity, so only those can be
    /// remembered. Registering the type for reflection is required too, and is
    /// done here for you.
    fn persist_stage_component<T>(&mut self) -> &mut Self
    where
        T: Component + Reflect + bevy::reflect::TypePath + bevy::reflect::GetTypeRegistration;
}

impl StageAppExt for App {
    fn register_stage_tag<T>(&mut self, key: &str) -> &mut Self
    where
        T: Component + DeserializeOwned,
    {
        self.world_mut()
            .resource_mut::<StageTagRegistry>()
            .register::<T>(key);
        self
    }

    fn register_stage_marker(&mut self, key: &str) -> &mut Self {
        self.world_mut()
            .resource_mut::<StageTagRegistry>()
            .register_marker(key);
        self
    }

    fn persist_stage_component<T>(&mut self) -> &mut Self
    where
        T: Component + Reflect + bevy::reflect::TypePath + bevy::reflect::GetTypeRegistration,
    {
        self.register_type::<T>();
        self.world_mut()
            .resource_mut::<PersistedComponents>()
            .register::<T>();
        self
    }
}
