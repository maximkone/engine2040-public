"""CT7 pass 1: assemble one fighter variant from Modular RPG Characters parts.

Loads human_male.blend (hero skeleton), appends female parts when needed,
keeps the curated part set, tiles UVs into atlas buckets, applies the
meter scale, joins to one mesh, saves a staged .blend + parts manifest.

Run: blender --background <male.blend> --python assemble.py -- \
  --variant hero --female-src <female.blend> --texdir <textures> \
  --staged <out.blend> --manifest <out.json>
"""
import bpy
import json
import os
import sys

SCALE = 1.8 / 4.1917  # male rest height -> 1.8 m hero

# role -> (source file, object name, atlas tile x,y,w in px of 2048, texture file)
ATLAS = 2048
PARTS = {
    "hero": {
        "parts": {
            "armor": ("male", "human_male_heavy_armor", "tex_human_female_heavy_armor.png"),
            "body": ("male", "human_male_body_default", "tex_human_male_body_1.png"),
            "head": ("male", "human_male_head_default", "tex_human_male_head_1.png"),
            "hair": ("male", "human_male_hair_1", "tex_human_male_hair_1.png"),
            "hands": ("male", "human_male_hands_default", "tex_human_male_hands_1.png"),
            "feet": ("male", "human_male_feet_default", "tex_human_male_feet_1.png"),
            "boots": ("male", "human_male_heavy_boots", "tex_human_female_heavy_boots.png"),
            "gloves": ("male", "human_male_heavy_gloves", "tex_human_female_heavy_gloves.png"),
            "helmet": ("male", "human_male_heavy_helmet", "tex_human_female_heavy_helmet.png"),
        },
    },
    "opp_male_light": {
        "parts": {
            "armor": ("male", "human_male_light_armor", "tex_human_male_light_armor.png"),
            "body": ("male", "human_male_body_default", "tex_human_male_body_1.png"),
            "head": ("male", "human_male_head_3", "tex_human_male_head_3.png"),
            "hair": ("male", "human_male_hair_4", "tex_human_male_hair_4.png"),
            "hands": ("male", "human_male_hands_default", "tex_human_male_hands_1.png"),
            "feet": ("male", "human_male_feet_default", "tex_human_male_feet_1.png"),
            "boots": ("male", "human_male_light_boots", "tex_human_female_light_boots.png"),
            "gloves": ("male", "human_male_light_gloves", "tex_human_male_light_gloves.png"),
        },
    },
    "opp_female_medium": {
        "parts": {
            "armor": ("female", "human_female_medium_armor", "tex_human_female_medium_armor.png"),
            "body": ("female", "human_female_body_default", "tex_human_female_body_1.png"),
            "head": ("female", "human_female_head_default", "tex_human_female_head_1.png"),
            "hair": ("female", "human_female_hair_2", "tex_human_female_hair_2.png"),
            "hands": ("female", "human_female_hands_default", "tex_human_female_hands_1.png"),
            "feet": ("female", "human_female_feet_default", "tex_human_female_feet_1.png"),
            "boots": ("female", "human_female_medium_boots", "tex_human_female_medium_boots.png"),
            "gloves": ("female", "human_female_medium_gloves", "tex_human_female_medium_gloves.png"),
            "helmet": ("female", "human_female_medium_helmet", "tex_human_female_medium_helmet.png"),
        },
    },
    "opp_female_light": {
        "parts": {
            "armor": ("female", "human_female_light_armor", "tex_human_female_light_armor.png"),
            "body": ("female", "human_female_body_default", "tex_human_female_body_1.png"),
            "head": ("female", "human_female_head_3", "tex_human_female_head_3.png"),
            "hair": ("female", "human_female_hair_1", "tex_human_female_hair_1.png"),
            "hands": ("female", "human_female_hands_default", "tex_human_female_hands_1.png"),
            "feet": ("female", "human_female_feet_default", "tex_human_female_feet_1.png"),
            "boots": ("female", "human_female_light_boots_2", "tex_human_female_light_boots_2.png"),
            "gloves": ("female", "human_female_light_gloves_3", "tex_human_female_light_gloves_3.png"),
        },
    },
}

TILES = {  # role -> (x, y_top, w) px in 2048 atlas
    "armor": (0, 0, 1024),
    "body": (1024, 0, 512),
    "head": (1536, 0, 512),
    "hair": (1024, 512, 512),
    "hands": (1536, 512, 512),
    "feet": (1024, 1024, 512),
    "boots": (1536, 1024, 512),
    "gloves": (0, 1536, 512),
    "helmet": (512, 1536, 512),
}


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1:]
    out = {}
    i = 0
    while i < len(argv):
        out[argv[i].lstrip("-").replace("-", "_")] = argv[i + 1]
        i += 2
    return out


