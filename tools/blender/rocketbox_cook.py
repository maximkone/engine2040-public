"""Cook one Microsoft Rocketbox avatar (MIT) into game files.

Run (the driver does this; the flags are the proven ones):
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/rocketbox_cook.py -- <fbx> <outdir> [--tris0 7400]

<fbx> is <home>/<Name>/Export/<Name>.fbx; the maps are read from the sibling
<home>/<Name>/Textures folder, the way the repo lays the avatar out.

Writes into <outdir>:
  lod0.glb .. lod3.glb   the skin and the armature kept, nothing applied to the skin
  textures/*.png         every map at 1024 px on its longest edge
                         (colour as sRGB, normal as non-colour)
  hero.png               3/4 view, the same camera and lights as the test render,
                         the figure about 80% of the frame's height
  cook.json              triangles per LOD, bones, height, maps, source hash, licence

The bones are renamed into the contract charset (lowercase, anything that is
not a letter or digit becomes "_"): "Bip01 L UpperArm" -> "bip01_l_upperarm",
the names tools/... and crates/assets/src/rocketbox_map.rs expect.

Exit 2 for a usage error, 3 for a cook that cannot be made (no meshes, a bone
collision, a missing map). Blender's own failures exit 1.
"""

import bpy
import hashlib
import json
import math
import os
import sys

from bpy_extras.object_utils import world_to_camera_view
from mathutils import Vector

EXIT_USAGE = 2
EXIT_FAIL = 3

MAX_EDGE = 1024
LOD_FRACTIONS = (1.0, 0.5, 0.25, 0.125)
HERO_RES = (900, 1100)
HERO_FILL = 0.80
HERO_UP = 0.05
ROUGHNESS = 0.6
CUTOUT_WORDS = ("opacity", "lash", "hair")


def fail(msg, code=EXIT_FAIL):
    print(f"ROCKETBOX FAIL: {msg}")
    sys.exit(code)


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    tris0 = 7400
    pos = []
    i = 0
    while i < len(argv):
        if argv[i] == "--tris0" and i + 1 < len(argv):
            tris0 = int(argv[i + 1])
            i += 2
        else:
            pos.append(argv[i])
            i += 1
    if len(pos) != 2 or tris0 <= 0:
        fail("usage: rocketbox_cook.py -- <fbx> <outdir> [--tris0 7400]", EXIT_USAGE)
    return pos[0], pos[1], tris0


def contract_name(name):
    """The anim contract charset: lowercase, every other character a underscore."""
    return "".join(ch.lower() if ch.isalnum() else "_" for ch in name)


def classify_map(filename):
    """(key, role) for a colour or normal TGA, else None. The key is the material's tag."""
    stem, ext = os.path.splitext(filename)
    if ext.lower() != ".tga":
        return None
    if stem.endswith("_normal"):
        return stem[: -len("_normal")], "normal"
    if "_color" in stem:
        return stem.split("_color")[0], "color"
    return None


def load_maps(tex_dir):
    maps = {}
    for fname in sorted(os.listdir(tex_dir)):
        hit = classify_map(fname)
        if hit is None:
            continue
        key, role = hit
        maps.setdefault(key, {})[role] = os.path.join(tex_dir, fname)
    return maps


def tail(key):
    return key.split("_", 1)[1] if "_" in key else key


def match_map(material_name, keys):
    """The map key a material takes: the same tail first, else the longest partial."""
    name = material_name.split(".")[0].lower()
    if name in keys:
        return name
    want = tail(name)
    for k in keys:
        if tail(k) == want:
            return k
    partial = [k for k in keys if tail(k) and (tail(k) in want or want in tail(k))]
    return max(partial, key=len) if partial else None


