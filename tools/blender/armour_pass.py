"""The armour pass: our plates on our rig.

A character with a sword is a character with a prop. A character with
pauldrons, bracers, greaves, a belt and a helm is a character in armour,
and that is the difference between a naked brute and a soldier at thirty
metres.

The plates are ours (tools/blender/prop_gen.py builds them), the rig is ours
(tools/blender/rig22.py), and this script is the join: it takes the rigged
body, hangs each plate on the bone that carries it, paints it in the
faction palette, and writes one asset back out.

Bone parented, not weighted: a plate is rigid, it does not bend with the
arm, and the old games did it the same way.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/armour_pass.py -- <rigged.glb> <out.glb> --class ashkin_gunner
"""

import json
import math
import os
import sys

import bpy
from mathutils import Matrix, Vector

sys.path.append(os.path.dirname(os.path.abspath(__file__)))
import prop_gen  # noqa: E402  the plate builders live with the props

EXIT_USAGE = 2
EXIT_FAIL = 3

# The plate for each bone: the builder, how it sits on the bone, and how
# big it is against the character's height.
PIECES = {
    "LeftShoulder": {"build": "pauldron", "align": "up", "out": 0.06, "scale": 1.0, "kind": "plate"},
    "RightShoulder": {"build": "pauldron", "align": "up", "out": 0.06, "scale": 1.0, "kind": "plate"},
    "LeftLowerArm": {"build": "bracer", "align": "bone", "out": 0.0, "scale": 1.0, "kind": "plate"},
    "RightLowerArm": {"build": "bracer", "align": "bone", "out": 0.0, "scale": 1.0, "kind": "plate"},
    "LeftLowerLeg": {"build": "greave", "align": "bone", "out": 0.0, "scale": 1.0, "kind": "plate"},
    "RightLowerLeg": {"build": "greave", "align": "bone", "out": 0.0, "scale": 1.0, "kind": "plate"},
    "Hips": {"build": "belt", "align": "up", "out": 0.0, "scale": 1.0, "kind": "strap"},
    "Head": {"build": "helm", "align": "up", "out": 0.03, "scale": 1.0, "kind": "plate"},
}

# The natural size of each plate, so a plate can be fitted to the limb it
# goes on.
NATURAL = {"pauldron": 0.17, "bracer": 0.075, "greave": 0.085, "belt": 0.19, "helm": 0.13}

# Which pieces each class wears, and what the sheet says the class is.
WARDEN_PIECES = ["LeftShoulder", "RightShoulder", "Hips"]
ASHKIN_PIECES = ["LeftShoulder", "RightShoulder", "LeftLowerArm", "RightLowerArm",
                 "LeftLowerLeg", "RightLowerLeg", "Hips"]

# The classes that go bare or light, because their sheet says so.
LIGHT = {"warden_rogue", "warden_assassin", "warden_archer", "warden_mage",
         "warden_enchanter", "warden_acolyte", "warden_priest",
         "ashkin_officiator", "ashkin_rune", "ashkin_life", "ashkin_shadow"}


class ArmourError(Exception):
    """An armour pass that could not be finished."""


def arg(name, fallback=None):
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if name in argv:
        i = argv.index(name)
        if i + 1 < len(argv):
            return argv[i + 1]
    return fallback


def catalogue():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
    path = os.path.join(root, "content", "design", "armour.json")
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def material(name, colour):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (colour[0], colour[1], colour[2], 1.0)
    return mat


def build_plate(build_key, palette):
    builder = prop_gen.BUILDERS.get(build_key)
    if builder is None:
        raise ArmourError(f"no builder named {build_key}")
    parts = builder(palette)
    bpy.ops.object.select_all(action="DESELECT")
    for ob in parts:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = parts[0]
    if len(parts) > 1:
        bpy.ops.object.join()
    ob = bpy.context.view_layer.objects.active
    ob.name = f"armour_{build_key}"
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    for polygon in ob.data.polygons:
        polygon.use_smooth = False
    return ob


def bone_matrix(armature, bone_name):
    """The bone's rest matrix in world space, and its length."""
    bone = armature.data.bones.get(bone_name)
    if bone is None:
        return None, 0.0
    head = armature.matrix_world @ bone.head_local
    tail = armature.matrix_world @ bone.tail_local
    return head, (tail - head).length


