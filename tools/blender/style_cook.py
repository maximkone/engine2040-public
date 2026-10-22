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
# The skin is not the faction. A face in black iron is a mask, not a face.
# Each faction has its own skin and its own hair, and the skin must not
# be the same brown as the faction's bronze cloth.
FACTION_SKIN = {
    "ashkin": "#6E5A4C",
    "wardens": "#C29A7C",
}
FACTION_HAIR = {
    "ashkin": "#1A1715",
    "wardens": "#2A3A4A",
}
# The palette entry the cloth takes. The Ashkin are black iron; the
# Wardens are bone and steel, and their blue is their own.
FACTION_DOMINANT = {
    "ashkin": 0,
    "wardens": 3,
}
CURRENT_SKIN = FACTION_SKIN["ashkin"]
CURRENT_HAIR = FACTION_HAIR["ashkin"]
CURRENT_DOMINANT = FACTION_DOMINANT["ashkin"]
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

    # The names the file carries are kept, and so are the faces' own
    # material indices. The cook used to rebuild the material list and
    # repaint every face onto the first material, so a file that already
    # carried a skin and a cloth came out with the whole figure in flesh.
    #
    # A material with no name is the skin: MPFB's skin is procedural, so
    # it is the one material that arrives empty, and that is the one case
    # the cook cannot get wrong.
    keep = meshes[0]
    keep.name = "body"
    for source in meshes:
        if not source.material_slots:
            material = bpy.data.materials.new(name="skin")
            material.use_nodes = True
            source.data.materials.append(material)
            for poly in source.data.polygons:
                poly.material_index = 0
            continue
        for slot in source.material_slots:
            if slot.material is None:
                material = bpy.data.materials.new(name="skin")
                material.use_nodes = True
                slot.material = material

    # The rest of the objects come across, each with its own material.
    # The join does not keep the material index of the faces that come
    # across: they land on the survivor's first slot, which is the skin's,
    # and the whole figure comes out in flesh. So the faces are counted
    # before the join, and the ones that came across are repainted after
    # it, onto the material they had.
    for source in meshes[1:]:
        material = None
        for slot in source.material_slots:
            if slot.material is not None:
                material = slot.material
                break
        if material is None:
            continue
        offset = len(keep.data.materials)
        keep.data.materials.append(material)
        source.data.materials.clear()
        source.data.materials.append(material)
        for poly in source.data.polygons:
            poly.material_index = 0
        faces_before = len(keep.data.polygons)
        bpy.ops.object.select_all(action="DESELECT")
        keep.select_set(True)
        source.select_set(True)
        bpy.context.view_layer.objects.active = keep
        bpy.ops.object.join()
        keep = bpy.context.view_layer.objects.active
        keep.name = "body"
        for index, poly in enumerate(keep.data.polygons):
            if index >= faces_before:
                poly.material_index = offset

    bpy.ops.object.select_all(action="DESELECT")
    keep.select_set(True)
    bpy.context.view_layer.objects.active = keep
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
    # A hue the palette does not have must not fall back to the nearest hue:
    # a blue shirt had no match, fell to bone, and dressed a black iron
    # brute in white. So the garment is painted in the faction's own colour,
    # and an accent colour is allowed only where the source's hue really
    # matches one. The light and shadow stay the picture's.
    # A hue the palette does not have must not fall back to the nearest
    # hue: a blue shirt had no match, fell to bone, and dressed a black
    # iron brute in white. So the garment is painted in the faction's own
    # colour, and an accent colour is allowed only where the source's hue
    # really matches one. The light and shadow stay the picture's.
    choice = dist.argmin(axis=1)
    dominant = CURRENT_DOMINANT
    if len(palette) > 1:
        accents = [i for i in range(len(palette)) if i != dominant]
        matched = dist[:, accents].min(axis=1)
        keep = (matched < 0.45) & (sat > 0.25)
        choice = np.where(keep, choice, dominant)
        # The largest area of the garment wears the faction's colour.
        # A beige apron was matching bone and dressing the Wardens in
        # white aprons; the rule is spatial, not per pixel: the big cloth
        # is the faction's, and the small panels keep their own hue.
        counts = np.bincount(choice, minlength=len(palette))
        if counts.argmax() != dominant and counts[dominant] < counts.max():
            biggest = int(counts.argmax())
            choice = np.where(choice == biggest, dominant, choice)
    out = pal[choice]
    # The shading is the source's own variation around its own mean, not its
    # absolute brightness. A painter takes the hue from the palette, the
    # local light and shadow from the picture, and the value from the
    # palette: black iron is always dark and bone is always light. Scaling by
    # the source's absolute value instead sent every bright texture to
    # white, because bone times 1.3 is white.
    mean = float(v.mean()) if v.size else 1.0
    mean = max(mean, 1e-4)
    shade = 0.62 + 0.76 * (v / mean)
    shade = np.clip(shade, 0.35, 1.90)
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
            if not base:
                return False
            if base.links:
                source = base.links[0].from_socket
                constant = None
            else:
                # A flat colour still has to take the painted light, or the
                # skin is a flat orange cut-out.
                source = None
                constant = tuple(base.default_value)

            attribute_node = tree.nodes.new("ShaderNodeVertexColor")
            attribute_node.layer_name = attribute
            attribute_node.location = (source.node.location.x - 400, source.node.location.y - 220) if source else (-600, -220)
            mix = tree.nodes.new("ShaderNodeMix")
            mix.data_type = "RGBA"
            mix.blend_type = "MULTIPLY"
            mix.location = (source.node.location.x - 180, source.node.location.y - 100) if source else (-380, -100)
            mix.inputs[0].default_value = 1.0
            if source is not None:
                tree.links.new(source, mix.inputs[6])
            else:
                mix.inputs[6].default_value = constant
            tree.links.new(attribute_node.outputs["Color"], mix.inputs[7])
            tree.links.new(mix.outputs[2], base)
            return True
    except Exception:
        return False


