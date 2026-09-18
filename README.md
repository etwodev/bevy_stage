# bevy_stage

A Bevy plugin for loading glTF scenes with artist-authored metadata.

Levels are authored as glTF. Gameplay meaning is attached by **tagging nodes**,
so a level artist can place a spawn point in Blender and have the game pick it
up without a programmer editing a spawn table.

Targets **Bevy 0.19**.

> Status: early but complete in scope. Everything described below works and is
> covered by tests that load real glTF files. See [Roadmap](#roadmap) for what
> is still missing.

## Quick start

```rust
use bevy::prelude::*;
use bevy_stage::prelude::*;

#[derive(Component, serde::Deserialize, Default, Reflect)]
#[reflect(Component)]
struct SpawnPoint {
    #[serde(default)]
    team: String,
}

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, StagePlugin::default()))
        .register_type::<SpawnPoint>()
        .register_stage_tag::<SpawnPoint>("spawn_point")
        .add_systems(Startup, load_level)
        .add_systems(Update, use_spawn_points)
        .run();
}

fn load_level(mut stages: ResMut<StageManager>) {
    stages.load("levels/atrium.gltf");
}

// An ordinary Bevy system. By the time this runs, the component is on the
// entity the glTF loader built for that node — transform, name and parent
// included.
fn use_spawn_points(spawns: Query<(&SpawnPoint, &GlobalTransform), Added<SpawnPoint>>) {
    for (spawn, transform) in &spawns {
        info!("{} spawn at {}", spawn.team, transform.translation());
    }
}
```

## Tagging nodes

There are three ways to tag a node. They all produce the same result, so pick
whichever suits your pipeline — you can mix them in one level.

### 1. By name

Name the object after the tag. Blender's `.001` duplicate suffixes are ignored,
and casing and separators do not matter, so `SpawnPoint.003`, `spawn_point` and
`Spawn Point` all match a tag registered as `spawn_point`.

```
SpawnPoint.001          -> SpawnPoint::default()
SpawnPoint[team=red]    -> SpawnPoint { team: "red" }
```

Zero setup, works in any DCC. A bare word in the brackets is a flag:
`Door[locked]` means `locked = true`.

### 2. By custom property

Add a custom property named `stage_tag`. Its siblings become the parameters.

| Property     | Value         |
| ------------ | ------------- |
| `stage_tag`  | `spawn_point` |
| `team`       | `blue`        |

`stage_tag` may also be a list, to put several tags on one node.

### 3. By reflected component

Name the property after a registered component and give it a RON literal. Any
`Reflect` component works, with no `register_stage_tag` call needed — only
`app.register_type::<T>()`. This matches the convention
[Blenvy](https://github.com/kaosat-dev/Blender_bevy_components_workflow) users
already know.

| Property     | Value            |
| ------------ | ---------------- |
| `SpawnPoint` | `(team: "red")`  |

### Which wins

If one node is tagged more than one way, the explicit channels win over the
name: **reflected component > custom property > name**. A tag that is not
registered under any name is reported once, rather than silently ignored —
a misspelled tag in a level is otherwise very hard to find.

### Doing something other than adding a component

Registering a component covers most cases. When you need to replace a
placeholder empty with a prefab, observe `StageTagFound`, which carries the
entity, the tag and its parameters.

```rust
app.add_observer(|found: On<StageTagFound>, mut commands: Commands| {
    if found.key != "treasure" { return; }
    commands.entity(found.entity).despawn();
    // ...spawn the real thing at that transform
});
```

## Seamless transitions

Two levels can be resident at once, each placed independently, so moving
between them needs no loading screen.

### Anchors

An **anchor** is a node tagged `anchor` with an `id`. It marks a connection
point. Anchors point **out of their own level**, like a normal on the level
boundary — a door in the north wall faces north. Two connected anchors
therefore face each other, which makes connections symmetric: the same pair
places B against A and A against B with identical maths, so walking back and
forth never drifts.

### Portals

A **portal** is a node tagged `portal` that names the level on the other side.
The node's own transform is the connection point — place the empty in the
doorway, facing the way the player travels.

| Property    | Value                  |
| ----------- | ---------------------- |
| `stage_tag` | `portal`               |
| `target`    | `levels/vault.gltf`    |
| `anchor`    | `entry_south`          |
| `preload`   | `40.0`                 |

Getting within `preload` of the portal starts loading the target in the
background. Walking through it hands over. Walking far away unloads it.

### The elevator problem

There is no "loading level" feature, because it does not need one. An elevator
or a connecting hallway is **just a small level with a portal at each end**.
Entering it from one side brings the player within range of the portal at the
other, which starts loading the destination while they are still walking.

```
  atrium.gltf            hallway.gltf              vault.gltf
 ┌───────────┐          ┌─────────────┐          ┌───────────┐
 │        [P]│─────────>│[A]       [P]│─────────>│[A]        │
 └───────────┘          └─────────────┘          └───────────┘
   walking here          entering here starts
   starts this load      this one
```

`preload` is the whole budget for hiding a load: it must cover the time it
takes to walk that distance. Make the interstitial space longer, or the preload
radius bigger, if players outrun it.

## Streaming large levels

A **sector** is a node tagged `sector`. There are two kinds, and which one you
get depends on whether the node names another file.

**External sectors** name a `source`. The file is brought in as a level in its
own right, positioned at the sector node, and is not in memory until something
comes near it. Because a sector is just a level, it gets tags, LOD and
everything else without special cases.

| Property    | Value                      |
| ----------- | -------------------------- |
| `stage_tag` | `sector`                   |
| `source`    | `levels/east_field.gltf`   |
| `radius`    | `80.0`                     |

**Inline sectors** have no `source`, so their contents ship with the level.
They are shown and hidden rather than loaded, which saves draw calls but not
memory.

Sectors leave at `radius * hysteresis` (1.3 by default) rather than at `radius`.
That gap is deliberate: without it, a player standing exactly on the boundary
would load and unload the sector every frame.

Sector size is also what bounds spawn hitches. A scene spawns atomically, so
the largest sector is the worst frame.

## Remembering what changed

Register the components you want kept:

```rust
app.persist_stage_component::<DoorState>();
```

When a level is unloaded its registered components are recorded, and when it
comes back they are put back. Anything destroyed during play stays destroyed,
so a picked-up item does not reappear.

Identity comes from the **glTF node index**, baked in at load time. Unlike a
node's name or its position in the hierarchy, artists do not change it by
renaming or re-parenting. Only nodes that carry a tag get an identity, so only
tagged nodes can be remembered — which is the same set as the things the game
knows about.

`StageDeltas` holds the record. Clear it per level or entirely to start a new
game. It is in-memory only; writing it to disk is up to the game.

## Level of detail

Export LOD variants as siblings named `Rock_LOD0`, `Rock_LOD1`, `Rock_LOD2`.
They are wired to Bevy's `VisibilityRange` automatically, with each level
fading out exactly where the next fades in. Distances are configurable through
the `LodConfig` resource. All levels must sit at the same position — the
crossfade is a dither, not a blend.

## Design notes

**Metadata is parsed once, at load.** Bevy 0.19's `GltfExtensionHandler` runs
while the glTF is being read, so parsing happens per asset load rather than per
spawn. It also avoids a trap in the alternative: Bevy puts `GltfExtras` on both
node entities *and* their primitive children, so a post-spawn query sees each
tagged mesh twice.

**Only nodes that could matter are recorded.** Most nodes in a level are plain
geometry. The loader is given the set of registered tag names so it can skip
them, rather than baking metadata onto every node just in case.

**Placement is computed before spawning.** A stage's position is derived from
the glTF asset's node data, so a level is in the right place on the first frame
it exists instead of snapping into position afterwards.

**Registration order matters.** Register tags during app setup. A tag
registered after a level has already loaded applies to later loads, but the
cached asset will not gain it.

## Roadmap

- [x] Tags from names, custom properties and reflected components
- [x] Several levels resident at once, placed by anchor
- [x] Portal preloading, hand-over and unloading
- [x] LOD groups wired to `VisibilityRange`
- [x] Streaming sectors, external and inline, with hysteresis
- [x] Persistence of level changes across unload and reload
- [ ] Frame-budgeted spawning: the tag resolution pass takes a per-frame budget,
      but scene spawning itself is still atomic per sector
- [ ] An asset retention cache, so bouncing between two levels does not re-read
      them from disk
- [ ] Runtime-spawned entities are not persisted, only authored nodes

## Examples

```bash
cargo run --example custom_tags   # one level, four nodes tagged three different ways
cargo run --example seamless      # WASD; walk north out of the atrium
```

`seamless` prints each level as it is requested, so you can watch the corridor
load while you are still walking toward it, and the vault load while you are
inside the corridor.

## Test fixtures

`assets/levels/` holds three small hand-authored levels used by the tests, as
plain-text glTF so the authored metadata is readable in a diff. They are
generated by `tools/gen_test_levels.py`.

## License

MIT
