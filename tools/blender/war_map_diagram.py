"""The war map: the contested middle drawn from the data.

A top-down battle map of the contested middle, built from
content/maps/contested_middle.json: the four gates, the two forts, the
keep ruin, the watchtowers, the two faction spawns and the three statue
points, each statue drawn as a capture ring with its radius to scale.

The ring is the thing the player has to read in three seconds, so the ring
is the loudest mark on the map and everything else stays quiet.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/war_map_diagram.py -- <out.png>
"""

import json
import math
import os
import sys

import bpy
from mathutils import Vector

EXIT_USAGE = 2
EXIT_FAIL = 3

WIDTH = 1400
HEIGHT = 1400

SAND = (0.788, 0.643, 0.416)
SAND_DARK = (0.706, 0.557, 0.333)
SEA = (0.243, 0.498, 0.478)
INK = (0.169, 0.153, 0.141)
BRONZE = (0.612, 0.420, 0.227)
BONE = (0.851, 0.796, 0.698)
EMBER = (0.784, 0.337, 0.165)
STEEL = (0.243, 0.353, 0.471)

FACTION_COLOUR = {"wardens": BONE, "ashkin": EMBER}


class DiagramError(Exception):
    """A diagram that could not be drawn."""


def load_map():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
    path = os.path.join(root, "content", "maps", "contested_middle.json")
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle), root


def disc(name, x, z, radius, colour, height=0.0, segments=48):
    bpy.ops.mesh.primitive_circle_add(vertices=segments, radius=radius,
                                      location=(x, z, height))
    ob = bpy.context.object
    ob.name = name
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (colour[0], colour[1], colour[2], 1.0)
    ob.data.materials.append(mat)
    return ob


def ring(name, x, z, radius, colour, width=1.6, height=0.05):
    bpy.ops.mesh.primitive_torus_add(major_radius=radius, minor_radius=width,
                                     location=(x, z, height))
    ob = bpy.context.object
    ob.name = name
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (colour[0], colour[1], colour[2], 1.0)
    ob.data.materials.append(mat)
    return ob


def box(name, x, z, sx, sz, colour, height=0.0):
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=(x, z, height))
    ob = bpy.context.object
    ob.name = name
    ob.scale = (sx, sz, 0.2)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (colour[0], colour[1], colour[2], 1.0)
    ob.data.materials.append(mat)
    return ob


