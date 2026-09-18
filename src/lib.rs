//! A Bevy plugin for loading glTF scenes with artist-authored metadata.
//!
//! Levels are authored as glTF and gameplay meaning is attached by tagging
//! nodes, so a level artist can place a spawn point in Blender without a
//! programmer editing a spawn table.
#![warn(missing_docs)]

pub mod loader;
pub mod persist;
pub mod plugin;
pub mod stage;
pub mod stream;
pub mod tag;
pub mod view;

/// The common imports.
pub mod prelude {
    pub use crate::plugin::{StageAppExt, StagePlugin, StageSystems, StageTagConfig};
    pub use crate::persist::{StageDeltas, StageNodeId};
    pub use crate::stage::{
        AnchorAlignment, StageAnchor, StageId, StageManager, StagePortal, StageRoot, StageStatus,
        StreamingSource,
    };
    pub use crate::stream::StageSector;
    pub use crate::view::{LodConfig, StageViewConfig};
    pub use crate::tag::{
        StageTag, StageTagFound, StageTagRegistry, StageTags, TagParams, TagSource, TagValue,
    };
}