def slug(name):
    safe = "".join(ch if ch.isalnum() or ch in "-_" else "_" for ch in (name or "map"))
    return safe[:24] or "map"


def write_atlas(image, palette, out_path):
    """Map one texture into the palette and write the atlas."""
    width, height = image.size
    buf = np.empty(width * height * 4, dtype=np.float32)
    image.pixels.foreach_get(buf)
    pixels = buf.reshape(height, width, 4)
    painted = nearest_palette(pixels, palette)
    atlas = bpy.data.images.new("atlas", width=ATLAS_SIZE, height=ATLAS_SIZE, alpha=True)
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
    unique = len(np.unique(np.round(small.reshape(-1, 4)[:, :3], 3), axis=0))
    return {"source_size": [width, height], "atlas": ATLAS_SIZE, "distinct_colors": int(unique)}


def fit_skin_under_cloth(ob, amount=0.035):
    """Shrink the skin, so the garment always covers it.

    The garment is fitted to the body, so the two surfaces are a hair
    apart, and the body shows through the cloth as patches of flesh.
    After the decimation the two surfaces move about, and the body pokes
    through in more places.

    The skin is pushed in towards the body's own axis, a little, and
    nothing else. It is not moved onto the garment: the first version
    did that with a ray from above, and the skin that the garment does
    not cover - the arms, the legs, the face - was dragged into long
    vertical ribbons, because the ray found the garment's surface far
    above the vertex and the vertex was moved there.

    The push is measured per vertex: the closer the vertex is to the
    body's centre line, the more it can give way, so the arms and the
    legs keep their shape and the torso gives way to the cloth.
    """
    me = ob.data
    if not me.vertices:
        return 0

    skin_slots = set()
    for index, slot in enumerate(ob.material_slots):
        material = slot.material
        if material is None:
            continue
        if "skin" in (material.name or "").lower():
            skin_slots.add(index)
    if not skin_slots:
        return 0

    xs = [v.co.x for v in me.vertices]
    centre = (min(xs) + max(xs)) * 0.5
    span = max(max(xs) - min(xs), 1e-6)

    moved = 0
    for poly in me.polygons:
        if poly.material_index not in skin_slots:
            continue
        for index in poly.vertices:
            co = me.vertices[index].co
            # How far the vertex is from the body's centre line, as a
            # share of the body's width. A vertex on the centre line can
            # give way the most; one out at the arm cannot give way at
            # all.
            away = min(1.0, abs(co.x - centre) / (span * 0.5))
            give = amount * (1.0 - away * away)
            if give <= 0.0:
                continue
            me.vertices[index].co = Vector((co.x - (co.x - centre) * give,
                                           co.y, co.z))
            moved += 1
    me.update()
    return moved


