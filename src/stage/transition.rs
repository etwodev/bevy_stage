//! Walking from one level to the next without a loading screen.
//!
//! The mechanism is a *portal*: a node tagged `portal` that names the level on
//! the other side of it. Getting near a portal starts loading that level in the
//! background; walking through it hands over. Nothing here knows what an
//! elevator or a hallway is — an interstitial space is just a small level with
//! a portal at each end, and entering it from one side is what pays for the
//! load of the other. That falls out of the design rather than being a feature.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use serde::Deserialize;

use super::anchor::AnchorAlignment;
use super::manager::{StageId, StageManager, StageRoot};

/// How far from a portal a level starts loading, when the portal does not say.
pub const DEFAULT_PRELOAD: f32 = 30.0;

fn default_preload() -> f32 {
    DEFAULT_PRELOAD
}

/// A doorway onto another level.
///
/// Authored as a node tagged `portal`. The node's own transform *is* the
/// connection point, so no separate anchor is needed on this side.
///
/// # Orientation convention
///
/// **A portal faces the way you travel through it** — toward the level it
/// names. Hand-over fires when something crosses the portal plane moving along
/// that facing, so a doorway with a portal in each direction needs the two
/// pointing opposite ways. Get this backwards and the portal will fire as the
/// player walks *away* from its target.
///
/// Note this is the opposite of the [`StageAnchor`](super::anchor::StageAnchor)
/// convention, which points out of its own level. That is deliberate: an
/// anchor marks a boundary, while a portal marks a direction of travel.
#[derive(Component, Debug, Clone, Deserialize, Reflect)]
#[reflect(Component)]
pub struct StagePortal {
    /// The asset path of the level on the other side.
    pub target: String,
    /// The anchor in that level to line up with this portal.
    #[serde(default)]
    pub anchor: String,
    /// How close the player must get before the level starts loading.
    ///
    /// Should comfortably exceed the time it takes to load: this distance is
    /// the entire budget for hiding the load.
    #[serde(default = "default_preload")]
    pub preload: f32,
    /// How the two connection points are oriented.
    #[serde(default)]
    pub alignment: AnchorAlignment,
}

impl Default for StagePortal {
    fn default() -> Self {
        Self {
            target: String::new(),
            anchor: String::new(),
            preload: DEFAULT_PRELOAD,
            alignment: AnchorAlignment::default(),
        }
    }
}

/// A portal that has already brought its level in, and which instance it got.
///
/// Also written onto the *far* side's matching portal when a level is loaded,
/// so walking back through does not load a second copy of where you came from.
#[derive(Component, Debug, Clone, Copy)]
pub struct PortalLink {
    /// The instance on the other side.
    pub stage: StageId,
}

/// Tracks which side of each portal a mover was on last frame.
#[derive(Component, Debug, Default)]
pub struct PortalCrossing {
    sides: HashMap<Entity, f32>,
}

/// Something whose position drives loading and unloading — usually the player,
/// sometimes the camera.
#[derive(Component, Debug, Clone)]
pub struct StreamingSource {
    /// Beyond this distance from a resident level's connection point, that
    /// level is unloaded.
    ///
    /// Must exceed every portal's `preload` distance, or a level will unload
    /// the instant it finishes loading and thrash. The gap between the two is
    /// the hysteresis band.
    pub unload_distance: f32,
}

impl Default for StreamingSource {
    fn default() -> Self {
        Self {
            unload_distance: DEFAULT_PRELOAD * 2.5,
        }
    }
}

/// Fired when the player walks through a portal into another level.
#[derive(EntityEvent, Debug, Clone)]
pub struct StageEntered {
    /// The stage root that was entered.
    pub entity: Entity,
    /// The level now considered active.
    pub stage: StageId,
    /// The level left behind.
    pub from: Option<StageId>,
}

/// Where a portal connected from, so the far side can be linked back.
///
/// Lives as its own entity between the moment a level is requested and the
/// moment it spawns, because the return portal does not exist until then.
#[derive(Component, Debug, Clone)]
pub struct PortalOrigin {
    /// The level the portal that spawned this one belongs to.
    stage: StageId,
    /// That level's asset path, for matching the return portal.
    path: String,
    /// The world-space connection point.
    at: GlobalTransform,
    /// Frames spent waiting, so a connection that never resolves is cleaned up
    /// instead of lingering forever.
    waited: u32,
}

/// How long to wait for a level to spawn before giving up on linking its
/// return portal. Generous: this is a load from disk, not a frame budget.
const RETURN_LINK_TIMEOUT_FRAMES: u32 = 1800;

/// How near the connection point a portal must be to count as the way back.
/// A doorway's two portals are essentially co-located; anything further away
/// is a different door.
const RETURN_LINK_RADIUS: f32 = 5.0;

