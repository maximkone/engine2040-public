# run_recipe.py - the DCC recipe runner (GEO1b). Licensed GPL-3.0-or-later.
# Reads <recipe.json> <out.glb> after `--` and writes the recipe's ops into a
# fresh factory scene, then exports one GLB. Own code; the op set is the DCC
# surface the geometry row names: cube, bevel, mirror, array. No engine import,
# no python key ever read (the cook refuses one before this starts), no network
# and no file outside the two paths. The same recipe on the same pinned Blender
# gives the same bytes.
import json
import sys

ALLOWED_OPS = ("cube", "bevel", "mirror", "array")


def die(text, code):
    sys.stdout.write(text + "\n")
    sys.stdout.flush()
    sys.exit(code)


def args_after_separator(argv):
    if "--" not in argv:
        die("recipe.missing_args", 4)
    at = argv.index("--")
    rest = argv[at + 1:]
    if len(rest) < 2:
        die("recipe.missing_args", 4)
    return rest[0], rest[1]


def recipe_ops(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            doc = json.load(handle)
    except (OSError, ValueError) as error:
        die("recipe.unreadable: %s" % error, 4)
    if not isinstance(doc, dict):
        die("recipe.not_an_object", 4)
    if "python" in doc:
        die("recipe.unknown_op: a python key is never read", 4)
    ops = doc.get("ops")
    if not isinstance(ops, list) or not ops:
        die("recipe.no_ops", 4)
    for op in ops:
        name = op.get("op") if isinstance(op, dict) else None
        if name not in ALLOWED_OPS:
            die("recipe.unknown_op: %s" % json.dumps(name), 4)
    return ops


def fresh_scene(bpy):
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    bpy.ops.object.select_all(action="DESELECT")


def apply_ops(bpy, ops):
    from mathutils import Vector

    for op in ops:
        name = op["op"]
        if name == "cube":
            size = op.get("size", [1.0, 1.0, 1.0])
            bpy.ops.mesh.primitive_cube_add(size=1.0)
            active = bpy.context.active_object
            active.scale = (float(size[0]), float(size[1]), float(size[2]))
            bpy.ops.object.transform_apply(scale=True)
        elif name == "bevel":
            active = bpy.context.active_object
            bevel = active.modifiers.new(name="bevel", type="BEVEL")
            bevel.width = float(op.get("width", 0.05))
            bevel.segments = int(op.get("segments", 2))
            bpy.context.view_layer.objects.active = active
            bpy.ops.object.modifier_apply(modifier=bevel.name)
        elif name == "mirror":
            active = bpy.context.active_object
            mirror = active.modifiers.new(name="mirror", type="MIRROR")
            axis = op.get("axis", "x")
            mirror.use_axis = (axis == "x", axis == "y", axis == "z")
            bpy.context.view_layer.objects.active = active
            bpy.ops.object.modifier_apply(modifier=mirror.name)
        elif name == "array":
            active = bpy.context.active_object
            array = active.modifiers.new(name="array", type="ARRAY")
            array.count = int(op.get("count", 3))
            offset = op.get("offset", [1.2, 0.0, 0.0])
            array.relative_offset_displace = (
                float(offset[0]),
                float(offset[1]),
                float(offset[2]),
            )
            bpy.context.view_layer.objects.active = active
            bpy.ops.object.modifier_apply(modifier=array.name)


def main():
    recipe_path, out_path = args_after_separator(sys.argv[1:])
    ops = recipe_ops(recipe_path)
    import bpy

    fresh_scene(bpy)
    apply_ops(bpy, ops)
    bpy.context.view_layer.update()
    bpy.ops.export_scene.gltf(
        filepath=out_path,
        export_format="GLB",
        export_yup=True,
        export_apply=True,
        export_extras=False,
    )
    sys.stdout.write("recipe.cooked %s\n" % out_path)


main()
