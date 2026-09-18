//! What the game says its tags mean.

use std::sync::{Arc, RwLock};

use bevy::ecs::world::EntityWorldMut;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use serde::de::DeserializeOwned;

use super::component::{StageTag, StageTags, TagSource};
use super::value::{normalize_key, NormalizedMap};

/// Inserts the component a tag maps to, given the tag's parameters.
type Inserter = Arc<dyn Fn(&mut EntityWorldMut, &StageTag) + Send + Sync>;

/// One registered tag.
#[derive(Clone)]
pub struct RegisteredTag {
    /// The canonical spelling, as the game registered it.
    pub key: String,
    insert: Option<Inserter>,
}

impl RegisteredTag {
    /// Apply this tag to an entity.
    pub fn apply(&self, entity: &mut EntityWorldMut, tag: &StageTag) {
        if let Some(insert) = &self.insert {
            insert(entity, tag);
        }
    }
}

/// The normalized names of every registered tag.
///
/// Shared with the glTF loader so it can tell, while loading, whether a node's
/// *name* is worth recording. Without this the loader would have to bake
/// metadata onto every named node in the file just in case the game had
/// registered a tag matching one of them.
pub type KnownTagNames = Arc<RwLock<HashSet<String>>>;

/// The set of tags the game understands.
///
/// Lookups ignore casing and separators (see [`normalize_key`]), so a tag
/// registered as `spawn_point` is found by a Blender object named `SpawnPoint`.
///
/// Tags should be registered during app setup. A tag registered after a level
/// has already been loaded is still honoured for levels loaded afterwards, but
/// the already-cached asset will not gain it.
#[derive(Resource, Default)]
pub struct StageTagRegistry {
    entries: NormalizedMap<RegisteredTag>,
    known_names: KnownTagNames,
}

impl StageTagRegistry {
    /// Register a tag that inserts `T`, built from the tag's parameters.
    pub fn register<T>(&mut self, key: &str)
    where
        T: Component + DeserializeOwned,
    {
        let key_owned = key.to_string();
        let type_name = std::any::type_name::<T>();
        let insert: Inserter = Arc::new(move |entity: &mut EntityWorldMut, tag: &StageTag| {
            match tag.params.deserialize::<T>() {
                Ok(component) => {
                    entity.insert(component);
                }
                Err(error) => {
                    // Don't fail the level: report the node so the artist can
                    // find the property they mistyped.
                    warn!(
                        "bevy_stage: tag `{}` on entity {} could not build `{}`: {}",
                        tag.key,
                        entity.id(),
                        type_name,
                        error,
                    );
                }
            }
        });

        self.entries.insert(
            key,
            RegisteredTag {
                key: key_owned,
                insert: Some(insert),
            },
        );
        self.publish(key);
    }

    /// Register a tag that carries no component of its own.
    ///
    /// Useful when the game only wants the [`StageTagFound`](super::StageTagFound)
    /// event, or when the plugin itself claims a tag name.
    pub fn register_marker(&mut self, key: &str) {
        self.entries.insert(
            key,
            RegisteredTag {
                key: key.to_string(),
                insert: None,
            },
        );
        self.publish(key);
    }

    /// Share a newly registered name with the loader.
    fn publish(&self, key: &str) {
        if let Ok(mut names) = self.known_names.write() {
            names.insert(normalize_key(key));
        }
    }

    /// The shared name set, for handing to the glTF loader.
    pub fn known_names(&self) -> KnownTagNames {
        self.known_names.clone()
    }

    /// Look up a tag by any spelling.
    pub fn get(&self, key: &str) -> Option<&RegisteredTag> {
        self.entries.get(key)
    }

    /// Whether any spelling of `key` is registered.
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains(key)
    }

    /// Whether nothing has been registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The outcome of matching a node's baked metadata against the registry.
#[derive(Debug, Default, PartialEq)]
pub struct ResolvedTags {
    /// Tags that matched a registered entry, canonically spelled and
    /// deduplicated, highest-precedence source winning.
    pub tags: Vec<StageTag>,
    /// Tag names the node asked for that nothing is registered under.
    ///
    /// Almost always a typo, and silently dropping them is how an artist loses
    /// an afternoon — so they are surfaced rather than ignored.
    pub unknown: Vec<String>,
}

