"""A Rocketbox avatar as a game fighter: a body GLB and its clip GLBs, on one skeleton.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/rocketbox_fighter.py -- \
      --target <rocketbox.fbx> --textures <dir> --name <variant> --out-dir <dir> \
      --clip idle=<mixamo.fbx> --clip run=<...> --clip attack1=<...> --clip attack2=<...> \
      --clip attack3=<...> --clip death=<...> [--weapon gladius.glb] [--shield scutum.glb]

What the game's look needs, and how this gives it:
- Joint names. The look finds the legs, the throwing arm, the cape and the
  ragdoll by the CT7 names (pelvis, torso, leg_upper_l, foot_l, palm_r, ...).
  The Biped bones that mean the same thing are renamed to them; the rest keep
  their Biped name in the contract charset (bip01_spine, bip01_neck, face).
  CT7's two weapon sockets and two heels are added as weightless bones.
- Clips on the body's own skeleton. Each clip is a Mixamo FBX put on this exact
  armature by tools/blender/retarget_mixamo.py's transfer, in place (the sim
  moves the body), so the look's retarget onto the body is the identity.
- One mesh with one material per part and its base colour embedded: the
  eyelash and hair cards (the alpha "opacity" material) are dropped, the look
  draws no alpha.
Writes the family tree the game binds (crates/game/src/fighter_bind.rs,
bind_rocketbox_from): <out-dir>/glb/<name>_body.glb,
<out-dir>/glb/clips/<name>_<role>.glb, <out-dir>/manifest.json and
<out-dir>/<name>.cook.json (joints, triangles, clips with frames and sha256).
Textures are cut to --max-texture px (default 1024) on the long edge.
"""

import hashlib
import importlib.util
import json
import os
import sys

import bpy
from mathutils import Vector

EXIT_USAGE = 2
EXIT_FAIL = 3

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("retarget_mixamo", os.path.join(HERE, "retarget_mixamo.py"))
rm = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rm)

# Biped bone -> CT7 joint name (crates/assets/src/ct7_map.rs, the game's rig).
TO_CT7 = {"Bip01 Pelvis": "pelvis", "Bip01 Spine2": "torso", "Bip01 Head": "head"}
for b, c in (("L", "l"), ("R", "r")):
    TO_CT7.update({
        f"Bip01 {b} Clavicle": f"shoulder_{c}", f"Bip01 {b} UpperArm": f"arm_upper_{c}",
        f"Bip01 {b} Forearm": f"arm_lower_{c}", f"Bip01 {b} Hand": f"palm_{c}",
        f"Bip01 {b} Finger0": f"thumb_upper_{c}", f"Bip01 {b} Finger01": f"thumb_lower_{c}",
        f"Bip01 {b} Finger1": f"index_upper_{c}", f"Bip01 {b} Finger11": f"index_lower_{c}",
        f"Bip01 {b} Finger2": f"middle_upper_{c}", f"Bip01 {b} Finger21": f"middle_lower_{c}",
        f"Bip01 {b} Finger3": f"ring_upper_{c}", f"Bip01 {b} Finger31": f"ring_lower_{c}",
        f"Bip01 {b} Finger4": f"pinky_upper_{c}", f"Bip01 {b} Finger41": f"pinky_lower_{c}",
        f"Bip01 {b} Thigh": f"leg_upper_{c}", f"Bip01 {b} Calf": f"leg_lower_{c}",
        f"Bip01 {b} Foot": f"foot_{c}", f"Bip01 {b} Toe0": f"toes_{c}",
    })
ROLES = ["idle", "run", "attack1", "attack2", "attack3", "death"]


def clips_arg():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    out = {}
    for i, a in enumerate(argv):
        if a == "--clip" and i + 1 < len(argv):
            role, _, path = argv[i + 1].partition("=")
            out[role] = path
    return out


def contract(name):
    return "".join(c.lower() if c.isalnum() else "_" for c in name)