def transfer_weights(plate, body):
    """Give the plate the body's own weights, nearest vertex to nearest vertex.

    A rigid weight of one on a single bone fights the bind pose and lands the
    plate a bone away from where it was measured. Copying the body's weights
    means the plate deforms exactly like the limb it sits on.
    """
    import numpy as np

    body_co = np.array([[(body.matrix_world @ v.co)[i] for i in range(3)]
                         for v in body.data.vertices], dtype=np.float32)
    if not len(body_co):
        return
    plate_co = np.array([[(plate.matrix_world @ v.co)[i] for i in range(3)]
                          for v in plate.data.vertices], dtype=np.float32)
    if not len(plate_co):
        return
    diff = plate_co[:, None, :] - body_co[None, :, :]
    nearest = (diff * diff).sum(axis=2).argmin(axis=1)

    for group in body.vertex_groups:
        weights = np.zeros(len(body_co), dtype=np.float32)
        for vertex in body.data.vertices:
            for entry in vertex.groups:
                if entry.group == group.index:
                    weights[vertex.index] = entry.weight
        target = plate.vertex_groups.get(group.name)
        if target is None:
            target = plate.vertex_groups.new(name=group.name)
        for plate_index, body_index in enumerate(nearest):
            if weights[body_index] > 0.0:
                target.add([plate_index], float(weights[body_index]), "REPLACE")


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(args) < 2:
        sys.stderr.write("usage: blender -b --factory-startup --python-exit-code 1 "
                         "-P armour_pass.py -- <rigged.glb> <out.glb> --class <id>\n")
        return EXIT_USAGE
    src, dest = args[0], args[1]
    class_id = arg("--class")
    if not class_id:
        raise ArmourError("--class <id> is required: the armour sheet is per class")

    spec = catalogue()
    entry = spec.get("classes", {}).get(class_id)
    if entry is None:
        raise ArmourError(f"no armour sheet for {class_id}")
    faction = entry.get("faction", "ashkin")
    palette = prop_gen.FACTION_PALETTE.get(faction, prop_gen.NEUTRAL)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=src)
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH" and o.name != "body"]
    bodies = [o for o in bpy.context.scene.objects if o.type == "MESH" and o.name == "body"]
    if not bodies:
        bodies = meshes[:1]
        meshes = meshes[1:]
    body = bodies[0]
    armatures = [o for o in bpy.context.scene.objects if o.type == "ARMATURE"]
    if not armatures:
        raise ArmourError("the file has no armature: run rig22.py first")
    armature = armatures[0]

    # Reset the pose to the rest pose. An imported rig can carry a pose that
    # is not its rest pose, and then the body follows its bind pose while the
    # plates follow the pose: the plates end up somewhere else entirely.
    for pose_bone in armature.pose.bones:
        rest_bone = armature.data.bones.get(pose_bone.name)
        if rest_bone is not None:
            pose_bone.matrix = rest_bone.matrix_local
    bpy.context.view_layer.update()

    low = Vector((1e9, 1e9, 1e9))
    high = Vector((-1e9, -1e9, -1e9))
    for corner in body.bound_box:
        co = body.matrix_world @ Vector(corner)
        for i in range(3):
            low[i] = min(low[i], co[i])
            high[i] = max(high[i], co[i])
    height = high.z - low.z
    scale = height / 2.0

    wanted = entry.get("pieces", [])

    worn = []
    for piece in wanted:
        bone_name = piece.get("bone")
        spec_piece = PIECES.get(bone_name)
        if spec_piece is None:
            continue
        head, length = bone_matrix(armature, bone_name)
        if head is None:
            continue
        bone = armature.data.bones.get(bone_name)
        tail = armature.matrix_world @ bone.tail_local
        direction = tail - head
        if direction.length < 1e-6:
            continue
        plate = build_plate(spec_piece["build"], palette)
        # The sheet's radius is the plate's size against the character: the
        # template's 0.20 is a full pauldron.
        size = float(piece.get("radius", 0.20)) / 0.20 * spec_piece["scale"]
        # Rigid skinning, not bone parenting: the plate is one bone at weight
        # one, and the engine reads joints and weights. A bone parented prop
        # does not survive the export, and the engine would not know what to
        # do with it.
        bone = armature.data.bones.get(bone_name)
        tail = armature.matrix_world @ bone.tail_local
        direction = tail - head
        if direction.length < 1e-6:
            continue
        # A rigid attachment lives in the bone's own space: the bone's local
        # frame has the bone along +Y from its head, and a plate worn on the
        # bone sits along that axis, while a plate set on the bone keeps the
        # world's up. Both are expressed in bone local space, so the rest
        # pose puts the plate exactly where it was measured and the rig
        # carries it afterwards.
        axis_world = direction.normalized() if spec_piece.get("align") == "bone" else Vector((0.0, 0.0, 1.0))
        bone_local = bone.matrix_local
        up_local = (bone_local.to_3x3().inverted() @ Vector((0.0, 0.0, 1.0))).normalized()
        if spec_piece.get("align") == "bone":
            axis_local = Vector((0.0, 1.0, 0.0))
            offset_local = Vector((0.0, length * 0.5, 0.0))
        else:
            axis_local = up_local
            offset_local = Vector((0.0, 0.0, 0.0))
        rotation_local = axis_local.to_track_quat("Z", "Y").to_matrix().to_4x4()
        # The plate is fitted to the body it goes on: the limb's own thickness
        # is measured from the body's vertices around the bone, and the plate
        # is scaled to clear it. A generated body is lumpy and a plate sized
        # for an ideal human ends up buried inside it.
        local = 0.0
        for vertex in body.data.vertices:
            point = body.matrix_world @ vertex.co
            offset = point - head
            if offset.length > 0.18:
                continue
            across = offset - axis_world * offset.dot(axis_world)
            if across.length > local:
                local = across.length
        natural = NATURAL.get(spec_piece["build"], 0.1)
        size = ((local + 0.02 * scale) / natural) * (float(piece.get("radius", 0.20)) / 0.20)
        size = max(0.5, min(2.2, size))
        if spec_piece.get("align") == "bone":
            seat = head + axis_world * (length * 0.5)
        else:
            # A plate set on a bone sits on it: a small lift, not a share of
            # the body's width, which would put a pauldron on the head.
            seat = head + axis_world * (0.02 * scale)
        plate.matrix_basis = (armature.matrix_world.inverted()
                              @ bone_local
                              @ Matrix.Translation(tuple(offset_local))
                              @ rotation_local
                              @ Matrix.Diagonal((size, size, size, 1.0)))
        # The world place, for the bone aligned pieces: the bone's own frame
        # is fine for the offset, but the seat has to be where the body is.
        if spec_piece.get("align") == "bone":
            world_seat = (armature.matrix_world
                          @ (bone_local @ Matrix.Translation(tuple(offset_local)))).to_translation()
            plate.matrix_basis = (armature.matrix_world.inverted()
                                  @ Matrix.Translation(tuple(world_seat))
                                  @ axis_world.to_track_quat("Z", "Y").to_matrix().to_4x4()
                                  @ Matrix.Diagonal((size, size, size, 1.0)))
        else:
            plate.matrix_basis = (armature.matrix_world.inverted()
                                  @ Matrix.Translation(tuple(seat))
                                  @ axis_world.to_track_quat("Z", "Y").to_matrix().to_4x4()
                                  @ Matrix.Diagonal((size, size, size, 1.0)))
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
        # The plate takes the body's own weights: the nearest body vertex
        # decides. A rigid weight of one on a single bone fights the bind
        # pose and lands the plate a bone away from where it was measured.
        transfer_weights(plate, body)
        # The plate joins the body: one mesh, one rig, and the plate is
        # weighted exactly like the body under it.
        bpy.ops.object.select_all(action="DESELECT")
        body.select_set(True)
        plate.select_set(True)
        bpy.context.view_layer.objects.active = body
        plate_tris = len(plate.data.polygons) * 2
        bpy.ops.object.join()
        body = bpy.context.view_layer.objects.active
        body.name = "body"
        worn.append({"bone": bone_name, "build": spec_piece["build"],
                     "kind": piece.get("kind", "plate"),
                     "triangles": plate_tris})

    # The body and the plates share one rig, and the plates keep their own
    # material, which is where the metal, cloth and leather split comes from.
    bpy.ops.object.select_all(action="DESELECT")
    body.select_set(True)
    for ob in meshes:
        ob.select_set(True)
    armature.select_set(True)
    bpy.context.view_layer.objects.active = body

    os.makedirs(os.path.dirname(os.path.abspath(dest)) or ".", exist_ok=True)
    try:
        bpy.ops.export_scene.gltf(filepath=dest, export_format="GLB", export_apply=True,
                                  export_texcoords=True, export_normals=True,
                                  export_colors=True, export_skins=True, use_selection=True)
    except TypeError:
        bpy.ops.export_scene.gltf(filepath=dest, export_format="GLB")
    if not os.path.exists(dest) or os.path.getsize(dest) == 0:
        raise ArmourError(f"could not write {dest}")

    metrics = {
        "source": os.path.abspath(src),
        "out": os.path.abspath(dest),
        "class": class_id,
        "faction": faction,
        "height_m": round(height, 4),
        "pieces": worn,
        "piece_count": len(worn),
    }
    print(json.dumps(metrics, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except ArmourError as err:
        sys.stderr.write(f"E_ARMOUR {err}\n")
        code = EXIT_FAIL
    except Exception as err:
        sys.stderr.write(f"E_ARMOUR {type(err).__name__}: {err}\n")
        traceback.print_exc()
        code = EXIT_FAIL
    if code != 0:
        sys.exit(code)
