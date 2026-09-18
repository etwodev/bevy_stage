//! Keeping several levels resident at once, each independently placed.

use bevy::gltf::{Gltf, GltfNode};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstance, WorldInstanceSpawner};

use super::anchor::{align_stage_to, AnchorAlignment};
use super::scan::{find_all_anchors, find_anchor};
use crate::plugin::StageTagConfig;

/// Identifies one resident copy of a level.
///
/// A level can be resident more than once — two ends of a corridor may both be
/// the same tiling room asset — so identity belongs to the instance, not the
/// file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Reflect)]
pub struct StageId(pub u32);

/// How far along a resident stage is.
#[derive(Debug, Clone, PartialEq, Eq, Reflect)]
pub enum StageStatus {
    /// The glTF is being read off disk.
    Loading,
    /// The asset is ready and the scene is being spawned.
    Spawning,
    /// Fully spawned and in position.
    Ready,
    /// The load failed; the message says why.
    Failed(String),
}

impl StageStatus {
    /// Whether the stage is fully spawned and placed.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Where a stage should sit in the world.
#[derive(Debug, Clone)]
pub enum Placement {
    /// At a fixed transform.
    Fixed(Transform),
    /// Positioned so one of its anchors lands on a world-space connection
    /// point. This is what makes two levels line up seamlessly.
    Anchored {
        /// The anchor in the stage being loaded.
        anchor: String,
        /// Where that anchor should end up.
        target: GlobalTransform,
        /// How the two should be oriented.
        alignment: AnchorAlignment,
    },
}

/// Marks the root entity of a resident stage. Everything the level spawns is a
/// descendant, so unloading is a single despawn.
#[derive(Component, Debug, Clone, Reflect)]
#[require(Transform, Visibility)]
pub struct StageRoot {
    /// Which instance this is.
    pub id: StageId,
    /// The asset it came from.
    pub path: String,
    /// How far along it is.
    pub status: StageStatus,
}

/// The connection points a resident stage offers, relative to its own root.
#[derive(Component, Debug, Clone, Default)]
pub struct StageAnchors(HashMap<String, GlobalTransform>);

impl StageAnchors {
    /// An anchor's transform relative to the stage root.
    pub fn local(&self, id: &str) -> Option<GlobalTransform> {
        self.0.get(id).copied()
    }

    /// Anchor ids.
    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }
}

/// The asset and placement a stage is waiting on while it loads.
///
/// An implementation detail of the load pipeline, public only because it
/// appears in a public system's signature. It is removed once the stage spawns.
#[derive(Component)]
pub struct PendingStage {
    handle: Handle<Gltf>,
    placement: Placement,
}

/// A queued request to bring a level into the world.
struct StageRequest {
    id: StageId,
    path: String,
    placement: Placement,
}

/// The set of levels currently in the world.
///
/// Requests are queued and carried out by the plugin's systems, so game code
/// can ask for a level from anywhere without needing exclusive world access.
#[derive(Resource)]
pub struct StageManager {
    next_id: u32,
    requests: Vec<StageRequest>,
    unloads: Vec<StageId>,
    resident: HashMap<StageId, Entity>,
    active: Option<StageId>,
    /// The most levels allowed in the world at once.
    ///
    /// A backstop, not a budget. A mis-authored level graph — two levels whose
    /// portals point at each other without a matching return anchor — would
    /// otherwise load levels without bound. Raise it if you legitimately need
    /// deeper residency; a level failing with "too many resident stages" means
    /// something is wrong with the connections, not with this number.
    pub max_resident: usize,
}

impl Default for StageManager {
    fn default() -> Self {
        Self {
            next_id: 0,
            requests: Vec::new(),
            unloads: Vec::new(),
            resident: HashMap::new(),
            active: None,
            max_resident: 8,
        }
    }
}

impl StageManager {
    /// Bring a level in at the world origin.
    pub fn load(&mut self, path: impl Into<String>) -> StageId {
        self.request(path, Placement::Fixed(Transform::IDENTITY))
    }

    /// Bring a level in at a fixed transform.
    pub fn load_at(&mut self, path: impl Into<String>, transform: Transform) -> StageId {
        self.request(path, Placement::Fixed(transform))
    }

    /// Bring a level in positioned against a world-space connection point.
    ///
    /// The named anchor in the incoming level is placed exactly on `target`,
    /// which is how a corridor meets the doorway it leads out of.
    pub fn load_connected(
        &mut self,
        path: impl Into<String>,
        anchor: impl Into<String>,
        target: GlobalTransform,
        alignment: AnchorAlignment,
    ) -> StageId {
        self.request(
            path,
            Placement::Anchored {
                anchor: anchor.into(),
                target,
                alignment,
            },
        )
    }

    fn request(&mut self, path: impl Into<String>, placement: Placement) -> StageId {
        let id = StageId(self.next_id);
        self.next_id += 1;
        self.requests.push(StageRequest {
            id,
            path: path.into(),
            placement,
        });
        id
    }

    /// Remove a level and everything it spawned.
    pub fn unload(&mut self, id: StageId) {
        self.unloads.push(id);
    }

    /// Mark a level as the one the player is in.
    ///
    /// Purely informational to the plugin — it does not change what is
    /// resident — but it is what streaming and transitions key off.
    pub fn activate(&mut self, id: StageId) {
        self.active = Some(id);
    }

    /// The level the player is currently in.
    pub fn active(&self) -> Option<StageId> {
        self.active
    }

    /// The root entity of a resident level.
    pub fn entity(&self, id: StageId) -> Option<Entity> {
        self.resident.get(&id).copied()
    }

