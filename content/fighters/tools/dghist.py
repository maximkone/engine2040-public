import bpy
import json

ob = bpy.data.objects["BODY_fighter"]
uv = ob.data.uv_layers["UV_bake"]
bins = [[0] * 8 for _ in range(8)]
for li in range(len(uv.data)):
    u, v = uv.data[li].uv
    iu = min(7, max(0, int(u * 8)))
    iv = min(7, max(0, int(v * 8)))
    bins[iv][iu] += 1
print("CT7JSON-BEGIN")
print(json.dumps({"bins_yup": bins}))
print("CT7JSON-END")
