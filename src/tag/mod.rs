//! Reading artist-authored tags off glTF nodes.

pub mod component;
pub mod parse;
pub mod registry;
pub mod resolve;
pub mod value;

pub use component::{StageTag, StageTags, TagSource};
pub use parse::{
    build_stage_tags, parse_extras, parse_node_name, TagParseError, DEFAULT_DISCRIMINATOR,
};
pub use registry::{resolve_tags, RegisteredTag, ResolvedTags, StageTagRegistry};
pub use resolve::{StageTagFound, StageTagQueue, UnknownTags};
pub use value::{normalize_key, NormalizedMap, TagDeserializeError, TagParams, TagValue};

pub use crate::plugin::StageTagConfig;
