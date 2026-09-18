//! Remembering what changed in a level, so it is still changed when you return.
//!
//! The hard part is not storing the data, it is **naming the thing the data
//! belongs to**. A door has to be recognisable as the same door after its level
//! has been unloaded and spawned afresh. The name used here is the glTF node
//! index, baked in at load time: unlike a node's name or its place in the
//! hierarchy, artists do not change it by renaming or re-parenting.
//!
//! Only nodes carrying metadata get an identity, so only those can be
//! persisted. That is the same set as "things the game knows about", which in
//! practice is the same set as "things worth remembering".

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::reflect::serde::{TypedReflectDeserializer, TypedReflectSerializer};
use bevy::reflect::TypeRegistryArc;
use serde::de::DeserializeSeed;

use crate::stage::manager::{StageId, StageManager, StageRoot, StageStatus};

/// A stable name for one node of a level, valid across unload and reload.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Reflect)]
#[reflect(Component)]
pub struct StageNodeId(pub u32);

/// Marks a stage whose remembered state has already been reapplied, so it is
/// not applied twice.
#[derive(Component)]
pub struct Restored;

/// The component types a game wants remembered.
#[derive(Resource, Default)]
pub struct PersistedComponents {
    types: Vec<String>,
}

impl PersistedComponents {
    /// Remember `T` on tagged nodes across unload and reload.
    pub fn register<T: Component + Reflect + bevy::reflect::TypePath>(&mut self) {
        let path = T::type_path().to_string();
        if !self.types.contains(&path) {
            self.types.push(path);
        }
    }

    /// The registered type paths.
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.types.iter()
    }
}

/// What changed in one level.
#[derive(Debug, Clone, Default)]
pub struct StageDelta {
    /// The nodes that still existed when the level was put away.
    ///
    /// Doubles as the record of what was destroyed: a node the level file
    /// contains but this set does not was removed during play, so it must not
    /// come back. That removes any need to hook entity despawns.
    present: HashSet<StageNodeId>,
    /// Serialized component values, by node.
    components: HashMap<StageNodeId, Vec<(String, String)>>,
}

impl StageDelta {
    /// Whether a node survived to the point the level was put away.
    pub fn is_present(&self, node: StageNodeId) -> bool {
        self.present.contains(&node)
    }

    /// How many nodes carry remembered values.
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// Whether nothing was remembered.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty() && self.present.is_empty()
    }
}

/// Everything remembered about levels not currently in the world.
///
/// Keyed by asset path, so a level restores whichever instance brings it back.
#[derive(Resource, Debug, Default)]
pub struct StageDeltas {
    levels: HashMap<String, StageDelta>,
}

impl StageDeltas {
    /// What is remembered about a level.
    pub fn get(&self, path: &str) -> Option<&StageDelta> {
        self.levels.get(path)
    }

    /// Forget a level, so it restores to how the artist authored it.
    pub fn clear(&mut self, path: &str) {
        self.levels.remove(path);
    }

    /// Forget everything — a new game.
    pub fn clear_all(&mut self) {
        self.levels.clear();
    }

    /// Whether anything at all is remembered.
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }
}

/// Snapshot levels that are about to be unloaded.
///
/// Runs before the unload itself, which is the only moment the state still
/// exists to be read.
pub fn capture_unloading_stages(world: &mut World) {
    let pending: Vec<StageId> = world.resource::<StageManager>().pending_unloads().to_vec();
    if pending.is_empty() {
        return;
    }

    let roots: Vec<(String, Entity)> = pending
        .iter()
        .filter_map(|id| {
            let entity = world.resource::<StageManager>().entity(*id)?;
            let path = world.get::<StageRoot>(entity)?.path.clone();
            Some((path, entity))
        })
        .collect();

    let registry = world.resource::<AppTypeRegistry>().clone();
    let type_paths: Vec<String> = world
        .resource::<PersistedComponents>()
        .iter()
        .cloned()
        .collect();

    let mut captured: Vec<(String, StageDelta)> = Vec::new();
    for (path, root) in roots {
        let nodes = collect_nodes(world, root);
        captured.push((path, snapshot(world, &registry, &type_paths, &nodes)));
    }

    let mut deltas = world.resource_mut::<StageDeltas>();
    for (path, delta) in captured {
        deltas.levels.insert(path, delta);
    }
}