def convert_map(path, role, out_dir, cache):
    """Scale a TGA to MAX_EDGE, save it as PNG under out_dir, and reload the PNG."""
    if path in cache:
        return cache[path]
    img = bpy.data.images.load(path, check_existing=True)
    w, h = img.size
    k = min(1.0, MAX_EDGE / float(max(w, h)))
    nw, nh = max(1, round(w * k)), max(1, round(h * k))
    if (nw, nh) != (w, h):
        img.scale(nw, nh)
    out = os.path.join(out_dir, os.path.splitext(os.path.basename(path))[0] + ".png")
    img.filepath_raw = out
    img.file_format = "PNG"
    img.save()
    img.filepath = out
    img.reload()
    if role == "normal":
        img.colorspace_settings.name = "Non-Color"
    cache[path] = img
    return img


def wire_material(mat, color_img, normal_img, cutout):
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = next((n for n in nt.nodes if n.type == "BSDF_PRINCIPLED"), None)
    if bsdf is None:
        fail(f"material {mat.name} has no Principled BSDF")
    # The FBX importer leaves its own image nodes behind; ours replace them.
    for node in [n for n in nt.nodes if n.type == "TEX_IMAGE"]:
        nt.nodes.remove(node)
    col = nt.nodes.new("ShaderNodeTexImage")
    col.image = color_img
    nt.links.new(col.outputs["Color"], bsdf.inputs["Base Color"])
    if cutout:
        nt.links.new(col.outputs["Alpha"], bsdf.inputs["Alpha"])
        if hasattr(mat, "surface_render_method"):
            mat.surface_render_method = "BLENDED"
        elif hasattr(mat, "blend_method"):
            mat.blend_method = "BLEND"
    if normal_img is not None:
        nrm = nt.nodes.new("ShaderNodeTexImage")
        nrm.image = normal_img
        nrm.image.colorspace_settings.name = "Non-Color"
        nmap = nt.nodes.new("ShaderNodeNormalMap")
        nt.links.new(nrm.outputs["Color"], nmap.inputs["Color"])
        nt.links.new(nmap.outputs["Normal"], bsdf.inputs["Normal"])
    bsdf.inputs["Roughness"].default_value = ROUGHNESS


def rename_bones(arm, meshes):
    """Bones first: Blender carries the vertex groups along with a bone's name.
    A vertex group still under an old name is renamed here; one that matches no
    bone at all is counted and left as it came."""
    mapping = {b.name: contract_name(b.name) for b in arm.data.bones}
    if len(set(mapping.values())) != len(mapping):
        fail("two bones take the same contract name after the rename")
    for bone in arm.data.bones:
        bone.name = mapping[bone.name] if bone.name in mapping else bone.name
    new_names = set(mapping.values())
    unmatched = 0
    for mesh in meshes:
        for vg in mesh.vertex_groups:
            if vg.name in mapping:
                vg.name = mapping[vg.name]
            elif vg.name not in new_names:
                unmatched += 1
    return mapping, unmatched


def tri_count(objs):
    total = 0
    for o in objs:
        o.data.calc_loop_triangles()
        total += len(o.data.loop_triangles)
    return total


def bounds(objs):
    lo = Vector((1e9, 1e9, 1e9))
    hi = Vector((-1e9, -1e9, -1e9))
    for o in objs:
        for corner in o.bound_box:
            w = o.matrix_world @ Vector(corner)
            lo = Vector(map(min, lo, w))
            hi = Vector(map(max, hi, w))
    return lo, hi


def lod_copies(sources, ratio, label):
    """A copy of every source mesh, decimated to ratio of its triangles, skin kept."""
    copies = []
    for src in sources:
        c = src.copy()
        c.data = src.data.copy()
        c.name = f"{src.name}__{label}"
        c.hide_viewport = False
        c.hide_render = False
        bpy.context.scene.collection.objects.link(c)
        if ratio < 0.9999:
            mod = c.modifiers.new("cook_decimate", "DECIMATE")
            mod.decimate_type = "COLLAPSE"
            mod.ratio = ratio
            c.modifiers.move(len(c.modifiers) - 1, 0)
            bpy.context.view_layer.objects.active = c
            c.select_set(True)
            bpy.ops.object.modifier_apply(modifier=mod.name)
            c.select_set(False)
        copies.append(c)
    return copies


