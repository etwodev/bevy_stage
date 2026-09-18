//! Keeping what is resident cheap to render.

pub mod lod;

use bevy::mesh::Mesh3d;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

pub use lod::{lod_ranges, parse_lod_name, LodConfig};

use crate::stage::manager::StageRoot;
use crate::stage::transition::StreamingSource;

/// Beyond this distance a resident stage is hidden rather than unloaded.
///
/// The middle tier between "in the world" and "gone": keeping a level loaded
/// but invisible costs memory and nothing else, and makes walking back into it
/// instant.
#[derive(Resource, Debug, Clone, Reflect)]
pub struct StageViewConfig {
    /// Distance beyond which a resident but inactive stage stops rendering.
    pub view_distance: f32,
    /// Whether to wire LOD groups found in levels.
    pub auto_lod: bool,
}

impl Default for StageViewConfig {
    fn default() -> Self {
        Self {
            view_distance: 400.0,
            auto_lod: true,
        }
    }
}

/// Marks a node that belongs to a LOD group, so the wiring runs once.
#[derive(Component)]
pub struct LodWired;

/// Nodes that have just appeared and might be part of a LOD group.
type LodCandidates<'w, 's> =
    Query<'w, 's, (Entity, &'static Name, Option<&'static ChildOf>), (Added<Name>, Without<LodWired>)>;

/// LOD group members, keyed by their parent and shared base name.
type LodGroups = HashMap<(Option<Entity>, String), Vec<(usize, Entity)>>;

/// Find `Foo_LOD0`/`Foo_LOD1`/... sibling groups and give them visibility ranges.
pub fn wire_lod_groups(
    candidates: LodCandidates,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    config: Res<LodConfig>,
    view: Res<StageViewConfig>,
    mut commands: Commands,
) {
    if !view.auto_lod {
        return;
    }

    // Group by parent and base name: `Rock_LOD0` under one parent is a
    // different group from `Rock_LOD0` under another.
    let mut groups: LodGroups = HashMap::new();
    for (entity, name, parent) in &candidates {
        let Some((base, level)) = parse_lod_name(name.as_str()) else {
            continue;
        };
        groups
            .entry((parent.map(ChildOf::parent), base.to_string()))
            .or_default()
            .push((level, entity));
    }

    for ((_, base), mut members) in groups {
        // A lone `Thing_LOD0` is not a LOD group; giving it a range would only
        // risk hiding it.
        if members.len() < 2 {
            continue;
        }
        members.sort_by_key(|(level, _)| *level);

        let ranges = lod_ranges(members.len(), &config);
        for ((level, entity), range) in members.iter().zip(ranges) {
            let _ = level;
            commands.entity(*entity).insert(LodWired);

            // `VisibilityRange` is not inherited, and in a glTF the mesh lives
            // on a child of the node, so the range has to go on the meshes
            // themselves.
            let mut applied = false;
            if let Ok(kids) = children.get(*entity) {
                for kid in kids.iter() {
                    if meshes.get(kid).is_ok() {
                        commands.entity(kid).insert(range.clone());
                        applied = true;
                    }
                }
            }
            if !applied {
                // No mesh child: put it on the node itself so a group of
                // non-mesh nodes still behaves.
                commands.entity(*entity).insert(range.clone());
            }
        }
        debug!("bevy_stage: wired LOD group `{base}`");
    }
}

/// Stop rendering resident stages the player is nowhere near.
pub fn cull_distant_stages(
    sources: Query<&GlobalTransform, With<StreamingSource>>,
    mut stages: Query<(&StageRoot, &GlobalTransform, &mut Visibility)>,
    config: Res<StageViewConfig>,
) {
    if sources.is_empty() {
        return;
    }

    for (root, transform, mut visibility) in &mut stages {
        if !root.status.is_ready() {
            continue;
        }
        let nearest = sources
            .iter()
            .map(|source| source.translation().distance(transform.translation()))
            .fold(f32::MAX, f32::min);

        let wanted = if nearest > config.view_distance {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        // Avoid touching the component unless it actually changes, so change
        // detection stays meaningful for anything else watching it.
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}
