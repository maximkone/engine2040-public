"""Make a body with MPFB, headless, and save it as a GLB.

The point of this script: a real, anatomical human to put our armour on,
instead of a generated lump. The assets are CC0, the tool is GPL, and the
body it makes is ours to shape, armour and paint.

Run:
  blender -b --python tools/blender/mpfb_body.py -- <out.glb> [--height 1.9] [--bulk 1.0]
      [--clothes male_worksuit01] [--hair short02] [--shoes shoes01] [--proxy proxy741]

  --hair none or --shoes none leaves that asset off.
"""

import json
import os

import bmesh
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


def cut_covered(ob):
    """Delete the vertices a garment covers ("Delete.<asset>" groups)."""
    covered = {g.index for g in ob.vertex_groups if g.name.startswith("Delete.")}
    doomed = [v.index for v in ob.data.vertices
              if any(g.group in covered and g.weight > 0.0 for g in v.groups)]
    for mod in list(ob.modifiers):
        if mod.type == "MASK":
            ob.modifiers.remove(mod)
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    bm.verts.ensure_lookup_table()
    bmesh.ops.delete(bm, geom=[bm.verts[i] for i in doomed], context="VERTS")
    bm.to_mesh(ob.data)
    bm.free()
    ob.data.update()
    print("covered skin cut on", ob.name, ":", len(doomed), "vertices,", len(ob.data.vertices), "left")


