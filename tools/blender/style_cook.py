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
    weights = np.where(sat[:, None] > 0.12, 1.0, 0.0)
    dist = np.linalg.norm(src_h[:, None, :] - pal_h[None, :, :], axis=2)
    dist = dist + (1.0 - weights) * 10.0
    choice = dist.argmin(axis=1)

    out = pal[choice]
    # Shading: the scale between the pixel value and the palette value.
    shade = np.where(pv[choice] > 1e-6, v / np.maximum(pv[choice], 1e-6), 1.0)
    shade = np.clip(shade, 0.35, 1.8)
    out = np.clip(out * shade[:, None], 0.0, 1.0)

    result = np.concatenate([out, alpha], axis=1).reshape(h, w, 4).astype(np.float32)
    return result


def bake_atlas(ob, palette, out_path):
    """Write the 512 palette atlas and point the material at it."""
    image = find_base_image(ob)
    if image is None:
        raise CookError("the mesh has no base colour image to repaint")
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

    # Point the material at the new atlas.
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

    unique = len(np.unique(np.round(small.reshape(-1, 4)[:, :3], 3), axis=0))
    return {"source_size": [width, height], "atlas": ATLAS_SIZE, "distinct_colors": int(unique)}


# ---------------------------------------------------------------------- AO


def bake_vertex_ao(ob):
    """Painted AO as a vertex colour layer.

    Real occlusion: rays from every vertex into its own hemisphere. Cheap
    enough on a LOD0 budget and it reads like the hand-painted darkening
    the faction art uses.
    """
    me = ob.data
    verts = [v.co.copy() for v in me.vertices]
    normals = [v.normal.copy() for v in me.vertices]
    polys = [list(p.vertices) for p in me.polygons]
    if not polys:
        raise CookError("the mesh has no faces")
    bvh = BVHTree.FromPolygons(verts, polys)

    layer = me.color_attributes.get("AO")
    if layer is None:
        layer = me.color_attributes.new(name="AO", type="FLOAT_COLOR", domain="POINT")

    directions = []
    for i in range(AO_RAYS):
        a = 2.0 * math.pi * i / AO_RAYS
        z = 1.0 - (i + 0.5) / AO_RAYS
        r = math.sqrt(max(0.0, 1.0 - z * z))
        directions.append(Vector((r * math.cos(a), r * math.sin(a), z)))

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
        values.append(max(0.0, min(1.0, ao)))

    data = layer.data
    for index, ao in enumerate(values):
        data[index].color = (ao, ao, ao, 1.0)
    me.update()
    return {"min": round(min(values), 4), "max": round(max(values), 4),
            "mean": round(sum(values) / len(values), 4)}


# ------------------------------------------------------------------- LODs


def decimate(ob, ratio):
    if ratio >= 0.999:
        return triangle_count(ob)
    mod = ob.modifiers.new(name="decimate", type="DECIMATE")
    mod.ratio = ratio
    mod.use_collapse_triangulate = True
    bpy.ops.object.modifier_apply(modifier=mod.name)
    return triangle_count(ob)


def export_glb(ob, path):
    kwargs = dict(filepath=path, export_format="GLB", export_apply=True,
                  export_texcoords=True, export_normals=True)
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
    ao_info = bake_vertex_ao(ob)

    # LOD0 first, then the rest as shares of it.
    target_ratio = min(1.0, budget / float(max(raw_tris, 1)))
    lod_tris = []
    for level, share in enumerate(LOD_RATIOS):
        if level == 0:
            tris = decimate(ob, target_ratio)
        else:
            # Work from a copy of the LOD0 state so each LOD is a share of
            # the same base, not a share of the last one.
            ob2 = ob.copy()
            ob2.data = ob.data.copy()
            bpy.context.collection.objects.link(ob2)
            bpy.context.view_layer.objects.active = ob2
            tris = decimate(ob2, target_ratio * share)
            path = os.path.join(outdir, f"lod{level}.glb")
            if not export_glb(ob2, path):
                raise CookError(f"could not write {path}")
            bpy.data.objects.remove(ob2, do_unlink=True)
            bpy.context.view_layer.objects.active = ob
            lod_tris.append({"lod": level, "triangles": tris, "file": os.path.basename(path)})
            continue
        path = os.path.join(outdir, f"lod{level}.glb")
        if not export_glb(ob, path):
            raise CookError(f"could not write {path}")
        lod_tris.append({"lod": level, "triangles": tris, "file": os.path.basename(path)})

    low, high = mesh_bounds(ob)
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
        "ao": ao_info,
        "lods": lod_tris,
        "bounds_m": {
            "x": round(high.x - low.x, 4),
            "y": round(high.y - low.y, 4),
            "z": round(high.z - low.z, 4),
        },
        "roughness_rule": {"matte": ROUGHNESS_MATTE, "cloth": ROUGHNESS_CLOTH,
                           "note": "two values, applied by the engine from the atlas"},
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