/// Reapply remembered state to levels that have just spawned.
pub fn restore_spawned_stages(world: &mut World) {
    let pending: Vec<(Entity, String)> = {
        let mut query = world.query_filtered::<(Entity, &StageRoot), Without<Restored>>();
        query
            .iter(world)
            .filter(|(_, root)| matches!(root.status, StageStatus::Ready))
            .map(|(entity, root)| (entity, root.path.clone()))
            .collect()
    };
    if pending.is_empty() {
        return;
    }

    let registry = world.resource::<AppTypeRegistry>().clone();

    for (root, path) in pending {
        world.entity_mut(root).insert(Restored);

        let Some(delta) = world.resource::<StageDeltas>().levels.get(&path).cloned() else {
            continue;
        };
        let nodes = collect_nodes(world, root);
        apply(world, &registry, &delta, &nodes);
    }
}

/// Every node of a stage that carries a stable identity.
fn collect_nodes(world: &mut World, root: Entity) -> Vec<(Entity, StageNodeId)> {
    let mut found = Vec::new();
    let mut stack = vec![root];

    while let Some(entity) = stack.pop() {
        if let Some(id) = world.get::<StageNodeId>(entity) {
            found.push((entity, *id));
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    found
}

/// Read the registered components off each node.
fn snapshot(
    world: &World,
    registry: &TypeRegistryArc,
    type_paths: &[String],
    nodes: &[(Entity, StageNodeId)],
) -> StageDelta {
    let types = registry.read();
    let mut delta = StageDelta::default();

    for (entity, node) in nodes {
        delta.present.insert(*node);

        let entity_ref = world.entity(*entity);
        let mut values = Vec::new();

        for type_path in type_paths {
            let Some(registration) = types.get_with_type_path(type_path) else {
                continue;
            };
            let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                continue;
            };
            let Some(value) = reflect_component.reflect(entity_ref) else {
                continue;
            };
            match ron::ser::to_string(&TypedReflectSerializer::new(
                value.as_partial_reflect(),
                &types,
            )) {
                Ok(text) => values.push((type_path.clone(), text)),
                Err(error) => warn!("bevy_stage: could not remember `{type_path}`: {error}"),
            }
        }

        if !values.is_empty() {
            delta.components.insert(*node, values);
        }
    }
    delta
}

/// Put remembered values back, and keep destroyed things destroyed.
fn apply(
    world: &mut World,
    registry: &TypeRegistryArc,
    delta: &StageDelta,
    nodes: &[(Entity, StageNodeId)],
) {
    let types = registry.read();

    for (entity, node) in nodes {
        // A node the level file still contains, but which was gone by the time
        // the level was put away, was destroyed during play. Removing it again
        // is what makes a picked-up item stay picked up.
        if !delta.present.contains(node) {
            world.entity_mut(*entity).despawn();
            continue;
        }

        let Some(values) = delta.components.get(node) else {
            continue;
        };
        for (type_path, text) in values {
            let Some(registration) = types.get_with_type_path(type_path) else {
                continue;
            };
            let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                continue;
            };
            let Ok(mut deserializer) = ron::Deserializer::from_str(text) else {
                continue;
            };
            match TypedReflectDeserializer::new(registration, &types).deserialize(&mut deserializer)
            {
                Ok(value) => {
                    let mut entity_mut = world.entity_mut(*entity);
                    reflect_component.insert(&mut entity_mut, value.as_partial_reflect(), &types);
                }
                Err(error) => warn!("bevy_stage: could not restore `{type_path}`: {error}"),
            }
        }
    }
}
