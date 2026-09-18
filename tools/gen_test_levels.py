#!/usr/bin/env python3
"""Generate the hand-checkable glTF fixture levels used by tests and examples.

Text glTF with a base64 data-URI buffer, so the fixtures stay diffable and the
repo carries no binary blobs. Run from the repo root:

    python3 tools/gen_test_levels.py
"""

import base64
import json
import pathlib
import struct

OUT = pathlib.Path(__file__).resolve().parent.parent / "assets" / "levels"

# A half turn about Y, as a glTF [x, y, z, w] quaternion.
YAW_180 = (0.0, 1.0, 0.0, 0.0)

# ---------------------------------------------------------------- geometry --

# Unit cube centred on the origin, 24 verts (4 per face) so each face gets a
# flat normal, plus a 36-index triangle list.
FACES = [
    ((0, 0, 1), [(-1, -1, 1), (1, -1, 1), (1, 1, 1), (-1, 1, 1)]),
    ((0, 0, -1), [(1, -1, -1), (-1, -1, -1), (-1, 1, -1), (1, 1, -1)]),
    ((0, 1, 0), [(-1, 1, 1), (1, 1, 1), (1, 1, -1), (-1, 1, -1)]),
    ((0, -1, 0), [(-1, -1, -1), (1, -1, -1), (1, -1, 1), (-1, -1, 1)]),
    ((1, 0, 0), [(1, -1, 1), (1, -1, -1), (1, 1, -1), (1, 1, 1)]),
    ((-1, 0, 0), [(-1, -1, -1), (-1, -1, 1), (-1, 1, 1), (-1, 1, -1)]),
]


def cube_arrays():
    positions, normals, indices = [], [], []
    for normal, corners in FACES:
        base = len(positions)
        for corner in corners:
            positions.append(tuple(c * 0.5 for c in corner))
            normals.append(normal)
        indices += [base, base + 1, base + 2, base, base + 2, base + 3]
    return positions, normals, indices


POSITIONS, NORMALS, INDICES = cube_arrays()


def build_buffer():
    """Pack positions, normals and indices into one buffer.

    Returns (data_uri, views, accessors). Offsets are kept 4-byte aligned,
    which both the spec and Bevy's loader expect.
    """
    pos_bytes = b"".join(struct.pack("<3f", *p) for p in POSITIONS)
    nrm_bytes = b"".join(struct.pack("<3f", *n) for n in NORMALS)
    idx_bytes = b"".join(struct.pack("<H", i) for i in INDICES)
    while len(idx_bytes) % 4:
        idx_bytes += b"\x00"

    blob = pos_bytes + nrm_bytes + idx_bytes
    views = [
        {"buffer": 0, "byteOffset": 0, "byteLength": len(pos_bytes), "target": 34962},
        {"buffer": 0, "byteOffset": len(pos_bytes), "byteLength": len(nrm_bytes), "target": 34962},
        {
            "buffer": 0,
            "byteOffset": len(pos_bytes) + len(nrm_bytes),
            "byteLength": len(idx_bytes),
            "target": 34963,
        },
    ]
    mins = [min(p[i] for p in POSITIONS) for i in range(3)]
    maxs = [max(p[i] for p in POSITIONS) for i in range(3)]
    accessors = [
        {
            "bufferView": 0,
            "componentType": 5126,
            "count": len(POSITIONS),
            "type": "VEC3",
            "min": mins,
            "max": maxs,
        },
        {"bufferView": 1, "componentType": 5126, "count": len(NORMALS), "type": "VEC3"},
        {"bufferView": 2, "componentType": 5123, "count": len(INDICES), "type": "SCALAR"},
    ]
    uri = "data:application/octet-stream;base64," + base64.b64encode(blob).decode("ascii")
    return uri, views, accessors, len(blob)


# ------------------------------------------------------------------ authoring --


def node(name, translation=None, scale=None, rotation=None, mesh=None, extras=None, children=None):
    """Declare a node. `children` names other nodes; indices are resolved later."""
    n = {"name": name}
    if translation:
        n["translation"] = list(translation)
    if scale:
        n["scale"] = list(scale)
    if rotation:
        n["rotation"] = list(rotation)
    if mesh is not None:
        n["mesh"] = mesh
    if extras:
        n["extras"] = extras
    n["_children"] = list(children or [])
    return n


