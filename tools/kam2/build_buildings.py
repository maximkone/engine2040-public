"""ART3 - assemble the KAM2 building kinds from the Kenney Fantasy Town Kit.

One table per kind, one .glb per state, every state on the kind's real footprint from
crates/kam2-sim/src/economy.rs:

  warehouse            3x3
  farm mill bakery
  woodcutter           2x2

The kit is modular: 1 world unit per wall cell. Walls, floors and roofs are placed on a
grid, all transforms baked into the mesh. Roof pieces get one flat style-lock roof red
material; the farm plot uses one flat dirt material. Every other kit material is as
authored. The kit is never written into the repo; only the rendered frames are.

Run from the repo root:

  blender --background --factory-startup --python tools/kam2/build_buildings.py -- ^
      --kit "<unzipped kit>/Models/GLB format" --out <dir> --kind farm --state finished

Output: <out>/<kind>/<state>.glb. States: finished, half_built.
Exit: 0 wrote the .glb - 1 build failed - 2 bad flags.
"""

import argparse
import json
import math
import os
import sys

import bpy
from mathutils import Matrix, Vector

# Footprints mirror HouseKind::footprint in crates/kam2-sim/src/economy.rs.
FOOTPRINTS = {
    "warehouse": (3, 3),
    "farm": (2, 2),
    "mill": (2, 2),
    "bakery": (2, 2),
    "woodcutter": (2, 2),
}

ROOF_RED = "#A33B2E"  # style lock roof red
DIRT_DARK = "#8A6A43"  # style lock dirt dark, the farm plot
WALL_Z = 1.0  # one kit wall panel is 1 unit tall

# Roof corner apex points at +X+Y; a 90 deg Z step turns it to the next corner.
HIP_CELLS = [((0.5, 0.5), 0.0), ((1.5, 0.5), 90.0), ((1.5, 1.5), 180.0), ((0.5, 1.5), 270.0)]


def fail(message):
    sys.stderr.write("E_BUILDINGS %s\n" % message)
    raise SystemExit(1)


def parse_args(argv):
    ours = list(argv)
    if "--" in ours:
        ours = ours[ours.index("--") + 1 :]
    parser = argparse.ArgumentParser(prog="build_buildings.py")
    parser.add_argument("--kit", help="folder holding the kit's .glb pieces")
    parser.add_argument("--out", help="output folder")
    parser.add_argument("--kind", choices=sorted(FOOTPRINTS))
    parser.add_argument("--state", choices=["finished", "half_built"])
    parser.add_argument("--list", action="store_true", help="print the kind table and exit")
    args = parser.parse_args(ours)
    if args.list:
        return args
    for name in ("kit", "out", "kind", "state"):
        if not getattr(args, name):
            parser.error("--%s is required" % name)
    if not os.path.isdir(args.kit):
        parser.error("--kit is not a folder: %s" % args.kit)
    return args


def hex_to_linear(value):
    text = value.lstrip("#")
    rgb = []
    for i in (0, 2, 4):
        c = int(text[i : i + 2], 16) / 255.0
        rgb.append(c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4)
    return (rgb[0], rgb[1], rgb[2], 1.0)


