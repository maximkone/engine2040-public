"""The style pass on the body: a caricature drawn over a real anatomy.

This is what separates a 2005 character from a real person. The 2005 games
did not use realistic bodies: they used chunky ones, with a head bigger than
life, shoulders wider than life, a waist pulled in, short thick legs, big
hands and big boots, and a painted face on top. A real anatomy can carry
that drawing. The anatomy is the base; the drawing is the style.

So: the body comes from MakeHuman (CC0) and this script bends it. Every
vertex knows how high it is, and the height picks a band out of
content/design/proportions.json, and the band says how much wider or longer
that part of the body is. The head is a separate lump scaled about the neck,
so the face keeps its shape while the head grows.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/proportion_pass.py -- <in.glb> <out.glb> --faction wardens
"""

import json
import os
import sys

import bpy

EXIT_USAGE = 2
EXIT_FAIL = 3

HEAD_TOP = 0.88
NECK = 0.86


class ProportionError(Exception):
    """A proportion pass that could not be finished."""


def arg(name, fallback=None):
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if name in argv:
        i = argv.index(name)
        if i + 1 < len(argv):
            return argv[i + 1]
    return fallback


def load_spec():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
    path = os.path.join(root, "content", "design", "proportions.json")
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def band_for(t, bands):
    for band in bands:
        if band["from"] <= t < band["to"]:
            return band
    return bands[-1]


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(args) < 2:
        sys.stderr.write("usage: blender -b --factory-startup --python-exit-code 1 "
                         "-P proportion_pass.py -- <in.glb> <out.glb> --faction wardens\n")
        return EXIT_USAGE
    src, dest = args[0], args[1]
    faction = str(arg("--faction", "wardens")).lower()
    class_id = arg("--class")

    spec = load_spec()
    if class_id:
        faction = spec.get("classes", {}).get(class_id, faction)
    profile = spec.get("factions", {}).get(faction)
    if profile is None:
        raise ProportionError(f"no proportions for {faction}")
    bands = profile["bands"]

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=src)
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    if not meshes:
        raise ProportionError(f"no mesh in {src}")
    bpy.ops.object.select_all(action="DESELECT")
    for ob in meshes:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    if len(meshes) > 1:
        bpy.ops.object.join()
    ob = bpy.context.view_layer.objects.active
    ob.name = "body"
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    if ob.data.shape_keys is not None:
        bpy.ops.object.convert(target="MESH")
        ob = bpy.context.view_layer.objects.active
        ob.name = "body"

    me = ob.data
    zs = [v.co.z for v in me.vertices]
    if not zs:
        raise ProportionError("the mesh has no vertices")
    z_min, z_max = min(zs), max(zs)
    height = max(z_max - z_min, 1e-6)
    centre_x = sum(v.co.x for v in me.vertices) / len(me.vertices)
    centre_y = sum(v.co.y for v in me.vertices) / len(me.vertices)

    head = profile.get("head", {})
    head_scale = float(head.get("scale", 1.0))
    head_x = float(head.get("x", 1.0))
    head_y = float(head.get("y", 1.0))
    neck_z = z_min + NECK * height

    for vertex in me.vertices:
        co = vertex.co
        t = (co.z - z_min) / height
        if t >= HEAD_TOP:
            # The head is a lump scaled about the neck: the face keeps its
            # shape and the head grows around it.
            co.z = neck_z + (co.z - neck_z) * head_scale
            co.x = centre_x + (co.x - centre_x) * (head_x * head_scale)
            co.y = centre_y + (co.y - centre_y) * (head_y * head_scale)
            continue
        band = band_for(t, bands)
        co.x = centre_x + (co.x - centre_x) * float(band["x"])
        co.y = centre_y + (co.y - centre_y) * float(band["y"])
        co.z = z_min + (co.z - z_min) * float(band["z"])
    me.update()

    os.makedirs(os.path.dirname(os.path.abspath(dest)) or ".", exist_ok=True)
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    try:
        bpy.ops.export_scene.gltf(filepath=dest, export_format="GLB", export_apply=True,
                                  export_texcoords=True, export_normals=True, use_selection=True)
    except TypeError:
        bpy.ops.export_scene.gltf(filepath=dest, export_format="GLB")
    if not os.path.exists(dest) or os.path.getsize(dest) == 0:
        raise ProportionError(f"export wrote nothing: {dest}")

    zs = [v.co.z for v in me.vertices]
    metrics = {
        "source": os.path.abspath(src),
        "out": os.path.abspath(dest),
        "faction": faction,
        "class": class_id,
        "vertices": len(me.vertices),
        "height_m": round(max(zs) - min(zs), 4),
        "head_scale": head_scale,
        "bands": len(bands),
    }
    print(json.dumps(metrics, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except ProportionError as err:
        sys.stderr.write(f"E_PROP {err}\n")
        code = EXIT_FAIL
    except Exception as err:
        sys.stderr.write(f"E_PROP {type(err).__name__}: {err}\n")
        code = EXIT_FAIL
    if code != 0:
        sys.exit(code)
