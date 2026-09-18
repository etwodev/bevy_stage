//! Sectors coming in and out around a moving player.

mod common;

use bevy::prelude::*;
use bevy_stage::stage::{StageManager, StageRoot, StreamingSource};

use common::{run_until, start, test_app, wait_for_stage};

fn resident_count(app: &mut App, path: &str) -> usize {
    let mut query = app.world_mut().query::<&StageRoot>();
    query
        .iter(app.world())
        .filter(|root| root.path == path)
        .count()
}

/// Load stage A and put a player at `position`.
fn level_with_player(position: Vec3) -> (App, Entity) {
    let mut app = test_app();
    start(&mut app);
    let a = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, a);
    app.world_mut().resource_mut::<StageManager>().activate(a);

    let player = app
        .world_mut()
        .spawn((
            Name::new("Player"),
            // A large unload distance so this test exercises sector streaming
            // rather than whole-level unloading.
            StreamingSource {
                unload_distance: 10_000.0,
            },
            Transform::from_translation(position),
            Visibility::default(),
        ))
        .id();
    app.update();
    (app, player)
}

fn move_to(app: &mut App, player: Entity, position: Vec3) {
    app.world_mut()
        .get_mut::<Transform>(player)
        .unwrap()
        .translation = position;
    for _ in 0..4 {
        app.update();
    }
}

#[test]
fn a_distant_sector_is_not_in_memory() {
    // The sector node sits at x = 60 with a radius of 50; the player starts at
    // the origin, so it should not have loaded.
    let (mut app, _player) = level_with_player(Vec3::new(0.0, 1.0, 0.0));
    for _ in 0..10 {
        app.update();
    }

    assert_eq!(
        resident_count(&mut app, "levels/sector_east.gltf"),
        0,
        "a sector beyond its radius should not be loaded"
    );
}

#[test]
fn approaching_a_sector_streams_it_in() {
    let (mut app, player) = level_with_player(Vec3::new(0.0, 1.0, 0.0));
    move_to(&mut app, player, Vec3::new(30.0, 1.0, 0.0));

    run_until(&mut app, "the east sector to stream in", |app| {
        resident_count(app, "levels/sector_east.gltf") == 1
    });

    // Its contents are real entities, and they went through the tag system
    // like any other level.
    let tower = app
        .world_mut()
        .query::<&Name>()
        .iter(app.world())
        .any(|name| name.as_str() == "EastTower");
    assert!(tower, "the sector's geometry should be in the world");
}

#[test]
fn walking_away_streams_the_sector_back_out() {
    let (mut app, player) = level_with_player(Vec3::new(0.0, 1.0, 0.0));
    move_to(&mut app, player, Vec3::new(30.0, 1.0, 0.0));
    run_until(&mut app, "the east sector to stream in", |app| {
        resident_count(app, "levels/sector_east.gltf") == 1
    });

    // Past the unload radius (50 * 1.3 = 65 from the node at x = 60).
    move_to(&mut app, player, Vec3::new(-60.0, 1.0, 0.0));
    run_until(&mut app, "the east sector to stream out", |app| {
        resident_count(app, "levels/sector_east.gltf") == 0
    });

    let tower_left = app
        .world_mut()
        .query::<&Name>()
        .iter(app.world())
        .any(|name| name.as_str() == "EastTower");
    assert!(!tower_left, "the sector's entities should be gone");
}

#[test]
fn loitering_on_the_boundary_does_not_reload_the_sector_every_frame() {
    let (mut app, player) = level_with_player(Vec3::new(0.0, 1.0, 0.0));

    // The sector node is at x = 60, radius 50, so the load boundary is x = 10.
    // Hover either side of it. Without hysteresis this thrashes.
    move_to(&mut app, player, Vec3::new(12.0, 1.0, 0.0));
    run_until(&mut app, "the east sector to stream in", |app| {
        resident_count(app, "levels/sector_east.gltf") == 1
    });

    for step in 0..20 {
        let x = if step % 2 == 0 { 8.0 } else { 12.0 };
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = Vec3::new(x, 1.0, 0.0);
        app.update();
        assert_eq!(
            resident_count(&mut app, "levels/sector_east.gltf"),
            1,
            "the sector should stay resident while hovering near the boundary \
             (step {step}, x = {x})"
        );
    }
}

#[test]
fn an_inline_sector_is_hidden_rather_than_unloaded() {
    let (mut app, player) = level_with_player(Vec3::new(0.0, 1.0, 0.0));

    // `Sector_North` has no `source`, so its contents ship with the level.
    // Far away it should be hidden, but still present.
    move_to(&mut app, player, Vec3::new(0.0, 1.0, 400.0));

    let mut query = app.world_mut().query::<(&Name, &Visibility)>();
    let north = query
        .iter(app.world())
        .find(|(name, _)| name.as_str() == "Sector_North")
        .map(|(_, visibility)| *visibility);

    assert_eq!(
        north,
        Some(Visibility::Hidden),
        "a distant inline sector should stop rendering"
    );
    let pillar_present = app
        .world_mut()
        .query::<&Name>()
        .iter(app.world())
        .any(|name| name.as_str() == "Pillar");
    assert!(
        pillar_present,
        "an inline sector's entities stay in the world; only visibility changes"
    );
}
