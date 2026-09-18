//! The core idea: an artist tags a node, the game decides what it means.
//!
//! Run with `cargo run --example custom_tags`.
//!
//! `assets/levels/stage_a.gltf` contains four nodes that mean "spawn point",
//! each tagged a different way — by name, by custom property, and by reflected
//! component. All four arrive here as the same component.

use bevy::prelude::*;
use bevy_stage::prelude::*;

/// A tag this game defines. `bevy_stage` knows nothing about this type.
#[derive(Component, serde::Deserialize, Reflect, Debug, Default)]
#[reflect(Component)]
struct SpawnPoint {
    /// Filled in from the tag's `team` property, where the artist set one.
    #[serde(default)]
    team: String,
}

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, StagePlugin::default()))
        .register_stage_tag::<SpawnPoint>("spawn_point")
        .add_systems(Startup, (setup_view, load_level))
        .add_systems(Update, mark_spawn_points)
        .run();
}

fn setup_view(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(18.0, 14.0, 18.0).looking_at(Vec3::new(0.0, 0.0, -4.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 8_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(6.0, 12.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn load_level(mut stages: ResMut<StageManager>) {
    stages.load("levels/stage_a.gltf");
}

/// Put a marker on every spawn point the level declared.
///
/// An ordinary Bevy system: by the time it runs, the tag is a component on the
/// entity the glTF loader built for that node, so the authored position is
/// already there.
fn mark_spawn_points(
    spawns: Query<(Entity, &SpawnPoint, &GlobalTransform), Added<SpawnPoint>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (entity, spawn, transform) in &spawns {
        let colour = match spawn.team.as_str() {
            "red" => Color::srgb(0.9, 0.2, 0.2),
            "blue" => Color::srgb(0.2, 0.4, 0.9),
            _ => Color::srgb(0.8, 0.8, 0.2),
        };

        info!(
            "spawn point (team {:?}) at {}",
            spawn.team,
            transform.translation()
        );

        // Parented to the tagged node, so it inherits the authored placement.
        commands.entity(entity).with_child((
            Mesh3d(meshes.add(Capsule3d::new(0.35, 1.0))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: colour,
                emissive: colour.to_linear() * 0.4,
                ..default()
            })),
        ));
    }
}