def split_hair(ob):
    """Give the hair its own material.

    MakeHuman's basemesh is one mesh: skin, scalp and hair are one surface
    with one material, so the cook paints the whole head in flesh and the
    hair reads as a bald cap.

    The head is a connected surface of its own. MPFB builds the body from
    parts, and the parts do not share vertices, so the head can be found
    as the component that holds the top of the figure. The split moves
    the faces of that component and leaves every other face where it is:
    the garment's collar reaches above the brow line on a short figure,
    and a split by height alone turns the collar into hair.

    Only the skin is split. The cloth keeps its own material, or a
    shoulder pad turns into hair.

    The hair is the top of the head: the crown, and the sides down to
    the temples. The split is a share of the head's own surface, not a
    height on the body, so it does not depend on where the head sits.
    The first version used the head's own median height, and it moved
    two faces: the median of a skull is in the middle of the skull, and
    nearly every face of the head is below that, because the face hangs
    down from the crown. So the whole head stayed skin.
    """
    me = ob.data
    count = len(me.vertices)
    if not count:
        return False

    # Only the skin is split. The cloth keeps its own material, or a
    # collar that reaches the brow line turns into hair.
    skin_slots = set()
    for index, slot in enumerate(ob.material_slots):
        material = slot.material
        if material is not None and "skin" in (material.name or "").lower():
            skin_slots.add(index)
    if not skin_slots:
        return False

    # Adjacency, by shared edges, over the skin's faces only.
    adjacency = [[] for _ in range(count)]
    for poly in me.polygons:
        if poly.material_index not in skin_slots:
            continue
        vs = list(poly.vertices)
        for i in range(len(vs)):
            for j in range(i + 1, len(vs)):
                adjacency[vs[i]].append(vs[j])
                adjacency[vs[j]].append(vs[i])

    # The head is the piece that reaches highest and is a head's worth
    # of figure. The skin's highest vertex belongs to a scrap of
    # fourteen vertices, and the head is below it, so the search cannot
    # start at the top vertex.
    zs = [v.co.z for v in me.vertices]
    z_min, z_max = min(zs), max(zs)
    figure_height = max(z_max - z_min, 1e-6)
    crown = z_min + figure_height * 0.80

    seen = set()
    pieces = []
    for start_vertex in range(count):
        if start_vertex in seen:
            continue
        stack = [start_vertex]
        seen.add(start_vertex)
        piece = []
        while stack:
            v = stack.pop()
            piece.append(v)
            for w in adjacency[v]:
                if w not in seen:
                    seen.add(w)
                    stack.append(w)
        pieces.append(piece)

    # The head is the biggest piece that reaches the crown.
    head = []
    for piece in pieces:
        if len(piece) > count * 0.25:
            # A piece that is most of the figure is not a head.
            continue
        piece_zs = [me.vertices[v].co.z for v in piece]
        if max(piece_zs) < crown:
            continue
        if len(piece) > len(head):
            head = piece
    if not head:
        return False
    head_set = set(head)

    # The head's own faces, and the height of each.
    head_faces = []
    for index, poly in enumerate(me.polygons):
        if poly.material_index not in skin_slots:
            continue
        verts = [me.vertices[v] for v in poly.vertices]
        if not verts:
            continue
        on_head = sum(1 for v in verts if v.index in head_set)
        if on_head < len(verts) * 0.6:
            continue
        head_faces.append((index, min(v.co.z for v in verts)))
    if len(head_faces) < 20:
        return False

    # The hair is the top of the head's own faces. The face of the head
    # hangs down from the crown, so the split is a share of the head's
    # surface and not a height.
    head_faces.sort(key=lambda item: item[1])
    cut = int(len(head_faces) * 0.58)
    hair_faces = {index for index, _ in head_faces[cut:]}

    hair = bpy.data.materials.new(name="hair")
    hair.use_nodes = True
    slot = me.materials.find(hair.name)
    if slot < 0:
        me.materials.append(hair)
        slot = len(me.materials) - 1

    moved = 0
    for index in hair_faces:
        me.polygons[index].material_index = slot
        moved += 1
    return moved > 0


