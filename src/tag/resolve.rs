//! Turning baked metadata into live components at spawn time.

use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::reflect::serde::TypedReflectDeserializer;
use bevy::reflect::TypeRegistry;
use serde::de::DeserializeSeed;

use super::component::{StageTags, TagSource};
use super::registry::{resolve_tags, StageTagRegistry};
use super::value::TagParams;

/// Emitted for every tag resolved on a node, after the tag's own component (if
/// any) has been inserted.
///
/// The declarative path — registering a component for a tag and reacting to
/// `Added<T>` — covers most games. This event is for the cases it does not:
/// replacing a placeholder empty with a prefab, or reacting to a tag that has
/// no component of its own.
#[derive(EntityEvent, Clone, Debug)]
pub struct StageTagFound {
    /// The entity the glTF loader created for the tagged node.
    pub entity: Entity,
    /// The tag's canonical name, as registered.
    pub key: String,
    /// The tag's parameters.
    pub params: TagParams,
    /// Which authoring channel the tag came from.
    pub source: TagSource,
}

/// Nodes whose baked metadata has not been resolved yet.
///
/// Resolution is queued rather than done inline so that a large level spawning
/// in one frame does not have to resolve every node in that same frame.
#[derive(Resource, Default)]
pub struct StageTagQueue {
    pending: Vec<Entity>,
}

impl StageTagQueue {
    /// Queue a node for resolution.
    pub fn push(&mut self, entity: Entity) {
        self.pending.push(entity);
    }

    /// Take up to `budget` nodes, or all of them when `budget` is zero.
    pub fn take(&mut self, budget: usize) -> Vec<Entity> {
        if budget == 0 || budget >= self.pending.len() {
            return core::mem::take(&mut self.pending);
        }
        self.pending.split_off(self.pending.len() - budget)
    }

    /// How many nodes are still waiting.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Tag names seen in levels that nothing is registered under.
///
/// Kept so each typo is reported once rather than once per node per spawn.
#[derive(Resource, Default)]
pub struct UnknownTags(HashSet<String>);

impl UnknownTags {
    /// Every unregistered tag name seen so far.
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.0.iter()
    }
}

/// Queue newly spawned nodes that carry baked metadata.
pub fn enqueue_tagged_nodes(
    nodes: Query<Entity, Added<StageTags>>,
    mut queue: ResMut<StageTagQueue>,
) {
    for entity in &nodes {
        queue.push(entity);
    }
}

/// Resolve queued nodes against the registry, inserting components and firing
/// [`StageTagFound`].
///
/// Exclusive because the reflection channel needs the type registry and
/// `&mut EntityWorldMut` together.
pub fn resolve_queued_tags(world: &mut World) {
    let budget = world.resource::<super::StageTagConfig>().resolve_budget;
    let discriminator = world
        .resource::<super::StageTagConfig>()
        .discriminator
        .clone();

    let batch = world.resource_mut::<StageTagQueue>().take(budget);
    if batch.is_empty() {
        return;
    }

    let type_registry = world.resource::<AppTypeRegistry>().clone();
    let mut events: Vec<StageTagFound> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();

    world.resource_scope(|world, tag_registry: Mut<StageTagRegistry>| {
        let types = type_registry.read();

        for entity in batch {
            // The entity may have been despawned between queueing and now.
            let Some(tags) = world.get::<StageTags>(entity).cloned() else {
                continue;
            };

            let resolved = resolve_tags(&tags, &tag_registry);
            unknown.extend(resolved.unknown);

            let mut entity_mut = world.entity_mut(entity);

            for tag in &resolved.tags {
                if let Some(registered) = tag_registry.get(&tag.key) {
                    registered.apply(&mut entity_mut, tag);
                }
            }

            // The reflection channel: any `extras` property whose key names a
            // registered reflected component, with a RON literal as its value.
            for (key, value) in tags.extras.iter() {
                if key == &discriminator {
                    continue;
                }
                let Some(text) = value.as_str() else {
                    continue;
                };
                let Some(registration) = types.get_with_short_type_path(key) else {
                    continue;
                };
                if let Err(error) = insert_reflected(&mut entity_mut, registration, text, &types) {
                    warn!(
                        "bevy_stage: property `{}` on entity {} names component `{}` but its value did not parse: {}",
                        key,
                        entity,
                        registration.type_info().type_path(),
                        error,
                    );
                    continue;
                }
                events.push(StageTagFound {
                    entity,
                    key: key.clone(),
                    params: TagParams::new(),
                    source: TagSource::Reflection,
                });
            }

            events.extend(resolved.tags.into_iter().map(|tag| StageTagFound {
                entity,
                key: tag.key,
                params: tag.params,
                source: tag.source,
            }));
        }
    });

    if !unknown.is_empty() {
        let mut seen = world.resource_mut::<UnknownTags>();
        for key in unknown {
            if seen.0.insert(key.clone()) {
                warn!(
                    "bevy_stage: level uses tag `{key}` but nothing is registered under that name \
                     (register it with `App::register_stage_tag`, or check the spelling)",
                );
            }
        }
    }

    for event in events {
        world.trigger(event);
    }
}

/// Deserialize a RON literal into a registered component and insert it.
fn insert_reflected(
    entity: &mut EntityWorldMut,
    registration: &bevy::reflect::TypeRegistration,
    text: &str,
    types: &TypeRegistry,
) -> Result<(), String> {
    let Some(reflect_component) = registration.data::<ReflectComponent>() else {
        return Err(format!(
            "`{}` is registered but not as a component (add `#[reflect(Component)]`)",
            registration.type_info().type_path()
        ));
    };

    let mut deserializer =
        ron::Deserializer::from_str(text).map_err(|error| error.to_string())?;
    let value = TypedReflectDeserializer::new(registration, types)
        .deserialize(&mut deserializer)
        .map_err(|error| error.to_string())?;

    reflect_component.insert(entity, value.as_partial_reflect(), types);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_respects_its_budget_and_keeps_the_rest() {
        let mut queue = StageTagQueue::default();
        for i in 0..5 {
            queue.push(Entity::from_raw_u32(i).unwrap());
        }

        assert_eq!(queue.take(2).len(), 2);
        assert_eq!(queue.len(), 3, "the rest must survive to the next frame");

        // A zero budget means "no limit", which is the right default for games
        // that would rather have one hitch than a tag popping in late.
        assert_eq!(queue.take(0).len(), 3);
        assert!(queue.is_empty());
    }
}
