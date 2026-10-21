"""The 30 m read test for any mesh: the gate a base has to pass.

Same picture and the same measurements as tools/blender/blockout.py, so a
generated base and a hand blockout can be compared number for number.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/mesh_silhouette.py -- <in.glb> <outdir>

Writes into <outdir>:
  silhouette_big.png   flat black on white, orthographic 3/4 front
  silhouette_30m.png   the same camera, a 30 m field, the figure about 90 px tall
  grey_studio.png      studio light, mid grey, 3/4 view
  silhouette.json      the measured silhouette

The gate: at 30 m a player reads the outline, the head, the stance and the
weapon. It does not read the face, the texture or the detail. A base that
fails here is a blob, and a blob dies here instead of costing a day.
"""

import json
import math
import os
import sys
import warnings

import bpy
from mathutils import Vector
from bpy_extras.object_utils import world_to_camera_view

try:
    import numpy as np
except ImportError:  # pragma: no cover - Blender ships numpy
    np = None

EXIT_USAGE = 2
EXIT_FAIL = 3

SIL_BIG = "silhouette_big.png"
SIL_30M = "silhouette_30m.png"
GREY_STUDIO = "grey_studio.png"

RES_BIG = (800, 1000)
TARGET_30M_PX = 90.0
FIELD_30M_M = 30.0
RES_30M_FALLBACK = (1280, 720)

CAM_AZ_DEG = -35.0
CAM_EL_DEG = 12.0
CAM_DIST = 6.0
MARGIN = 0.06
INK_SRGB = 0.5
TARGET_HEIGHT_M = 2.0


class SilhouetteError(Exception):
    """A silhouette that could not be measured."""


def srgb(linear):
    if linear <= 0.0031308:
        return linear * 12.92
    return 1.055 * (linear ** (1.0 / 2.4)) - 0.055


def read_pixels(path):
    image = bpy.data.images.load(path, check_existing=False)
    width, height = image.size
    if np is not None:
        buf = np.empty(width * height * 4, dtype=np.float32)
        image.pixels.foreach_get(buf)
        return buf.reshape(height, width, 4), width, height, image
    flat = list(image.pixels)
    rows = [[flat[(y * width + x) * 4: (y * width + x) * 4 + 4] for x in range(width)]
            for y in range(height)]
    return rows, width, height, image


def ink_mask(pixels, width, height):
    if np is not None:
        alpha = pixels[:, :, 3]
        if alpha[0, 0] < 0.5:
            return alpha > 0.5
        lum = pixels[:, :, :3].mean(axis=2)
        mask = np.empty_like(lum, dtype=bool)
        for i in range(lum.shape[0]):
            for j in range(lum.shape[1]):
                mask[i, j] = srgb(float(lum[i, j])) < INK_SRGB
        return mask
    alpha_bg = pixels[0][0][3] < 0.5
    return [[(px[3] > 0.5) if alpha_bg else (srgb((px[0] + px[1] + px[2]) / 3.0) < INK_SRGB)
             for px in row] for row in pixels]


def measure(mask, width, height):
    if np is not None:
        rows = np.where(mask.any(axis=1))[0]
        cols = np.where(mask.any(axis=0))[0]
        if rows.size == 0:
            raise SilhouetteError("the silhouette render is empty")
        r0, r1 = int(rows[0]), int(rows[-1])
        c0, c1 = int(cols[0]), int(cols[-1])
        box_w = c1 - c0 + 1
        box_h = r1 - r0 + 1
        fill = float(mask.sum()) / float(box_w * box_h)
        return {"x0": c0, "x1": c1, "y0": r0, "y1": r1,
                "width_px": box_w, "height_px": box_h, "fill": fill}
    rows = [y for y in range(height) if any(mask[y][x] for x in range(width))]
    cols = [x for x in range(width) if any(mask[y][x] for y in range(height))]
    if not rows:
        raise SilhouetteError("the silhouette render is empty")
    box_w = cols[-1] - cols[0] + 1
    box_h = rows[-1] - rows[0] + 1
    ink = sum(1 for y in range(height) for x in range(width) if mask[y][x])
    return {"x0": cols[0], "x1": cols[-1], "y0": rows[0], "y1": rows[-1],
            "width_px": box_w, "height_px": box_h, "fill": ink / float(box_w * box_h)}


def corner_is_white(pixels, width, height):
    if np is not None:
        if float(pixels[0, 0, 3]) < 0.5:
            return False
        return srgb(float(pixels[0, 0, :3].mean())) > 0.92
    px = pixels[0][0]
    if px[3] < 0.5:
        return False
    return srgb((px[0] + px[1] + px[2]) / 3.0) > 0.92


