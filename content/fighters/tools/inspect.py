import bpy
import json
import sys

out = {"file": bpy.data.filepath}
out["blender_version"] = list(bpy.app.version)
out["objects"] = []
for ob in bpy.data.objects:
    out["objects"].append({"name": ob.name, "type": ob.type,
                           "parent": ob.parent.name if ob.parent else None})
out["armatures"] = []
for arm in bpy.data.armatures:
    bones = []
    for b in arm.bones:
        bones.append({"name": b.name,
                      "parent": b.parent.name if b.parent else None,
                      "head": [round(float(x), 5) for x in b.head_local],
                      "tail": [round(float(x), 5) for x in b.tail_local]})
    out["armatures"].append({"name": arm.name, "bones": bones})
out["actions"] = []
for act in bpy.data.actions:
    try:
        fr = [float(act.frame_range[0]), float(act.frame_range[1])]
    except Exception:
        fr = None
    # count fcurves (legacy) or layers (layered 4.x)
    fc = 0
    try:
        fc = len(act.fcurves)
    except Exception:
        fc = -1
    out["actions"].append({"name": act.name, "frames": fr, "fcurves": fc})
out["meshes"] = []
for me in bpy.data.meshes:
    tris = sum(1 for p in me.polygons if len(p.vertices) == 3)
    quads = sum(1 for p in me.polygons if len(p.vertices) == 4)
    ngons = sum(1 for p in me.polygons if len(p.vertices) > 4)
    mats = [m.name if m else None for m in me.materials]
    uv = [u.name for u in me.uv_layers]
    out["meshes"].append({"name": me.name, "verts": len(me.vertices),
                          "tris": tris, "quads": quads, "ngons": ngons,
                          "materials": mats, "uvs": uv})
out["images"] = [{"name": i.name, "size": list(i.size),
                  "packed": i.packed_file is not None}
                 for i in bpy.data.images]
print("CT7JSON-BEGIN")
print(json.dumps(out))
print("CT7JSON-END")