def bake_atlas(ob, palette, out_dir):
    """One palette atlas per material, and the skin is not the faction.

    A body can carry several materials: the skin and the clothes. Painting
    them all with one atlas is what put a white face on an armoured brute,
    because the atlas was built from whichever material came first. So each
    material gets its own atlas, made from its own texture, and the skin
    keeps a skin tone instead of a faction colour.
    """
    written = []
    for index, slot in enumerate(ob.material_slots):
        material = slot.material
        if material is None:
            # A slot with no material: the exporter dropped the skin
            # because MPFB's skin material is procedural. Give it one, in
            # skin, so the face is a face and not a hole.
            material = bpy.data.materials.new(name=f"skin{index}")
            material.use_nodes = True
            slot.material = material
        node = None
        image = None
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                tree = getattr(material, "node_tree", None)
                if tree:
                    # The base colour is the one that matters. A material
                    # carries a normal map and an AO map too, and repainting
                    # the first image node found repaints the normal map and
                    # leaves the base colour white.
                    bsdf = tree.nodes.get("Principled BSDF")
                    base = bsdf.inputs.get("Base Color") if bsdf else None
                    if base and base.links:
                        source = base.links[0].from_node
                        if source.type == "TEX_IMAGE" and source.image:
                            node = source
                            image = source.image
                    if image is None:
                        for candidate in tree.nodes:
                            if candidate.type == "TEX_IMAGE" and candidate.image:
                                node = candidate
                                image = candidate.image
                                break
        except Exception:
            continue
        if image is None or node is None:
            # No base colour image: the material is a solid colour, and a
            # solid white face is a mask. Give it the skin tone if it is
            # skin, the palette's first entry if it is anything else.
            name = (material.name or "").lower()
            is_skin = "skin" in name
            is_hair = "hair" in name
            if is_hair:
                colour = hex_linear(CURRENT_HAIR)
            elif is_skin:
                colour = hex_linear(CURRENT_SKIN)
            else:
                colour = hex_linear(palette[CURRENT_DOMINANT])
            try:
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", DeprecationWarning)
                    tree = getattr(material, "node_tree", None)
                    bsdf = tree.nodes.get("Principled BSDF") if tree else None
                    if bsdf:
                        base = bsdf.inputs.get("Base Color")
                        if base:
                            base.default_value = (colour[0], colour[1], colour[2], 1.0)
                            # A flat colour still takes the painted light.
                            wire_vertex_light(material)
                            written.append({"material": material.name, "image": "(solid)",
                                            "atlas": "(solid)", "skin": bool(is_skin),
                                            "source_size": [0, 0], "atlas_size": 0,
                                            "distinct_colors": 1})
            except Exception:
                pass
            continue
        name = (material.name or "").lower()
        image_name = (image.name or "").lower()
        is_skin = "skin" in name or "skin" in image_name
        is_hair = "hair" in name
        if is_hair:
            entry = [CURRENT_HAIR]
        elif is_skin:
            entry = [CURRENT_SKIN]
        else:
            entry = palette
        path = os.path.join(out_dir, f"atlas_{index}_{slug(image.name)}.png")
        info = write_atlas(image, entry, path)
        node.image = bpy.data.images.load(path, check_existing=True)
        wire_vertex_light(material)
        written.append({"material": material.name, "image": image.name,
                        "atlas": os.path.basename(path), "skin": bool(is_skin), **info})
    if not written:
        path = os.path.join(out_dir, "atlas.png")
        info = write_synthetic_atlas(palette, path)
        written.append({"material": "(none)", "image": "(none)", "atlas": "atlas.png",
                        "skin": False, **info})
    return written



def drop_normal_maps(ob):
    """Take the normal maps out of the export.

    Our look is painted: the atlas carries the colour and the LIGHT layer
    carries the light. A normal map adds nothing the engine can use, and
    the suit's normal map is 4K, so every LOD carried 7 MB of a texture
    nobody looks at. Four LODs of 1,100 triangles each should be kilobytes.
    """
    dropped = 0
    for slot in ob.material_slots:
        material = slot.material
        if material is None:
            continue
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                tree = getattr(material, "node_tree", None)
                if not tree:
                    continue
                for node in list(tree.nodes):
                    if node.type == "NORMAL_MAP":
                        tree.nodes.remove(node)
                        dropped += 1
                    elif node.type == "TEX_IMAGE":
                        # An image that is not the base colour is a normal
                        # map or an AO map: the engine has no slot for it.
                        pass
        except Exception:
            continue
    return dropped


def write_synthetic_atlas(palette, out_path):
    """No texture to repaint: a flat palette sheet with a vertical ramp."""
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
    return {"source_size": [0, 0], "atlas": ATLAS_SIZE, "distinct_colors": ATLAS_SIZE,
            "synthetic": True}


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


