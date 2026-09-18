//! LOD groups authored as sibling nodes, wired automatically.

mod common;

use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;
use bevy_stage::stage::StageManager;
use bevy_stage::view::{LodConfig, StageViewConfig};

use common::{start, test_app, wait_for_stage};

/// The visibility range attached to a LOD node, found via its mesh child.
fn range_for(app: &mut App, node_name: &str) -> Option<VisibilityRange> {
    let mut named = app.world_mut().query::<(Entity, &Name)>();
    let node = named
        .iter(app.world())
        .find(|(_, name)| name.as_str() == node_name)
        .map(|(entity, _)| entity)?;

    // The range goes on the mesh entity, which in a glTF is a child of the node.
    if let Some(range) = app.world().get::<VisibilityRange>(node) {
        return Some(range.clone());
    }
    let children: Vec<Entity> = app
        .world()
        .get::<Children>(node)
        .map(|c| c.iter().collect())
        .unwrap_or_default();
    children
        .into_iter()
        .find_map(|child| app.world().get::<VisibilityRange>(child).cloned())
}

#[test]
fn lod_siblings_are_wired_into_a_chain() {
    let mut app = test_app();
    start(&mut app);

    let id = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, id);
    app.update();

    let lod0 = range_for(&mut app, "Rock_LOD0").expect("Rock_LOD0 should be wired");
    let lod1 = range_for(&mut app, "Rock_LOD1").expect("Rock_LOD1 should be wired");
    let lod2 = range_for(&mut app, "Rock_LOD2").expect("Rock_LOD2 should be wired");

    // Visible from the camera outwards.
    assert_eq!(lod0.start_margin, 0.0..0.0);
    // Each level takes over exactly where the last lets go.
    assert_eq!(lod0.end_margin, lod1.start_margin);
    assert_eq!(lod1.end_margin, lod2.start_margin);
    // The coarsest level never disappears, so there is no hole in the distance.
    assert_eq!(lod2.end_margin.start, f32::MAX);

    // And the thresholds match the configured distances.
    let config = app.world().resource::<LodConfig>().clone();
    assert!(
        (lod0.end_margin.start - (config.distances[0] - config.crossfade / 2.0)).abs() < 1e-3,
        "LOD0 hands over at {:?}, expected around {}",
        lod0.end_margin,
        config.distances[0]
    );
}

#[test]
fn ordinary_meshes_are_left_without_a_range() {
    let mut app = test_app();
    start(&mut app);

    let id = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, id);
    app.update();

    // `Ground` is not part of a LOD group and must render at every distance.
    assert!(
        range_for(&mut app, "Ground").is_none(),
        "a mesh outside a LOD group must not get a visibility range"
    );
}

#[test]
fn lod_wiring_can_be_turned_off() {
    let mut app = test_app();
    app.world_mut().resource_mut::<StageViewConfig>().auto_lod = false;
    start(&mut app);

    let id = app
        .world_mut()
        .resource_mut::<StageManager>()
        .load("levels/stage_a.gltf");
    wait_for_stage(&mut app, id);
    app.update();

    assert!(range_for(&mut app, "Rock_LOD0").is_none());
}
