//! Streaming parts of a level in and out around the player.
//!
//! A **sector** is a node tagged `sector`. There are two kinds, and which one
//! you get depends on whether the node names another glTF file:
//!
//! - **External** (`source` is set): the file is brought in as a level in its
//!   own right, positioned at the sector node. This is real streaming — the
//!   geometry is not in memory until it is needed. Because a sector is just a
//!   stage, it gets tags, LOD and everything else for free.
//! - **Inline** (no `source`): the subtree is already part of the level, so
//!   there is nothing to load. It is shown and hidden instead, which costs
//!   nothing to do and saves draw calls but not memory.
//!
//! Splitting a large level into external sectors is also what bounds spawn
//! hitches: a scene spawns atomically, so sector size is the lever that
//! controls the worst-case frame.

use bevy::prelude::*;
use serde::Deserialize;

use crate::stage::manager::{StageId, StageManager};
use crate::stage::transition::StreamingSource;

/// A streamable piece of a level.
#[derive(Component, Debug, Clone, Deserialize, Reflect)]
#[reflect(Component)]
pub struct StageSector {
    /// Another glTF to bring in at this node, if the sector lives in its own
    /// file. Leave unset for a sector whose contents are already in this level.
    #[serde(default)]
    pub source: Option<String>,
    /// How close something must be before the sector comes in.
    #[serde(default = "default_radius")]
    pub radius: f32,
    /// How much further than `radius` it must go before the sector leaves.
    ///
    /// The gap is deliberate: without it, something loitering exactly on the
    /// boundary would load and unload every frame. Expressed as a multiplier
    /// so it scales with the sector.
    #[serde(default = "default_hysteresis")]
    pub hysteresis: f32,
}

fn default_radius() -> f32 {
    80.0
}

fn default_hysteresis() -> f32 {
    1.3
}

impl Default for StageSector {
    fn default() -> Self {
        Self {
            source: None,
            radius: default_radius(),
            hysteresis: default_hysteresis(),
        }
    }
}

impl StageSector {
    /// The distance at which this sector leaves again.
    pub fn unload_radius(&self) -> f32 {
        self.radius * self.hysteresis.max(1.0)
    }
}

/// Tracks the stage an external sector brought in.
#[derive(Component, Debug, Clone, Copy)]
pub struct SectorLoaded {
    /// The stage instance holding this sector's contents.
    pub stage: StageId,
}

/// Whether a sector should be resident, given how far away the nearest
/// streaming source is.
///
/// Separated out so the hysteresis rule is testable without a world: it is the
/// one piece of streaming logic that is easy to get subtly wrong and hard to
/// notice, because the symptom is a stutter rather than a failure.
pub fn should_be_resident(sector: &StageSector, currently_resident: bool, distance: f32) -> bool {
    if currently_resident {
        distance <= sector.unload_radius()
    } else {
        distance <= sector.radius
    }
}

/// Bring external sectors in and out as sources move.
pub fn stream_external_sectors(
    sources: Query<&GlobalTransform, With<StreamingSource>>,
    sectors: Query<(Entity, &StageSector, &GlobalTransform, Option<&SectorLoaded>)>,
    mut manager: ResMut<StageManager>,
    mut commands: Commands,
) {
    if sources.is_empty() {
        return;
    }

    for (entity, sector, transform, loaded) in &sectors {
        let Some(source_path) = &sector.source else {
            continue;
        };

        let distance = sources
            .iter()
            .map(|source| source.translation().distance(transform.translation()))
            .fold(f32::MAX, f32::min);

        let resident = loaded.is_some_and(|l| manager.is_resident(l.stage));
        let wanted = should_be_resident(sector, resident, distance);

        match (resident, wanted) {
            (false, true) => {
                // Place the sector exactly where its node sits, so the level
                // author positions it in the parent file and nothing else has
                // to agree on coordinates.
                let stage = manager.load_at(source_path.clone(), transform.compute_transform());
                commands.entity(entity).insert(SectorLoaded { stage });
            }
            (true, false) => {
                if let Some(loaded) = loaded {
                    manager.unload(loaded.stage);
                }
                commands.entity(entity).remove::<SectorLoaded>();
            }
            _ => {}
        }
    }
}

/// Show and hide inline sectors.
pub fn cull_inline_sectors(
    sources: Query<&GlobalTransform, With<StreamingSource>>,
    mut sectors: Query<(&StageSector, &GlobalTransform, &mut Visibility)>,
) {
    if sources.is_empty() {
        return;
    }

    for (sector, transform, mut visibility) in &mut sectors {
        if sector.source.is_some() {
            continue;
        }

        let distance = sources
            .iter()
            .map(|source| source.translation().distance(transform.translation()))
            .fold(f32::MAX, f32::min);

        let resident = *visibility != Visibility::Hidden;
        let wanted = if should_be_resident(sector, resident, distance) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sector() -> StageSector {
        StageSector {
            source: Some("levels/north.gltf".into()),
            radius: 100.0,
            hysteresis: 1.3,
        }
    }

    #[test]
    fn a_sector_comes_in_at_its_radius() {
        assert!(should_be_resident(&sector(), false, 99.0));
        assert!(!should_be_resident(&sector(), false, 101.0));
    }

    #[test]
    fn a_resident_sector_stays_until_well_past_that_radius() {
        // The whole point of hysteresis: between 100 and 130 the answer
        // depends on which side you came from.
        assert!(should_be_resident(&sector(), true, 120.0));
        assert!(!should_be_resident(&sector(), true, 131.0));
    }

    #[test]
    fn loitering_on_the_boundary_does_not_thrash() {
        let sector = sector();
        let mut resident = false;
        let mut flips = 0;

        // Walk back and forth across the load radius, which is exactly the
        // motion that would thrash without a hysteresis band.
        for step in 0..40 {
            let distance = if step % 2 == 0 { 99.0 } else { 101.0 };
            let wanted = should_be_resident(&sector, resident, distance);
            if wanted != resident {
                flips += 1;
                resident = wanted;
            }
        }

        assert_eq!(
            flips, 1,
            "the sector should load once and stay, not reload every frame"
        );
    }

    #[test]
    fn a_degenerate_hysteresis_still_never_unloads_inside_the_load_radius() {
        // A level author setting hysteresis below 1 would otherwise create a
        // band where the sector unloads immediately after loading.
        let sector = StageSector {
            hysteresis: 0.5,
            ..sector()
        };
        assert!(should_be_resident(&sector, true, 100.0));
    }
}