/// Match a node's baked metadata against the registry.
///
/// Pure, so the precedence rules between the three authoring channels can be
/// tested without a world. The reflection channel is resolved separately,
/// because it needs the type registry.
pub fn resolve_tags(tags: &StageTags, registry: &StageTagRegistry) -> ResolvedTags {
    let mut resolved: Vec<StageTag> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut unknown = Vec::new();

    // The discriminator is the explicit channel, so it wins over the name.
    for tag in &tags.tags {
        let Some(entry) = registry.get(&tag.key) else {
            unknown.push(tag.key.clone());
            continue;
        };
        if seen.insert(normalize_key(&tag.key)) {
            resolved.push(StageTag {
                // Canonicalize to the spelling the game registered, so game
                // code compares against one known string.
                key: entry.key.clone(),
                params: tag.params.clone(),
                source: TagSource::Extras,
            });
        }
    }

    // The name channel only fires when the name matches something registered.
    // A node named `Ground` is not an unknown tag, it is just a node.
    if let Some(entry) = registry.get(&tags.base_name)
        && seen.insert(normalize_key(&tags.base_name))
    {
        resolved.push(StageTag {
            key: entry.key.clone(),
            params: tags.name_params.clone(),
            source: TagSource::Name,
        });
    }

    ResolvedTags {
        tags: resolved,
        unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag::value::{TagParams, TagValue};

    #[derive(Component, serde::Deserialize, Default)]
    struct SpawnPoint {
        #[serde(default)]
        #[allow(dead_code)]
        team: String,
    }

    fn registry() -> StageTagRegistry {
        let mut registry = StageTagRegistry::default();
        registry.register::<SpawnPoint>("spawn_point");
        registry
    }

    #[test]
    fn a_node_named_like_a_tag_resolves_with_no_extras_at_all() {
        let tags = StageTags {
            base_name: "SpawnPoint".into(),
            ..Default::default()
        };
        let resolved = resolve_tags(&tags, &registry());

        assert_eq!(resolved.tags.len(), 1);
        assert_eq!(resolved.tags[0].key, "spawn_point");
        assert_eq!(resolved.tags[0].source, TagSource::Name);
    }

    #[test]
    fn the_explicit_channel_outranks_the_name() {
        // Same node tagged both ways with different params: the discriminator
        // is what the artist typed deliberately, so it must win.
        let tags = StageTags {
            base_name: "SpawnPoint".into(),
            name_params: TagParams::from_iter([(
                "team".to_string(),
                TagValue::String("from_name".into()),
            )]),
            tags: vec![StageTag {
                key: "spawn_point".into(),
                params: TagParams::from_iter([(
                    "team".to_string(),
                    TagValue::String("from_extras".into()),
                )]),
                source: TagSource::Extras,
            }],
            ..Default::default()
        };
        let resolved = resolve_tags(&tags, &registry());

        assert_eq!(resolved.tags.len(), 1, "the tag must not be applied twice");
        assert_eq!(resolved.tags[0].source, TagSource::Extras);
        assert_eq!(resolved.tags[0].params.get_str("team"), Some("from_extras"));
    }

    #[test]
    fn resolution_canonicalizes_to_the_registered_spelling() {
        let tags = StageTags {
            base_name: "spawn-point".into(),
            ..Default::default()
        };
        let resolved = resolve_tags(&tags, &registry());
        // Game code should only ever have to compare against one string.
        assert_eq!(resolved.tags[0].key, "spawn_point");
    }

    #[test]
    fn a_misspelled_tag_is_reported_rather_than_dropped() {
        let tags = StageTags {
            tags: vec![StageTag::new("spwan_point", TagSource::Extras)],
            ..Default::default()
        };
        let resolved = resolve_tags(&tags, &registry());

        assert!(resolved.tags.is_empty());
        assert_eq!(resolved.unknown, vec!["spwan_point".to_string()]);
    }

    #[test]
    fn an_ordinary_node_name_is_not_an_unknown_tag() {
        let tags = StageTags {
            base_name: "Ground".into(),
            ..Default::default()
        };
        let resolved = resolve_tags(&tags, &registry());

        assert!(resolved.tags.is_empty());
        assert!(
            resolved.unknown.is_empty(),
            "most nodes are just geometry and must not generate warnings"
        );
    }

    #[test]
    fn one_node_can_resolve_several_distinct_tags() {
        let mut registry = registry();
        registry.register_marker("anchor");
        let tags = StageTags {
            tags: vec![
                StageTag::new("anchor", TagSource::Extras),
                StageTag::new("spawn_point", TagSource::Extras),
            ],
            ..Default::default()
        };
        let resolved = resolve_tags(&tags, &registry);
        assert_eq!(resolved.tags.len(), 2);
    }
}
