"""CT19 pass 1b: graft same-pack CC0 weapons onto the staged fighter assembly.

Appends sword/shield objects from the IK-rig blend (same OGA CC0 pack, SHA in
manifest), grips them at the staged wep_pos bone heads with the author's
identity orientation (source rest matrices differ by the assemble SCALE, so
the graft re-derives the placement in staged meter space per CT21 and shrinks
verts by STAGE_SCALE about the grip), bakes
to world (meter) space, weights 100% to the wep_pos bone (animated in every
shipped clip), retile UVs into the weapon atlas strip, and joins into
BODY_fighter. Mesh/material/node counts are unchanged (still 4+4), so the
frozen CT1/CT2 proof harness runs unmodified.

Run: blender --background <staged.blend> --python weapons.py -- \
  --ikblend <animations_bake_human_male.blend> --sword main|two \
  --shield 0|1 --report <out.json>
"""
import bpy
import json
import os
import sys

SCALE_TWO = 0.8  # two-hander documented cook transform (1.87 m -> 1.50 m)

# CT21: the IK-rig blend is at the pack's original scale (male rest height
# 4.1917 m) while the staged assembly was scaled to the 1.8 m hero in
# assemble.py (SCALE = 1.8 / 4.1917). Appended weapon verts AND the stored
# object location are still in original-scale units, and the stored location
# equals the ORIGINAL-scale bone head, so reusing it verbatim as the
# bone-parent offset in the meter-scale staged file puts the baked weapons
# ~2.33x too far out and ~2.33x too large (CT20: Q2 bbox x +/-1.4 m,
# z up to 3.87 m). Fix: shrink verts to staged meter space about the grip
# (the object origin) and place the grip at the STAGED bone head.
STAGE_SCALE = 1.8 / 4.1917  # must match assemble.py SCALE

SWORDS = {"main": "main_hand_sword_old", "two": "two_handed_sword"}
SHIELD = "shield_old"

# Weapon atlas strip, tile-UV space (px strip at atlas x[0,512) y[1024,1536)).
BLADE_U = (8 / 2048.0, 200 / 2048.0)
GUARD_U = (208 / 2048.0, 248 / 2048.0)
SHIELD_U = (264 / 2048.0, 456 / 2048.0)
GRIP_U = (464 / 2048.0, 504 / 2048.0)
V_TOP = 1.0 - 1028 / 2048.0  # 0.498
V_BOT = 1.0 - 1532 / 2048.0  # 0.252


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1:]
    out = {}
    i = 0
    while i < len(argv):
        out[argv[i].lstrip("-").replace("-", "_")] = argv[i + 1]
        i += 2
    return out


def uv_layer_for_join(body):
    uv = body.data.uv_layers.get("UV_bake") or body.data.uv_layers.active
    if uv is None:
        raise RuntimeError("BODY_fighter has no UVs")
    return uv.name


def tile_sword(ob, uv_name):
    me = ob.data
    if uv_name not in me.uv_layers:
        me.uv_layers.get(ob.data.uv_layers.active.name).name = uv_name
    uv = me.uv_layers[uv_name]
    xs = [v.co[0] for v in me.vertices]
    ys = [v.co[1] for v in me.vertices]
    xmin, xmax = min(xs), max(xs)
    ymin, ymax = min(ys), max(ys)
    xr = (xmax - xmin) or 1.0
    yr = (ymax - ymin) or 1.0
    for loop in me.loops:
        v = me.vertices[loop.vertex_index]
        fx = (v.co[0] - xmin) / xr
        t = (v.co[1] - ymin) / yr
        if t < 0.72:
            u = BLADE_U[0] + fx * (BLADE_U[1] - BLADE_U[0])
            vv = V_BOT + (t / 0.72) * (V_TOP - V_BOT)
        elif t < 0.80:
            u = GUARD_U[0] + fx * (GUARD_U[1] - GUARD_U[0])
            vv = V_BOT + ((t - 0.72) / 0.08) * (V_TOP - V_BOT)
        else:
            u = GRIP_U[0] + fx * (GRIP_U[1] - GRIP_U[0])
            vv = V_BOT + ((t - 0.80) / 0.20) * (V_TOP - V_BOT)
        uv.data[loop.index].uv = (u, vv)