class Kit:
    """Imports kit pieces and bakes position, rotation and scale into the mesh."""

    def __init__(self, folder):
        self.folder = folder
        self.roofs = []
        self.soil = []

    def place(self, name, x, y, z=0.0, rz=0.0, rx=0.0, ry=0.0, scale=None):
        path = os.path.join(self.folder, name + ".glb")
        if not os.path.exists(path):
            fail("piece not found: %s" % path)
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=path)
        fresh = [ob for ob in bpy.data.objects if ob not in before]
        meshes = [ob for ob in fresh if ob.type == "MESH"]
        if not meshes:
            fail("piece imported no mesh: %s" % name)
        transform = Matrix.Translation((x, y, z))
        transform = transform @ Matrix.Rotation(math.radians(rz), 4, "Z")
        transform = transform @ Matrix.Rotation(math.radians(ry), 4, "Y")
        transform = transform @ Matrix.Rotation(math.radians(rx), 4, "X")
        if scale is not None:
            transform = transform @ Matrix.Diagonal((scale[0], scale[1], scale[2], 1.0))
        worlds = {ob: transform @ ob.matrix_world for ob in fresh}
        for ob in fresh:
            ob.parent = None
        for ob in meshes:
            ob.data.transform(worlds[ob])
            ob.matrix_world = Matrix.Identity(4)
        for ob in fresh:
            if ob.type != "MESH":
                bpy.data.objects.remove(ob, do_unlink=True)
        return meshes

    def paint(self, objects, colour, roughness):
        mat = bpy.data.materials.new("KAM2 %s" % colour)
        bsdf = mat.node_tree.nodes.get("Principled BSDF")
        if bsdf is not None:
            bsdf.inputs["Base Color"].default_value = hex_to_linear(colour)
            bsdf.inputs["Roughness"].default_value = roughness
            bsdf.inputs["Metallic"].default_value = 0.0
        for ob in objects:
            ob.data.materials.clear()
            ob.data.materials.append(mat)
        return mat

    def roof(self, objects):
        self.roofs.extend(objects)

    def plot(self, objects):
        self.soil.extend(objects)


# ---------------------------------------------------------------------------
# Shared construction helpers. A footprint is [ox, ox+w] x [oy, oy+h] in world units.


def floor(kit, w, h, ox=0.0, oy=0.0, piece="planks", z=0.0):
    for i in range(w):
        for j in range(h):
            kit.place(piece, ox + i + 0.5, oy + j + 0.5, z)


def ring(kit, w, h, ox, oy, z, edges):
    """Place wall panels on the four footprint edges.

    edges: south/north lists of w piece names, west/east lists of h names. None skips a cell.
    """
    for i, piece in enumerate(edges["south"]):
        if piece:
            kit.place(piece, ox + i + 0.5, oy + 0.5, z, 270.0)
    for i, piece in enumerate(edges["north"]):
        if piece:
            kit.place(piece, ox + i + 0.5, oy + h - 0.5, z, 90.0)
    for j, piece in enumerate(edges["west"]):
        if piece:
            kit.place(piece, ox + 0.5, oy + j + 0.5, z, 180.0)
    for j, piece in enumerate(edges["east"]):
        if piece:
            kit.place(piece, ox + w - 0.5, oy + j + 0.5, z, 0.0)


def corners(kit, w, h, ox=0.0, oy=0.0, heights=(0.0,)):
    for z in heights:
        for cx, cy in ((ox, oy), (ox + w, oy), (ox + w, oy + h), (ox, oy + h)):
            kit.place("pillar-wood", cx, cy, z, 45.0)


def hip2(kit, ox, oy, z=WALL_Z, piece="roof-corner"):
    roof = []
    for (cx, cy), rot in HIP_CELLS:
        roof.extend(kit.place(piece, ox + cx, oy + cy, z, rot))
    kit.roof(roof)


def hip3(kit, ox, oy, z=WALL_Z):
    """3x3 hip: four corner quarters, four edge slopes, one flat cap.

    The edge slope's apex sits on its +X edge. South needs +Y (90), east -X (180),
    north -Y (270), west +X (0).
    """
    roof = []
    for (cx, cy), rot in HIP_CELLS:
        roof.extend(kit.place("roof-high-corner", ox + cx, oy + cy, z, rot))
    for cx, cy in ((1.5, 0.5), (2.5, 1.5), (1.5, 2.5), (0.5, 1.5)):
        rot = {(1.5, 0.5): 90.0, (2.5, 1.5): 180.0, (1.5, 2.5): 270.0, (0.5, 1.5): 0.0}[(cx, cy)]
        roof.extend(kit.place("roof-high", ox + cx, oy + cy, z, rot))
    roof.extend(kit.place("roof-high-flat", ox + 1.5, oy + 1.5, z + 1.0))
    kit.roof(roof)


def gable(kit, w, ox, oy, z=WALL_Z):
    """A gable roof with its ridge along X, w cells long."""
    roof = []
    for i in range(w):
        roof.extend(kit.place("roof-high-gable", ox + i + 0.5, oy + 0.5, z, 0.0))
    kit.roof(roof)


# ---------------------------------------------------------------------------
# One builder per kind. Each takes the kit and the state name.


