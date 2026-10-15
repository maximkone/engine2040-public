import bpy
import json
import sys

argv = sys.argv[sys.argv.index("--") + 1:]
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
bpy.ops.import_scene.gltf(filepath=argv[0])

out = {"mats": [], "head_faces": []}
for m in bpy.data.materials:
    imgs = []
    if m.use_nodes:
        for n in m.node_tree.nodes:
            if n.type == 'TEX_IMAGE' and n.image is not None:
                imgs.append({"image": n.image.name, "size": list(n.image.size)})
    out["mats"].append({"material": m.name, "images": imgs})
# head faces: verts with z in head band of the *modeled* (unscaled? scaled) mesh:
# find faces whose centroid z > 1.45 (head region post-scale ~1.45-1.8)
for ob in bpy.data.objects:
    if ob.type != 'MESH':
        continue
    me = ob.data
    uv = me.uv_layers.active
    n = 0
    for poly in me.polygons:
        cx = cy = cz = 0.0
        for vi in poly.vertices:
            co = ob.matrix_world @ me.vertices[vi].co
            cx += co.x
            cy += co.y
            cz += co.z
        k = len(poly.vertices)
        cx, cy, cz = cx / k, cy / k, cz / k
        if cz > 1.45 and abs(cx) < 0.35 and cy < 0.35:
            mat = me.materials[poly.material_index].name if poly.material_index < len(me.materials) else None
            loops = []
            for li in poly.loop_indices:
                loops.append([round(float(uv.data[li].uv[0]), 4), round(float(uv.data[li].uv[1]), 4)])
            out["head_faces"].append({"mat_index": poly.material_index, "mat": mat, "uvs": loops})
            n += 1
            if n >= 6:
                break
print("CT7JSON-BEGIN")
print(json.dumps(out))
print("CT7JSON-END")
