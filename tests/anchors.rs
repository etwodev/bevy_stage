//! Reading connection points straight out of the glTF asset.

mod common;

use bevy::gltf::{Gltf, GltfNode};
use bevy::prelude::*;
use bevy_stage::stage::{align_stage_to, find_all_anchors, find_anchor, AnchorAlignment};
use bevy_stage::tag::DEFAULT_DISCRIMINATOR;

/// Run `f` with the loaded level's asset data.
fn with_level<R>(path: &str, f: impl FnOnce(&Gltf, &Assets<GltfNode>) -> R) -> R {
    let mut app = common::test_app();
    common::start(&mut app);
    let handle = common::load_gltf(&mut app, path);

    let world = app.world();
    let gltf = world.resource::<Assets<Gltf>>().get(&handle).unwrap();
    let nodes = world.resource::<Assets<GltfNode>>();
    f(gltf, nodes)
}

#[test]
fn an_anchor_is_found_without_spawning_anything() {
    let found = with_level("levels/stage_a.gltf", |gltf, nodes| {
        find_anchor(gltf, nodes, "exit_north", DEFAULT_DISCRIMINATOR)
    });

    let anchor = found.expect("stage_a declares an `exit_north` anchor");
    // Authored at (0, 0, -20) in the fixture.
    assert!(
        anchor.translation().distance(Vec3::new(0.0, 0.0, -20.0)) < 1e-3,
        "anchor at {:?}",
        anchor.translation()
    );
}

#[test]
fn both_ends_of_the_hallway_are_found() {
    let anchors = with_level("levels/hallway.gltf", |gltf, nodes| {
        let mut found = find_all_anchors(gltf, nodes, DEFAULT_DISCRIMINATOR);
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    });

    let ids: Vec<&str> = anchors.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["entry_a", "exit_b"]);
}

#[test]
fn a_missing_anchor_is_reported_as_absent_not_guessed() {
    let found = with_level("levels/stage_a.gltf", |gltf, nodes| {
        find_anchor(gltf, nodes, "no_such_anchor", DEFAULT_DISCRIMINATOR)
    });
    assert!(found.is_none());
}

#[test]
fn a_hallway_placed_against_a_door_has_its_entry_on_that_door() {
    // The end-to-end placement claim, on real authored data: load stage A,
    // find its exit; load the hallway, find its entry; align and check the
    // two land on each other.
    let mut app = common::test_app();
    common::start(&mut app);
    let a = common::load_gltf(&mut app, "levels/stage_a.gltf");
    let hall = common::load_gltf(&mut app, "levels/hallway.gltf");

    let world = app.world();
    let gltfs = world.resource::<Assets<Gltf>>();
    let nodes = world.resource::<Assets<GltfNode>>();

    let door = find_anchor(
        gltfs.get(&a).unwrap(),
        nodes,
        "exit_north",
        DEFAULT_DISCRIMINATOR,
    )
    .expect("stage_a exit");
    let entry = find_anchor(
        gltfs.get(&hall).unwrap(),
        nodes,
        "entry_a",
        DEFAULT_DISCRIMINATOR,
    )
    .expect("hallway entry");

    let hall_root = align_stage_to(&door, &entry, AnchorAlignment::Facing);
    let entry_in_world = GlobalTransform::from(hall_root) * entry;

    assert!(
        entry_in_world
            .translation()
            .distance(door.translation())
            < 1e-3,
        "hallway entry landed at {:?}, stage A's door is at {:?}",
        entry_in_world.translation(),
        door.translation()
    );

    // The hallway is 30 units long with its entry at +15, so with a facing
    // connection its far end must sit beyond the door, not back inside stage A.
    let exit = find_anchor(
        gltfs.get(&hall).unwrap(),
        nodes,
        "exit_b",
        DEFAULT_DISCRIMINATOR,
    )
    .unwrap();
    let exit_in_world = GlobalTransform::from(hall_root) * exit;
    assert!(
        exit_in_world.translation().z < door.translation().z,
        "hallway should extend away from stage A: exit at {:?}, door at {:?}",
        exit_in_world.translation(),
        door.translation()
    );
}
