import bpy
import json

out = []
src = "human_female"
for name in ["human_female_body_default",
             "human_female_light_armor", "human_female_light_armor_2",
             "human_female_light_armor_3", "human_female_light_armor_4",
             "human_female_medium_armor",
             "human_male_light_armor", "human_male_light_armor_3",
             "human_male_medium_armor", "human_male_heavy_armor"]:
    ob = bpy.data.objects.get(name)
    if ob is None:
        out.append({"name": name, "missing": True})
        continue
    zs = [v.co.z for v in ob.data.vertices]
    out.append({"name": name, "verts": len(ob.data.vertices),
                "z": [round(min(zs), 3), round(max(zs), 3)]})
print("CT7JSON-BEGIN")
print(json.dumps(out))
print("CT7JSON-END")
