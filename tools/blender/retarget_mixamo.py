"""Put a Mixamo clip on a Rocketbox body, offline, and export it as a GLB.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/retarget_mixamo.py -- \
      --target <rocketbox.fbx> --source <mixamo_clip.fbx> --out <clip.glb> \
      [--textures <dir>] [--render <strip.png>] [--in-place]

The transfer, per mapped bone and per frame, in world space:
  D  = W_src(t) @ W0_src^-1          the source bone's turn away from its own rest
  C  = the turn that lays the target's rest bone direction on the source's
       (Rocketbox rests in an A-pose, Mixamo in a T-pose; without C the arms
       ride ~30 degrees low for the whole clip)
  W_tgt(t) = D @ C @ W0_tgt
Bones are posed parent first. The pelvis follows the Mixamo hips, scaled by
the two legs' length ratio; --in-place keeps only its height (the sim moves
the body). Bones with no Mixamo partner (the face) keep their rest.
The same world-delta method was measured on the SOMA-77 path at 0.01 degrees
on the arms (tools/motion/mixamo_npz.py).
"""

import os
import sys

import bpy
from mathutils import Matrix, Quaternion, Vector

EXIT_USAGE = 2
EXIT_FAIL = 3

# Mixamo bone (prefix stripped) -> Rocketbox Biped bone, each with the child
# whose head gives the bone's direction (None: no direction alignment).
PAIRS = [
    ("Hips", "Bip01 Pelvis", None, None),
    ("Spine", "Bip01 Spine", "Spine1", "Bip01 Spine1"),
    ("Spine1", "Bip01 Spine1", "Spine2", "Bip01 Spine2"),
    ("Spine2", "Bip01 Spine2", "Neck", "Bip01 Neck"),
    ("Neck", "Bip01 Neck", "Head", "Bip01 Head"),
    ("Head", "Bip01 Head", None, None),
]
for mx, rb in (("Left", "L"), ("Right", "R")):
    PAIRS += [
        (f"{mx}Shoulder", f"Bip01 {rb} Clavicle", f"{mx}Arm", f"Bip01 {rb} UpperArm"),
        (f"{mx}Arm", f"Bip01 {rb} UpperArm", f"{mx}ForeArm", f"Bip01 {rb} Forearm"),
        (f"{mx}ForeArm", f"Bip01 {rb} Forearm", f"{mx}Hand", f"Bip01 {rb} Hand"),
        (f"{mx}Hand", f"Bip01 {rb} Hand", f"{mx}HandMiddle1", f"Bip01 {rb} Finger2"),
        (f"{mx}UpLeg", f"Bip01 {rb} Thigh", f"{mx}Leg", f"Bip01 {rb} Calf"),
        (f"{mx}Leg", f"Bip01 {rb} Calf", f"{mx}Foot", f"Bip01 {rb} Foot"),
        (f"{mx}Foot", f"Bip01 {rb} Foot", f"{mx}ToeBase", f"Bip01 {rb} Toe0"),
        (f"{mx}ToeBase", f"Bip01 {rb} Toe0", None, None),
    ]
    thumb = ["Finger0", "Finger01", "Finger02"]
    PAIRS += [(f"{mx}HandThumb{i + 1}", f"Bip01 {rb} {thumb[i]}",
               f"{mx}HandThumb{i + 2}" if i < 2 else None,
               f"Bip01 {rb} {thumb[i + 1]}" if i < 2 else None) for i in range(3)]
    for k, finger in enumerate(("Index", "Middle", "Ring", "Pinky"), start=1):
        names = [f"Finger{k}", f"Finger{k}1", f"Finger{k}2"]
        PAIRS += [(f"{mx}Hand{finger}{i + 1}", f"Bip01 {rb} {names[i]}",
                   f"{mx}Hand{finger}{i + 2}" if i < 2 else None,
                   f"Bip01 {rb} {names[i + 1]}" if i < 2 else None) for i in range(3)]


def arg(name, fallback=None):
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if name in argv and argv.index(name) + 1 < len(argv):
        return argv[argv.index(name) + 1]
    return fallback


def flag(name):
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    return name in argv


def strip(name):
    return name.split(":")[-1]


def rot(m):
    """The pure rotation of a 4x4 world matrix (rigs carry 0.01 scales)."""
    return m.to_quaternion().to_matrix()


def import_fbx(path):
    before = set(bpy.data.objects)
    if path.lower().endswith(".bvh"):
        # A BVH source (the combat-motion sidecar writes HumanML3D-22 with
        # Mixamo-style names, no fingers): Y-up file, Blender turns it Z-up.
        bpy.ops.import_anim.bvh(filepath=path, axis_forward="-Z", axis_up="Y",
                                update_scene_fps=True, update_scene_duration=True)
    else:
        bpy.ops.import_scene.fbx(filepath=path, automatic_bone_orientation=False)
    new = [o for o in set(bpy.data.objects) - before]
    arms = [o for o in new if o.type == "ARMATURE"]
    if len(arms) != 1:
        raise RuntimeError(f"{path}: expected one armature, found {len(arms)}")
    return arms[0], [o for o in new if o.type == "MESH"]


