import bpy
import json
import sys

out = {"objects": []}
for ob in bpy.data.objects:
    d = {"name": ob.name, "type": ob.type,
         "parent": ob.parent.name if ob.parent else None,
         "parent_type": str(ob.parent_type) if ob.parent else None,
         "scale": [round(float(x), 5) for x in ob.scale],
         "loc": [round(float(x), 4) for x in ob.location]}
    if ob.type == 'MESH':
        ws = [tuple(round(float(c), 4) for c in ob.matrix_world.translation)]
        bb = [tuple(round(float(c), 4) for c in (ob.matrix_world @ v.co)) for v in ob.data.vertices]
        zs = [c[2] for c in bb]
        d["world_z"] = [round(min(zs), 4), round(max(zs), 4)]
        d["materials"] = [m.name if m else None for m in ob.data.materials]
        d["mat_index_used"] = sorted({p.material_index for p in ob.data.polygons})
    if ob.type == 'ARMATURE':
        bs = [b for b in ob.data.bones]
        d["n_bones"] = len(bs)
    out["objects"].append(d)
print("CT7JSON-BEGIN")
print(json.dumps(out))
print("CT7JSON-END")
