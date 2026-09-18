//! Walking between levels with no loading screen.
//!
//! Run with `cargo run --example seamless`, then drive with WASD.
//!
//! Head north (W) out of the atrium. The corridor loads while you are still
//! walking toward it; entering the corridor is what starts loading the vault at
//! its far end. Nothing here knows what a corridor is — it is just a small
//! level with a portal at each end.

use bevy::prelude::*;
use bevy_stage::prelude::*;

#[derive(Component, serde::Deserialize, Reflect, Debug, Default)]
#[reflect(Component)]
struct SpawnPoint {
    #[serde(default)]
    team: String,
}

#[derive(Component)]
struct Player;

#[derive(Component)]
struct Readout;

const WALK_SPEED: f32 = 14.0;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, StagePlugin::default()))
        .register_stage_tag::<SpawnPoint>("spawn_point")
        .add_systems(Startup, (setup, load_first_level))
        .add_systems(
            Update,
            (drive_player, follow_with_camera, update_readout, log_level_changes),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 12.0, 16.0).looking_at(Vec3::ZERO, Vec3::Y),
        // Fog hides the far edge of what is loaded, which is the cheapest way
        // to stop distant streaming from being visible as pop-in.
        DistanceFog {
            color: Color::srgb(0.5, 0.55, 0.62),
            falloff: FogFalloff::from_visibility(320.0),
            ..default()
        },
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(8.0, 16.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // The player. `StreamingSource` is what portals and sectors measure
    // distance from — without one, nothing ever streams.
    commands.spawn((
        Player,
        Name::new("Player"),
        StreamingSource::default(),
        Mesh3d(meshes.add(Capsule3d::new(0.4, 1.2))),
        MeshMaterial3d(materials.add(Color::srgb(0.95, 0.85, 0.3))),
        Transform::from_xyz(0.0, 1.0, 6.0),
    ));

    commands.spawn((
        Readout,
        Text::new("loading..."),
        TextFont {
            font_size: bevy::text::FontSize::Px(15.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(12.0),
            ..default()
        },
    ));
}

fn load_first_level(mut stages: ResMut<StageManager>) {
    let atrium = stages.load("levels/stage_a.gltf");
    // Marking the starting level active stops it being unloaded as the player
    // walks away from its origin.
    stages.activate(atrium);
}

fn drive_player(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut player: Query<&mut Transform, With<Player>>,
) {
    let mut direction = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        direction.z -= 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        direction.z += 1.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        direction.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        direction.x += 1.0;
    }

    let Some(direction) = direction.try_normalize() else {
        return;
    };
    for mut transform in &mut player {
        transform.translation += direction * WALK_SPEED * time.delta_secs();
    }
}

fn follow_with_camera(
    player: Query<&Transform, With<Player>>,
    mut camera: Query<&mut Transform, (With<Camera3d>, Without<Player>)>,
) {
    let Ok(player) = player.single() else {
        return;
    };
    for mut camera in &mut camera {
        let wanted = player.translation + Vec3::new(0.0, 11.0, 14.0);
        camera.translation = camera.translation.lerp(wanted, 0.1);
        camera.look_at(player.translation, Vec3::Y);
    }
}

/// Log levels arriving and leaving, so the streaming is visible in the console
/// as well as on screen.
fn log_level_changes(
    arrived: Query<&StageRoot, Added<StageRoot>>,
    player: Query<&GlobalTransform, With<Player>>,
) {
    let at = player
        .single()
        .map(|t| t.translation())
        .unwrap_or(Vec3::ZERO);
    for root in &arrived {
        info!("level requested: {} (player at {at})", root.path);
    }
}

/// Show what is resident, so the streaming is visible rather than magic.
fn update_readout(
    stages: Res<StageManager>,
    roots: Query<&StageRoot>,
    mut readout: Query<&mut Text, With<Readout>>,
) {
    let active = stages.active();
    let mut lines = vec!["WASD to walk. Head north (W) out of the atrium.".to_string(), String::new()];

    let mut rows: Vec<String> = roots
        .iter()
        .map(|root| {
            let marker = if Some(root.id) == active { ">" } else { " " };
            let status = match &root.status {
                StageStatus::Loading => "loading".to_string(),
                StageStatus::Spawning => "spawning".to_string(),
                StageStatus::Ready => "ready".to_string(),
                StageStatus::Failed(error) => format!("failed: {error}"),
            };
            format!("{marker} {}  [{status}]", root.path)
        })
        .collect();
    rows.sort();
    lines.extend(rows);

    for mut text in &mut readout {
        **text = lines.join("\n");
    }
}
