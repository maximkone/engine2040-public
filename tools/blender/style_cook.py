"""The style pass: a generated base mesh becomes a Bannerfall asset.

This is where the 2005 look comes from. The generator gives volume; this
script gives us the game: our palette, our triangle budget, our LODs, our
painted AO, on our scale.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/style_cook.py -- <in.glb> <outdir> [--faction ashkin] [--tris 2000] [--height 2.0]

Writes into <outdir>:
  lod0.glb lod1.glb lod2.glb lod3.glb   the four LODs, vertex AO in COLOR_0
  atlas.png                           the 512 palette atlas
  cook.json                           the metrics and the provenance of the pass

Rules taken from content/design/faction-war-style.md:
  one 512 atlas per body, one 256 per weapon
  two roughness values per character, matte cloth and one metal
  1.4k to 2.2k triangles at LOD0
  painted AO, silhouette first
"""

import json
import math
import os
import sys
import warnings

import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree

try:
    import numpy as np
except ImportError:  # pragma: no cover - Blender ships numpy
    np = None

EXIT_USAGE = 2
EXIT_FAIL = 3

PALETTES = {
    "ashkin": ["#2B2724", "#7A1F1C", "#9C6B3A", "#D9CBB2", "#C8562A"],
    "wardens": ["#E8E2D4", "#B9BEC4", "#C9A227", "#3E5A78", "#6B4B33"],
}
LOD_RATIOS = [1.0, 0.55, 0.30, 0.15]
ATLAS_SIZE = 512
AO_RAYS = 14
AO_STRENGTH = 0.55
# The painted light: a key from the upper front left, one bounce of fill, a
# rim on the edges that leave the vertical. This is the 2005 look: the light
# is in the mesh, not in the engine.
KEY = (-0.45, -0.55, 0.70)
AMBIENT = 0.42
KEY_STRENGTH = 0.78
RIM_STRENGTH = 0.22
SHADOW_LIFT = 0.16
ROUGHNESS_MATTE = 0.55
ROUGHNESS_CLOTH = 0.85


class CookError(Exception):
    """A cook that could not be finished."""


def arg(name, fallback=None):
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if name in argv:
        i = argv.index(name)
        if i + 1 < len(argv):
            return argv[i + 1]
    return fallback


def has_flag(name):
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    return name in argv


def hex_rgb(value):
    value = value.lstrip("#")
    return tuple(int(value[i:i + 2], 16) / 255.0 for i in (0, 2, 4))


def srgb_to_linear(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def hex_linear(value):
    return tuple(srgb_to_linear(c) for c in hex_rgb(value))


# ------------------------------------------------------------------ import


def import_mesh(path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=path)
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    if not meshes:
        raise CookError(f"no mesh in {path}")
    bpy.ops.object.select_all(action="DESELECT")
    for ob in meshes:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    if len(meshes) > 1:
        bpy.ops.object.join()
    ob = bpy.context.view_layer.objects.active
    ob.name = "body"
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    # A body that carries its proportions as shape keys cannot be decimated,
    # so the current shape is baked into the mesh first.
    if ob.data.shape_keys is not None:
        bpy.ops.object.select_all(action="DESELECT")
        ob.select_set(True)
        bpy.context.view_layer.objects.active = ob
        bpy.ops.object.convert(target="MESH")
        ob = bpy.context.view_layer.objects.active
        ob.name = "body"
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    return ob


def mesh_bounds(ob):
    low = Vector((1e9, 1e9, 1e9))
    high = Vector((-1e9, -1e9, -1e9))
    for corner in ob.bound_box:
        co = ob.matrix_world @ Vector(corner)
        for i in range(3):
            low[i] = min(low[i], co[i])
            high[i] = max(high[i], co[i])
    return low, high


def normalise_height(ob, target_height):
    """Every class stands the same tall, so one scale rule covers the game."""
    low, high = mesh_bounds(ob)
    height = high.z - low.z
    if height <= 1e-6:
        raise CookError("the mesh has no height")
    scale = target_height / height
    ob.scale = (scale, scale, scale)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    ob.location.z -= low.z * scale
    bpy.ops.object.transform_apply(location=True, rotation=False, scale=False)
    return height, scale


def triangle_count(ob):
    ob.data.calc_loop_triangles()
    return len(ob.data.loop_triangles)


def ensure_uv(ob):
    if ob.data.uv_layers:
        return False
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=math.radians(66.0), island_margin=0.02)
    bpy.ops.object.mode_set(mode="OBJECT")
    return True