def sha(path):
    with open(path, "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


def fold_face_rig(arm, meshes):
    """Fold the face bones into the head: weights merged, bones deleted.

    The game's GPU skin draws up to 64 joints per mesh, and the Rocketbox
    Biped carries 28 face bones (jaw, lips, cheeks, brows, lids, eyes) under
    the head that nothing here animates. Every descendant of the head moves
    its vertex weight onto the head, so the face rides the skull as before.
    """
    head = arm.data.bones["Bip01 Head"]
    face = [b.name for b in head.children_recursive]
    for ob in meshes:
        target = ob.vertex_groups.get("Bip01 Head") or ob.vertex_groups.new(name="Bip01 Head")
        groups = {ob.vertex_groups[n].index: n for n in face if n in ob.vertex_groups}
        for v in ob.data.vertices:
            moved = sum(g.weight for g in v.groups if g.group in groups)
            if moved > 0.0:
                target.add([v.index], moved, "ADD")
        for index in sorted(groups, reverse=True):
            ob.vertex_groups.remove(ob.vertex_groups[groups[index]])
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.mode_set(mode="EDIT")
    for name in face:
        bone = arm.data.edit_bones.get(name)
        if bone is not None:
            arm.data.edit_bones.remove(bone)
    bpy.ops.object.mode_set(mode="OBJECT")
    print(f"FIGHTER folded {len(face)} face bones into the head, {len(arm.data.bones)} bones left")


def drop_alpha_cards(meshes):
    """Delete the faces of the alpha (opacity) material and its slot."""
    import bmesh
    for ob in meshes:
        slots = [i for i, s in enumerate(ob.material_slots)
                 if s.material is not None and "opacity" in s.material.name.lower()]
        if not slots:
            continue
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        doomed = [f for f in bm.faces if f.material_index in slots]
        bmesh.ops.delete(bm, geom=doomed, context="FACES")
        bm.to_mesh(ob.data)
        bm.free()
        for index in sorted(slots, reverse=True):
            ob.active_material_index = index
            bpy.context.view_layer.objects.active = ob
            bpy.ops.object.material_slot_remove()
        loose = [v for v in ob.data.vertices]
        print(f"FIGHTER dropped {len(doomed)} alpha faces on {ob.name}, {len(loose)} vertices left")


def hand_frame(arm, side):
    """The hand's rest frame in world space: grip centre, finger, thumb and palm axes.

    f runs from the wrist to the middle finger's knuckle; t is the thumb's
    direction with its f part removed (a hammer grip holds the handle along
    it); n = f x t closes the frame.
    """
    from mathutils import Matrix
    w = arm.matrix_world
    head = lambda name: w @ arm.data.bones[name].head_local
    wrist = head(f"Bip01 {side} Hand")
    knuckle = head(f"Bip01 {side} Finger2")
    f = (knuckle - wrist).normalized()
    t = head(f"Bip01 {side} Finger01") - head(f"Bip01 {side} Finger0")
    t = (t - f * t.dot(f)).normalized()
    n = f.cross(t).normalized()
    grip = wrist + (knuckle - wrist) * 0.8
    return grip, f, t, n


def graft(arm, body, path, bone, axes):
    """Import a kit GLB, set it in the hand by `axes` (local X, Y, Z -> world),
    weight it fully to `bone` and join it into the body mesh."""
    from mathutils import Matrix
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=path)
    parts = [o for o in set(bpy.data.objects) - before if o.type == "MESH"]
    for ob in set(bpy.data.objects) - before:
        if ob.type != "MESH":
            bpy.data.objects.remove(ob, do_unlink=True)
    grip, x, y, z = axes
    place = Matrix.Identity(4)
    for row in range(3):
        place[row][0], place[row][1], place[row][2], place[row][3] = x[row], y[row], z[row], grip[row]
    # glTF import turns the kit's Y-up file back to Blender's Z-up, so its
    # parts already sit in Blender axes with the grip at the origin.
    for ob in parts:
        ob.matrix_world = place @ ob.matrix_world
        group = ob.vertex_groups.new(name=bone)
        group.add(range(len(ob.data.vertices)), 1.0, "REPLACE")
    bpy.ops.object.select_all(action="DESELECT")
    for ob in parts:
        ob.select_set(True)
    body.select_set(True)
    bpy.context.view_layer.objects.active = body
    bpy.ops.object.join()
    print(f"FIGHTER grafted {os.path.basename(path)} on {bone}")


def action_fcurves(action):
    """Every F-curve of an action, legacy or layered (Blender 4.4+ slots)."""
    legacy = getattr(action, "fcurves", None)
    if legacy is not None and len(legacy):
        return list(legacy)
    out = []
    for layer in getattr(action, "layers", []):
        for strip in layer.strips:
            for bag in getattr(strip, "channelbags", []):
                out.extend(bag.fcurves)
    return out