def warehouse(kit, state):
    """3x3, two floors: stone-plaster base, timber upper, hipped roof, loaded cart."""
    w = h = 3
    floor(kit, w, h)
    if state == "finished":
        ring(
            kit, w, h, 0, 0, 0.0,
            {
                "south": ["wall", "wall-doorway-square-wide", "wall"],
                "east": ["wall", "wall-window-small", "wall"],
                "north": ["wall", "wall", "wall"],
                "west": ["wall", "wall", "wall"],
            },
        )
        ring(
            kit, w, h, 0, 0, WALL_Z,
            {
                "south": ["wall-wood-window-small", "wall-wood", "wall-wood-window-small"],
                "east": ["wall-wood", "wall-wood-window-small", "wall-wood"],
                "north": ["wall-wood", "wall-wood-window-small", "wall-wood"],
                "west": ["wall-wood-window-small", "wall-wood", "wall-wood"],
            },
        )
        corners(kit, w, h, heights=(0.0, WALL_Z))
        hip3(kit, 0, 0, 2 * WALL_Z)
        # The kit has no crates or bales; a loaded cart is the warehouse goods stand-in.
        kit.place("cart-high", 2.55, 0.95, 0.0, 0.0)
        kit.place("lantern", 0.28, 0.22, 0.0, 0.0)
        kit.place("banner-red", 1.12, 0.5, WALL_Z, 270.0)
    else:
        # Ground floor up, upper floor and roof still open, scaffold on the two open sides.
        ring(
            kit, w, h, 0, 0, 0.0,
            {
                "south": ["wall", "wall-doorway-square-wide", "wall"],
                "west": ["wall", "wall", "wall"],
                "east": [None, None, None],
                "north": [None, None, None],
            },
        )
        corners(kit, w, h, heights=(0.0,))
        for cx, cy, rot in ((2.5, 0.5, 0.0), (2.5, 1.5, 0.0), (0.5, 2.5, 90.0), (1.5, 2.5, 90.0)):
            kit.place("poles", cx, cy, 0.0, rot)


def farm(kit, state):
    """2x2: low barn on the north half, fenced crop strip on the south half."""
    w = h = 2
    # Soil plot under the crop rows.
    plot = []
    for i in range(2):
        plot.extend(kit.place("planks", 0.5 + i, 0.5, 0.0))
    kit.plot(plot)
    if state == "finished":
        # Field: three rows of low hedge, the kit's closest crop row, behind a fence.
        for row in (0.25, 0.55, 0.85):
            for i in range(2):
                kit.place("hedge", i + 0.5, row, 0.06, 270.0)
        for i, piece in enumerate(("fence", "fence-gate")):
            kit.place(piece, i + 0.5, 0.5, 0.0, 270.0)
        kit.place("fence", 0.5, 0.5, 0.0, 180.0)
        kit.place("fence", 1.5, 0.5, 0.0, 0.0)
        # Barn shell on the north half, door facing the field.
        ring(
            kit, 2, 1, 0, 1, 0.0,
            {
                "south": ["wall-wood-door", "wall-wood-window-small"],
                "north": ["wall-wood-window-small", "wall-wood"],
                "west": ["wall-wood"],
                "east": ["wall-wood"],
            },
        )
        corners(kit, 2, 1, 0, 1)
        gable(kit, 2, 0, 1, WALL_Z)
    else:
        # Field fence and a gate, barn walls half up, no roof.
        for i, piece in enumerate(("fence", "fence-gate")):
            kit.place(piece, i + 0.5, 0.5, 0.0, 270.0)
        kit.place("fence", 0.5, 0.5, 0.0, 180.0)
        kit.place("fence", 1.5, 0.5, 0.0, 0.0)
        ring(
            kit, 2, 1, 0, 1, 0.0,
            {
                "south": ["wall-wood-half", "wall-wood-half"],
                "north": ["wall-wood", "wall-wood"],
                "west": ["wall-wood"],
                "east": [None],
            },
        )
        corners(kit, 2, 1, 0, 1)
        kit.place("poles", 1.5, 1.5, 0.0, 0.0)