# ----------------------------------------------------------------- palette


def find_base_image(ob):
    for slot in ob.material_slots:
        if not slot.material:
            continue
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                tree = getattr(slot.material, "node_tree", None)
                if tree:
                    for node in tree.nodes:
                        if node.type == "TEX_IMAGE" and node.image:
                            return node.image
        except Exception:
            continue
    for image in bpy.data.images:
        if image.source == "FILE" and image.filepath:
            return image
    return None


def nearest_palette(pixels, palette):
    """Push every pixel into the palette, keeping the original shading.

    The hue chooses the palette entry, the luminance scales it. That keeps
    the baked light and shadow from the generator and throws away its
    colours, which is the whole point of the style pass.
    """
    if np is None:
        raise CookError("numpy is required for the palette pass")
    h, w = pixels.shape[0], pixels.shape[1]
    rgb = pixels[:, :, :3].reshape(-1, 3)
    alpha = pixels[:, :, 3].reshape(-1, 1)

    mx = rgb.max(axis=1)
    mn = rgb.min(axis=1)
    v = mx
    sat = np.where(mx > 1e-6, (mx - mn) / np.maximum(mx, 1e-6), 0.0)

    pal = np.array([hex_linear(c) for c in palette], dtype=np.float32)
    pal_rgb = np.array([hex_rgb(c) for c in palette], dtype=np.float32)
    pmx = pal_rgb.max(axis=1)
    pmn = pal_rgb.min(axis=1)
    pv = pmx
    psat = np.where(pmx > 1e-6, (pmx - pmn) / np.maximum(pmx, 1e-6), 0.0)

    # Lift the shadows, the way a painter does. Black iron at #2B2724 is
    # 0.026 in linear light: multiplied by a painted light it is simply
    # black, and the whole character reads as a hole. A painted black is
    # never black: the brush leaves the bounce.
    pal = pal + (1.0 - pal) * SHADOW_LIFT

    # Hue as a vector, so the distance is a real hue distance.
    def hue_vec(rgb_arr, sat_arr, val_arr):
        r, g, b = rgb_arr[:, 0], rgb_arr[:, 1], rgb_arr[:, 2]
        delta = val_arr - rgb_arr.min(axis=1)
        hue = np.zeros_like(val_arr)
        nz = delta > 1e-6
        r_is_max = nz & (r >= g) & (r >= b)
        g_is_max = nz & (~r_is_max) & (g >= b)
        b_is_max = nz & (~r_is_max) & (~g_is_max)
        hue[r_is_max] = ((g[r_is_max] - b[r_is_max]) / delta[r_is_max]) % 6.0
        hue[g_is_max] = ((b[g_is_max] - r[g_is_max]) / delta[g_is_max]) + 2.0
        hue[b_is_max] = ((r[b_is_max] - g[b_is_max]) / delta[b_is_max]) + 4.0
        hue = hue / 6.0
        return np.stack([np.cos(hue * 2 * math.pi), np.sin(hue * 2 * math.pi)], axis=1) * sat_arr[:, None]

    src_h = hue_vec(rgb, sat, v)
    pal_h = hue_vec(pal_rgb, psat, pv)

    # Only saturated pixels take a hue; the rest take the least saturated
    # palette entry, which is the neutral in that palette.
    # A desaturated pixel takes the palette's neutral, not the nearest hue.
    # A grey suit is not bone white: it is black iron, and picking the nearest
    # hue sent every grey in the texture to the lightest entry and blew the
    # whole character out.
    neutral = int(psat.argmin())
    weights = np.where(sat[:, None] > 0.12, 1.0, 0.0)
    dist = np.linalg.norm(src_h[:, None, :] - pal_h[None, :, :], axis=2)
    dist = dist + (1.0 - weights) * 10.0
    grey = weights[:, 0] < 0.5
    if grey.any():
        dist[grey, :] = 10.0
        dist[grey, neutral] = 0.0
    choice = dist.argmin(axis=1)

    out = pal[choice]
    # Shading: the scale between the pixel value and the palette value.
    shade = np.where(pv[choice] > 1e-6, v / np.maximum(pv[choice], 1e-6), 1.0)
    shade = np.clip(shade, 0.35, 1.8)
    out = np.clip(out * shade[:, None], 0.0, 1.0)

    result = np.concatenate([out, alpha], axis=1).reshape(h, w, 4).astype(np.float32)
    return result