/// Start loading the level behind any portal the player is approaching.
pub fn preload_through_portals(
    sources: Query<&GlobalTransform, With<StreamingSource>>,
    portals: Query<(Entity, &StagePortal, &GlobalTransform), Without<PortalLink>>,
    stage_of: Query<&ChildOf>,
    roots: Query<&StageRoot>,
    mut manager: ResMut<StageManager>,
    mut commands: Commands,
) {
    for (entity, portal, portal_transform) in &portals {
        if portal.target.is_empty() {
            continue;
        }

        let near = sources.iter().any(|source| {
            source.translation().distance(portal_transform.translation()) <= portal.preload
        });
        if !near {
            continue;
        }

        let owner = owning_stage(entity, &stage_of, &roots);
        let stage = manager.load_connected(
            portal.target.clone(),
            portal.anchor.clone(),
            *portal_transform,
            portal.alignment,
        );

        let mut portal_entity = commands.entity(entity);
        portal_entity.insert(PortalLink { stage });
        if let Some((owner_id, owner_path)) = owner {
            // Remember where this came from so the level being loaded can have
            // its return portal wired straight back, instead of loading a
            // second copy of the level the player is standing in.
            commands.spawn(PortalOrigin {
                stage: owner_id,
                path: owner_path,
                at: *portal_transform,
                waited: 0,
            });
        }
    }
}

/// Wire up the return portal of a level that has just spawned.
///
/// Without this, walking back through the door you came in would load a second
/// copy of the level you are standing in.
pub fn link_return_portals(
    mut origins: Query<(Entity, &mut PortalOrigin)>,
    portals: Query<(Entity, &StagePortal, &GlobalTransform), Without<PortalLink>>,
    stage_of: Query<&ChildOf>,
    roots: Query<&StageRoot>,
    mut commands: Commands,
) {
    for (origin_entity, mut origin) in &mut origins {
        let mut linked = false;

        for (entity, portal, transform) in &portals {
            if portal.target != origin.path {
                continue;
            }
            if transform.translation().distance(origin.at.translation()) > RETURN_LINK_RADIUS {
                continue;
            }
            // A portal in the source level itself points the same way; only the
            // one on the far side is the way back.
            if owning_stage(entity, &stage_of, &roots).map(|(id, _)| id) == Some(origin.stage) {
                continue;
            }
            commands.entity(entity).insert(PortalLink {
                stage: origin.stage,
            });
            linked = true;
        }

        origin.waited += 1;
        if linked || origin.waited > RETURN_LINK_TIMEOUT_FRAMES {
            commands.entity(origin_entity).despawn();
        }
    }
}

/// Hand over to the level on the other side when the player walks through.
pub fn detect_portal_crossings(
    sources: Query<(Entity, &GlobalTransform), With<StreamingSource>>,
    mut portals: Query<(
        &StagePortal,
        &GlobalTransform,
        &PortalLink,
        &mut PortalCrossing,
    )>,
    roots: Query<&StageRoot>,
    mut manager: ResMut<StageManager>,
    mut commands: Commands,
) {
    for (portal, transform, link, mut crossing) in &mut portals {
        let _ = portal;
        for (source, source_transform) in &sources {
            let offset = source_transform.translation() - transform.translation();
            // Signed distance along the portal's forward axis: the side the
            // player is on.
            let side = offset.dot(*transform.forward());
            // Only count a crossing that happens near the doorway, not one
            // that happens fifty metres to the left of it.
            let lateral = (offset - *transform.forward() * side).length();

            let previous = crossing.sides.insert(source, side);
            let Some(previous) = previous else {
                continue;
            };

            let crossed = previous <= 0.0 && side > 0.0;
            if !crossed || lateral > 6.0 {
                continue;
            }

            // Already there: crossing a portal back into the level you are
            // standing in is not an event worth firing.
            if manager.active() == Some(link.stage) {
                continue;
            }
            if !manager.is_resident(link.stage) {
                continue;
            }
            let Some(entity) = manager.entity(link.stage) else {
                continue;
            };
            if !roots.get(entity).map(|r| r.status.is_ready()).unwrap_or(false) {
                // The level behind the portal has not finished loading. This is
                // the case the preload distance exists to prevent; if it
                // happens the player has outrun the load.
                warn!(
                    "bevy_stage: crossed into a stage that is not ready yet — \
                     increase the portal's `preload` distance"
                );
                continue;
            }

            let from = manager.active();
            manager.activate(link.stage);
            commands.trigger(StageEntered {
                entity,
                stage: link.stage,
                from,
            });
        }
    }
}

/// Drop levels the player has left far behind.
pub fn unload_distant_stages(
    sources: Query<(&GlobalTransform, &StreamingSource)>,
    stages: Query<(&StageRoot, &GlobalTransform)>,
    mut manager: ResMut<StageManager>,
) {
    let active = manager.active();
    let mut drop = Vec::new();

    for (root, transform) in &stages {
        if Some(root.id) == active || !root.status.is_ready() {
            continue;
        }
        let far = sources.iter().all(|(source, settings)| {
            source.translation().distance(transform.translation()) > settings.unload_distance
        });
        // With no sources at all nothing is "far", so a game that has not set
        // one up never has levels vanish underneath it.
        if far && !sources.is_empty() {
            drop.push(root.id);
        }
    }

    for id in drop {
        manager.unload(id);
    }
}

/// Walk up to the stage root that owns an entity.
fn owning_stage(
    entity: Entity,
    parents: &Query<&ChildOf>,
    roots: &Query<&StageRoot>,
) -> Option<(StageId, String)> {
    let mut current = entity;
    loop {
        if let Ok(root) = roots.get(current) {
            return Some((root.id, root.path.clone()));
        }
        current = parents.get(current).ok()?.parent();
    }
}

/// Give every portal the state it needs to detect crossings.
pub fn attach_portal_state(
    portals: Query<Entity, (With<StagePortal>, Without<PortalCrossing>)>,
    mut commands: Commands,
) {
    for entity in &portals {
        commands.entity(entity).insert(PortalCrossing::default());
    }
}