def mill(kit, state):
    """2x2 windmill tower: two floors, hipped cap, four sails on the south face."""
    w = h = 2
    floor(kit, w, h)
    if state == "finished":
        ring(
            kit, w, h, 0, 0, 0.0,
            {
                "south": ["wall", "wall-door"],
                "east": ["wall", "wall-window-small"],
                "north": ["wall", "wall"],
                "west": ["wall-window-small", "wall"],
            },
        )
        ring(
            kit, w, h, 0, 0, WALL_Z,
            {
                "south": ["wall-wood-window-small", "wall-wood"],
                "east": ["wall-wood", "wall-wood-window-small"],
                "north": ["wall-wood-window-small", "wall-wood"],
                "west": ["wall-wood", "wall-wood-window-small"],
            },
        )
        corners(kit, w, h, heights=(0.0, WALL_Z))
        hip2(kit, 0, 0, 2 * WALL_Z)
        # The kit's windmill piece is the four sails and hub, no tower. Scaled to 0.72
        # (span 2.24) so it reads on a 2x2 footprint; hub stands off the south face on
        # a short wooden axle so the sails clear the roof.
        kit.place("windmill", 1.0, -0.42, 2.55, 270.0, scale=(0.72, 0.72, 0.72))
        kit.place("pillar-wood", 1.0, -0.12, 2.55, 0.0, rx=90.0, scale=(1.0, 1.0, 0.3))
    else:
        ring(
            kit, w, h, 0, 0, 0.0,
            {
                "south": ["wall", "wall-door"],
                "west": ["wall", "wall"],
                "east": [None, None],
                "north": [None, None],
            },
        )
        corners(kit, w, h, heights=(0.0,))
        kit.place("poles", 1.5, 0.5, 0.0, 0.0)
        kit.place("poles", 1.5, 1.5, 0.0, 0.0)
        kit.place("poles", 0.5, 1.5, 0.0, 90.0)


def bakery(kit, state):
    """2x2 shop: wide doorway, shutters, hip roof, tall chimney, a bench outside."""
    w = h = 2
    floor(kit, w, h)
    if state == "finished":
        ring(
            kit, w, h, 0, 0, 0.0,
            {
                "south": ["wall-wood-window-small", "wall-wood-doorway-square-wide"],
                "east": ["wall-window-shutters", "wall-wood"],
                "north": ["wall-wood", "wall-wood"],
                "west": ["wall-wood", "wall-window-shutters"],
            },
        )
        corners(kit, w, h)
        hip2(kit, 0, 0)
        # Chimney breaks the roof line: base through the roof, cap above the apex.
        kit.place("chimney", 1.18, 1.18, 0.95, 0.0)
        kit.place("chimney-top", 1.18, 1.18, 1.55, 0.0)
        kit.place("stall-bench", 1.45, -0.22, 0.0, 270.0)
        kit.place("banner-red", 0.82, 0.5, 0.0, 270.0)
    else:
        ring(
            kit, w, h, 0, 0, 0.0,
            {
                "south": ["wall-wood-half", "wall-wood-half"],
                "west": ["wall-wood", "wall-wood"],
                "east": [None, None],
                "north": [None, None],
            },
        )
        corners(kit, w, h)
        for cx, cy, rot in ((1.5, 0.5, 0.0), (1.5, 1.5, 0.0), (0.5, 1.5, 90.0)):
            kit.place("poles", cx, cy, 0.0, rot)
        kit.place("chimney", 1.18, 1.18, 0.0, 0.0)


