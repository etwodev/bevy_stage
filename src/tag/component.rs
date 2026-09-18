//! The component that carries parsed metadata from load time to spawn time.

use bevy::prelude::*;

use super::value::TagParams;

/// Which authoring channel a tag came from.
///
/// Recorded so that conflicts resolve predictably and so diagnostics can tell
/// an artist *where* a tag was picked up from when it is not the one they meant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, Default)]
pub enum TagSource {
    /// The reserved discriminator property in the node's glTF `extras`.
    #[default]
    Extras,
    /// The node's name matched a registered tag.
    Name,
    /// An `extras` property whose key named a registered reflected component.
    Reflection,
}

/// One tag resolved on a node.
#[derive(Clone, Debug, PartialEq, Reflect, Default)]
pub struct StageTag {
    /// The tag name as authored.
    pub key: String,
    /// The tag's parameters.
    pub params: TagParams,
    /// Where the tag was read from.
    pub source: TagSource,
}

impl StageTag {
    /// A tag with no parameters.
    pub fn new(key: impl Into<String>, source: TagSource) -> Self {
        Self {
            key: key.into(),
            params: TagParams::new(),
            source,
        }
    }
}

/// Parsed metadata for one glTF node, baked into the loaded asset.
///
/// This is produced once per node at **load** time and stored in the asset, so
/// the JSON parsing cost is paid once no matter how many times the level is
/// spawned. Interpreting it against the game's registry happens at spawn time,
/// where the type registry and [`Commands`] are available.
///
/// Deliberately holds no policy: the loader records what it found, and
/// resolution decides what it means. That keeps the loader free of any
/// dependency on what the game has registered, which in turn means registering
/// a tag after a level has already been loaded still works.
#[derive(Component, Clone, Debug, PartialEq, Reflect, Default)]
#[reflect(Component)]
pub struct StageTags {
    /// Tags named explicitly by the discriminator property.
    pub tags: Vec<StageTag>,
    /// Every property found in the node's `extras`, parsed but uninterpreted.
    ///
    /// Retained in full because the reflection channel cannot be resolved
    /// without the type registry, and because games may want to read
    /// properties the plugin knows nothing about.
    pub extras: TagParams,
    /// The node name with Blender's `.001` duplicate suffix and any inline
    /// parameter block removed.
    pub base_name: String,
    /// Parameters parsed out of an inline `Name[key=value]` block.
    pub name_params: TagParams,
}

impl StageTags {
    /// Whether this node carried no metadata of any kind.
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.extras.is_empty() && self.name_params.is_empty()
    }

    /// The first tag with the given key, if any.
    pub fn tag(&self, key: &str) -> Option<&StageTag> {
        self.tags.iter().find(|t| t.key == key)
    }
}
