"""Make a body with MPFB, headless, and save it as a GLB.

The point of this script: a real, anatomical human to put our armour on,
instead of a generated lump. The assets are CC0, the tool is GPL, and the
body it makes is ours to shape, armour and paint.

Run:
  blender -b --python tools/blender/mpfb_body.py -- <out.glb> [--height 1.9] [--bulk 1.0]
"""

import json
import os
import warnings
import sys

import bpy

EXIT_USAGE = 2
EXIT_FAIL = 3


def arg(name, fallback=None):
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if name in argv:
        i = argv.index(name)
        if i + 1 < len(argv):
            return argv[i + 1]
    return fallback


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if not args:
        sys.stderr.write("usage: blender -b --python mpfb_body.py -- <out.glb> "
                         "[--height 1.9] [--bulk 1.0]\n")
        return EXIT_USAGE
    out = args[0]
    height = float(arg("--height", "1.9"))
    bulk = float(arg("--bulk", "1.0"))

    bpy.ops.preferences.addon_enable(module="bl_ext.user_default.mpfb")
    import bl_ext.user_default.mpfb as mpfb
    from bl_ext.user_default.mpfb.services.humanservice import HumanService

    print("MPFB", getattr(mpfb, "VERSION", "unknown"))
    methods = [m for m in dir(HumanService) if not m.startswith("_")]
    print("HUMAN SERVICE", methods)

    human = None
    maker = getattr(HumanService, "create_human", None)
    if maker is None:
        raise RuntimeError("HumanService has no create_human")
    # The signature is (mask_helpers, detailed_helpers, extra_vertex_groups,
    # feet_on_ground, scale, macro_detail_dict).
    try:
        # The helpers are the mask cubes and the detail spheres: they are
        # useful in the editor and useless in a game asset.
        human = maker(mask_helpers=False, detailed_helpers=False,
                      extra_vertex_groups=False, feet_on_ground=True, scale=0.1)
        print("created a human")
    except TypeError as err:
        try:
            human = maker()
            print("created a human")
        except Exception as inner:
            raise RuntimeError(f"create_human failed: {inner}") from err
    except Exception as err:
        raise RuntimeError(f"create_human failed: {err}") from err
    if human is None:
        raise RuntimeError("create_human returned nothing")

    # The rig: MPFB's human has no skeleton and no vertex groups until the
    # built-in rig is added, and the rig is what tells the proportion pass and
    # the armour pass which part of the body a vertex is. The rig names in
    # this build are "default", "default_no_toes", "cmu_mb" and "human_toes".
    for rig_name in ("default", "human_toes", "default_no_toes"):
        try:
            HumanService.add_builtin_rig(human, rig_name)
            print("rig added:", rig_name)
            break
        except Exception as err:
            print(f"no rig {rig_name}:", err)

    # The shape: our proportions, on the body MPFB gives us.
    try:
        human.set_property("macro/width", bulk)
        human.set_property("macro/length", height / 1.8)
    except Exception as err:
        print("shape not applied:", err)

    # The garment. MakeHuman's basemesh wears a loincloth shell; the real
    # clothes are a separate asset, and they are what makes the body read as
    # a character rather than a mannequin. The .mhclo file is the asset, and
    # it carries its own textures.
    clothes_name = arg("--clothes", "male_casualsuit01")
    if clothes_name:
        ext = os.path.join(os.path.expandvars(r"%APPDATA%"), "Blender Foundation",
                           "Blender", "5.2", "extensions", "user_default", "mpfb", "data", "clothes")
        mhclo = os.path.join(ext, clothes_name, f"{clothes_name}.mhclo")
        if os.path.exists(mhclo):
            try:
                HumanService.add_mhclo_asset(mhclo, human, asset_type="Clothes",
                                             set_up_rigging=True, interpolate_weights=True)
                print("clothes added:", clothes_name)
            except Exception as err:
                print("no clothes:", err)
        else:
            print("no clothes file:", mhclo)

    bpy.context.view_layer.update()
    # MPFB builds its materials procedurally, so the glTF exporter has no
    # image to write and the exported body comes out plain white. The pack
    # has the diffuse maps, so the suit's texture is wired to the material
    # here, by hand, before the export.
    ext_data = os.path.join(os.path.expandvars(r"%APPDATA%"), "Blender Foundation",
                             "Blender", "5.2", "extensions", "user_default", "mpfb", "data")
    suit_dir = os.path.join(ext_data, "clothes", clothes_name or "")
    diffuse = os.path.join(suit_dir, f"{clothes_name}_diffuse.png")
    for ob in bpy.context.scene.objects:
        if ob.type != "MESH":
            continue
        for slot in ob.material_slots:
            material = slot.material
            if material is None:
                continue
            name = (material.name or "").lower()
            if not (clothes_name and clothes_name.lower() in name):
                continue
            if not os.path.exists(diffuse):
                continue
            try:
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", DeprecationWarning)
                    tree = getattr(material, "node_tree", None)
                    bsdf = tree.nodes.get("Principled BSDF") if tree else None
                    if not bsdf:
                        continue
                    image = bpy.data.images.load(diffuse, check_existing=True)
                    tex = tree.nodes.new("ShaderNodeTexImage")
                    tex.image = image
                    tex.location = (-600, 300)
                    base = bsdf.inputs.get("Base Color")
                    if base:
                        tree.links.new(tex.outputs["Color"], base)
                    print("suit texture wired:", diffuse)
            except Exception as err:
                print("suit texture not wired:", err)

    # Name the materials so the cook can tell the skin from the cloth: the
    # glTF export renames everything to "Material" otherwise, and the cook
    # would paint the face with the faction's armour.
    for ob in bpy.context.scene.objects:
        if ob.type != "MESH":
            continue
        label = "skin" if ob.name.lower() == "human" else "cloth"
        for slot in ob.material_slots:
            if slot.material is not None:
                slot.material.name = label

    bpy.context.view_layer.update()
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    for ob in meshes:
        if len(ob.data.vertices) < 1000:
            bpy.data.objects.remove(ob, do_unlink=True)
    bpy.context.view_layer.update()
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    print("meshes", [(o.name, len(o.data.vertices)) for o in meshes])

    os.makedirs(os.path.dirname(os.path.abspath(out)) or ".", exist_ok=True)
    bpy.ops.object.select_all(action="DESELECT")
    for ob in meshes:
        ob.select_set(True)
    armatures = [o for o in bpy.context.scene.objects if o.type =="ARMATURE"]
    for ob in armatures:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0] if meshes else None
    bpy.ops.export_scene.gltf(filepath=out, export_format="GLB", export_apply=False,
                              export_texcoords=True, export_normals=True,
                              export_skins=True, use_selection=True)
    if not os.path.exists(out) or os.path.getsize(out) == 0:
        raise RuntimeError(f"export wrote nothing: {out}")
    metrics = {
        "out": os.path.abspath(out),
        "bytes": os.path.getsize(out),
        "meshes": len(meshes),
        "vertices": sum(len(o.data.vertices) for o in meshes),
        "height_m": height,
        "bulk": bulk,
    }
    print(json.dumps(metrics, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except Exception as err:
        sys.stderr.write(f"E_MPFB {type(err).__name__}: {err}\n")
        code = EXIT_FAIL
    if code != 0:
        sys.exit(code)