def resolve(nodes):
    """Turn name-based child references into indices and compute the roots.

    Hand-maintained index lists are a reliable source of silent breakage —
    inserting one node shifts every later reference. Names are what the
    fixtures are actually about, so the generator resolves them itself.
    """
    index_of = {}
    for i, n in enumerate(nodes):
        if n["name"] in index_of:
            raise ValueError(f"duplicate node name {n['name']!r}")
        index_of[n["name"]] = i

    claimed = set()
    for n in nodes:
        kids = n.pop("_children")
        if not kids:
            continue
        resolved = []
        for kid in kids:
            if kid not in index_of:
                raise ValueError(f"node {n['name']!r} references unknown child {kid!r}")
            resolved.append(index_of[kid])
            claimed.add(kid)
        n["children"] = resolved

    roots = [i for i, n in enumerate(nodes) if n["name"] not in claimed]
    return nodes, roots


def document(scene_name, nodes, scene_extras=None):
    nodes, roots = resolve(nodes)
    uri, views, accessors, length = build_buffer()
    scene = {"name": scene_name, "nodes": roots}
    if scene_extras:
        scene["extras"] = scene_extras
    return {
        "asset": {"version": "2.0", "generator": "bevy_stage tools/gen_test_levels.py"},
        "scene": 0,
        "scenes": [scene],
        "nodes": nodes,
        "meshes": [
            {
                "name": "Cube",
                "primitives": [
                    {"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "material": 0}
                ],
            }
        ],
        "materials": [
            {
                "name": "Default",
                "pbrMetallicRoughness": {
                    "baseColorFactor": [0.8, 0.8, 0.8, 1.0],
                    "metallicFactor": 0.0,
                    "roughnessFactor": 0.9,
                },
            }
        ],
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": length, "uri": uri}],
    }


def write(name, doc):
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / name
    # Keep the data URI on its own line so diffs of the geometry stay isolated
    # from diffs of the interesting authored metadata.
    path.write_text(json.dumps(doc, indent=2) + "\n")
    print(f"wrote {path.relative_to(OUT.parent.parent)}")


# -------------------------------------------------------------------- levels --


def stage_a():
    """The starting level. Deliberately exercises all three tag channels so the
    parser tests have one fixture covering every authoring style."""
    nodes = [
        node("Ground", translation=(0, -0.5, 0), scale=(40, 1, 40), mesh=0),
        # Name channel: no extras at all, the Blender-style .001 suffix is
        # stripped and the name itself resolves to the `spawn_point` tag.
        node("SpawnPoint.001", translation=(0, 1, 0)),
        node("SpawnPoint.002", translation=(3, 1, 0)),
        # Discriminator channel: flat sibling properties become the params.
        node(
            "PlayerStart",
            translation=(-3, 1, 0),
            extras={"stage_tag": "spawn_point", "team": "blue"},
        ),
        # Reflection channel: the key is the component's short type path and
        # the value is a RON literal, matching the Blenvy convention.
        node(
            "RedBase",
            translation=(0, 1, -8),
            extras={"SpawnPoint": '(team: "red")'},
        ),
        # Anchor that stage-to-stage alignment pins against.
        # Anchors point OUT of their own stage, so two connected anchors face
        # each other. Stage A's north exit faces -Z, which is Bevy's forward,
        # so it needs no rotation.
        node(
            "ExitNorth",
            translation=(0, 0, -20),
            extras={"stage_tag": "anchor", "id": "exit_north"},
        ),
        # Portal declaring its neighbour; proximity to it starts the load.
        node(
            "ToHallway",
            translation=(0, 1.5, -19),
            scale=(3, 3, 0.5),
            extras={
                "stage_tag": "portal",
                "target": "levels/hallway.gltf",
                "anchor": "entry_a",
                "preload": 40.0,
            },
        ),
        # A streamable sector and its contents.
        node(
            "Sector_North",
            translation=(0, 0, -30),
            extras={"stage_tag": "sector", "radius": 60.0},
            children=["Pillar"],
        ),
        node("Pillar", translation=(0, 2, 0), scale=(1, 4, 1), mesh=0),
        # An external sector: its contents live in their own file and are not
        # in memory until something comes near this node.
        node(
            "Sector_East",
            translation=(60, 0, 0),
            extras={
                "stage_tag": "sector",
                "source": "levels/sector_east.gltf",
                "radius": 50.0,
            },
        ),
        # A tag nothing is registered under. The level must still load, and the
        # plugin must say something rather than silently ignoring it — this is
        # what a typo in Blender looks like.
        node(
            "MysteryProp",
            translation=(-8, 1, 4),
            extras={"stage_tag": "not_a_real_tag"},
        ),
        # LOD siblings, auto-wired to VisibilityRange by distance.
        node("Rock_LOD0", translation=(8, 0.5, 4), mesh=0),
        node("Rock_LOD1", translation=(8, 0.5, 4), scale=(0.98, 0.98, 0.98), mesh=0),
        node("Rock_LOD2", translation=(8, 0.5, 4), scale=(0.95, 0.95, 0.95), mesh=0),
    ]
    return document(
        "Stage",
        nodes,
        scene_extras={"stage_tag": "stage", "display_name": "Atrium", "ambient": 0.3},
    )


def sector_east():
    """An external sector, streamed in by proximity rather than loaded with
    the level that references it."""
    nodes = [
        node("EastFloor", translation=(0, -0.5, 0), scale=(20, 1, 20), mesh=0),
        node("EastTower", translation=(0, 6, 0), scale=(3, 12, 3), mesh=0),
        node("SpawnPoint.001", translation=(4, 1, 4)),
    ]
    return document("Stage", nodes, scene_extras={"stage_tag": "stage"})


def hallway():
    """The interstitial 'bridge' level. Nothing marks it as special — it is an
    ordinary stage that happens to be small and have a portal at each end."""
    nodes = [
        node("Floor", translation=(0, -0.5, 0), scale=(4, 1, 30), mesh=0),
        node("WallL", translation=(-2, 1.5, 0), scale=(0.3, 4, 30), mesh=0),
        node("WallR", translation=(2, 1.5, 0), scale=(0.3, 4, 30), mesh=0),
        # Faces +Z, out of the hallway's near end, so it meets stage A's exit.
        node(
            "EntryA",
            translation=(0, 0, 15),
            rotation=YAW_180,
            extras={"stage_tag": "anchor", "id": "entry_a"},
        ),
        # Faces -Z, out of the far end.
        node("ExitB", translation=(0, 0, -15), extras={"stage_tag": "anchor", "id": "exit_b"}),
        # A portal faces the way you travel through it. Heading back to the
        # atrium means heading +Z, so this one is turned around.
        node(
            "BackToAtrium",
            translation=(0, 1.5, 14),
            rotation=YAW_180,
            scale=(3, 3, 0.5),
            extras={
                "stage_tag": "portal",
                "target": "levels/stage_a.gltf",
                "anchor": "exit_north",
                "preload": 30.0,
            },
        ),
        # Crossing into the hallway from A is what preloads B: this portal sits
        # at the far end but its preload radius covers the whole corridor.
        node(
            "OnToVault",
            translation=(0, 1.5, -14),
            scale=(3, 3, 0.5),
            extras={
                "stage_tag": "portal",
                "target": "levels/stage_b.gltf",
                "anchor": "entry_south",
                "preload": 35.0,
            },
        ),
    ]
    return document(
        "Stage",
        nodes,
        scene_extras={"stage_tag": "stage", "display_name": "Connecting Hall"},
    )


def stage_b():
    """The destination level."""
    nodes = [
        node("Ground", translation=(0, -0.5, 0), scale=(30, 1, 30), mesh=0),
        # Faces +Z, out of the vault's south entrance.
        node(
            "EntrySouth",
            translation=(0, 0, 14),
            rotation=YAW_180,
            extras={"stage_tag": "anchor", "id": "entry_south"},
        ),
        # Faces +Z, back toward the hallway.
        node(
            "BackToHall",
            translation=(0, 1.5, 13),
            rotation=YAW_180,
            scale=(3, 3, 0.5),
            extras={
                "stage_tag": "portal",
                "target": "levels/hallway.gltf",
                "anchor": "exit_b",
                "preload": 30.0,
            },
        ),
        node("SpawnPoint.001", translation=(0, 1, 6)),
        node("Obelisk", translation=(0, 4, -4), scale=(2, 8, 2), mesh=0),
        # A door whose open/closed state should survive leaving and returning.
        node(
            "VaultDoor",
            translation=(6, 1.5, -4),
            scale=(2, 3, 0.4),
            mesh=0,
            extras={"stage_tag": "door", "locked": True, "persist": True},
        ),
    ]
    return document(
        "Stage",
        nodes,
        scene_extras={"stage_tag": "stage", "display_name": "Vault"},
    )


if __name__ == "__main__":
    write("stage_a.gltf", stage_a())
    write("sector_east.gltf", sector_east())
    write("hallway.gltf", hallway())
    write("stage_b.gltf", stage_b())