def label(text, x, z, size, colour, height=0.2):
    curve = bpy.data.curves.new("label", type="FONT")
    curve.body = text
    curve.size = size
    curve.align_x = "CENTER"
    curve.align_y = "CENTER"
    ob = bpy.data.objects.new("label", curve)
    bpy.context.collection.objects.link(ob)
    ob.location = (x, z, height)
    ob.rotation_euler = (0.0, 0.0, 0.0)
    mat = bpy.data.materials.new("label_mat")
    mat.diffuse_color = (colour[0], colour[1], colour[2], 1.0)
    ob.data.materials.append(mat)
    return ob


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if not args:
        sys.stderr.write("usage: blender -b --factory-startup --python-exit-code 1 "
                         "-P war_map_diagram.py -- <out.png>\n")
        return EXIT_USAGE
    out = args[0]

    data, root = load_map()
    size = data.get("size_m", {}).get("x", 300)
    half = size / 2.0

    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.render.resolution_x = WIDTH
    scene.render.resolution_y = HEIGHT
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = False
    scene.render.image_settings.file_format = "PNG"
    scene.display.shading.light = "FLAT"
    scene.display.shading.color_type = "MATERIAL"
    scene.display.shading.show_shadows = False
    try:
        scene.view_settings.view_transform = "Standard"
    except Exception:
        pass

    world = bpy.data.worlds.new("war_world")
    scene.world = world
    world.color = (SAND[0], SAND[1], SAND[2])
    scene.display.shading.background_type = "WORLD"

    # The ground, and the sea along the edge.
    box("ground", 0, 0, size, size, SAND, height=-0.4)
    box("sea_n", 0, half - 18, size, 36, SEA, height=-0.3)
    box("sea_s", 0, -half + 18, size, 36, SEA, height=-0.3)

    # The four gates and the two forts.
    for node in data.get("nodes", []):
        kind = node.get("kind")
        x, z = node.get("x", 0), node.get("z", 0)
        if kind == "gate":
            box(f"gate_{node['id']}", x, z, 12, 12, INK, height=0.0)
        elif kind == "fort":
            box(f"fort_{node['id']}", x, z, 20, 20, SAND_DARK, height=0.0)
        elif kind == "statue":
            disc(f"statue_{node['id']}", x, z, 6, INK, height=0.05)
            ring(f"ring_{node['id']}", x, z, node.get("radius", 10), BRONZE, height=0.1)
            label(node["id"].replace("statue_", "").upper(), x, z, 5.0, BONE, height=0.3)

    # The keep ruin in the middle, the watchtowers on the flanks.
    for landmark in data.get("landmarks", []):
        x, z = landmark.get("x", 0), landmark.get("z", 0)
        kind = landmark.get("kind", "")
        if kind == "keep":
            disc(f"keep_{landmark['id']}", x, z, 16, SAND_DARK, height=0.02)
        elif kind == "watchtower":
            disc(f"tower_{landmark['id']}", x, z, 8, SAND_DARK, height=0.02)
        elif kind == "tower":
            disc(f"tower_{landmark['id']}", x, z, 9, SAND_DARK, height=0.02)
        elif kind == "gate_arch":
            box(f"arch_{landmark['id']}", x, z, 8, 8, SAND_DARK, height=0.02)
        elif kind == "road_post":
            disc(f"post_{landmark['id']}", x, z, 4, SAND_DARK, height=0.02)

    # The two faction spawns, in their own colour. The mark sits on the ground
    # and the name is set beside it, never on it.
    for spawn in data.get("spawns", []):
        x, z = spawn.get("x", 0), spawn.get("z", 0)
        colour = FACTION_COLOUR.get(spawn.get("faction"), STEEL)
        disc(f"spawn_{spawn['id']}", x, z, 9, colour, height=0.6)
        ring(f"spawn_ring_{spawn['id']}", x, z, 13, colour, width=1.2, height=0.62)
        inward = 18.0 if z > 0 else -18.0
        label(spawn.get("faction", "?").upper(), x, z + inward, 5.0, INK, height=0.7)

    # The title, in the corners of the ground, clear of the sea.
    label("STATUE WAR", -half + 46, half - 34, 11.0, INK, height=0.7)
    label("THE CONTESTED MIDDLE", half - 62, -half + 34, 7.0, INK, height=0.7)

    cam_data = bpy.data.cameras.new("war_cam")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = size * 1.02
    cam = bpy.data.objects.new("war_cam", cam_data)
    bpy.context.collection.objects.link(cam)
    cam.location = (0.0, 0.0, 400.0)
    cam.rotation_euler = (0.0, 0.0, 0.0)
    scene.camera = cam

    os.makedirs(os.path.dirname(os.path.abspath(out)) or ".", exist_ok=True)
    scene.render.filepath = out
    bpy.ops.render.render(write_still=True)
    if not os.path.exists(out) or os.path.getsize(out) == 0:
        raise DiagramError(f"the render wrote nothing: {out}")

    statues = [n for n in data.get("nodes", []) if n.get("kind") == "statue"]
    metrics = {
        "out": os.path.abspath(out),
        "map": data.get("id"),
        "size_m": size,
        "statues": [{"id": n["id"], "x": n["x"], "z": n["z"], "radius": n.get("radius")} for n in statues],
        "spawns": len(data.get("spawns", [])),
        "landmarks": len(data.get("landmarks", [])),
        "rings_drawn": len(statues),
    }
    print(json.dumps(metrics, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except DiagramError as err:
        sys.stderr.write(f"E_DIAGRAM {err}\n")
        code = EXIT_FAIL
    except Exception as err:
        sys.stderr.write(f"E_DIAGRAM {type(err).__name__}: {err}\n")
        code = EXIT_FAIL
    if code != 0:
        sys.exit(code)
