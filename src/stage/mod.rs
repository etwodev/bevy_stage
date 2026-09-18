//! Levels as resident, independently placed instances.

pub mod anchor;
pub mod manager;
pub mod scan;
pub mod transition;

pub use scan::{find_all_anchors, find_anchor, ANCHOR_TAG};
pub use anchor::{align_stage_to, AnchorAlignment, StageAnchor};
pub use manager::{
    Placement, StageAnchors, StageId, StageManager, StageRoot, StageStatus,
};
pub use transition::{
    PortalLink, StageEntered, StagePortal, StreamingSource, DEFAULT_PRELOAD,
};