def tile_shield(ob, uv_name):
    me = ob.data
    if uv_name not in me.uv_layers:
        me.uv_layers.get(ob.data.uv_layers.active.name).name = uv_name
    uv = me.uv_layers[uv_name]
    xs = [v.co[0] for v in me.vertices]
    ys = [v.co[1] for v in me.vertices]
    xmin, xmax = min(xs), max(xs)
    ymin, ymax = min(ys), max(ys)
    xr = (xmax - xmin) or 1.0
    yr = (ymax - ymin) or 1.0
    for loop in me.loops:
        v = me.vertices[loop.vertex_index]
        u = SHIELD_U[0] + ((v.co[0] - xmin) / xr) * (SHIELD_U[1] - SHIELD_U[0])
        vv = V_BOT + ((v.co[1] - ymin) / yr) * (V_TOP - V_BOT)
        uv.data[loop.index].uv = (u, vv)


def graft(name, bone, uv_name, tiler, extra_scale):
    ob = bpy.data.objects.get(name)
    if ob is None:
        raise RuntimeError("append failed for " + name)
    _loc, rot = tuple(ob.location), tuple(ob.rotation_euler)
    arm = bpy.data.objects["human_male"]
    if bone not in arm.data.bones:
        raise RuntimeError("bone missing: " + bone)
    # CT21: the author's grip sits at the bone head with identity rotation in
    # the source file; re-derive that placement in STAGED (meter) space
    # instead of reusing the original-scale location verbatim. The bake is
    # done UNPARENTED at the staged head with the author's rotation: reusing
    # BONE parenting here would also reuse the source file's
    # matrix_parent_inverse (built for the original-scale head), which throws
    # the baked weapons meters off the body (CT20). The 100% wep_pos weight
    # below still binds the baked verts to the animated hand bone.
    head = tuple(arm.data.bones[bone].head_local)
    # Shrink the appended verts to staged meter space about the grip point
    # (the object origin), keeping the author's hand placement.
    for v in ob.data.vertices:
        v.co = (v.co[0] * STAGE_SCALE, v.co[1] * STAGE_SCALE, v.co[2] * STAGE_SCALE)
    scene_coll = bpy.context.scene.collection
    if ob.name not in scene_coll.objects:
        scene_coll.objects.link(ob)
    ob.parent = None
    ob.location = head
    ob.rotation_euler = rot
    ob.scale = (1.0, 1.0, 1.0)
    bpy.context.view_layer.update()
    # UVs + weights first (local space), then bake world into verts.
    tiler(ob, uv_name)
    grp = ob.vertex_groups.new(name=bone)
    grp.add([v.index for v in ob.data.vertices], 1.0, "REPLACE")
    mw = ob.matrix_world.copy()
    me = ob.data
    if extra_scale != 1.0:
        # Documented cook transform: shrink about the grip point (the
        # bone-head origin), keeping the author's hand placement.
        o = mw.translation.copy()
        for v in me.vertices:
            v.co = o + (mw * v.co - o) * extra_scale
    else:
        me.transform(mw)
    ob.parent = None
    ob.matrix_world.identity()
    me.materials.clear()
    return ob


def main():
    args = parse_args()
    arm = bpy.data.objects["human_male"]
    body = bpy.data.objects["BODY_fighter"]
    for b in arm.pose.bones:
        if not b.matrix_basis.is_identity:
            raise RuntimeError("staged pose not at rest")
    uv_name = uv_layer_for_join(body)

    want = []
    sword_src = SWORDS[args["sword"]]
    want.append(sword_src)
    if args.get("shield", "0") == "1":
        want.append(SHIELD)
    with bpy.data.libraries.load(args["ikblend"], link=False) as (data_from, data_to):
        data_to.objects = [n for n in want if n in data_from.objects]

    before_verts = len(body.data.vertices)
    grafted = []
    ob = graft(sword_src, "wep_pos_R", uv_name, tile_sword,
               SCALE_TWO if args["sword"] == "two" else 1.0)
    grafted.append(ob)
    if args.get("shield", "0") == "1":
        grafted.append(graft(SHIELD, "wep_pos_L", uv_name, tile_shield, 1.0))

    # Join weapons into the body (BODY_fighter active).
    bpy.ops.object.select_all(action="DESELECT")
    body.select_set(True)
    for o in grafted:
        o.select_set(True)
    bpy.context.view_layer.objects.active = body
    bpy.ops.object.join()
    joined = bpy.context.view_layer.objects.active
    report = {
        "sword": sword_src,
        "shield": args.get("shield", "0") == "1",
        "body_verts_before": before_verts,
        "body_verts_after": len(joined.data.vertices),
        "uv_layer": uv_name,
    }
    bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
    with open(args["report"], "w") as f:
        json.dump(report, f, indent=1)
    print("CT19-WEAPON %s sword=%s shield=%s verts=%d->%d" % (
        args.get("variant", "?"), sword_src, report["shield"],
        before_verts, report["body_verts_after"]))


main()