def separate_parts(ob):
    """The mesh's connected parts, each on its own.

    MPFB builds the body from parts, and the parts do not share vertices,
    so a loose separation gives the head, the hands, the boots, the limbs
    and the garment's panels back as they were.

    The separation runs on a copy, because Blender's separate operator
    frees the object it was made from: the mesh moves onto the new
    objects and the old one is removed. The figure the cook is holding,
    and the pristine base every LOD is copied from, both died at the
    first separation that way.

    The work copy is one of the parts: the separation leaves it holding
    the last part it split off, and leaving it out loses the garment.
    """
    work = ob.copy()
    work.data = ob.data.copy()
    bpy.context.collection.objects.link(work)
    work.name = "parts_work"
    bpy.ops.object.select_all(action="DESELECT")
    work.select_set(True)
    bpy.context.view_layer.objects.active = work
    bpy.ops.mesh.separate(type="LOOSE")
    # The parts are found by name: "every mesh that is not the work
    # copy" would catch the figure the cook is holding and the pristine
    # base as well.
    return [o for o in bpy.context.scene.objects
            if o.type == "MESH" and o.name.startswith("parts_work")]


def part_centre(part):
    """Where a part sits, so the small ones can find their neighbour."""
    verts = part.data.vertices
    if not verts:
        return (0.0, 0.0, 0.0)
    count = len(verts)
    return (sum(v.co.x for v in verts) / count,
            sum(v.co.y for v in verts) / count,
            sum(v.co.z for v in verts) / count)


def part_role(part, figure, cloth_names):
    """What a part is, read off its own shape.

    The rig's vertex groups do not survive the export, so the parts are
    told apart by where they are and how big they are.
    """
    me = part.data
    if not me.vertices:
        return "body"
    zs = [v.co.z for v in me.vertices]
    xs = [v.co.x for v in me.vertices]
    z_min, z_max = min(zs), max(zs)
    x_min, x_max = min(xs), max(xs)
    height = max(figure["z_max"] - figure["z_min"], 1e-6)
    width = max(figure["x_max"] - figure["x_min"], 1e-6)

    # A part of the garment is cloth, and the cloth is judged on its own.
    for index in {p.material_index for p in me.polygons}:
        if index < len(part.material_slots):
            material = part.material_slots[index].material
            if material is not None and (material.name or "").lower() in cloth_names:
                return "cloth"

    if z_max >= figure["z_max"] - height * 0.02:
        return "head"
    if z_min <= figure["z_min"] + height * 0.04:
        return "foot"
    centre = (x_min + x_max) * 0.5
    if (z_max - z_min) < height * 0.13 and abs(centre) > width * 0.22:
        return "hand"
    return "body"


# What each part is worth, per face, when the triangle budget is spent.
# The head and the hands are what a player reads at thirty metres, so they
# get the most; the body and the garment fill in.
ROLE_WEIGHT = {"head": 2.5, "hand": 2.0, "foot": 1.5, "body": 1.0, "cloth": 1.2}

# A part smaller than this is a scrap, not a part of the body. MPFB's
# basemesh is full of them.
SCRAP_FACES = 100


def fold_scraps(parts):
    """Fold the small parts into their nearest big neighbour.

    MPFB's basemesh is 443 loose pieces, most of them two-face scraps.
    Budgeting the triangle count across 443 parts gave every scrap four
    faces, and the scraps ate the whole budget while the head and the
    hands were starved. So the scraps go into the big part nearest to
    them, and the budget is spent on the parts a player reads.
    """
    big = [p for p in parts if len(p.data.polygons) >= SCRAP_FACES]
    scraps = [p for p in parts if len(p.data.polygons) < SCRAP_FACES]
    if not big or not scraps:
        return big or parts

    centres = {p.name: part_centre(p) for p in big}
    for scrap in scraps:
        centre = part_centre(scrap)
        nearest = min(big, key=lambda p: sum(
            (a - b) ** 2 for a, b in zip(centres[p.name], centre)))
        bpy.ops.object.select_all(action="DESELECT")
        nearest.select_set(True)
        scrap.select_set(True)
        bpy.context.view_layer.objects.active = nearest
        bpy.ops.object.join()
        centres[nearest.name] = part_centre(nearest)
    return big


