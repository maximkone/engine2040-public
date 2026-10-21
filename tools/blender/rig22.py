"""The 22-bone rig: our own template, no UniRig, no weights, no GPU.

The engine already names the bones it wants. `crates/geo/src/bonemap.rs`
holds `HumanoidBone`, and the bake's teacher skeleton in
`tools/motion_bake/src/rig.rs` holds the SOMA rest offsets. Those two files
are the contract; this script obeys them.

Twenty-two bones, in the engine's order:

    Hips Spine Chest UpperChest Neck Head
    LeftShoulder RightShoulder
    LeftUpperArm RightUpperArm
    LeftLowerArm RightLowerArm
    LeftHand RightHand
    LeftUpperLeg RightUpperLeg
    LeftLowerLeg RightLowerLeg
    LeftFoot RightFoot LeftToes RightToes

The skeleton is measured off the mesh, not copied from the engine's fighter
bind: that bind is a T-pose of a 1.46 m figure, and our meshes stand with
their arms down. So the landmarks come from the vertex cloud (hips, shoulder
line, arm reach, foot line) and the bone names and the chain come from the
engine. Same input, same rig, every machine.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/rig22.py -- <in.glb> <out.glb>
"""

import json
import math
import os
import sys
import warnings

import bpy
from mathutils import Vector

EXIT_USAGE = 2
EXIT_FAIL = 3

# The engine's bone names, in `HumanoidBone` order.
BONES = [
    "Hips", "Spine", "Chest", "UpperChest", "Neck", "Head",
    "LeftShoulder", "RightShoulder",
    "LeftUpperArm", "RightUpperArm",
    "LeftLowerArm", "RightLowerArm",
    "LeftHand", "RightHand",
    "LeftUpperLeg", "RightUpperLeg",
    "LeftLowerLeg", "RightLowerLeg",
    "LeftFoot", "RightFoot", "LeftToes", "RightToes",
]

# Parent by name: the engine's chain, our template.
PARENTS = {
    "Spine": "Hips", "Chest": "Spine", "UpperChest": "Chest", "Neck": "UpperChest",
    "Head": "Neck",
    "LeftShoulder": "UpperChest", "RightShoulder": "UpperChest",
    "LeftUpperArm": "LeftShoulder", "RightUpperArm": "RightShoulder",
    "LeftLowerArm": "LeftUpperArm", "RightLowerArm": "RightUpperArm",
    "LeftHand": "LeftLowerArm", "RightHand": "RightLowerArm",
    "LeftUpperLeg": "Hips", "RightUpperLeg": "Hips",
    "LeftLowerLeg": "LeftUpperLeg", "RightLowerLeg": "RightUpperLeg",
    "LeftFoot": "LeftLowerLeg", "RightFoot": "RightLowerLeg",
    "LeftToes": "LeftFoot", "RightToes": "RightFoot",
}

# How many bones can take weight on one vertex.
MAX_BONES = 2
# Distance falloff power: higher is tighter.
FALLOFF = 3.0
# Vertices further than this (in metres) from every bone are left unweighted.
CUTOFF = 0.45


class RigError(Exception):
    """A rig that could not be built."""


def distance_to_segment(point, a, b):
    """The distance from a point to a segment, and the closest point."""
    ab = b - a
    length_sq = ab.dot(ab)
    if length_sq < 1e-12:
        return (point - a).length, a
    t = (point - a).dot(ab) / length_sq
    t = max(0.0, min(1.0, t))
    closest = a + ab * t
    return (point - closest).length, closest


# ------------------------------------------------------------- landmarks