def wire_textures(meshes, tex_dir):
    """Rocketbox maps by part name: <id>_<part>_color.tga, _normal.tga."""
    if not tex_dir or not os.path.isdir(tex_dir):
        return
    files = os.listdir(tex_dir)
    for ob in meshes:
        for slot in ob.material_slots:
            m = slot.material
            if m is None:
                continue
            key = m.name.lower()
            part = "head" if "head" in key else ("opacity" if "opacity" in key else "body")
            color = [f for f in files if f.endswith("_color.tga") and part in f]
            if not color:
                color = [f for f in files if f.endswith(".tga") and "color" in f and part in f]
            if not color:
                continue
            m.use_nodes = True
            nt = m.node_tree
            bsdf = nt.nodes.get("Principled BSDF")
            if not bsdf:
                continue
            c = nt.nodes.new("ShaderNodeTexImage")
            c.image = bpy.data.images.load(os.path.join(tex_dir, color[0]), check_existing=True)
            nt.links.new(c.outputs["Color"], bsdf.inputs["Base Color"])
            if part == "opacity":
                nt.links.new(c.outputs["Alpha"], bsdf.inputs["Alpha"])
            normal = color[0].replace("_color", "_normal")
            if normal in files:
                n = nt.nodes.new("ShaderNodeTexImage")
                n.image = bpy.data.images.load(os.path.join(tex_dir, normal), check_existing=True)
                n.image.colorspace_settings.name = "Non-Color"
                nm = nt.nodes.new("ShaderNodeNormalMap")
                nt.links.new(n.outputs["Color"], nm.inputs["Color"])
                nt.links.new(nm.outputs["Normal"], bsdf.inputs["Normal"])
            bsdf.inputs["Roughness"].default_value = 0.6


def retarget_onto(tgt, src, in_place, action_name):
    """Key the source armature's action onto the target as a new action.

    Returns (action, first, last, pairs, scale). The target's pose is reset
    first, so several clips can be put on one body in a row.
    """
    action = src.animation_data.action if src.animation_data else None
    if action is None:
        raise RuntimeError(f"{src.name}: no animation")
    first, last = (int(round(v)) for v in action.frame_range)
    scene = bpy.context.scene

    sb = {strip(b.name): b.name for b in src.data.bones}
    tb = {b.name: b.name for b in tgt.data.bones}
    pairs = [p for p in PAIRS if p[0] in sb and p[1] in tb]
    if not any(p[0] == "Hips" for p in pairs):
        raise RuntimeError("no Hips/Bip01 Pelvis pair: not a Mixamo source on a Rocketbox target")

    sw, tw = src.matrix_world, tgt.matrix_world

    def src_head_rest(name):
        return sw @ src.data.bones[sb[name]].head_local

    def tgt_head_rest(name):
        return tw @ tgt.data.bones[name].head_local

    # Rest: world rotations, and the alignment C per pair.
    rest_src = {p[0]: rot(sw @ src.data.bones[sb[p[0]]].matrix_local) for p in pairs}
    rest_tgt = {p[1]: rot(tw @ tgt.data.bones[p[1]].matrix_local) for p in pairs}
    align = {}
    for mx, rb, mx_child, rb_child in pairs:
        if mx_child and mx_child in sb and rb_child in tb:
            ds = (src_head_rest(mx_child) - src_head_rest(mx)).normalized()
            dt = (tgt_head_rest(rb_child) - tgt_head_rest(rb)).normalized()
            align[rb] = dt.rotation_difference(ds).to_matrix()
        else:
            align[rb] = Matrix.Identity(3)

    def leg(head):
        names = ("LeftUpLeg", "LeftLeg", "LeftFoot") if head == "src" else (
            "Bip01 L Thigh", "Bip01 L Calf", "Bip01 L Foot")
        pts = [src_head_rest(n) if head == "src" else tgt_head_rest(n) for n in names]
        return sum((pts[i + 1] - pts[i]).length for i in range(2))

    scale = leg("tgt") / max(leg("src"), 1e-9)
    hips0 = src_head_rest("Hips")
    pelvis0 = tgt_head_rest("Bip01 Pelvis")

    # Parent-first order on the target.
    order = [b.name for b in tgt.data.bones]
    by_tgt = {p[1]: p for p in pairs}
    tgt.animation_data_create()
    out_action = bpy.data.actions.new(name=action_name)
    tgt.animation_data.action = out_action
    for pb in tgt.pose.bones:
        pb.rotation_mode = "QUATERNION"
        pb.matrix_basis = Matrix.Identity(4)
    bpy.context.view_layer.update()

    for frame in range(first, last + 1):
        scene.frame_set(frame)
        for name in order:
            if name not in by_tgt:
                continue
            mx, rb, _, _ = by_tgt[name]
            pb = tgt.pose.bones[rb]
            d = rot(sw @ src.pose.bones[sb[mx]].matrix) @ rest_src[mx].inverted()
            world_rot = d @ align[rb] @ rest_tgt[rb]
            if mx == "Hips":
                travel = (sw @ src.pose.bones[sb["Hips"]].head) - hips0
                if in_place:
                    travel = Vector((0.0, 0.0, travel.z))
                head = pelvis0 + travel * scale
            else:
                head = tw @ pb.head
            world = Matrix.Translation(head) @ world_rot.to_4x4()
            pb.matrix = tw.inverted() @ world
            bpy.context.view_layer.update()
        for pb in tgt.pose.bones:
            if pb.name in by_tgt:
                pb.keyframe_insert("rotation_quaternion", frame=frame)
                if pb.name == "Bip01 Pelvis":
                    pb.keyframe_insert("location", frame=frame)
    return out_action, first, last, pairs, scale


