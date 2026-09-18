//! The headline behaviour: walking between levels with no loading screen.

mod common;

use bevy::prelude::*;
use bevy_stage::stage::{StageManager, StageRoot, StageStatus, StreamingSource};

use common::{run_until, start, test_app, wait_for_stage};

/// Put a mover in the world at `position` and let a frame settle so portals
/// record which side of them it started on.
fn spawn_player(app: &mut App, position: Vec3) -> Entity {
    let player = app
        .world_mut()
        .spawn((
            Name::new("Player"),
            StreamingSource::default(),
            Transform::from_translation(position),
            Visibility::default(),
        ))
        .id();
    app.update();
    player
}

/// Walk to `position` over several frames, so crossing detection sees the
/// move rather than a teleport it could miss.
fn walk_to(app: &mut App, player: Entity, position: Vec3) {
    let start = app.world().get::<Transform>(player).unwrap().translation;
    for step in 1..=8 {
        let t = step as f32 / 8.0;
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = start.lerp(position, t);
        app.update();
    }
}

/// How many copies of a level are resident.
fn copies_of(app: &mut App, path: &str) -> usize {
    let mut query = app.world_mut().query::<&StageRoot>();
    query
        .iter(app.world())
        .filter(|root| root.path == path)
        .count()
}

fn resident_paths(app: &mut App) -> Vec<String> {
    let mut query = app.world_mut().query::<&StageRoot>();
    let mut paths: Vec<String> = query.iter(app.world()).map(|r| r.path.clone()).collect();
    paths.sort();
    paths
}

#[test]
fn approaching_a_portal_loads_the_level_behind_it_in_the_background() {
    let mut app = test_app();
    start(&mut app);

    let a = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, a);
    app.world_mut().resource_mut::<StageManager>().activate(a);

    // Nothing else is resident until something comes near the portal.
    assert_eq!(app.world().resource::<StageManager>().len(), 1);

    spawn_player(&mut app, Vec3::new(0.0, 1.0, 0.0));

    run_until(&mut app, "the hallway to load", |app| {
        copies_of(app, "levels/hallway.gltf") == 1
    });

    // The player is still standing in stage A: the next level arrived without
    // anything being unloaded or interrupted.
    assert_eq!(
        app.world().resource::<StageManager>().active(),
        Some(a),
        "the player should still be in stage A while the hallway loads"
    );
    assert_eq!(copies_of(&mut app, "levels/stage_a.gltf"), 1);
}

#[test]
fn walking_through_a_portal_hands_over_to_the_next_level() {
    let mut app = test_app();
    start(&mut app);

    let a = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, a);
    app.world_mut().resource_mut::<StageManager>().activate(a);

    let player = spawn_player(&mut app, Vec3::new(0.0, 1.0, 0.0));
    run_until(&mut app, "the hallway to be ready", |app| {
        let mut query = app.world_mut().query::<&StageRoot>();
        query
            .iter(app.world())
            .any(|r| r.path == "levels/hallway.gltf" && r.status.is_ready())
    });

    // Walk through the doorway at z = -19.
    walk_to(&mut app, player, Vec3::new(0.0, 1.0, -24.0));

    let active = app.world().resource::<StageManager>().active();
    assert_ne!(active, Some(a), "crossing the portal should hand over");

    let mut query = app.world_mut().query::<&StageRoot>();
    let active_path = query
        .iter(app.world())
        .find(|r| Some(r.id) == active)
        .map(|r| r.path.clone());
    assert_eq!(active_path.as_deref(), Some("levels/hallway.gltf"));
}

#[test]
fn the_interstitial_level_pays_for_the_next_load() {
    // This is the elevator/hallway case. Nothing in the crate knows what a
    // hallway is: entering it simply brings the player within range of the
    // portal at its far end, which starts loading the destination while the
    // player is still walking.
    let mut app = test_app();
    start(&mut app);

    let a = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, a);
    app.world_mut().resource_mut::<StageManager>().activate(a);

    let player = spawn_player(&mut app, Vec3::new(0.0, 1.0, 0.0));
    run_until(&mut app, "the hallway to be ready", |app| {
        let mut query = app.world_mut().query::<&StageRoot>();
        query
            .iter(app.world())
            .any(|r| r.path == "levels/hallway.gltf" && r.status.is_ready())
    });

    walk_to(&mut app, player, Vec3::new(0.0, 1.0, -24.0));

    run_until(&mut app, "the vault to load from inside the hallway", |app| {
        copies_of(app, "levels/stage_b.gltf") == 1
    });

    assert_eq!(
        resident_paths(&mut app),
        vec![
            "levels/hallway.gltf".to_string(),
            "levels/stage_a.gltf".to_string(),
            "levels/stage_b.gltf".to_string(),
        ],
        "all three levels should be resident mid-transition"
    );
}

#[test]
fn walking_back_does_not_load_a_second_copy_of_where_you_came_from() {
    let mut app = test_app();
    start(&mut app);

    let a = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, a);
    app.world_mut().resource_mut::<StageManager>().activate(a);

    let player = spawn_player(&mut app, Vec3::new(0.0, 1.0, 0.0));
    run_until(&mut app, "the hallway to be ready", |app| {
        let mut query = app.world_mut().query::<&StageRoot>();
        query
            .iter(app.world())
            .any(|r| r.path == "levels/hallway.gltf" && r.status.is_ready())
    });

    // Into the hallway, then back out again.
    walk_to(&mut app, player, Vec3::new(0.0, 1.0, -24.0));
    walk_to(&mut app, player, Vec3::new(0.0, 1.0, -10.0));
    for _ in 0..30 {
        app.update();
    }

    assert_eq!(
        copies_of(&mut app, "levels/stage_a.gltf"),
        1,
        "the hallway's return portal should reuse the level the player came from"
    );
}

#[test]
fn levels_further_than_one_hop_away_stay_out_of_memory() {
    // Preloading must reach exactly one step ahead. If it reached further, a
    // connected world would pull itself entirely into memory the moment the
    // player stood still, which is the opposite of what streaming is for.
    let mut app = test_app();
    start(&mut app);

    let a = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, a);
    app.world_mut().resource_mut::<StageManager>().activate(a);

    spawn_player(&mut app, Vec3::new(0.0, 1.0, 6.0));
    for _ in 0..600 {
        app.update();
    }

    assert_eq!(
        copies_of(&mut app, "levels/hallway.gltf"),
        1,
        "the next level along should be ready"
    );
    assert_eq!(
        copies_of(&mut app, "levels/stage_b.gltf"),
        0,
        "the level beyond that should not be, while the player stands still"
    );
}

#[test]
fn a_level_that_fails_to_load_does_not_take_the_others_with_it() {
    let mut app = test_app();
    start(&mut app);

    let good = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    let bad = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/does_not_exist.gltf");

    wait_for_stage(&mut app, good);
    run_until(&mut app, "the missing level to report failure", |app| {
        let mut query = app.world_mut().query::<&StageRoot>();
        query
            .iter(app.world())
            .any(|r| r.id == bad && matches!(r.status, StageStatus::Failed(_)))
    });

    let mut query = app.world_mut().query::<&StageRoot>();
    let good_ok = query
        .iter(app.world())
        .any(|r| r.id == good && r.status.is_ready());
    assert!(good_ok, "a broken level must not break a working one");
}