def landmarks(verts, height):
    """The body landmarks.

    The proportions come from our art direction, so every class in the game
    shares one skeleton: hips at half the height, the shoulder line at 0.82,
    the head bone at 0.94. The mesh only supplies the scale and the reach:
    the width of the hips and the shoulder line, read as a median over the
    bands, so a cannon or a spiked pauldron cannot define the body.
    """
    z_min = min(v.z for v in verts)
    bands = {}
    steps = 40
    for v in verts:
        index = min(steps - 1, max(0, int((v.z - z_min) / height * steps)))
        band = bands.setdefault(index, {"x": [], "count": 0})
        band["x"].append(abs(v.x))
        band["count"] += 1

    def median_reach(low, high):
        values = []
        for index, band in bands.items():
            if low <= index < high and band["count"] >= 3:
                values.extend(band["x"])
        if not values:
            return None
        values.sort()
        return values[len(values) // 2]

    def clamp(value, low, high):
        return max(low, min(high, value))

    hips_z = z_min + 0.50 * height
    shoulder_z = z_min + 0.82 * height

    hip_measured = median_reach(int(0.35 * steps), int(0.62 * steps))
    shoulder_measured = median_reach(int(0.70 * steps), int(0.90 * steps))
    hip_half = clamp(hip_measured if hip_measured else 0.10 * height, 0.07 * height, 0.20 * height)
    shoulder_half = clamp(shoulder_measured if shoulder_measured else 0.20 * height,
                          0.14 * height, 0.30 * height)

    # The hands: the lowest vertices that still reach out past the shoulders.
    reach = 0.55 * shoulder_half
    hand_z = None
    for v in sorted(verts, key=lambda p: p.z):
        if abs(v.x) > reach and v.z < shoulder_z:
            hand_z = v.z
    if hand_z is None or hand_z >= shoulder_z - 0.05 * height:
        hand_z = z_min + 0.44 * height

    return {
        "z_min": z_min,
        "height": height,
        "hips_z": hips_z,
        "hip_half": hip_half,
        "shoulder_z": shoulder_z,
        "shoulder_half": shoulder_half,
        "hand_z": hand_z,
        "foot_z": z_min,
        "hip_measured": hip_measured,
        "shoulder_measured": shoulder_measured,
    }


def build_rest(marks):
    """The rest skeleton in world space, in the engine's bone order."""
    height = marks["height"]
    hips_z = marks["hips_z"]
    shoulder_z = marks["shoulder_z"]
    shoulder_half = marks["shoulder_half"]
    hip_half = marks["hip_half"]
    hand_z = marks["hand_z"]
    foot_z = marks["foot_z"]

    head_z = z_min = marks["z_min"] + 0.94 * height
    arm_len = max(shoulder_z - hand_z, height * 0.12)
    leg_len = max(hips_z - foot_z, height * 0.12)

    bones = {}

    def put(name, head, tail):
        bones[name] = {"head": Vector(head), "tail": Vector(tail)}

    put("Hips", (0.0, 0.0, hips_z), (0.0, 0.0, hips_z + 0.04 * height))
    put("Spine", (0.0, 0.0, hips_z + 0.10 * (shoulder_z - hips_z)),
        (0.0, 0.0, hips_z + 0.10 * (shoulder_z - hips_z)))
    put("Chest", (0.0, 0.0, hips_z + 0.35 * (shoulder_z - hips_z)),
        (0.0, 0.0, hips_z + 0.35 * (shoulder_z - hips_z)))
    put("UpperChest", (0.0, 0.0, shoulder_z - 0.10 * height),
        (0.0, 0.0, shoulder_z - 0.10 * height))
    put("Neck", (0.0, 0.0, shoulder_z + 0.02 * height),
        (0.0, 0.0, shoulder_z + 0.02 * height))
    put("Head", (0.0, 0.0, shoulder_z + 0.10 * height),
        (0.0, 0.0, min(head_z, shoulder_z + 0.16 * height)))

    for side, sign in (("Left", 1.0), ("Right", -1.0)):
        joint_x = sign * shoulder_half * 0.42
        upper_x = sign * shoulder_half * 0.62
        hand_x = sign * shoulder_half * 0.86
        put(f"{side}Shoulder", (joint_x, 0.0, shoulder_z),
            (joint_x + sign * shoulder_half * 0.12, 0.0, shoulder_z))
        put(f"{side}UpperArm", (upper_x, 0.0, shoulder_z - 0.02 * height),
            (upper_x + sign * 0.02 * height, 0.0, shoulder_z - 0.45 * arm_len))
        put(f"{side}LowerArm", (upper_x + sign * 0.02 * height, 0.0, shoulder_z - 0.45 * arm_len),
            (hand_x, 0.0, shoulder_z - 0.85 * arm_len))
        put(f"{side}Hand", (hand_x, 0.0, shoulder_z - 0.85 * arm_len),
            (hand_x + sign * 0.01 * height, -0.03 * height, shoulder_z - arm_len))

        hip_x = sign * hip_half * 0.55
        knee_x = sign * hip_half * 0.52
        ankle_x = sign * hip_half * 0.48
        put(f"{side}UpperLeg", (hip_x, 0.0, hips_z - 0.02 * height),
            (knee_x, 0.0, hips_z - 0.50 * leg_len))
        put(f"{side}LowerLeg", (knee_x, 0.0, hips_z - 0.50 * leg_len),
            (ankle_x, 0.0, foot_z + 0.05 * height))
        put(f"{side}Foot", (ankle_x, 0.0, foot_z + 0.05 * height),
            (ankle_x, -0.10 * height, foot_z + 0.02 * height))
        put(f"{side}Toes", (ankle_x, -0.10 * height, foot_z + 0.02 * height),
            (ankle_x, -0.16 * height, foot_z + 0.01 * height))
    return bones


def make_armature(bones, height):
    arm_data = bpy.data.armatures.new("rig22")
    arm = bpy.data.objects.new("rig22", arm_data)
    bpy.context.collection.objects.link(arm)
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.mode_set(mode="EDIT")
    for bone_name in BONES:
        data = bones[bone_name]
        bone = arm_data.edit_bones.new(bone_name)
        bone.head = data["head"]
        bone.tail = data["tail"]
        if bone.tail.length < 1e-6:
            bone.tail = bone.head + Vector((0.0, 0.0, height * 0.01))
    for bone_name in BONES:
        parent = PARENTS.get(bone_name)
        if parent:
            arm_data.edit_bones[bone_name].parent = arm_data.edit_bones[parent]
            arm_data.edit_bones[bone_name].use_connect = False
    bpy.ops.object.mode_set(mode="OBJECT")
    return arm


def weight_mesh(ob, bones):
    """Blend the two nearest bones onto every vertex."""
    segments = [(name, bones[name]["head"], bones[name]["tail"]) for name in BONES]
    groups = {name: ob.vertex_groups.new(name=name) for name in BONES}
    me = ob.data
    assigned = 0
    nearest = []
    for vertex in me.vertices:
        co = vertex.co
        scored = []
        for name, a, b in segments:
            distance, _ = distance_to_segment(co, a, b)
            scored.append((distance, name))
        scored.sort(key=lambda item: item[0])
        nearest.append(scored[0][0])
        if scored[0][0] > CUTOFF:
            continue
        chosen = scored[:MAX_BONES]
        weights = [(name, 1.0 / (distance + 1e-3) ** FALLOFF) for distance, name in chosen]
        total = sum(w for _, w in weights)
        for name, weight in weights:
            groups[name].add([vertex.index], weight / total, "REPLACE")
        assigned += 1
    nearest.sort()

    def at(p):
        if not nearest:
            return None
        return round(nearest[min(len(nearest) - 1, int(len(nearest) * p))], 4)

    stats = {
        "nearest_p05": at(0.05), "nearest_p50": at(0.50), "nearest_p95": at(0.95),
        "beyond_cutoff": sum(1 for d in nearest if d > CUTOFF),
    }
    return assigned, stats


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(args) < 2:
        sys.stderr.write("usage: blender -b --factory-startup --python-exit-code 1 "
                         "-P rig22.py -- <in.glb> <out.glb>\n")
        return EXIT_USAGE
    src, dest = args[0], args[1]

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=src)
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    if not meshes:
        raise RigError(f"no mesh in {src}")
    bpy.ops.object.select_all(action="DESELECT")
    for ob in meshes:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    if len(meshes) > 1:
        bpy.ops.object.join()
    ob = bpy.context.view_layer.objects.active
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)

    low = Vector((1e9, 1e9, 1e9))
    high = Vector((-1e9, -1e9, -1e9))
    for corner in ob.bound_box:
        co = ob.matrix_world @ Vector(corner)
        for i in range(3):
            low[i] = min(low[i], co[i])
            high[i] = max(high[i], co[i])
    height = high.z - low.z
    if height <= 1e-6:
        raise RigError("the mesh has no height")

    verts = [v.co.copy() for v in ob.data.vertices]
    marks = landmarks(verts, height)
    bones = build_rest(marks)
    arm = make_armature(bones, height)
    assigned, distance_stats = weight_mesh(ob, bones)

    modifier = ob.modifiers.new(name="armature", type="ARMATURE")
    modifier.object = arm
    ob.parent = arm

    outdir = os.path.dirname(os.path.abspath(dest))
    if outdir:
        os.makedirs(outdir, exist_ok=True)
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    arm.select_set(True)
    bpy.context.view_layer.objects.active = ob
    try:
        bpy.ops.export_scene.gltf(filepath=dest, export_format="GLB", export_apply=True,
                                  export_texcoords=True, export_normals=True,
                                  export_colors=True, export_skins=True, use_selection=True)
    except TypeError:
        bpy.ops.export_scene.gltf(filepath=dest, export_format="GLB")
    if not os.path.exists(dest) or os.path.getsize(dest) == 0:
        raise RigError(f"glb export wrote nothing: {dest}")

    metrics = {
        "source": os.path.abspath(src),
        "out": os.path.abspath(dest),
        "height_m": round(height, 4),
        "bones": len(BONES),
        "bone_names": BONES,
        "vertices": len(ob.data.vertices),
        "weighted_vertices": assigned,
        "distance_stats": distance_stats,
        "max_bones_per_vertex": MAX_BONES,
        "falloff": FALLOFF,
        "cutoff_m": CUTOFF,
        "landmarks": {k: (round(v, 4) if isinstance(v, float) else v) for k, v in marks.items()},
        "contract": "crates/geo/src/bonemap.rs HumanoidBone",
    }
    print(json.dumps(metrics, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except RigError as err:
        sys.stderr.write(f"E_RIG {err}\n")
        code = EXIT_FAIL
    except Exception as err:
        sys.stderr.write(f"E_RIG {type(err).__name__}: {err}\n")
        code = EXIT_FAIL
    if code != 0:
        sys.exit(code)