def drop_copies(copies):
    for c in copies:
        data = c.data
        bpy.data.objects.remove(c, do_unlink=True)
        if data is not None and data.users == 0:
            bpy.data.meshes.remove(data)


def export_glb(copies, arm, path):
    bpy.ops.object.select_all(action="DESELECT")
    for c in copies:
        c.select_set(True)
    arm.select_set(True)
    bpy.context.view_layer.objects.active = arm
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        use_selection=True,
        export_skins=True,
        export_animations=False,
        export_apply=False,
        export_image_format="AUTO",
        export_materials="EXPORT",
        export_yup=True,
    )
    bpy.ops.object.select_all(action="DESELECT")
    if not os.path.exists(path) or os.path.getsize(path) == 0:
        fail(f"export wrote nothing: {path}")
    return os.path.getsize(path)


def frame_hero(objs, cam_obj, scene, out_path):
    """Dolly the camera until the figure is HERO_FILL of the frame's height, then render."""
    # The framing maths reads the frame's shape, so the portrait size goes first.
    scene.render.resolution_x, scene.render.resolution_y = HERO_RES
    scene.render.resolution_percentage = 100
    lo, hi = bounds(objs)
    height = hi.z - lo.z
    ctr = (lo + hi) / 2.0
    cam_obj.data.sensor_fit = "AUTO"
    fov = cam_obj.data.angle_y
    direction = Vector((-0.55, -0.85, 0.0)).normalized()
    dist = height / (HERO_FILL * 2.0 * math.tan(fov / 2.0))
    span = 0.0
    for _ in range(4):
        cam_obj.location = ctr + direction * dist + Vector((0.0, 0.0, height * HERO_UP))
        cam_obj.rotation_euler = (ctr - cam_obj.location).to_track_quat("-Z", "Y").to_euler()
        bpy.context.view_layer.update()
        # Every vertex, not the box: the box overstates the figure by a tenth.
        vs = []
        for o in objs:
            for v in o.data.vertices:
                vs.append(world_to_camera_view(scene, cam_obj, o.matrix_world @ v.co).y)
        span = max(vs) - min(vs)
        if span <= 0:
            break
        dist *= span / HERO_FILL
    scene.render.filepath = out_path
    scene.render.resolution_x, scene.render.resolution_y = HERO_RES
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    bpy.ops.render.render(write_still=True)
    if not os.path.exists(out_path) or os.path.getsize(out_path) == 0:
        fail(f"hero render wrote nothing: {out_path}")
    return height, span


def set_camera_and_lights(scene):
    cam = bpy.data.objects.new("hero_cam", bpy.data.cameras.new("hero_cam"))
    cam.data.lens = 50
    scene.collection.objects.link(cam)
    scene.camera = cam
    for rot, energy in (((math.radians(50), 0, math.radians(-40)), 4.0),
                        ((math.radians(70), 0, math.radians(140)), 1.5)):
        sun = bpy.data.objects.new("hero_sun", bpy.data.lights.new("hero_sun", "SUN"))
        sun.data.energy = energy
        sun.rotation_euler = rot
        scene.collection.objects.link(sun)
    world = bpy.data.worlds.new("hero_world")
    world.use_nodes = True
    bg = world.node_tree.nodes["Background"]
    bg.inputs[0].default_value = (0.08, 0.08, 0.09, 1)
    bg.inputs[1].default_value = 1.0
    scene.world = world
    scene.render.engine = "BLENDER_EEVEE"
    return cam


