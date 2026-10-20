"""Write the FBX import fixture for GEO9a with the pinned Blender.

Run once, headless, through the pin in `tools/dcc/blender.pin.json`:

    blender -b --factory-startup -noaudio --python-exit-code 1 \
        --python tools/dcc/blender/make_fbx_fixture.py

Builds a 1 m cube and a UV sphere, exports both with `bpy.ops.export_scene.fbx`
(axis forward -Z, up Y: the file's own axes, so the importer has to convert),
and writes the triangle count of each object as Blender sees it. Both files go
to `crates/geo/tests/data/` unless `--out-dir <dir>` names another folder.
Prints one `E2040-FIXTURE` line per written file and exits nonzero on any
unexpected state (missing exporter, missing object), never on a guess.
"""

import argparse
import json
import os
import sys

import bpy

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
DEFAULT_OUT = os.path.join(ROOT, "crates", "geo", "tests", "data")
FBX_NAME = "two_solids.fbx"
JSON_NAME = "two_solids.json"

# Segment and ring counts of the sphere, pinned so a Blender update cannot
# silently change the triangle count the test compares against the JSON.
SPHERE_SEGMENTS = 32
SPHERE_RINGS = 16


def out_dir(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", default=DEFAULT_OUT)
    return os.path.abspath(parser.parse_args(argv).out_dir)


def build_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)

    bpy.ops.mesh.primitive_cube_add(size=1.0, location=(0.0, 0.0, 0.5))
    cube = bpy.context.active_object
    cube.name = "Cube"

    bpy.ops.mesh.primitive_uv_sphere_add(
        segments=SPHERE_SEGMENTS,
        ring_count=SPHERE_RINGS,
        radius=0.5,
        location=(2.0, 0.0, 0.5),
    )
    sphere = bpy.context.active_object
    sphere.name = "Sphere"
    bpy.ops.object.shade_smooth()

    return [cube, sphere]


def triangles(obj):
    """Triangles Blender would draw for the object, after modifiers."""
    depsgraph = bpy.context.evaluated_depsgraph_get()
    mesh = obj.evaluated_get(depsgraph).to_mesh()
    try:
        mesh.calc_loop_triangles()
        return len(mesh.loop_triangles)
    finally:
        obj.evaluated_get(depsgraph).to_mesh_clear()


def export_fbx(path, objects):
    if not hasattr(bpy.ops.export_scene, "fbx"):
        raise SystemExit("blender.fbx_exporter_missing: bpy.ops.export_scene.fbx is gone")
    bpy.ops.object.select_all(action="DESELECT")
    for obj in objects:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]
    bpy.ops.export_scene.fbx(
        filepath=path,
        check_existing=False,
        use_selection=True,
        object_types={"MESH"},
        axis_forward="-Z",
        axis_up="Y",
        apply_unit_scale=True,
        apply_scale_options="FBX_SCALE_NONE",
        bake_space_transform=False,
        use_mesh_modifiers=True,
        mesh_smooth_type="FACE",
        add_leaf_bones=False,
        bake_anim=False,
        path_mode="STRIP",
        embed_textures=False,
    )


def main(argv):
    folder = out_dir(argv)
    os.makedirs(folder, exist_ok=True)
    objects = build_scene()
    counts = [{"name": obj.name, "triangles": triangles(obj)} for obj in objects]
    if any(row["triangles"] == 0 for row in counts):
        raise SystemExit(f"blender.fixture_empty: {counts}")

    fbx_path = os.path.join(folder, FBX_NAME)
    json_path = os.path.join(folder, JSON_NAME)
    export_fbx(fbx_path, objects)

    payload = {
        "script": "tools/dcc/blender/make_fbx_fixture.py",
        "blender": bpy.app.version_string,
        "fbx": FBX_NAME,
        "axes": {"forward": "-Z", "up": "Y"},
        "unit_meters": 1.0,
        "meshes": counts,
    }
    with open(json_path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(payload, f, indent=2)
        f.write("\n")

    for path in (fbx_path, json_path):
        print("E2040-FIXTURE", path, os.path.getsize(path))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []))
