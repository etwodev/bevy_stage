//! End-to-end: load a real glTF file and check that artist-authored tags
//! become live components.

mod common;

use bevy::prelude::*;
use bevy_stage::prelude::*;
use common::{spawn_level, start, test_app};

/// A tag the "game" defines. The plugin knows nothing about this type.
#[derive(Component, serde::Deserialize, Debug, Default, Reflect)]
#[reflect(Component)]
struct SpawnPoint {
    #[serde(default)]
    team: String,
}

/// A second tag, to check that one level can drive several.
#[derive(Component, serde::Deserialize, Debug, Default)]
struct Anchor {
    #[serde(default)]
    id: String,
}

fn app() -> App {
    let mut app = test_app();
    app.register_type::<SpawnPoint>()
        .register_stage_tag::<SpawnPoint>("spawn_point")
        .register_stage_tag::<Anchor>("anchor");
    start(&mut app);
    app
}

#[test]
fn a_node_named_like_a_tag_becomes_a_component() {
    let mut app = app();
    spawn_level(&mut app, "levels/stage_a.gltf");

    let spawns: Vec<String> = app
        .world_mut()
        .query::<&SpawnPoint>()
        .iter(app.world())
        .map(|s| s.team.clone())
        .collect();

    // stage_a.gltf has four spawn points across three authoring channels:
    // two named `SpawnPoint.001`/`.002`, one via the discriminator property
    // (team "blue"), and one via a reflected RON literal (team "red").
    assert_eq!(
        spawns.len(),
        4,
        "expected every authoring channel to produce a SpawnPoint, got {spawns:?}"
    );

    let mut teams: Vec<&str> = spawns.iter().map(String::as_str).collect();
    teams.sort_unstable();
    assert_eq!(teams, ["", "", "blue", "red"], "got {spawns:?}");
}

#[test]
fn tags_land_on_the_node_entity_keeping_its_transform_and_name() {
    let mut app = app();
    spawn_level(&mut app, "levels/stage_a.gltf");

    let mut query = app
        .world_mut()
        .query_filtered::<(&Name, &Transform), With<SpawnPoint>>();
    let named: Vec<(String, Vec3)> = query
        .iter(app.world())
        .map(|(name, transform)| (name.to_string(), transform.translation))
        .collect();

    // The component must land on the entity the loader already built for the
    // node, so the authored placement comes along for free.
    let player_start = named
        .iter()
        .find(|(name, _)| name == "PlayerStart")
        .expect("PlayerStart node should carry the tag");
    assert_eq!(player_start.1, Vec3::new(-3.0, 1.0, 0.0));

    // Blender's duplicate suffix is stripped for matching but the entity keeps
    // its real name, so the level stays debuggable.
    assert!(named.iter().any(|(name, _)| name == "SpawnPoint.001"));
}

#[test]
fn a_second_tag_type_resolves_from_the_same_level() {
    let mut app = app();
    spawn_level(&mut app, "levels/stage_a.gltf");

    let anchors: Vec<String> = app
        .world_mut()
        .query::<&Anchor>()
        .iter(app.world())
        .map(|a| a.id.clone())
        .collect();

    assert_eq!(anchors, vec!["exit_north".to_string()]);
}

#[test]
fn untagged_geometry_is_left_alone() {
    let mut app = app();
    spawn_level(&mut app, "levels/stage_a.gltf");

    // `Ground` is ordinary geometry; it must not acquire gameplay components
    // and must not be reported as an unknown tag.
    let ground_is_tagged = app
        .world_mut()
        .query_filtered::<&Name, With<SpawnPoint>>()
        .iter(app.world())
        .any(|name| name.as_str() == "Ground");
    assert!(!ground_is_tagged);

    let unknown: Vec<String> = app
        .world()
        .resource::<bevy_stage::tag::UnknownTags>()
        .iter()
        .cloned()
        .collect();
    // `Ground` is ordinary geometry and must never be mistaken for a tag.
    assert!(
        !unknown.contains(&"Ground".to_string()),
        "plain geometry was mistaken for a tag: {unknown:?}"
    );
}

#[test]
fn an_unregistered_tag_is_reported_rather_than_silently_dropped() {
    let mut app = app();
    spawn_level(&mut app, "levels/stage_a.gltf");

    let unknown: Vec<String> = app
        .world()
        .resource::<bevy_stage::tag::UnknownTags>()
        .iter()
        .cloned()
        .collect();

    // The fixture deliberately contains a tag nothing is registered under.
    assert!(
        unknown.contains(&"not_a_real_tag".to_string()),
        "expected the unregistered tag to be reported, got {unknown:?}"
    );
}
