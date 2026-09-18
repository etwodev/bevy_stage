//! Remembering what the player changed, across leaving and returning.

mod common;

use bevy::prelude::*;
use bevy_stage::prelude::*;

use common::{start, test_app, wait_for_stage};

/// A door authored in `stage_b.gltf` as `stage_tag: door, locked: true`.
#[derive(Component, serde::Deserialize, Reflect, Debug, Default, PartialEq)]
#[reflect(Component)]
struct Door {
    #[serde(default)]
    locked: bool,
}

/// A marker so the fixture's spawn points are tagged nodes. Only tagged nodes
/// get a stable identity, and so only they can be remembered.
#[derive(Component, serde::Deserialize, Reflect, Debug, Default)]
#[reflect(Component)]
struct SpawnPoint;

fn app() -> App {
    let mut app = test_app();
    app.register_stage_tag::<Door>("door")
        .register_stage_tag::<SpawnPoint>("spawn_point")
        .persist_stage_component::<Door>();
    start(&mut app);
    app
}

/// Load the vault, returning its stage id.
fn load_vault(app: &mut App) -> StageId {
    let id = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_b.gltf");
    wait_for_stage(app, id);
    app.update();
    id
}

fn door_state(app: &mut App) -> Option<bool> {
    let mut query = app.world_mut().query::<&Door>();
    query.iter(app.world()).next().map(|door| door.locked)
}

fn unload(app: &mut App, id: StageId) {
    app.world_mut().resource_mut::<StageManager>().unload(id);
    for _ in 0..3 {
        app.update();
    }
}

#[test]
fn the_level_starts_in_the_state_the_artist_authored() {
    let mut app = app();
    load_vault(&mut app);
    assert_eq!(
        door_state(&mut app),
        Some(true),
        "the door is authored locked"
    );
}

#[test]
fn a_change_survives_leaving_and_returning() {
    let mut app = app();
    let first = load_vault(&mut app);

    // The player unlocks the door.
    let mut query = app.world_mut().query::<&mut Door>();
    query.iter_mut(app.world_mut()).next().unwrap().locked = false;

    unload(&mut app, first);
    assert_eq!(door_state(&mut app), None, "the level should be gone");

    load_vault(&mut app);
    assert_eq!(
        door_state(&mut app),
        Some(false),
        "the door should still be unlocked on returning"
    );
}

#[test]
fn something_destroyed_stays_destroyed() {
    let mut app = app();
    let first = load_vault(&mut app);

    // Take the obelisk's spawn point out of the world, as picking up an item
    // would.
    let mut named = app.world_mut().query::<(Entity, &Name)>();
    let target = named
        .iter(app.world())
        .find(|(_, name)| name.as_str() == "SpawnPoint.001")
        .map(|(entity, _)| entity)
        .expect("fixture has a spawn point");
    app.world_mut().entity_mut(target).despawn();
    app.update();

    unload(&mut app, first);
    load_vault(&mut app);

    let came_back = app
        .world_mut()
        .query::<&Name>()
        .iter(app.world())
        .any(|name| name.as_str() == "SpawnPoint.001");
    assert!(
        !came_back,
        "a node destroyed during play must not reappear when the level reloads"
    );

    // ...but everything else does come back.
    let obelisk = app
        .world_mut()
        .query::<&Name>()
        .iter(app.world())
        .any(|name| name.as_str() == "Obelisk");
    assert!(obelisk, "untouched geometry should still load");
}

#[test]
fn forgetting_a_level_restores_it_to_how_it_was_authored() {
    let mut app = app();
    let first = load_vault(&mut app);

    let mut query = app.world_mut().query::<&mut Door>();
    query.iter_mut(app.world_mut()).next().unwrap().locked = false;
    unload(&mut app, first);

    // Starting a new game.
    app.world_mut()
        .resource_mut::<StageDeltas>()
        .clear("levels/stage_b.gltf");

    load_vault(&mut app);
    assert_eq!(
        door_state(&mut app),
        Some(true),
        "clearing the record should bring back the authored state"
    );
}

#[test]
fn a_level_never_visited_has_nothing_remembered() {
    let mut app = app();
    assert!(app.world().resource::<StageDeltas>().is_empty());

    let id = load_vault(&mut app);
    // Still nothing until it is put away.
    assert!(app.world().resource::<StageDeltas>().is_empty());

    unload(&mut app, id);
    assert!(!app.world().resource::<StageDeltas>().is_empty());
}