def composite_over_white(path):
    pixels, width, height, image = read_pixels(path)
    if np is not None:
        flat = pixels.reshape(-1, 4)
        alpha = flat[:, 3:4]
        rgb = flat[:, :3] * alpha + (1.0 - alpha)
        out = np.concatenate([rgb, np.ones_like(alpha)], axis=1).reshape(-1)
        image.pixels.foreach_set(out.astype(np.float32))
    else:
        out = []
        for row in pixels:
            for px in row:
                a = px[3]
                out.extend([px[0] * a + (1.0 - a), px[1] * a + (1.0 - a),
                            px[2] * a + (1.0 - a), 1.0])
        image.pixels[:] = out
    image.file_format = "PNG"
    image.filepath_raw = path
    image.save()
    bpy.data.images.remove(image)


def make_world(scene, color):
    scene.display.shading.background_type = "WORLD"
    world = bpy.data.worlds.new("sil_world")
    scene.world = world
    world.color = (color[0], color[1], color[2])
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            if getattr(world, "node_tree", None) is None and hasattr(world, "use_nodes"):
                world.use_nodes = True
        tree = getattr(world, "node_tree", None)
        if tree:
            node = tree.nodes.get("Background")
            if node:
                node.inputs[0].default_value = (color[0], color[1], color[2], 1.0)
    except Exception:
        pass
    return world


def world_bbox(objects):
    low = Vector((1e9, 1e9, 1e9))
    high = Vector((-1e9, -1e9, -1e9))
    for ob in objects:
        if ob.type != "MESH":
            continue
        matrix = ob.matrix_world
        for corner in ob.bound_box:
            point = matrix @ Vector(corner)
            for axis in range(3):
                low[axis] = min(low[axis], point[axis])
                high[axis] = max(high[axis], point[axis])
    return low, high


def view_extents(ortho_scale, resolution):
    width, height = resolution
    if width >= height:
        return ortho_scale, ortho_scale * height / width
    return ortho_scale * width / height, ortho_scale


def projected_span(scene, cam, corners):
    us, vs = [], []
    for point in corners:
        co = world_to_camera_view(scene, cam, point)
        us.append(co.x)
        vs.append(co.y)
    return min(us), max(us), min(vs), max(vs)


def frame_camera(scene, cam, target_obj, corners, margin=MARGIN):
    data = cam.data
    resolution = (scene.render.resolution_x, scene.render.resolution_y)
    data.ortho_scale = 1.0
    bpy.context.view_layer.update()
    u0, u1, v0, v1 = projected_span(scene, cam, corners)
    du, dv = u1 - u0, v1 - v0
    usable = 1.0 - 2.0 * margin
    data.ortho_scale = max(du / usable, dv / usable)
    bpy.context.view_layer.update()
    uc, vc = (u0 + u1) * 0.5, (v0 + v1) * 0.5
    view_w, view_h = view_extents(data.ortho_scale, resolution)
    basis = cam.matrix_world.to_3x3()
    shift = basis @ Vector((1.0, 0.0, 0.0)) * ((uc - 0.5) * view_w)
    shift += basis @ Vector((0.0, 1.0, 0.0)) * ((vc - 0.5) * view_h)
    cam.location = cam.location + shift
    target_obj.location = target_obj.location + shift
    bpy.context.view_layer.update()
    return data.ortho_scale


def render_to(scene, cam, path, resolution, ortho_scale):
    scene.render.resolution_x, scene.render.resolution_y = resolution
    scene.render.resolution_percentage = 100
    cam.data.ortho_scale = ortho_scale
    scene.render.filepath = path
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    bpy.ops.render.render(write_still=True)
    if not os.path.exists(path) or os.path.getsize(path) == 0:
        raise SilhouetteError(f"render wrote nothing: {path}")


def render_silhouette(scene, cam, path, resolution, ortho_scale):
    scene.display.shading.light = "FLAT"
    scene.display.shading.color_type = "SINGLE"
    scene.display.shading.single_color = (0.0, 0.0, 0.0)
    scene.display.shading.show_shadows = False
    scene.display.shading.show_cavity = False
    scene.render.film_transparent = False
    make_world(scene, (1.0, 1.0, 1.0))
    render_to(scene, cam, path, resolution, ortho_scale)

    pixels, width, height, image = read_pixels(path)
    bpy.data.images.remove(image)
    if corner_is_white(pixels, width, height):
        mode = "world_white"
    else:
        scene.render.film_transparent = True
        render_to(scene, cam, path, resolution, ortho_scale)
        pixels, width, height, image = read_pixels(path)
        bpy.data.images.remove(image)
        composite_over_white(path)
        mode = "film_transparent_flattened"

    pixels, width, height, image = read_pixels(path)
    bpy.data.images.remove(image)
    return mode, measure(ink_mask(pixels, width, height), width, height)


def render_studio(scene, cam, path, resolution, ortho_scale):
    scene.display.shading.light = "STUDIO"
    scene.display.shading.color_type = "MATERIAL"
    scene.display.shading.show_shadows = True
    scene.display.shading.shadow_intensity = 0.35
    scene.display.shading.show_cavity = True
    try:
        scene.display.shading.cavity_type = "SCREEN"
    except Exception:
        pass
    scene.render.film_transparent = False
    make_world(scene, (0.18, 0.18, 0.18))
    render_to(scene, cam, path, resolution, ortho_scale)