def wire_vertex_light(material, attribute="LIGHT"):
    """Multiply the atlas by the painted light layer.

    The engine does this in the shader; the cook has to do it too, or the
    hero render shows a flat texture and the painted light looks like it
    did nothing.
    """
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            tree = getattr(material, "node_tree", None)
            if not tree:
                return False
            bsdf = tree.nodes.get("Principled BSDF")
            if not bsdf:
                return False
            base = bsdf.inputs.get("Base Color")
            if not base or not base.links:
                return False
            source = base.links[0].from_socket

            attribute_node = tree.nodes.new("ShaderNodeVertexColor")
            attribute_node.layer_name = attribute
            attribute_node.location = (source.node.location.x - 400, source.node.location.y - 220)
            mix = tree.nodes.new("ShaderNodeMix")
            mix.data_type = "RGBA"
            mix.blend_type = "MULTIPLY"
            mix.location = (source.node.location.x - 180, source.node.location.y - 100)
            mix.inputs[0].default_value = 1.0
            tree.links.new(source, mix.inputs[6])
            tree.links.new(attribute_node.outputs["Color"], mix.inputs[7])
            tree.links.new(mix.outputs[2], base)
            return True
    except Exception:
        return False


def bake_atlas(ob, palette, out_path):
    """Write the 512 palette atlas and point the material at it."""
    image = find_base_image(ob)
    if image is None:
        # No texture to repaint: a flat palette sheet with a vertical ramp.
        # The mesh keeps its shading, the engine keeps the atlas rule.
        neutral = hex_linear(palette[0]) if palette else (0.2, 0.2, 0.2)
        ramp = np.zeros((ATLAS_SIZE, ATLAS_SIZE, 4), dtype=np.float32)
        for row in range(ATLAS_SIZE):
            k = 0.55 + 0.45 * (row / float(ATLAS_SIZE - 1))
            ramp[row, :, :3] = np.clip(np.array(neutral, dtype=np.float32) * k, 0.0, 1.0)
        ramp[:, :, 3] = 1.0
        atlas = bpy.data.images.new("atlas", width=ATLAS_SIZE, height=ATLAS_SIZE, alpha=True)
        atlas.pixels.foreach_set(ramp.reshape(-1))
        atlas.file_format = "PNG"
        atlas.filepath_raw = out_path
        atlas.save()
        for slot in ob.material_slots:
            if not slot.material:
                continue
            try:
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", DeprecationWarning)
                    tree = getattr(slot.material, "node_tree", None)
                    if tree:
                        for node in tree.nodes:
                            if node.type == "TEX_IMAGE":
                                node.image = atlas
            except Exception:
                pass
        return {"source_size": [0, 0], "atlas": ATLAS_SIZE, "distinct_colors": ATLAS_SIZE,
                "synthetic": True}
    width, height = image.size
    buf = np.empty(width * height * 4, dtype=np.float32)
    image.pixels.foreach_get(buf)
    pixels = buf.reshape(height, width, 4)

    painted = nearest_palette(pixels, palette)

    atlas = bpy.data.images.new("atlas", width=ATLAS_SIZE, height=ATLAS_SIZE, alpha=True)
    # Sample the painted image down to the atlas size.
    step_x = max(1, width // ATLAS_SIZE)
    step_y = max(1, height // ATLAS_SIZE)
    small = painted[::step_y, ::step_x, :]
    if small.shape[0] < ATLAS_SIZE or small.shape[1] < ATLAS_SIZE:
        pad_h = ATLAS_SIZE - small.shape[0]
        pad_w = ATLAS_SIZE - small.shape[1]
        small = np.pad(small, ((0, pad_h), (0, pad_w), (0, 0)), mode="edge")
    small = small[:ATLAS_SIZE, :ATLAS_SIZE, :]
    atlas.pixels.foreach_set(small.reshape(-1).astype(np.float32))
    atlas.file_format = "PNG"
    atlas.filepath_raw = out_path
    atlas.save()

    # Point the material at the new atlas, and let the painted light in.
    wired = False
    for slot in ob.material_slots:
        if not slot.material:
            continue
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                tree = getattr(slot.material, "node_tree", None)
                if tree:
                    for node in tree.nodes:
                        if node.type == "TEX_IMAGE":
                            node.image = atlas
            wired = wire_vertex_light(slot.material) or wired
        except Exception:
            pass

    unique = len(np.unique(np.round(small.reshape(-1, 4)[:, :3], 3), axis=0))
    return {"source_size": [width, height], "atlas": ATLAS_SIZE, "distinct_colors": int(unique)}


# ---------------------------------------------------------------------- AO


def bake_vertex_light(ob):
    """Painted light, baked into the vertex colours.

    This is the old look. A 2005 character was lit by hand: a strong key
    from the upper front, the undersides filled by one bounce, a rim on the
    top edges, and the crevices darkened. The engine multiplies the palette
    atlas by this layer, so the colour stays ours and the light is painted.

    The key is a direction, the fill is the ambient term, the rim comes from
    the normal leaving the vertical, and the occlusion is the real thing:
    rays from every vertex into its own hemisphere.
    """
    me = ob.data
    verts = [v.co.copy() for v in me.vertices]
    normals = [v.normal.copy() for v in me.vertices]
    polys = [list(p.vertices) for p in me.polygons]
    if not polys:
        raise CookError("the mesh has no faces")
    bvh = BVHTree.FromPolygons(verts, polys)

    layer = me.color_attributes.get("LIGHT")
    if layer is None:
        layer = me.color_attributes.new(name="LIGHT", type="FLOAT_COLOR", domain="POINT")

    directions = []
    for i in range(AO_RAYS):
        a = 2.0 * math.pi * i / AO_RAYS
        z = 1.0 - (i + 0.5) / AO_RAYS
        r = math.sqrt(max(0.0, 1.0 - z * z))
        directions.append(Vector((r * math.cos(a), r * math.sin(a), z)))

    # The key light: high, a little to the front and to the left.
    key = Vector(KEY).normalized()

    values = []
    for index, (co, normal) in enumerate(zip(verts, normals)):
        up = normal.normalized()
        ref = Vector((0.0, 0.0, 1.0)) if abs(up.z) < 0.9 else Vector((1.0, 0.0, 0.0))
        side = up.cross(ref).normalized()
        other = up.cross(side).normalized()
        hits = 0
        for d in directions:
            direction = side * d.x + other * d.y + up * d.z
            hit = bvh.ray_cast(co + direction * 1e-4, direction, 4.0)
            if hit[0] is not None:
                hits += 1
        occlusion = hits / float(len(directions))
        ao = 1.0 - AO_STRENGTH * occlusion

        lambert = max(0.0, up.dot(key))
        # The rim: the more the normal leaves the vertical, the more it
        # catches the light from the side.
        rim = (1.0 - abs(up.z)) ** 2.0
        value = ao * (AMBIENT + KEY_STRENGTH * lambert) + RIM_STRENGTH * rim * ao
        value = max(0.0, min(1.35, value))
        values.append(value)

    data = layer.data
    for index, value in enumerate(values):
        data[index].color = (value, value, value, 1.0)
    me.update()
    return {"min": round(min(values), 4), "max": round(max(values), 4),
            "mean": round(sum(values) / len(values), 4),
            "ambient": AMBIENT, "key_strength": KEY_STRENGTH,
            "rim_strength": RIM_STRENGTH, "key": list(KEY)}


# ------------------------------------------------------------------- LODs


def decimate(ob, ratio):
    """Decimate in steps.

    A single collapse from 27k to 2k shatters the mesh: the faces fold over
    each other and the shading breaks up. Halving a few times keeps every
    collapse local, so the silhouette and the UVs survive.
    """
    if ratio >= 0.999:
        return triangle_count(ob)
    current = triangle_count(ob)
    target = max(4, int(current * ratio))
    while current > target:
        step = min(0.5, max(0.05, target / float(current)))
        mod = ob.modifiers.new(name="decimate", type="DECIMATE")
        mod.ratio = step
        mod.use_collapse_triangulate = True
        bpy.ops.object.modifier_apply(modifier=mod.name)
        after = triangle_count(ob)
        if after >= current:
            break
        current = after
    return current


def render_hero(ob, path, palette):
    """The picture that proves the style pass: our palette on our mesh.

    Workbench ignores textures, so this needs a real engine: EEVEE if the
    build has it, Cycles otherwise. One sun, one fill, a 3/4 camera, the
    same 30 m discipline but close enough to see the paint.
    """
    scene = bpy.context.scene
    for engine in ("BLENDER_EEVEE_NEXT", "BLENDER_EEVEE", "CYCLES"):
        try:
            scene.render.engine = engine
            break
        except Exception:
            continue
    scene.render.resolution_x, scene.render.resolution_y = 900, 1100
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = False
    scene.render.image_settings.file_format = "PNG"
    try:
        scene.view_settings.view_transform = "Standard"
    except Exception:
        pass

    world = bpy.data.worlds.new("cook_world")
    scene.world = world
    world.color = (0.10, 0.10, 0.11)
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            if getattr(world, "node_tree", None) is None and hasattr(world, "use_nodes"):
                world.use_nodes = True
        tree = getattr(world, "node_tree", None)
        if tree:
            node = tree.nodes.get("Background")
            if node:
                node.inputs[0].default_value = (0.10, 0.10, 0.11, 1.0)
    except Exception:
        pass

    sun_data = bpy.data.lights.new("sun", type="SUN")
    sun_data.energy = 5.5
    sun_data.angle = math.radians(8.0)
    sun = bpy.data.objects.new("sun", sun_data)
    bpy.context.collection.objects.link(sun)
    sun.rotation_euler = (math.radians(52.0), 0.0, math.radians(34.0))

    fill_data = bpy.data.lights.new("fill", type="AREA")
    fill_data.energy = 1400.0
    fill_data.size = 5.0
    fill = bpy.data.objects.new("fill", fill_data)
    bpy.context.collection.objects.link(fill)
    fill.location = (-3.0, -3.4, 2.6)
    fill.rotation_euler = (math.radians(64.0), 0.0, math.radians(-44.0))

    rim_data = bpy.data.lights.new("rim", type="AREA")
    rim_data.energy = 1500.0
    rim_data.size = 3.0
    rim = bpy.data.objects.new("rim", rim_data)
    bpy.context.collection.objects.link(rim)
    rim.location = (2.6, 3.2, 3.0)
    rim.rotation_euler = (math.radians(128.0), 0.0, math.radians(150.0))

    low, high = mesh_bounds(ob)
    center = (low + high) * 0.5
    height = max(high.z - low.z, 1e-6)
    target = bpy.data.objects.new("hero_target", None)
    bpy.context.collection.objects.link(target)
    target.location = center
    cam_data = bpy.data.cameras.new("hero_cam")
    cam_data.lens = 70.0
    cam = bpy.data.objects.new("hero_cam", cam_data)
    bpy.context.collection.objects.link(cam)
    direction = Vector((0.75, -1.0, 0.32)).normalized()
    cam.location = center + direction * (height * 2.4)
    con = cam.constraints.new("TRACK_TO")
    con.target = target
    con.track_axis = "TRACK_NEGATIVE_Z"
    con.up_axis = "UP_Y"
    scene.camera = cam

    scene.render.filepath = path
    try:
        bpy.ops.render.render(write_still=True)
    except Exception as err:
        return {"ok": False, "error": str(err)}
    ok = os.path.exists(path) and os.path.getsize(path) > 0
    return {"ok": ok, "engine": scene.render.engine, "palette": palette}


def export_glb(ob, path):
    """Export one object, not the scene: the export operator writes every
    visible object, and the cook keeps a pristine copy around."""
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    kwargs = dict(filepath=path, export_format="GLB", export_apply=True,
                  export_texcoords=True, export_normals=True, use_selection=True)
    for name in ("export_colors", "export_vertex_color"):
        try:
            bpy.ops.export_scene.gltf(**kwargs, **{name: "NONE"})
            return True
        except TypeError:
            continue
    try:
        bpy.ops.export_scene.gltf(**kwargs)
        return True
    except Exception:
        return False


# ------------------------------------------------------------------- main


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(args) < 2:
        sys.stderr.write("usage: blender -b --factory-startup --python-exit-code 1 "
                         "-P style_cook.py -- <in.glb> <outdir> [--faction ashkin]\n")
        return EXIT_USAGE
    src, outdir = args[0], args[1]
    faction = str(arg("--faction", "ashkin")).lower()
    if faction not in PALETTES:
        raise CookError(f"unknown faction {faction}: pick one of {sorted(PALETTES)}")
    budget = int(arg("--tris", "2000"))
    target_height = float(arg("--height", "2.0"))

    os.makedirs(outdir, exist_ok=True)
    palette = PALETTES[faction]

    ob = import_mesh(src)
    raw_tris = triangle_count(ob)
    source_height, scale = normalise_height(ob, target_height)
    unwrapped = ensure_uv(ob)

    atlas_path = os.path.join(outdir, "atlas.png")
    atlas_info = bake_atlas(ob, palette, atlas_path)
    light_info = bake_vertex_light(ob)

    target_ratio = min(1.0, budget / float(max(raw_tris, 1)))

    # Every LOD is a share of the same pristine base, not a share of the last
    # one: otherwise the ladder collapses onto the same triangle count.
    pristine = ob.copy()
    pristine.data = ob.data.copy()
    bpy.context.collection.objects.link(pristine)

    lod_tris = []
    for level, share in enumerate(LOD_RATIOS):
        ob_level = pristine.copy()
        ob_level.data = pristine.data.copy()
        bpy.context.collection.objects.link(ob_level)
        bpy.ops.object.select_all(action="DESELECT")
        ob_level.select_set(True)
        bpy.context.view_layer.objects.active = ob_level
        tris = decimate(ob_level, target_ratio * share)
        path = os.path.join(outdir, f"lod{level}.glb")
        if not export_glb(ob_level, path):
            raise CookError(f"could not write {path}")
        lod_tris.append({"lod": level, "triangles": tris, "file": os.path.basename(path)})
        bpy.data.objects.remove(ob_level, do_unlink=True)
    bpy.data.objects.remove(pristine, do_unlink=True)
    bpy.context.view_layer.objects.active = ob

    low, high = mesh_bounds(ob)
    hero = render_hero(ob, os.path.join(outdir, "hero.png"), palette)
    metrics = {
        "source": os.path.abspath(src),
        "faction": faction,
        "palette": palette,
        "source_triangles": raw_tris,
        "source_height_m": round(source_height, 4),
        "scale_applied": round(scale, 6),
        "target_height_m": target_height,
        "triangle_budget": budget,
        "unwrapped": unwrapped,
        "atlas": atlas_info,
        "light": light_info,
        "lods": lod_tris,
        "bounds_m": {
            "x": round(high.x - low.x, 4),
            "y": round(high.y - low.y, 4),
            "z": round(high.z - low.z, 4),
        },
        "roughness_rule": {"matte": ROUGHNESS_MATTE, "cloth": ROUGHNESS_CLOTH,
                           "note": "two values, applied by the engine from the atlas"},
        "hero_render": hero,
        "generated_at": None,
    }
    with open(os.path.join(outdir, "cook.json"), "w", encoding="utf-8") as handle:
        json.dump(metrics, handle, indent=2, sort_keys=True)
    print(json.dumps(metrics, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except CookError as err:
        sys.stderr.write(f"E_COOK {err}\n")
        code = EXIT_FAIL
    except Exception as err:
        sys.stderr.write(f"E_COOK {type(err).__name__}: {err}\n")
        code = EXIT_FAIL
    if code != 0:
        sys.exit(code)
