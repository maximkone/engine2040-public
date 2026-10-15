import bpy
import json

out = []
for ob in bpy.data.objects:
    if ob.type != 'MESH':
        continue
    me = ob.data
    layers = []
    for ul in me.uv_layers:
        us = [ul.data[li].uv[0] for li in range(len(ul.data))]
        vs = [ul.data[li].uv[1] for li in range(len(ul.data))]
        layers.append({"name": ul.name, "active": ul == me.uv_layers.active,
                       "u": [round(min(us), 4), round(max(us), 4)],
                       "v": [round(min(vs), 4), round(max(vs), 4)]})
    out.append({"object": ob.name, "layers": layers})
print("CT7JSON-BEGIN")
print(json.dumps(out))
print("CT7JSON-END")