def decimate_by_part(ob, budget, share=1.0):
    """Spend the triangle budget on the parts, where they are read from.

    One collapse from 44k faces to a thousand shatters the mesh: the hands
    come apart, holes open where the garment meets the body, and a limb
    comes loose and hangs between the feet. So each part is decimated on
    its own, in steps of at most a half, and the budget is shared out by
    what the part is: the head, the hands and the boots keep their shape,
    the body and the garment give way.

    The live object is handed back: the separation frees the object it was
    made from, so the caller's handle on it is dead by the time the parts
    are put back together.
    """
    faces_budget = max(4, int(budget * share))

    # The figure's own bounds, read before the separation frees the mesh.
    zs = [v.co.z for v in ob.data.vertices]
    xs = [v.co.x for v in ob.data.vertices]
    figure = {"z_min": min(zs), "z_max": max(zs),
              "x_min": min(xs), "x_max": max(xs)}

    parts = separate_parts(ob)
    if len(parts) <= 1:
        tris = decimate(ob, min(1.0, faces_budget / max(triangle_count(ob), 1)))
        return tris, ob

    cloth_names = set()
    for slot in ob.material_slots:
        material = slot.material
        if material is not None and "cloth" in (material.name or "").lower():
            cloth_names.add((material.name or "").lower())

    parts = fold_scraps(parts)
    parts = [p for p in parts if p.name in bpy.data.objects]
    roles = [part_role(part, figure, cloth_names) for part in parts]
    weights = [ROLE_WEIGHT.get(role, 1.0) * max(len(part.data.polygons), 1)
               for part, role in zip(parts, roles)]
    total_weight = sum(weights) or 1.0

    for part, weight in zip(parts, weights):
        faces = len(part.data.polygons)
        target = max(4, int(faces_budget * weight / total_weight))
        target = min(target, faces)
        decimate(part, target / float(max(faces, 1)))

    # The parts go back together, and the faces keep their material. A
    # joined object is freed, so the live one is asked for after every
    # join: the list of parts would otherwise hold dead references.
    #
    # The material list is built by hand. The join merges the lists, and
    # a part's faces keep their own index, so a part with a material the
    # survivor does not have - the head's hair - loses it: the head's
    # faces land on the survivor's skin and the hair is gone.
    keep = parts[0]
    for part in parts[1:]:
        # Where each of the part's materials lands in the survivor's list.
        remap = {}
        for index, slot in enumerate(part.material_slots):
            material = slot.material
            if material is None:
                continue
            target = None
            for i, existing in enumerate(keep.data.materials):
                if existing is material or existing.name == material.name:
                    target = i
                    break
            if target is None:
                keep.data.materials.append(material)
                target = len(keep.data.materials) - 1
            remap[index] = target
        for poly in part.data.polygons:
            if poly.material_index in remap:
                poly.material_index = remap[poly.material_index]
        bpy.ops.object.select_all(action="DESELECT")
        keep.select_set(True)
        part.select_set(True)
        bpy.context.view_layer.objects.active = keep
        bpy.ops.object.join()
        keep = bpy.context.view_layer.objects.active
    # A name of its own: Blender frees an object when a new one takes its
    # name, and this part was called "body" like the figure the cook is
    # holding, so the figure died here and the hero render had nothing
    # to draw.
    keep.name = "lod_part"
    bpy.context.view_layer.objects.active = keep
    return triangle_count(keep), keep


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
    # The part has to be the active object: Blender's modifier_apply
    # works on the active object, and the active object was whatever the
    # last join left, so the parts were barely decimated at all.
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    while current > target:
        # Halve while there is far to go, then take the last step exactly.
        # A floor on the step made one collapse do all the work: a single
        # collapse from 44k to a thousand shatters the mesh, and the hands
        # come apart and holes open where the garment meets the body.
        if current > target * 2:
            step = 0.5
        else:
            step = target / float(current)
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

    # The picture the engine will draw: the atlas multiplied by the painted
    # light, flat. Scene lights on top of that blew the paint out to white
    # and hid the very thing the cook is supposed to prove.
    for slot in ob.material_slots:
        material = slot.material
        if material is None:
            continue
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                tree = getattr(material, "node_tree", None)
                if not tree:
                    continue
                bsdf = tree.nodes.get("Principled BSDF")
                output = tree.nodes.get("Material Output")
                if not bsdf or not output:
                    continue
                base = bsdf.inputs.get("Base Color")
                source = base.links[0].from_socket if base and base.links else None
                emission = tree.nodes.new("ShaderNodeEmission")
                emission.location = (bsdf.location.x + 120, bsdf.location.y - 180)
                if source is not None:
                    tree.links.new(source, emission.inputs["Color"])
                else:
                    colour = hex_linear(CURRENT_SKIN)
                    emission.inputs["Color"].default_value = (colour[0], colour[1], colour[2], 1.0)
                tree.links.new(emission.outputs["Emission"], output.inputs["Surface"])
        except Exception:
            continue

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
    # The painted light has to travel with the mesh: the engine multiplies
    # the atlas by the LIGHT attribute, so a file without it is a file with
    # no light in it.
    for name in ("export_colors", "export_vertex_color"):
        try:
            bpy.ops.export_scene.gltf(**kwargs, **{name: "ACTIVE"})
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

    global CURRENT_SKIN, CURRENT_HAIR, CURRENT_DOMINANT
    CURRENT_SKIN = FACTION_SKIN.get(faction, FACTION_SKIN["ashkin"])
    CURRENT_HAIR = FACTION_HAIR.get(faction, FACTION_HAIR["ashkin"])
    CURRENT_DOMINANT = FACTION_DOMINANT.get(faction, 0)

    ob = import_mesh(src)
    ob.name = "figure"
    raw_tris = triangle_count(ob)
    source_height, scale = normalise_height(ob, target_height)
    unwrapped = ensure_uv(ob)

    # The hair gets its own material before the atlas pass, so the head is
    # not all flesh.
    hair_faces = split_hair(ob)
    # A normal map adds nothing the engine can use: the atlas carries the
    # colour and the LIGHT layer carries the light. The suit's map is 4K,
    # so every LOD carried 7 MB of a texture nobody looks at.
    # The skin is fitted under the garment, so the body does not show
    # through the cloth as patches of flesh.
    fitted = fit_skin_under_cloth(ob)
    normals_stripped = drop_normal_maps(ob)
    atlas_info = bake_atlas(ob, palette, outdir)
    light_info = bake_vertex_light(ob)

    target_ratio = min(1.0, budget / float(max(raw_tris, 1)))

    # Every LOD is a share of the same pristine base, not a share of the last
    # one: otherwise the ladder collapses onto the same triangle count.
    print("STEP before pristine", sorted(o.name for o in bpy.data.objects))
    pristine = ob.copy()
    pristine.data = ob.data.copy()
    bpy.context.collection.objects.link(pristine)
    # A name of its own: Blender frees an object when a new one takes its
    # name, and the LOD's joined part took the name "body", which is the
    # name the figure and the pristine base both had.
    pristine.name = "pristine_base"

    print("STEP before ladder", sorted(o.name for o in bpy.data.objects))
    lod_tris = []
    for level, share in enumerate(LOD_RATIOS):
        print("STEP level start", level, sorted(o.name for o in bpy.data.objects))
        ob_level = pristine.copy()
        ob_level.data = pristine.data.copy()
        bpy.context.collection.objects.link(ob_level)
        # A name of its own, for the same reason: the part the join
        # produces takes a name, and a name that is taken frees the
        # object that had it.
        ob_level.name = "lod_%d" % level
        bpy.ops.object.select_all(action="DESELECT")
        ob_level.select_set(True)
        bpy.context.view_layer.objects.active = ob_level
        print("STEP before decimate", level, sorted(o.name for o in bpy.data.objects))
        tris, ob_level = decimate_by_part(ob_level, budget, share)
        print("STEP after decimate", level, sorted(o.name for o in bpy.data.objects))
        path = os.path.join(outdir, f"lod{level}.glb")
        if not export_glb(ob_level, path):
            raise CookError(f"could not write {path}")
        lod_tris.append({"lod": level, "triangles": tris, "file": os.path.basename(path)})
        print("STEP before remove", level, sorted(o.name for o in bpy.data.objects))
        bpy.data.objects.remove(ob_level, do_unlink=True)
        print("STEP after remove", level, sorted(o.name for o in bpy.data.objects))
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
        "hair_faces": hair_faces,
        "skin_fitted": fitted,
        "normals_stripped": normals_stripped,
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
        import traceback
        traceback.print_exc()
        code = EXIT_FAIL
    if code != 0:
        sys.exit(code)