def prepare(src):
    """Import the mesh and stand it at the canonical height."""
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.display.render_aa = "8"
    scene.render.resolution_x, scene.render.resolution_y = RES_BIG
    try:
        scene.view_settings.view_transform = "Standard"
    except Exception:
        pass

    bpy.ops.import_scene.gltf(filepath=src)
    meshes = [o for o in scene.objects if o.type == "MESH"]
    if not meshes:
        raise SilhouetteError(f"no mesh in {src}")
    bpy.ops.object.select_all(action="DESELECT")
    for ob in meshes:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    if len(meshes) > 1:
        bpy.ops.object.join()
    ob = bpy.context.view_layer.objects.active
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)

    low, high = world_bbox([ob])
    height = high.z - low.z
    if height <= 1e-6:
        raise SilhouetteError("the mesh has no height")
    scale = TARGET_HEIGHT_M / height
    ob.scale = (scale, scale, scale)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    ob.location.z -= low.z * scale
    bpy.ops.object.transform_apply(location=True, rotation=False, scale=False)
    return scene, ob, height, scale


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(args) < 2:
        sys.stderr.write("usage: blender -b --factory-startup --python-exit-code 1 "
                         "-P mesh_silhouette.py -- <in.glb> <outdir>\n")
        return EXIT_USAGE
    src, outdir = args[0], args[1]
    os.makedirs(outdir, exist_ok=True)

    scene, ob, source_height, scale = prepare(src)

    low, high = world_bbox([ob])
    target = bpy.data.objects.new("target", None)
    bpy.context.collection.objects.link(target)
    cam_data = bpy.data.cameras.new("cam")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = 1.0
    cam_data.clip_start = 0.05
    cam_data.clip_end = 200.0
    cam = bpy.data.objects.new("cam", cam_data)
    bpy.context.collection.objects.link(cam)

    center = (low + high) * 0.5
    target.location = center
    az = math.radians(CAM_AZ_DEG)
    el = math.radians(CAM_EL_DEG)
    direction = Vector((math.sin(az) * math.cos(el), -math.cos(az) * math.cos(el), math.sin(el)))
    cam.location = center + direction * CAM_DIST
    con = cam.constraints.new("TRACK_TO")
    con.target = target
    con.track_axis = "TRACK_NEGATIVE_Z"
    con.up_axis = "UP_Y"
    scene.camera = cam
    bpy.context.view_layer.update()

    corners = []
    for corner in ob.bound_box:
        corners.append(ob.matrix_world @ Vector(corner))
    ortho_scale = frame_camera(scene, cam, target, corners)

    big_path = os.path.join(outdir, SIL_BIG)
    mode, big = render_silhouette(scene, cam, big_path, RES_BIG, ortho_scale)

    view_h_big = ortho_scale
    projected_h = big["height_px"] * view_h_big / float(RES_BIG[1])
    res_x = int(round(TARGET_30M_PX * FIELD_30M_M / max(projected_h, 1e-6) / 2.0)) * 2
    res_x = max(640, min(res_x, 2560))
    res_y = int(round(res_x * 9.0 / 16.0 / 2.0)) * 2
    if res_y >= res_x:
        res_x, res_y = RES_30M_FALLBACK
    path_30m = os.path.join(outdir, SIL_30M)
    mode_30m, small = render_silhouette(scene, cam, path_30m, (res_x, res_y), FIELD_30M_M)

    render_studio(scene, cam, os.path.join(outdir, GREY_STUDIO), RES_BIG, ortho_scale)

    ob.data.calc_loop_triangles()
    metrics = {
        "source": os.path.abspath(src),
        "source_height_m": round(source_height, 4),
        "scale_applied": round(scale, 6),
        "triangles": len(ob.data.loop_triangles),
        "height_px": round(big["height_px"], 2),
        "width_px": big["width_px"],
        "bbox_fill": round(big["fill"], 4),
        "figure_height_m": round(projected_h, 4),
        "px_per_m": round(RES_BIG[1] / view_h_big, 2),
        "ortho_scale_m": round(ortho_scale, 4),
        "height_30m_px": round(small["height_px"], 2),
        "fill_30m": round(small["fill"], 4),
        "resolution_30m": [res_x, res_y],
        "field_width_m_30m": FIELD_30M_M,
        "background": mode,
        "background_30m": mode_30m,
    }
    with open(os.path.join(outdir, "silhouette.json"), "w", encoding="utf-8") as handle:
        json.dump(metrics, handle, indent=2, sort_keys=True)
    print(json.dumps(metrics, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except SilhouetteError as err:
        sys.stderr.write(f"SILHOUETTE {err}\n")
        code = EXIT_FAIL
    except Exception as err:
        sys.stderr.write(f"SILHOUETTE {type(err).__name__}: {err}\n")
        code = EXIT_FAIL
    if code != 0:
        sys.exit(code)