def rename_and_socket(arm, actions):
    """CT7 names where the meaning matches, contract names elsewhere, plus sockets.

    Blender's bone rename fixes the paths of the action assigned at that
    moment only; the other clips would keep the old names and drive nothing
    (the whole clip set froze on the last clip's pose). So every action's
    channel paths are renamed here, by hand, with the same table.
    """
    renames = {b.name: TO_CT7.get(b.name, contract(b.name)) for b in arm.data.bones}
    arm.animation_data.action = None
    for action in actions:
        for fc in action_fcurves(action):
            path = fc.data_path
            if path.startswith('pose.bones["'):
                old = path[len('pose.bones["'):path.index('"]')]
                if old in renames:
                    fc.data_path = path.replace(f'pose.bones["{old}"]', f'pose.bones["{renames[old]}"]', 1)
            if fc.group is not None and fc.group.name in renames:
                fc.group.name = renames[fc.group.name]
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.mode_set(mode="EDIT")
    eb = arm.data.edit_bones
    for bone in list(eb):
        bone.name = renames.get(bone.name, contract(bone.name))
    for c in ("l", "r"):
        palm, foot = eb[f"palm_{c}"], eb[f"foot_{c}"]
        sock = eb.new(f"wep_pos_{c}")
        sock.head = palm.tail.copy()
        sock.tail = palm.tail + (palm.tail - palm.head) * 0.5
        sock.parent = palm
        sock.use_deform = False
        heel = eb.new(f"heel_{c}")
        heel.head = Vector((foot.head.x, foot.head.y + 0.04, foot.head.z - 0.07))
        heel.tail = heel.head + Vector((0.0, 0.0, -0.03))
        heel.parent = foot
        heel.use_deform = False
    bpy.ops.object.mode_set(mode="OBJECT")
    # The armature object itself is a joint in the CT7 skeleton (its root node).
    arm.name = "human_male"
    arm.data.name = "human_male"


def strip_scale_channels(path):
    """Drop every `scale` animation channel from a GLB, in place.

    The look's skin import takes rotation and translation channels only, and
    no bone here scales. Samplers are re-indexed; the orphaned accessors stay
    (the importer reads through channels, never the accessor list).
    """
    import struct
    with open(path, "rb") as handle:
        data = handle.read()
    json_len = struct.unpack_from("<I", data, 12)[0]
    doc = json.loads(data[20:20 + json_len])
    rest = data[20 + json_len:]
    for anim in doc.get("animations", []):
        keep = [c for c in anim["channels"] if c["target"].get("path") != "scale"]
        used = sorted({c["sampler"] for c in keep})
        remap = {old: new for new, old in enumerate(used)}
        anim["samplers"] = [anim["samplers"][i] for i in used]
        for c in keep:
            c["sampler"] = remap[c["sampler"]]
        anim["channels"] = keep
    text = json.dumps(doc, separators=(",", ":")).encode()
    text += b" " * ((4 - len(text) % 4) % 4)
    header = struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(text) + len(rest))
    with open(path, "wb") as handle:
        handle.write(header + struct.pack("<I", len(text)) + b"JSON" + text + rest)


def export(path, arm, meshes, animations, images):
    bpy.ops.object.select_all(action="DESELECT")
    arm.select_set(True)
    for ob in meshes:
        ob.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=path, export_format="GLB", use_selection=True, export_skins=True,
        export_animations=animations, export_animation_mode="ACTIVE_ACTIONS",
        export_apply=False, export_image_format="AUTO" if images else "NONE",
        export_def_bones=False,
        # The look's skin import takes LINEAR channels only; the exporter's
        # size optimisation writes every static channel as two STEP keys.
        export_optimize_animation_size=False, export_force_sampling=True)