def main():
    args = parse_args()
    variant = args["variant"]
    cfg = PARTS[variant]
    texdir = args["texdir"]
    arm = bpy.data.objects["human_male"]
    bone_names = sorted(b.name for b in arm.data.bones)

    # Append female parts when needed.
    female_names = [o for (src, o, _) in cfg["parts"].values() if src == "female"]
    if female_names:
        with bpy.data.libraries.load(args["female_src"], link=False) as (data_from, data_to):
            data_to.objects = [n for n in female_names if n in data_from.objects]
        for ob in data_to.objects:
            if ob is None:
                raise RuntimeError("append failed for a female part")
            bpy.context.scene.collection.objects.link(ob)

    # Clear pose and action so assembly happens at rest.
    for b in arm.pose.bones:
        b.matrix_basis.identity()
    if arm.animation_data is not None:
        arm.animation_data.action = None

    # Unparent everything first: scaling parented children alongside their
    # parent double-applies (world scale goes to s*s). All parts must sit
    # at identity transforms (fail loudly otherwise); re-parent after apply.
    parts_obs = [bpy.data.objects[o] for (_, o, _) in cfg["parts"].values()]
    for ob in parts_obs:
        loc = tuple(ob.location)
        rot = tuple(ob.rotation_euler)
        scl = tuple(ob.scale)
        if any(abs(x) > 1e-6 for x in loc):
            raise RuntimeError("part %s has location %s" % (ob.name, str(loc)))
        if any(abs(x) > 1e-6 for x in rot):
            raise RuntimeError("part %s has rotation" % ob.name)
        if any(abs(x - 1.0) > 1e-6 for x in scl):
            raise RuntimeError("part %s has scale %s" % (ob.name, str(scl)))
        ob.parent = None
        ob.matrix_world.identity()

    keep = {o for (_, o, _) in cfg["parts"].values()}
    # Delete unneeded objects (meshes not kept, lights, cameras).
    for ob in list(bpy.data.objects):
        if ob == arm:
            continue
        if ob.type == "MESH" and ob.name in keep:
            continue
        bpy.data.objects.remove(ob, do_unlink=True)

    # Verify vertex groups resolve against the hero bones.
    hero = set(bone_names)
    manifest_parts = []
    for role, (src, oname, tex) in sorted(cfg["parts"].items()):
        ob = bpy.data.objects.get(oname)
        if ob is None:
            raise RuntimeError("missing part object " + oname)
        unknown = sorted({g.name for g in ob.vertex_groups} - hero)
        if unknown:
            raise RuntimeError("part %s has non-hero groups: %s" % (oname, unknown))
        texpath = os.path.join(texdir, tex)
        if not os.path.isfile(texpath):
            raise RuntimeError("missing texture file " + texpath)
        manifest_parts.append({"role": role, "object": oname, "texture": tex,
                               "tile": list(TILES[role])})

    # Tile UVs into atlas buckets (UV space, y-up).
    for role, (src, oname, tex) in sorted(cfg["parts"].items()):
        ob = bpy.data.objects[oname]
        me = ob.data
        uv = me.uv_layers.get("UV_bake") or me.uv_layers.active
        if uv is None:
            raise RuntimeError("no UVs on " + oname)
        tx, ty, tw = TILES[role]
        sc = tw / ATLAS
        ox = tx / ATLAS
        oy = 1.0 - (ty + tw) / ATLAS
        for loop in me.loops:
            u, v = uv.data[loop.index].uv
            uv.data[loop.index].uv = (u * sc + ox, v * sc + oy)

    # Apply the meter scale to the armature and every part.
    for ob in [arm] + [bpy.data.objects[o] for (_, o, _) in cfg["parts"].values()]:
        ob.scale = (SCALE, SCALE, SCALE)
    bpy.ops.object.select_all(action="DESELECT")
    arm.select_set(True)
    for (_, oname, _) in cfg["parts"].values():
        bpy.data.objects[oname].select_set(True)
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)

    # Make every kept part visible and in the scene collection (2.79 layers
    # can land in excluded collections, which ops.join silently skips).
    scene_coll = bpy.context.scene.collection
    for (_, oname, _) in cfg["parts"].values():
        ob = bpy.data.objects[oname]
        ob.hide_viewport = False
        ob.hide_set(False)
        ob.hide_render = False
        if ob.name not in scene_coll.objects:
            scene_coll.objects.link(ob)

    # Join parts into one mesh (body part first = active).
    bpy.ops.object.select_all(action="DESELECT")
    body_name = cfg["parts"]["body"][1]
    for (_, oname, _) in sorted(cfg["parts"].values()):
        bpy.data.objects[oname].select_set(True)
    bpy.context.view_layer.objects.active = bpy.data.objects[body_name]
    bpy.ops.object.join()
    joined = bpy.context.view_layer.objects.active
    joined.name = "BODY_fighter"  # identical across variants: one skeleton
    joined.parent = arm
    # Exactly one armature modifier on the joined mesh.
    for m in [m for m in joined.modifiers if m.type == "ARMATURE"]:
        joined.modifiers.remove(m)
    mod = joined.modifiers.new("Armature", "ARMATURE")
    mod.object = arm
    # Drop all materials; pass 2 assigns the single atlas material.
    joined.data.materials.clear()

    bpy.ops.wm.save_as_mainfile(filepath=args["staged"])
    manifest = {"variant": variant, "scale": SCALE, "bones": bone_names,
                "joined_verts": len(joined.data.vertices),
                "parts": manifest_parts}
    with open(args["manifest"], "w") as f:
        json.dump(manifest, f, indent=1)
    print("CT7-ASSEMBLED " + variant + " verts=" + str(len(joined.data.vertices)))


main()
