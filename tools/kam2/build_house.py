"""ART2 - assemble one KAM2 house from the Kenney Fantasy Town Kit, one .glb per build state.

The kit is modular (1 world unit per wall cell), so a house is a table of pieces, not a
download. Every state sits on the same 2x2 footprint, x and y in [0, 2], floor at z 0, so
render_frames.py can share one span and one ground row across the three states.

Roof pieces get one flat style-lock roof red material; every other kit material is as authored.

Run from the repo root:

  blender --background --factory-startup --python tools/kam2/build_house.py -- ^
      --kit "<unzipped kit>/Models/GLB format" --out C:/tmp/kam2/house --state finished

Output: <out>/<state>.glb. States: finished, half_built, damaged.
Exit: 0 wrote the .glb - 1 build failed - 2 bad flags.
"""

import argparse
import math
import os
import sys

import bpy
from mathutils import Matrix

# Kit geometry, read off the pieces (tools probe, not guessed):
#   wall panels occupy the +X edge of their 1x1 cell, x 0.4..0.5, full y span, z 0..1;
#   roof-corner is a quarter hip whose apex is at its +X+Y corner, z 0..0.63;
#   a rotation of 90 deg steps turns a piece about its cell centre.
WALL_CELLS = {
    # edge name: (cell centre, rotation that puts the panel on that edge)
    "south": ((0.5, 0.5), 270.0),
    "south2": ((1.5, 0.5), 270.0),
    "east": ((1.5, 0.5), 0.0),
    "east2": ((1.5, 1.5), 0.0),
    "west": ((0.5, 0.5), 180.0),
    "west2": ((0.5, 1.5), 180.0),
    "north": ((0.5, 1.5), 90.0),
    "north2": ((1.5, 1.5), 90.0),
}

ROOF_CELLS = {
    # cell centre, rotation that points the hip apex at the house centre (1, 1)
    "sw": ((0.5, 0.5), 0.0),
    "se": ((1.5, 0.5), 90.0),
    "ne": ((1.5, 1.5), 180.0),
    "nw": ((0.5, 1.5), 270.0),
}

ROOF_Z = 1.0  # wall tops
CHIMNEY = ("chimney", (1.28, 1.34), 0.0, 0.95)
FLOOR_CELLS = [(0.5, 0.5), (1.5, 0.5), (0.5, 1.5), (1.5, 1.5)]

# The kit ships a teal roof; the style lock fixes roof red #A33B2E, so roof pieces get one
# flat material in that colour. Every other kit material is left as authored.
ROOF_RED = "#A33B2E"

# A state is a list of (piece, x, y, rot_z_deg, z). Nothing is moved after placement.
STATES = {
    "finished": {
        "walls": [
            ("wall-wood-window-small", "south"),
            ("wall-door", "south2"),
            ("wall-window-small", "east"),
            ("wall", "east2"),
            ("wall", "west"),
            ("wall-wood", "west2"),
            ("wall", "north"),
            ("wall-wood", "north2"),
        ],
        "roof": ["sw", "se", "ne", "nw"],
        "extra": [CHIMNEY],
        "corners": True,
    },
    "half_built": {
        # Two walls up, two sides open, no roof: the silhouette the player sees while building.
        "walls": [("wall", "south"), ("wall", "south2"), ("wall", "west"), ("wall", "west2")],
        "roof": [],
        "extra": [
            ("poles", (1.5, 0.5), 0.0, 0.0),      # scaffold line on the open east edge
            ("poles", (1.5, 1.5), 90.0, 0.0),     # and on the open north edge
            ("pillar-wood", (2.0, 0.0), 0.0, 0.0),
            ("pillar-wood", (2.0, 2.0), 0.0, 0.0),
        ],
        "corners": True,
    },
    "damaged": {
        # Same house after a hit: the south-east wall is broken out, one roof quarter is gone.
        "walls": [
            ("wall-wood-window-small", "south"),
            ("wall-broken", "south2"),
            ("wall-window-small", "east"),
            ("wall", "east2"),
            ("wall", "west"),
            ("wall-wood", "west2"),
            ("wall", "north"),
            ("wall-wood", "north2"),
        ],
        "roof": ["sw", "ne", "nw"],
        "extra": [CHIMNEY, ("pillar-wood", (1.9, 0.55), 78.0, 0.08)],
        "corners": True,
    },
}

CORNERS = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]