def main():
    fbx, out_dir, tris0 = parse_args()
    fbx = os.path.abspath(fbx)
    out_dir = os.path.abspath(out_dir)
    if not os.path.isfile(fbx):
        fail(f"no FBX at {fbx}")
    tex_dir = os.path.join(os.path.dirname(os.path.dirname(fbx)), "Textures")
    avatar = os.path.splitext(os.path.basename(fbx))[0]
    os.makedirs(os.path.join(out_dir, "textures"), exist_ok=True)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.fbx(filepath=fbx)
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    arms = [o for o in bpy.context.scene.objects if o.type == "ARMATURE"]
    if not meshes:
        fail("the FBX has no mesh")
    if len(arms) != 1:
        fail(f"expected one armature, found {len(arms)}")
    arm = arms[0]
    bone_map, unmatched = rename_bones(arm, meshes)
    bones = len(arm.data.bones)
    print(f"RB bones {bones}, vertex groups without a bone {unmatched}")

    keys_maps = load_maps(tex_dir)
    if not keys_maps:
        fail(f"no colour maps in {tex_dir}")
    cache = {}
    texture_rows = []
    material_rows = []
    done = set()
    for o in meshes:
        for slot in o.material_slots:
            mat = slot.material
            if mat is None or mat.name in done:
                continue
            done.add(mat.name)
            key = match_map(mat.name, list(keys_maps))
            material_rows.append({"name": mat.name, "map": key})
            if key is None:
                print(f"RB material {mat.name} has no map, left as it came")
                continue
            maps = keys_maps[key]
            if "color" not in maps:
                fail(f"map {key} has no colour file")
            color = convert_map(maps["color"], "color", os.path.join(out_dir, "textures"), cache)
            normal = None
            if "normal" in maps:
                normal = convert_map(maps["normal"], "normal", os.path.join(out_dir, "textures"), cache)
            cutout = any(word in key for word in CUTOUT_WORDS)
            wire_material(mat, color, normal, cutout)
    for img in cache.values():
        texture_rows.append({
            "name": os.path.basename(img.filepath),
            "width": img.size[0],
            "height": img.size[1],
            "bytes": os.path.getsize(img.filepath),
        })

    source_tris = tri_count(meshes)
    r0 = min(1.0, tris0 / float(source_tris))
    lod0 = lod_copies(meshes, r0, "lod0")
    for o in meshes:
        o.hide_render = True
        o.hide_viewport = True

    scene = bpy.context.scene
    set_camera_and_lights(scene)
    height, span = frame_hero(lod0, scene.camera, scene, os.path.join(out_dir, "hero.png"))
    print(f"RB height {height:.3f} m, hero span {span:.3f}")

    lods = []
    lods.append({"lod": 0, "copies": lod0, "ratio": r0})
    for k, frac in enumerate(LOD_FRACTIONS[1:], start=1):
        lods.append({"lod": k, "copies": None, "ratio": r0 * frac})

    summary = []
    for entry in lods:
        k = entry["lod"]
        copies = entry["copies"] if entry["copies"] is not None else lod_copies(meshes, entry["ratio"], f"lod{k}")
        tris = tri_count(copies)
        path = os.path.join(out_dir, f"lod{k}.glb")
        size = export_glb(copies, arm, path)
        summary.append({"lod": k, "file": f"lod{k}.glb", "triangles": tris, "bytes": size,
                        "ratio": round(entry["ratio"], 4)})
        print(f"RB lod{k} {tris} tris {size} B")
        drop_copies(copies)

    sha = hashlib.sha256()
    with open(fbx, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            sha.update(chunk)
    cook = {
        "avatar": avatar,
        "licence": "MIT",
        "source": {"fbx": os.path.basename(fbx), "sha256": sha.hexdigest(), "bytes": os.path.getsize(fbx)},
        "tris0": tris0,
        "source_triangles": source_tris,
        "height_m": round(height, 4),
        "bones": bones,
        "bone_map": bone_map,
        "vertex_groups_without_bone": unmatched,
        "materials": material_rows,
        "textures": texture_rows,
        "lods": summary,
        "hero": "hero.png",
    }
    with open(os.path.join(out_dir, "cook.json"), "w", encoding="utf-8") as handle:
        json.dump(cook, handle, indent=2, sort_keys=True)
    print(f"RB cook done: {out_dir}")


main()
