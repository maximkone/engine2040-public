import bpy
import json

out = []
for name in ["human_male_head_3", "human_male_head_default",
             "human_male_body_default", "human_male_light_armor",
             "human_female_head_3", "human_female_light_armor"]:
    ob = bpy.data.objects.get(name)
    if ob is None:
        out.append({"name": name, "missing": True})
        continue
    uv = ob.data.uv_layers.get("UV_bake") or ob.data.uv_layers.active
    us = [uv.data[li].uv[0] for li in range(len(uv.data))]
    vs = [uv.data[li].uv[1] for li in range(len(uv.data))]
    out.append({"name": name, "u": [round(min(us), 4), round(max(us), 4)],
                "v": [round(min(vs), 4), round(max(vs), 4)],
                "uv_layers": [l.name for l in ob.data.uv_layers]})
print("CT7JSON-BEGIN")
print(json.dumps(out))
print("CT7JSON-END")