    /// Whether a level is in the world, at any stage of loading.
    pub fn is_resident(&self, id: StageId) -> bool {
        self.resident.contains_key(&id)
    }

    /// Levels queued to be removed this frame.
    ///
    /// Exposed so state can be captured before the entities go away.
    pub fn pending_unloads(&self) -> &[StageId] {
        &self.unloads
    }

    /// Every resident level.
    pub fn resident(&self) -> impl Iterator<Item = (&StageId, &Entity)> {
        self.resident.iter()
    }

    /// How many levels are resident.
    pub fn len(&self) -> usize {
        self.resident.len()
    }

    /// Whether nothing is resident.
    pub fn is_empty(&self) -> bool {
        self.resident.is_empty()
    }
}

/// Turn queued requests into root entities and start loading their assets.
pub fn begin_stage_loads(
    mut manager: ResMut<StageManager>,
    mut commands: Commands,
    assets: Res<AssetServer>,
) {
    let requests = core::mem::take(&mut manager.requests);
    for request in requests {
        if manager.resident.len() >= manager.max_resident {
            error!(
                "bevy_stage: refusing to load `{}` — already holding {} levels. \
                 Check that connected levels have matching anchors, or raise \
                 `StageManager::max_resident`.",
                request.path, manager.max_resident
            );
            let entity = commands
                .spawn((
                    StageRoot {
                        id: request.id,
                        path: request.path.clone(),
                        status: StageStatus::Failed(format!(
                            "refused: already holding {} levels",
                            manager.max_resident
                        )),
                    },
                    Name::new(format!("Stage({}) [refused]", request.path)),
                ))
                .id();
            manager.resident.insert(request.id, entity);
            continue;
        }

        let handle: Handle<Gltf> = assets.load(request.path.clone());

        // Hidden until placed. An anchored stage has no meaningful position
        // until its asset arrives, and a level flashing at the origin for a
        // frame is exactly the kind of seam this crate exists to avoid.
        let entity = commands
            .spawn((
                StageRoot {
                    id: request.id,
                    path: request.path.clone(),
                    status: StageStatus::Loading,
                },
                Name::new(format!("Stage({})", request.path)),
                Visibility::Hidden,
                PendingStage {
                    handle,
                    placement: request.placement,
                },
            ))
            .id();

        manager.resident.insert(request.id, entity);
    }
}

/// Once a stage's asset is ready, place it and spawn its scene.
///
/// Placement is computed from the asset before the scene is spawned, so the
/// level is in the right place on the first frame it exists rather than
/// snapping into position afterwards.
pub fn spawn_loaded_stages(
    mut stages: Query<(Entity, &mut StageRoot, &PendingStage)>,
    gltfs: Res<Assets<Gltf>>,
    nodes: Res<Assets<GltfNode>>,
    assets: Res<AssetServer>,
    config: Res<StageTagConfig>,
    mut commands: Commands,
) {
    for (entity, mut root, pending) in &mut stages {
        if !matches!(root.status, StageStatus::Loading) {
            continue;
        }

        if let bevy::asset::LoadState::Failed(error) = assets.load_state(&pending.handle) {
            root.status = StageStatus::Failed(error.to_string());
            commands.entity(entity).remove::<PendingStage>();
            continue;
        }

        let Some(gltf) = gltfs.get(&pending.handle) else {
            continue;
        };
        let Some(scene) = gltf.default_scene.clone() else {
            root.status =
                StageStatus::Failed(format!("`{}` declares no default scene", root.path));
            commands.entity(entity).remove::<PendingStage>();
            continue;
        };

        let anchors: HashMap<String, GlobalTransform> =
            find_all_anchors(gltf, &nodes, &config.discriminator)
                .into_iter()
                .collect();

        let transform = match &pending.placement {
            Placement::Fixed(transform) => *transform,
            Placement::Anchored {
                anchor,
                target,
                alignment,
            } => {
                let Some(local) = find_anchor(gltf, &nodes, anchor, &config.discriminator) else {
                    root.status = StageStatus::Failed(format!(
                        "`{}` has no anchor named `{}` (found: {})",
                        root.path,
                        anchor,
                        anchors
                            .keys()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    commands.entity(entity).remove::<PendingStage>();
                    continue;
                };
                align_stage_to(target, &local, *alignment)
            }
        };

        root.status = StageStatus::Spawning;
        commands
            .entity(entity)
            .remove::<PendingStage>()
            .insert((
                transform,
                Visibility::Inherited,
                StageAnchors(anchors),
                WorldAssetRoot(scene),
            ));
    }
}

/// Promote stages whose scene has finished spawning.
pub fn finish_stage_spawns(
    mut stages: Query<(&mut StageRoot, &WorldInstance)>,
    spawner: Res<WorldInstanceSpawner>,
) {
    for (mut root, instance) in &mut stages {
        if matches!(root.status, StageStatus::Spawning) && spawner.instance_is_ready(**instance) {
            root.status = StageStatus::Ready;
        }
    }
}

/// Remove stages that were asked to unload.
pub fn process_stage_unloads(mut manager: ResMut<StageManager>, mut commands: Commands) {
    let unloads = core::mem::take(&mut manager.unloads);
    for id in unloads {
        if let Some(entity) = manager.resident.remove(&id) {
            // `despawn` recurses in Bevy 0.19, so everything the level spawned
            // goes with it, releasing the GPU assets it was holding.
            commands.entity(entity).despawn();
        }
        if manager.active == Some(id) {
            manager.active = None;
        }
    }
}