def fail(message):
    sys.stderr.write("E_HOUSE %s\n" % message)
    raise SystemExit(1)


def parse_args(argv):
    ours = list(argv)
    if "--" in ours:
        ours = ours[ours.index("--") + 1 :]
    parser = argparse.ArgumentParser(prog="build_house.py")
    parser.add_argument("--kit", required=True, help="folder holding the kit's .glb pieces")
    parser.add_argument("--out", required=True, help="output folder")
    parser.add_argument("--state", required=True, choices=sorted(STATES))
    args = parser.parse_args(ours)
    if not os.path.isdir(args.kit):
        parser.error("--kit is not a folder: %s" % args.kit)
    return args


def place(kit, name, x, y, rot_deg, z):
    """Import one piece and bake it at a cell with a 90 deg step rotation.

    The placement is baked into the mesh vertices, so the exported scene is a flat list of
    meshes with identity transforms: nothing depends on empties or parent order.
    """
    path = os.path.join(kit, name + ".glb")
    if not os.path.exists(path):
        fail("piece not found: %s" % path)
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=path)
    fresh = [ob for ob in bpy.data.objects if ob not in before]
    meshes = [ob for ob in fresh if ob.type == "MESH"]
    if not meshes:
        fail("piece imported no mesh: %s" % name)
    slot = Matrix.Translation((x, y, z)) @ Matrix.Rotation(math.radians(rot_deg), 4, "Z")
    worlds = {ob: slot @ ob.matrix_world for ob in fresh}
    for ob in fresh:
        ob.parent = None
    for ob in meshes:
        ob.data.transform(worlds[ob])
        ob.matrix_world = Matrix.Identity(4)
    for ob in fresh:
        if ob.type != "MESH":
            bpy.data.objects.remove(ob, do_unlink=True)
    return meshes


def hex_to_linear(value):
    text = value.lstrip("#")
    rgb = []
    for i in (0, 2, 4):
        c = int(text[i : i + 2], 16) / 255.0
        rgb.append(c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4)
    return (rgb[0], rgb[1], rgb[2], 1.0)


def roof_material():
    mat = bpy.data.materials.new("KAM2 roof red")
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    if bsdf is not None:
        bsdf.inputs["Base Color"].default_value = hex_to_linear(ROOF_RED)
        bsdf.inputs["Roughness"].default_value = 0.62
        bsdf.inputs["Metallic"].default_value = 0.0
    return mat


def build(kit, state_name):
    spec = STATES[state_name]
    bpy.ops.wm.read_factory_settings(use_empty=True)
    for piece, edge in spec["walls"]:
        (cx, cy), rot = WALL_CELLS[edge]
        place(kit, piece, cx, cy, rot, 0.0)
    for cx, cy in FLOOR_CELLS:
        place(kit, "planks", cx, cy, 0.0, 0.0)
    roof = []
    for key in spec["roof"]:
        (cx, cy), rot = ROOF_CELLS[key]
        roof.extend(place(kit, "roof-corner", cx, cy, rot, ROOF_Z))
    if roof:
        mat = roof_material()
        for ob in roof:
            ob.data.materials.clear()
            ob.data.materials.append(mat)
    if spec["corners"]:
        for cx, cy in CORNERS:
            place(kit, "pillar-wood", cx, cy, 45.0, 0.0)
    for extra in spec["extra"]:
        name, (cx, cy), rot, z = extra
        place(kit, name, cx, cy, rot, z)
    meshes = [ob for ob in bpy.data.objects if ob.type == "MESH"]
    if not meshes:
        fail("state %s built nothing" % state_name)
    return len(meshes)


def main():
    args = parse_args(sys.argv[1:])
    out_dir = os.path.abspath(args.out)
    os.makedirs(out_dir, exist_ok=True)
    count = build(args.kit, args.state)
    target = os.path.join(out_dir, args.state + ".glb")
    bpy.ops.export_scene.gltf(
        filepath=target,
        export_format="GLB",
        export_apply=True,
        export_yup=True,
        use_visible=False,
        use_renderable=False,
    )
    sys.stdout.write("KAM2 HOUSE BUILD OK state=%s meshes=%d out=%s\n" % (args.state, count, target))
    sys.stdout.flush()


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception:
        import traceback

        traceback.print_exc()
        sys.stderr.write("E_HOUSE build failed\n")
        sys.exit(1)
