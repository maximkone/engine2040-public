import bpy
import json

out = {}
# modifiers + vertex groups per mesh object
out["skinning"] = []
for ob in bpy.data.objects:
    if ob.type != 'MESH':
        continue
    mods = [{"type": m.type, "object": m.object.name if getattr(m, 'object', None) else None}
            for m in ob.modifiers]
    vg = [g.name for g in ob.vertex_groups]
    nweights = sum(1 for v in ob.data.vertices for g in v.groups if g.weight > 0.0)
    out["skinning"].append({"object": ob.name, "modifiers": mods,
                            "num_groups": len(vg), "weighted_verts": nweights,
                            "total_verts": len(ob.data.vertices)})
# action internals for 3 sample actions
out["action_detail"] = []
for name in ["human_male_idle", "human_male_run", "human_male_cast_main_hand_melee"]:
    act = bpy.data.actions.get(name)
    if act is None:
        out["action_detail"].append({"name": name, "missing": True})
        continue
    d = {"name": name, "users": act.users, "frame_range": [float(x) for x in act.frame_range]}
    try:
        d["legacy_fcurves"] = len(act.fcurves)
        d["groups"] = sorted(set(fc.group.name if fc.group else '' for fc in act.fcurves))[:60]
        chans = {}
        for fc in act.fcurves:
            chans[fc.data_path] = chans.get(fc.data_path, 0) + 1
        d["channels"] = chans
    except Exception as e:
        d["legacy_error"] = str(e)
    try:
        d["layers"] = [l.name for l in act.layers]
        d["slots"] = [(s.name, s.target_id_type) for s in act.slots]
    except Exception as e:
        d["layer_error"] = str(e)
    out["action_detail"].append(d)
# armature animation data / NLA
out["nla"] = []
for ob in bpy.data.objects:
    if ob.type == 'ARMATURE' and ob.animation_data:
        ad = ob.animation_data
        out["nla"].append({"object": ob.name,
                           "active_action": ad.action.name if ad.action else None,
                           "nla_tracks": [t.name for t in ad.nla_tracks]})
print("CT7JSON-BEGIN")
print(json.dumps(out))
print("CT7JSON-END")