def woodcutter(kit, state):
    """2x2 open shed: back and side walls, four posts, hip roof, log stack inside."""
    w = h = 2
    floor(kit, w, h)
    if state == "finished":
        ring(
            kit, w, h, 0, 0, 0.0,
            {
                "south": [None, None],
                "north": ["wall-wood", "wall-wood"],
                "west": [None, "wall-wood"],
                "east": [None, "wall-wood"],
            },
        )
        corners(kit, w, h)
        hip2(kit, 0, 0)
        # Logs at the open front, where the light reaches them: a stack of three cut
        # posts plus two loose logs, and a chopping block.
        for i, z in enumerate((0.09, 0.27, 0.45)):
            kit.place("pillar-wood", 0.68, 0.55, z, 0.0, ry=90.0, scale=(1.0, 1.0, 1.15))
        kit.place("pillar-wood", 0.42, 0.42, 0.09, 18.0, ry=90.0, scale=(1.0, 1.0, 0.95))
        kit.place("pillar-wood", 0.35, 0.72, 0.09, 8.0, ry=90.0, scale=(1.0, 1.0, 0.85))
        kit.place("pillar-wood", 1.45, 1.35, 0.11, 0.0, scale=(0.4, 0.4, 0.3))
    else:
        ring(
            kit, w, h, 0, 0, 0.0,
            {
                "south": [None, None],
                "north": ["wall-wood", "wall-wood"],
                "west": [None, None],
                "east": [None, None],
            },
        )
        corners(kit, w, h)
        kit.place("poles", 0.5, 0.5, 0.0, 180.0)
        kit.place("poles", 1.5, 0.5, 0.0, 270.0)
        for i, z in enumerate((0.09, 0.27)):
            kit.place("pillar-wood", 0.9, 1.45, z, 0.0, ry=90.0, scale=(1.0, 1.0, 1.1))


BUILDERS = {
    "warehouse": warehouse,
    "farm": farm,
    "mill": mill,
    "bakery": bakery,
    "woodcutter": woodcutter,
}


def bounds():
    deps = bpy.context.evaluated_depsgraph_get()
    lo = Vector((math.inf, math.inf, math.inf))
    hi = Vector((-math.inf, -math.inf, -math.inf))
    for ob in bpy.context.scene.objects:
        if ob.type != "MESH" or ob.hide_render:
            continue
        evaluated = ob.evaluated_get(deps)
        mesh = evaluated.to_mesh()
        for vertex in mesh.vertices:
            p = evaluated.matrix_world @ vertex.co
            for i in range(3):
                lo[i] = min(lo[i], p[i])
                hi[i] = max(hi[i], p[i])
        evaluated.to_mesh_clear()
    return lo, hi


def main():
    args = parse_args(sys.argv[1:])
    if args.list:
        for kind, (w, h) in sorted(FOOTPRINTS.items()):
            sys.stdout.write("%s %dx%d\n" % (kind, w, h))
        return
    bpy.ops.wm.read_factory_settings(use_empty=True)
    kit = Kit(os.path.abspath(args.kit))
    BUILDERS[args.kind](kit, args.state)
    if kit.roofs and os.environ.get("KAM2_DEBUG_ROOF"):
        palette = ["#E04040", "#40C040", "#4060E0", "#E0D040", "#E060C0", "#40D0D0", "#E08020", "#8040E0", "#FFFFFF"]
        for i, ob in enumerate(kit.roofs):
            kit.paint([ob], palette[i % len(palette)] if i < len(palette) else "#808080", 0.7)
    elif kit.roofs:
        kit.paint(kit.roofs, ROOF_RED, 0.62)
    if kit.soil:
        kit.paint(kit.soil, DIRT_DARK, 0.9)
    meshes = [ob for ob in bpy.context.scene.objects if ob.type == "MESH"]
    if not meshes:
        fail("kind %s state %s built nothing" % (args.kind, args.state))
    lo, hi = bounds()
    out_dir = os.path.join(os.path.abspath(args.out), args.kind)
    os.makedirs(out_dir, exist_ok=True)
    target = os.path.join(out_dir, args.state + ".glb")
    bpy.ops.export_scene.gltf(
        filepath=target,
        export_format="GLB",
        export_apply=True,
        export_yup=True,
        use_visible=False,
        use_renderable=False,
    )
    w, h = FOOTPRINTS[args.kind]
    sys.stdout.write(
        "KAM2 BUILDINGS BUILD OK kind=%s state=%s footprint=%dx%d meshes=%d "
        "bounds=[%.3f,%.3f,%.3f]-[%.3f,%.3f,%.3f] out=%s\n"
        % (args.kind, args.state, w, h, len(meshes), lo.x, lo.y, lo.z, hi.x, hi.y, hi.z, target)
    )
    sys.stdout.flush()


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception:
        import traceback

        traceback.print_exc()
        sys.stderr.write("E_BUILDINGS build failed\n")
        sys.exit(1)