def delete_helpers(human):
    """Cut the basemesh down to the body a player sees.

    MakeHuman's basemesh carries helper geometry beside the body: a skirt,
    tights, a long hair shell and joint cubes, there for fitting assets in the
    editor. Only the vertices in the "body" group are the figure; the rest
    exported as a tan dress between the legs and strands of hair on the chest.

    A garment also names the body faces it covers ("Delete.<asset>" groups,
    which MPFB shows as an inverted mask modifier). The export does not apply
    modifiers (the armature must stay live), so the covered skin is deleted
    here, or it pokes through the cloth once the LODs thin it.
    """
    groups = {g.name: g.index for g in human.vertex_groups}
    body = groups.get("body")
    if body is None:
        raise RuntimeError("the basemesh has no 'body' vertex group")
    covered = {i for name, i in groups.items() if name.startswith("Delete.")}
    doomed = []
    for v in human.data.vertices:
        ids = {g.group for g in v.groups if g.weight > 0.0}
        if body not in ids or ids & covered:
            doomed.append(v.index)
    for mod in list(human.modifiers):
        if mod.type == "MASK":
            human.modifiers.remove(mod)
    bm = bmesh.new()
    bm.from_mesh(human.data)
    bm.verts.ensure_lookup_table()
    bmesh.ops.delete(bm, geom=[bm.verts[i] for i in doomed], context="VERTS")
    bm.to_mesh(human.data)
    bm.free()
    human.data.update()
    print("helpers deleted:", len(doomed), "vertices,", len(human.data.vertices), "left")


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
    # The proxy: a low-poly body MakeHuman ships for games, with hands made
    # of a few clean loops. The full basemesh (13k vertices, three-segment
    # fingers) crushed into a 2k triangle budget gives claws; the proxy
    # needs almost no decimation. It is added before the clothes so MPFB
    # moves each garment's covered-skin group onto it.
    ext_data = os.path.join(os.path.expandvars(r"%APPDATA%"), "Blender Foundation",
                            "Blender", "5.2", "extensions", "user_default", "mpfb", "data")
    proxy = None
    proxy_name = arg("--proxy", "proxy741")
    if proxy_name and proxy_name != "none":
        path = os.path.join(ext_data, "proxymeshes", proxy_name, f"{proxy_name}.proxy")
        if not os.path.exists(path):
            raise RuntimeError(f"no proxy {proxy_name}: {path}")
        before = set(bpy.data.objects)
        HumanService.add_mhclo_asset(path, human, asset_type="Proxymeshes",
                                     set_up_rigging=True, interpolate_weights=True)
        added = [o for o in set(bpy.data.objects) - before if o.type == "MESH"]
        if len(added) != 1:
            raise RuntimeError(f"proxy {proxy_name} added {len(added)} meshes, expected 1")
        proxy = added[0]
        print("proxy added:", proxy_name, len(proxy.data.vertices), "vertices")

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

    # The hair and the shoes are assets of their own, like the garment. The
    # basemesh's own hair is a helper shell (long strands down to the chest)
    # that is deleted below with the other helpers, so without a hair asset
    # the figure is bald.
    labels = {}
    for kind, folder, asset_type, fallback in (("hair", "hair", "Hair", "short02"),
                                               ("cloth", "clothes", "Clothes", "shoes01")):
        flag = "--hair" if kind == "hair" else "--shoes"
        name = arg(flag, fallback)
        if not name or name == "none":
            continue
        mhclo = os.path.join(ext_data, folder, name, f"{name}.mhclo")
        if not os.path.exists(mhclo):
            raise RuntimeError(f"no {folder} asset {name}: {mhclo}")
        before = set(bpy.data.objects)
        HumanService.add_mhclo_asset(mhclo, human, asset_type=asset_type,
                                     set_up_rigging=True, interpolate_weights=True)
        for ob in set(bpy.data.objects) - before:
            labels[ob.name] = (kind, os.path.join(ext_data, folder, name, f"{name}_diffuse.png"))
        print(f"{folder} added:", name)

    if proxy is not None:
        # The proxy is the skin; the basemesh only drove the fitting.
        cut_covered(proxy)
        labels[proxy.name] = ("skin", "")
        bpy.data.objects.remove(human, do_unlink=True)
    else:
        delete_helpers(human)

    bpy.context.view_layer.update()
    # MPFB builds its materials procedurally, so the glTF exporter has no
    # image to write and the exported body comes out plain white. The pack
    # has the diffuse maps, so the suit's texture is wired to the material
    # here, by hand, before the export.
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
            texture = diffuse
            if ob.name in labels:
                texture = labels[ob.name][1]
            elif not (clothes_name and clothes_name.lower() in name):
                continue
            if not os.path.exists(texture):
                continue
            try:
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", DeprecationWarning)
                    tree = getattr(material, "node_tree", None)
                    bsdf = tree.nodes.get("Principled BSDF") if tree else None
                    if not bsdf:
                        continue
                    image = bpy.data.images.load(texture, check_existing=True)
                    tex = tree.nodes.new("ShaderNodeTexImage")
                    tex.image = image
                    tex.location = (-600, 300)
                    base = bsdf.inputs.get("Base Color")
                    if base:
                        tree.links.new(tex.outputs["Color"], base)
                    print("texture wired:", texture)
            except Exception as err:
                print("suit texture not wired:", err)

    # Name the materials so the cook can tell the skin from the cloth: the
    # glTF export renames everything to "Material" otherwise, and the cook
    # would paint the face with the faction's armour.
    #
    # The body gets a slot of its own even though it has none: MPFB's skin
    # is procedural, so the exporter has nothing to write for it and the
    # slot comes back empty, which the cook reads as a hole in the face.
    # A slot with a plain material in it is written out with the name it
    # has, and the cook can then paint the face.
    for ob in bpy.context.scene.objects:
        if ob.type != "MESH":
            continue
        if ob.name.lower() == "human":
            label = "skin"
        else:
            label = labels.get(ob.name, ("cloth", None))[0]
        if not ob.material_slots:
            material = bpy.data.materials.new(name=label)
            material.use_nodes = True
            ob.data.materials.append(material)
            for poly in ob.data.polygons:
                poly.material_index = 0
            continue
        for slot in ob.material_slots:
            if slot.material is None:
                material = bpy.data.materials.new(name=label)
                material.use_nodes = True
                slot.material = material
            else:
                slot.material.name = label

    bpy.context.view_layer.update()
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    # Small meshes the build leaves behind go; the assets asked for stay,
    # however few vertices a short haircut or a shoe has.
    for ob in meshes:
        if len(ob.data.vertices) < 1000 and ob.name not in labels:
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