def main():
    target, out_dir, name = rm.arg("--target"), rm.arg("--out-dir"), rm.arg("--name")
    clips = clips_arg()
    missing = [r for r in ROLES if r not in clips]
    if not (target and out_dir and name) or missing:
        sys.stderr.write(__doc__ + (f"\nmissing clips: {missing}\n" if missing else ""))
        return EXIT_USAGE
    if contract(name) != name or not name[0].isalpha():
        # The game's binder refuses any other name (rocketbox_variants).
        sys.stderr.write(f"--name {name!r}: lowercase letters, digits and _ only, a letter first\n")
        return EXIT_USAGE
    bpy.ops.wm.read_factory_settings(use_empty=True)
    arm, meshes = rm.import_fbx(target)
    # The FBX armature carries a 0.01 scale and a 90 degree turn on its
    # object; the game reads joint transforms, not the root node's, so the
    # body came out a hundred times too big. Bake the object transform into
    # the bones and the mesh before anything is cooked, so every clip is in
    # metres too.
    # The FBX also brings its own animation data with object-level keys
    # (that very 0.01 and the turn): every frame_set of the retarget re-read
    # it and put the transform back. Clear it, and the Biped's Footsteps
    # helper, before applying.
    for ob in [arm, *meshes, *arm.children]:
        ob.animation_data_clear()
    for ob in list(arm.children):
        if ob.type == "EMPTY":
            bpy.data.objects.remove(ob, do_unlink=True)
    bpy.ops.object.select_all(action="DESELECT")
    arm.select_set(True)
    for ob in meshes:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    print(f"FIGHTER applied object transform, height {max((arm.matrix_world @ b.head_local).z for b in arm.data.bones):.3f} m to the head")
    rm.wire_textures(meshes, rm.arg("--textures"))
    drop_alpha_cards(meshes)
    fold_face_rig(arm, meshes)
    cap = int(rm.arg("--max-texture", "1024"))
    for image in bpy.data.images:
        w, h = image.size
        if max(w, h) > cap:
            k = cap / max(w, h)
            image.scale(max(1, int(w * k)), max(1, int(h * k)))

    actions = {}
    for role in ROLES:
        src, src_meshes = rm.import_fbx(clips[role])
        for ob in src_meshes:
            bpy.data.objects.remove(ob, do_unlink=True)
        action, first, last, pairs, _ = rm.retarget_onto(arm, src, True, f"{name}_{role}")
        actions[role] = (action, first, last)
        bpy.data.objects.remove(src, do_unlink=True)
        print(f"FIGHTER clip {role} frames={last - first + 1} pairs={len(pairs)} <- {os.path.basename(clips[role])}")

    weapon, shield = rm.arg("--weapon"), rm.arg("--shield")
    if weapon or shield:
        body = max(meshes, key=lambda ob: len(ob.data.vertices))
        if weapon:
            grip, f, t, n = hand_frame(arm, "R")
            graft(arm, body, weapon, "Bip01 R Hand", (grip, f, n, t))
        if shield:
            grip, f, t, n = hand_frame(arm, "L")
            graft(arm, body, shield, "Bip01 L Hand", (grip, n, f, t))
        meshes = [ob for ob in bpy.context.scene.objects if ob.type == "MESH"]

    rename_and_socket(arm, [a for a, _, _ in actions.values()])
    glb_dir = os.path.join(out_dir, "glb")
    os.makedirs(os.path.join(glb_dir, "clips"), exist_ok=True)

    # The body: rest pose, no animation, base colours embedded.
    arm.animation_data.action = None
    for pb in arm.pose.bones:
        pb.matrix_basis.identity()
    body_path = os.path.join(glb_dir, f"{name}_body.glb")
    export(body_path, arm, meshes, animations=False, images=True)

    receipt = {"variant": name, "family": "rocketbox", "source": os.path.basename(target),
               "licence": "Microsoft Rocketbox MIT; motion Adobe Mixamo",
               "body": os.path.basename(body_path), "body_sha256": sha(body_path),
               "joints": len(arm.data.bones) + 1,
               "tris": sum(len(p.vertices) - 2 for ob in meshes for p in ob.data.polygons),
               "clips": []}
    scene = bpy.context.scene
    for role in ROLES:
        action, first, last = actions[role]
        arm.animation_data.action = action
        scene.frame_start, scene.frame_end = first, last
        path = os.path.join(glb_dir, "clips", f"{name}_{role}.glb")
        export(path, arm, meshes, animations=True, images=False)
        strip_scale_channels(path)
        receipt["clips"].append({"role": role, "file": f"glb/clips/{os.path.basename(path)}",
                                 "frames": last - first + 1, "fps": scene.render.fps,
                                 "source": os.path.basename(clips[role]), "sha256": sha(path)})
    with open(os.path.join(out_dir, f"{name}.cook.json"), "w", encoding="utf-8") as handle:
        json.dump(receipt, handle, indent=1, sort_keys=True)
    # The manifest is the game's only variant list (fighter_bind.rs,
    # rocketbox_variants): this variant's row replaces its old one in place
    # and every other variant's row is kept, so cooking a second avatar into
    # the same tree adds it instead of dropping the first.
    row = {"name": name, "body_glb_sha256": receipt["body_sha256"], "joints": receipt["joints"],
           "clips": len(receipt["clips"]), "tris_lod0": receipt["tris"],
           "source": receipt["source"], "licence": receipt["licence"]}
    manifest_path = os.path.join(out_dir, "manifest.json")
    manifest = {"family": "rocketbox", "variants": []}
    if os.path.exists(manifest_path):
        with open(manifest_path, encoding="utf-8") as handle:
            manifest = json.load(handle)
    rows = manifest.setdefault("variants", [])
    at = next((i for i, r in enumerate(rows) if r.get("name") == name), None)
    if at is None:
        rows.append(row)
    else:
        rows[at] = row
    with open(manifest_path, "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=1)
    preview = rm.arg("--render")
    if preview:
        action, first, last = actions[rm.arg("--render-clip", "attack1")]
        arm.animation_data.action = action
        rm.render_strip(arm, meshes, first, last, preview)
    print(f"FIGHTER {name} joints={receipt['joints']} tris={receipt['tris']} "
          f"clips={len(receipt['clips'])} body={body_path}")
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except Exception as err:  # noqa: BLE001 - one line for the driver
        import traceback
        traceback.print_exc()
        sys.stderr.write(f"E_FIGHTER {type(err).__name__}: {err}\n")
        code = EXIT_FAIL
    if code:
        sys.exit(code)
