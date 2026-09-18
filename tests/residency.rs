//! Several levels resident at once, positioned against each other.

mod common;

use bevy::prelude::*;
use bevy_stage::stage::{AnchorAlignment, StageManager, StageRoot, StageStatus};

use common::{anchor_in_world, start, test_app, wait_for_stage};

#[test]
fn a_stage_loads_spawns_and_reports_ready() {
    let mut app = test_app();
    start(&mut app);

    let id = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, id);

    assert!(app.world().resource::<StageManager>().is_resident(id));
    assert_eq!(app.world().resource::<StageManager>().len(), 1);

    // The level's contents are real entities in the world, under the stage root.
    let ground = app
        .world_mut()
        .query::<&Name>()
        .iter(app.world())
        .any(|name| name.as_str() == "Ground");
    assert!(ground, "the level's geometry should be spawned");
}

#[test]
fn two_stages_are_resident_at_once_and_meet_at_their_anchors() {
    let mut app = test_app();
    start(&mut app);

    let a = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, a);

    // Where stage A's north exit is in the world.
    let door = anchor_in_world(&mut app, a, "exit_north");

    // Bring the hallway in against that door, which is exactly what a
    // transition does.
    let hall = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load_connected(
            "levels/hallway.gltf",
            "entry_a",
            door,
            AnchorAlignment::Facing,
        );
    wait_for_stage(&mut app, hall);

    assert_eq!(
        app.world().resource::<StageManager>().len(),
        2,
        "both levels should be resident simultaneously"
    );

    // The seam: the hallway's entry must land on stage A's door.
    let entry = anchor_in_world(&mut app, hall, "entry_a");
    assert!(
        entry.translation().distance(door.translation()) < 1e-3,
        "hallway entry at {:?}, stage A door at {:?}",
        entry.translation(),
        door.translation()
    );

    // And the hallway must extend away from stage A rather than back over it.
    let exit = anchor_in_world(&mut app, hall, "exit_b");
    assert!(
        exit.translation().z < door.translation().z,
        "hallway runs the wrong way: exit at {:?}, door at {:?}",
        exit.translation(),
        door.translation()
    );
}

#[test]
fn unloading_removes_the_level_and_everything_it_spawned() {
    let mut app = test_app();
    start(&mut app);

    let id = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, id);
    let before = app.world_mut().query::<&Name>().iter(app.world()).count();
    assert!(before > 0);

    app.world_mut().resource_mut::<StageManager>().unload(id);
    app.update();
    app.update();

    assert!(!app.world().resource::<StageManager>().is_resident(id));
    let ground_left = app
        .world_mut()
        .query::<&Name>()
        .iter(app.world())
        .any(|name| name.as_str() == "Ground");
    assert!(!ground_left, "the level's entities should be gone");
}

#[test]
fn a_missing_anchor_fails_the_stage_with_a_message_naming_the_options() {
    let mut app = test_app();
    start(&mut app);

    let id = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load_connected(
            "levels/hallway.gltf",
            "no_such_anchor",
            GlobalTransform::IDENTITY,
            AnchorAlignment::Facing,
        );

    common::run_until(&mut app, "the stage to fail", |app| {
        let mut query = app.world_mut().query::<&StageRoot>();
        query
            .iter(app.world())
            .any(|root| matches!(root.status, StageStatus::Failed(_)))
    });

    let mut query = app.world_mut().query::<&StageRoot>();
    let message = query
        .iter(app.world())
        .find(|root| root.id == id)
        .and_then(|root| match &root.status {
            StageStatus::Failed(message) => Some(message.clone()),
            _ => None,
        })
        .expect("the stage should have failed");

    // The message should say what anchors the level actually has, so the fix
    // is obvious without opening the file in Blender.
    assert!(message.contains("no_such_anchor"), "{message}");
    assert!(message.contains("entry_a"), "{message}");
}