def main():
    target_path, source_path, out = arg("--target"), arg("--source"), arg("--out")
    if not (target_path and source_path and out):
        sys.stderr.write(__doc__)
        return EXIT_USAGE
    bpy.ops.wm.read_factory_settings(use_empty=True)
    tgt, tgt_meshes = import_fbx(target_path)
    src, src_meshes = import_fbx(source_path)
    for ob in src_meshes:
        bpy.data.objects.remove(ob, do_unlink=True)
    wire_textures(tgt_meshes, arg("--textures"))
    in_place = flag("--in-place")
    _, first, last, pairs, scale = retarget_onto(
        tgt, src, in_place, os.path.splitext(os.path.basename(out))[0])

    scene = bpy.context.scene
    scene.frame_start, scene.frame_end = first, last
    os.makedirs(os.path.dirname(os.path.abspath(out)) or ".", exist_ok=True)
    bpy.ops.object.select_all(action="DESELECT")
    tgt.select_set(True)
    for ob in tgt_meshes:
        ob.select_set(True)
    bpy.data.objects.remove(src, do_unlink=True)
    bpy.ops.export_scene.gltf(filepath=out, export_format="GLB", use_selection=True,
                              export_skins=True, export_animations=True, export_apply=False)
    print(f"RETARGET frames={last - first + 1} pairs={len(pairs)} scale={scale:.4f} "
          f"in_place={in_place} out={out}")

    strip_png = arg("--render")
    if strip_png:
        render_strip(tgt, tgt_meshes, first, last, strip_png)
    return 0


def render_strip(tgt, meshes, first, last, path):
    """Four frames of the clip side by side, the hero camera, for a look."""
    import math
    scene = bpy.context.scene
    lo = Vector((1e9, 1e9, 1e9))
    hi = Vector((-1e9, -1e9, -1e9))
    scene.frame_set(first)
    for ob in meshes:
        for c in ob.bound_box:
            w = ob.matrix_world @ Vector(c)
            lo = Vector((min(lo.x, w.x), min(lo.y, w.y), min(lo.z, w.z)))
            hi = Vector((max(hi.x, w.x), max(hi.y, w.y), max(hi.z, w.z)))
    h = hi.z - lo.z
    ctr = (lo + hi) / 2
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    scene.collection.objects.link(cam)
    cam.data.lens = 50
    d = h * 2.4
    cam.location = ctr + Vector((-d * 0.55, -d * 0.85, h * 0.05))
    cam.rotation_euler = (ctr - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.camera = cam
    for rot_e, energy in (((math.radians(50), 0, math.radians(-40)), 4.0),
                          ((math.radians(70), 0, math.radians(140)), 1.5)):
        sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
        sun.data.energy = energy
        sun.rotation_euler = rot_e
        scene.collection.objects.link(sun)
    world = bpy.data.worlds.new("w")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.3, 0.3, 0.32, 1)
    scene.world = world
    r = scene.render
    r.engine = "BLENDER_EEVEE"
    r.resolution_x, r.resolution_y = 600, 800
    import numpy as np
    shots = []
    for k in range(4):
        frame = first + round((last - first) * k / 3)
        scene.frame_set(frame)
        r.filepath = f"{path}.{k}.png"
        bpy.ops.render.render(write_still=True)
        img = bpy.data.images.load(r.filepath)
        shots.append(np.array(img.pixels[:]).reshape(800, 600, 4))
    strip = np.concatenate(shots, axis=1)
    out = bpy.data.images.new("strip", width=2400, height=800, alpha=True)
    out.pixels.foreach_set(strip.reshape(-1).astype(np.float32))
    out.file_format = "PNG"
    out.filepath_raw = path
    out.save()
    print(f"RENDER {path}")


if __name__ == "__main__":
    try:
        code = main()
    except Exception as err:  # noqa: BLE001 - one line for the driver
        sys.stderr.write(f"E_RETARGET {type(err).__name__}: {err}\n")
        code = EXIT_FAIL
    if code:
        sys.exit(code)
