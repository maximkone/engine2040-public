import bpy
import json
import os

out = {"objects": []}
for ob in bpy.data.objects:
    if ob.type != 'MESH':
        continue
    mats = []
    for slot in ob.material_slots:
        m = slot.material
        if m is None or m.use_nodes is False:
            mats.append({"material": m.name if m else None, "images": []})
            continue
        imgs = []
        for n in m.node_tree.nodes:
            if n.type == 'TEX_IMAGE' and n.image is not None:
                img = n.image
                fp = bpy.path.abspath(img.filepath) if img.filepath else ""
                exists = os.path.isfile(fp) if fp else False
                imgs.append({"image": img.name, "size": list(img.size),
                             "filepath": fp, "exists": exists,
                             "has_pixels": len(getattr(img, 'pixels', [])) > 0})
        mats.append({"material": m.name, "images": imgs})
    entry = {"object": ob.name,
             "parent": ob.parent.name if ob.parent else None,
             "modifiers": [m.type for m in ob.modifiers],
             "groups": sorted(g.name for g in ob.vertex_groups),
             "materials": mats}
    out["objects"].append(entry)
print("CT7JSON-BEGIN")
print(json.dumps(out))
print("CT7JSON-END")
