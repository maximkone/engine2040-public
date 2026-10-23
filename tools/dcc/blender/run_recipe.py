# run_recipe.py - the DCC recipe runner (GEO1b, GEO14). Licensed GPL-3.0-or-later.
# Reads <recipe.json> <out.glb> after `--` and writes the recipe's ops into a
# fresh factory scene, then exports one GLB. Own code; the op set is the DCC
# surface the geometry rows name: cube, bevel, mirror, array, and the bake op
# (GEO14: ambient occlusion, curvature grime and a bevel normal on a piece).
# No engine import, no python key ever read (the cook refuses one before this
# starts), no network and no file outside the named paths. The same recipe on
# the same pinned Blender gives the same bytes.
import json
import math
import os
import struct
import sys
import time
import zlib

ALLOWED_OPS = ("cube", "bevel", "mirror", "array", "bake")

# The tree this script sits in (tools/dcc/blender/run_recipe.py -> repo root).
# A bake recipe names its source relative to it, so a cook from any cwd reads
# the same piece the Node guard checked.
ROOT = os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)

# One flag per baked map. A bake op names at least one; `grime` also needs the
# occlusion bake it multiplies, so it turns that pass on by itself.
BAKE_MAPS = ("ao", "grime", "normal")

# Bound the atlas so a piece cooks inside a second on the pinned 5.2: 256 is the
# measured size (0.38 s for AO) and 1024 is the hard ceiling.
BAKE_SIZE_MIN = 16
BAKE_SIZE_MAX = 1024

# Cycles bake: one thread, fixed seed and sample count, no denoise, so two cooks
# of one recipe give one set of texels. Margin bleeds the islands into the
# unused atlas space so mips do not pull the background in.
BAKE_SAMPLES = 8
BAKE_MARGIN = 4
BAKE_BEVEL_RADIUS = 0.01
BAKE_BEVEL_SAMPLES = 4

# Grime constants: pointiness above GRIME_FLAT is convex (no grime), below
# GRIME_CONCAVE is a crevice (full grime). Fixed, never fitted per piece, so
# the map is a pure function of the baked curvature.
GRIME_FLAT = 0.56
GRIME_CONCAVE = 0.46
# Grime strength floor on a fully lit texel; occlusion adds the rest.
GRIME_LIT = 0.45


def die(text, code):
    sys.stdout.write(text + "\n")
    sys.stdout.flush()
    sys.exit(code)


def args_after_separator(argv):
    if "--" not in argv:
        die("recipe.missing_args", 4)
    at = argv.index("--")
    rest = argv[at + 1:]
    if len(rest) < 2:
        die("recipe.missing_args", 4)
    return rest[0], rest[1]


def check_bake(op):
    src = op.get("src")
    if not isinstance(src, str) or not src.lower().endswith((".gltf", ".glb")):
        die("recipe.bad_bake: src wants a .gltf or .glb path", 4)
    if os.path.isabs(src) or ".." in src.replace("\\", "/").split("/"):
        die("recipe.bad_bake: src must stay under the tree", 4)
    size = op.get("size", 256)
    if not isinstance(size, int) or isinstance(size, bool) or not BAKE_SIZE_MIN <= size <= BAKE_SIZE_MAX:
        die("recipe.bad_bake: size wants %d..%d" % (BAKE_SIZE_MIN, BAKE_SIZE_MAX), 4)
    flags = 0
    for name in BAKE_MAPS:
        value = op.get(name, False)
        if not isinstance(value, bool):
            die("recipe.bad_bake: %s wants true or false" % name, 4)
        flags += int(value)
    if flags == 0:
        die("recipe.no_maps: name ao, grime or normal", 4)
    radius = op.get("bevel", BAKE_BEVEL_RADIUS)
    if not isinstance(radius, (int, float)) or isinstance(radius, bool) or not 0.0 < float(radius) <= 1.0:
        die("recipe.bad_bake: bevel wants (0, 1] meters", 4)


def recipe_ops(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            doc = json.load(handle)
    except (OSError, ValueError) as error:
        die("recipe.unreadable: %s" % error, 4)
    if not isinstance(doc, dict):
        die("recipe.not_an_object", 4)
    if "python" in doc:
        die("recipe.unknown_op: a python key is never read", 4)
    ops = doc.get("ops")
    if not isinstance(ops, list) or not ops:
        die("recipe.no_ops", 4)
    for op in ops:
        name = op.get("op") if isinstance(op, dict) else None
        if name not in ALLOWED_OPS:
            die("recipe.unknown_op: %s" % json.dumps(name), 4)
    if any(op.get("op") == "bake" for op in ops):
        if len(ops) != 1:
            die("recipe.bad_bake: a bake recipe holds one bake op", 4)
        check_bake(ops[0])
    return ops


def fresh_scene(bpy):
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    bpy.ops.object.select_all(action="DESELECT")


def apply_ops(bpy, ops):
    from mathutils import Vector

    for op in ops:
        name = op["op"]
        if name == "cube":
            size = op.get("size", [1.0, 1.0, 1.0])
            bpy.ops.mesh.primitive_cube_add(size=1.0)
            active = bpy.context.active_object
            active.scale = (float(size[0]), float(size[1]), float(size[2]))
            bpy.ops.object.transform_apply(scale=True)
        elif name == "bevel":
            active = bpy.context.active_object
            bevel = active.modifiers.new(name="bevel", type="BEVEL")
            bevel.width = float(op.get("width", 0.05))
            bevel.segments = int(op.get("segments", 2))
            bpy.context.view_layer.objects.active = active
            bpy.ops.object.modifier_apply(modifier=bevel.name)
        elif name == "mirror":
            active = bpy.context.active_object
            mirror = active.modifiers.new(name="mirror", type="MIRROR")
            axis = op.get("axis", "x")
            mirror.use_axis = (axis == "x", axis == "y", axis == "z")
            bpy.context.view_layer.objects.active = active
            bpy.ops.object.modifier_apply(modifier=mirror.name)
        elif name == "array":
            active = bpy.context.active_object
            array = active.modifiers.new(name="array", type="ARRAY")
            array.count = int(op.get("count", 3))
            offset = op.get("offset", [1.2, 0.0, 0.0])
            array.relative_offset_displace = (
                float(offset[0]),
                float(offset[1]),
                float(offset[2]),
            )
            bpy.context.view_layer.objects.active = active
            bpy.ops.object.modifier_apply(modifier=array.name)


# ---------------------------------------------------------------------------
# Bake op (GEO14).
# ---------------------------------------------------------------------------


def png_bytes(width, height, rgba):
    """Deterministic PNG: filter 0 rows, zlib 9, no time or text chunks."""
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw.extend(rgba[y * width * 4:(y + 1) * width * 4])

    def chunk(tag, body):
        return (
            struct.pack(">I", len(body))
            + tag
            + body
            + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)
        )

    head = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", head)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def write_png(path, width, height, rgba):
    directory = os.path.dirname(path)
    if directory:
        os.makedirs(directory, exist_ok=True)
    with open(path, "wb") as handle:
        handle.write(png_bytes(width, height, rgba))


def image_pixels(image):
    count = image.size[0] * image.size[1] * 4
    buf = [0.0] * count
    image.pixels.foreach_get(buf)
    return buf


def new_image(bpy, name, size, fill):
    image = bpy.data.images.new(name, size, size, alpha=True)
    image.colorspace_settings.name = "Non-Color"
    image.pixels.foreach_set(list(fill) * (size * size))
    return image


def to_bytes(pixels):
    """Quantise one float channel to 8 bits, clamping outside 0..1."""
    out = bytearray(len(pixels))
    for index, value in enumerate(pixels):
        if value <= 0.0:
            out[index] = 0
        elif value >= 1.0:
            out[index] = 255
        else:
            out[index] = int(value * 255.0 + 0.5)
    return out


def gray_rgba(red):
    count = len(red)
    out = bytearray(count * 4)
    for index in range(count):
        value = red[index]
        out[index * 4] = value
        out[index * 4 + 1] = value
        out[index * 4 + 2] = value
        out[index * 4 + 3] = 255
    return bytes(out)


def grime_rgba(pointiness, ao):
    """Curvature grime from the pointiness bake, dark where light cannot reach.

    `pointiness` is the EMIT bake of Geometry.Pointiness (0..1); `ao` is the
    occlusion bake. Both come from the same camera-free bake, so the mask is a
    pure function of the piece. This is the authored mask (R=G=grime, B=0);
    `orm_rgba` packs it into the ORM map the material reads.
    """
    count = len(pointiness)
    out = bytearray(count * 4)
    for index in range(count):
        p = pointiness[index]
        concave = (GRIME_FLAT - p) / (GRIME_FLAT - GRIME_CONCAVE)
        if concave < 0.0:
            concave = 0.0
        elif concave > 1.0:
            concave = 1.0
        lit = 1.0 - ao[index]
        if lit < 0.0:
            lit = 0.0
        value = concave * (GRIME_LIT + (1.0 - GRIME_LIT) * lit)
        if value <= 0.0:
            byte = 0
        elif value >= 1.0:
            byte = 255
        else:
            byte = int(value * 255.0 + 0.5)
        out[index * 4] = byte
        out[index * 4 + 1] = byte
        out[index * 4 + 2] = 0
        out[index * 4 + 3] = 255
    return bytes(out)


def orm_rgba(ao, grime, rough, metal):
    """glTF ORM pack: R=occlusion, G=roughness, B=metal.

    The kit keeps its authored surface in the factors (roughness 0.65..0.96,
    metal 0..0.3); a linked ORM texture replaces both factors with the texel,
    so the piece's own values are packed in: roughness starts at the authored
    floor and grime raises it toward 1, metal stays the authored constant.
    """
    floor = int(round(min(1.0, max(0.0, rough)) * 255.0))
    metal_byte = int(round(min(1.0, max(0.0, metal)) * 255.0))
    count = len(ao)
    out = bytearray(count * 4)
    for index in range(count):
        g = floor + ((255 - floor) * grime[index] + 127) // 255
        out[index * 4] = ao[index]
        out[index * 4 + 1] = g
        out[index * 4 + 2] = metal_byte
        out[index * 4 + 3] = 255
    return bytes(out)


def authored_surface(materials):
    """The piece's authored (roughness, metal) from its Principled BSDF."""
    for mat in materials:
        if not mat.use_nodes:
            continue
        bsdf = next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
        if bsdf is not None:
            return (
                float(bsdf.inputs["Roughness"].default_value),
                float(bsdf.inputs["Metallic"].default_value),
            )
    return (0.9, 0.0)


def repack_uvs(bpy, meshes, size):
    """Give every part its own chart inside one grid cell of the atlas.

    The kit's source UVs map every face into 0..1, so they overlap; a bake into
    that layout would average every face onto the same texels. Lightmap-pack
    each object, then scale it into its cell. Deterministic: no RNG.
    """
    cols = max(1, int(math.ceil(math.sqrt(len(meshes)))))
    cell = 1.0 / cols
    inset = 1.0 / size
    for index, ob in enumerate(meshes):
        bpy.ops.object.select_all(action="DESELECT")
        ob.select_set(True)
        bpy.context.view_layer.objects.active = ob
        bpy.ops.object.mode_set(mode="EDIT")
        bpy.ops.mesh.select_all(action="SELECT")
        bpy.ops.uv.lightmap_pack(PREF_CONTEXT="ALL_FACES", PREF_MARGIN_DIV=0.1)
        bpy.ops.object.mode_set(mode="OBJECT")
        col, row = index % cols, index // cols
        layer = ob.data.uv_layers.active
        if layer is None:
            die('recipe.no_uv: object "%s" has no TEXCOORD_0' % ob.name, 4)
        for loop_uv in layer.data:
            u, v = loop_uv.uv
            loop_uv.uv = (
                col * cell + inset + u * (cell - 2.0 * inset),
                row * cell + inset + v * (cell - 2.0 * inset),
            )


def mesh_materials(meshes):
    materials = []
    for ob in meshes:
        for mat in ob.data.materials:
            if mat is not None and mat not in materials:
                materials.append(mat)
    return materials


def add_target(bpy, materials, image):
    made = []
    for mat in materials:
        if not mat.use_nodes:
            mat.use_nodes = True
        nodes = mat.node_tree.nodes
        node = nodes.new("ShaderNodeTexImage")
        node.image = image
        node.select = True
        nodes.active = node
        made.append((nodes, node))
    return made


def drop_targets(made):
    for nodes, node in made:
        nodes.remove(node)


def select_meshes(bpy, meshes):
    bpy.ops.object.select_all(action="DESELECT")
    for ob in meshes:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]


def add_bevel_nodes(materials, radius):
    made = []
    for mat in materials:
        nodes = mat.node_tree.nodes
        bsdf = next((n for n in nodes if n.type == "BSDF_PRINCIPLED"), None)
        if bsdf is None or "Normal" not in bsdf.inputs:
            continue
        bevel = nodes.new("ShaderNodeBevel")
        bevel.samples = BAKE_BEVEL_SAMPLES
        bevel.inputs["Radius"].default_value = radius
        mat.node_tree.links.new(bevel.outputs["Normal"], bsdf.inputs["Normal"])
        made.append((mat, bevel))
    return made


def drop_bevel_nodes(made):
    for mat, bevel in made:
        mat.node_tree.nodes.remove(bevel)


def swap_emit_pointiness(materials):
    """Point the material's surface at an emission of Geometry.Pointiness."""
    saved = []
    for mat in materials:
        nodes = mat.node_tree.nodes
        links = mat.node_tree.links
        output = next((n for n in nodes if n.type == "OUTPUT_MATERIAL"), None)
        if output is None:
            continue
        source = output.inputs["Surface"].links[0].from_socket if output.inputs["Surface"].links else None
        geometry = nodes.new("ShaderNodeNewGeometry")
        emit = nodes.new("ShaderNodeEmission")
        links.new(geometry.outputs["Pointiness"], emit.inputs["Color"])
        links.new(emit.outputs["Emission"], output.inputs["Surface"])
        saved.append((mat, output, source, geometry, emit))
    return saved


def restore_surface(saved):
    for mat, output, source, geometry, emit in saved:
        nodes = mat.node_tree.nodes
        nodes.remove(geometry)
        nodes.remove(emit)
        if source is not None:
            mat.node_tree.links.new(source, output.inputs["Surface"])


def occlusion_group(bpy):
    group = bpy.data.node_groups.get("glTF Material Output")
    if group is not None:
        return group
    group = bpy.data.node_groups.new("glTF Material Output", "ShaderNodeTree")
    group.interface.new_socket(name="Occlusion", in_out="INPUT", socket_type="NodeSocketFloat")
    return group


def wire_material(bpy, mat, orm_image, normal_image):
    """Wire ORM (occlusion + roughness + metal) and the normal for the exporter."""
    if not mat.use_nodes:
        mat.use_nodes = True
    nodes = mat.node_tree.nodes
    links = mat.node_tree.links
    bsdf = next((n for n in nodes if n.type == "BSDF_PRINCIPLED"), None)
    if bsdf is None:
        return
    if orm_image is not None:
        tex = nodes.new("ShaderNodeTexImage")
        tex.image = orm_image
        split = nodes.new("ShaderNodeSeparateColor")
        links.new(tex.outputs["Color"], split.inputs["Color"])
        links.new(split.outputs["Green"], bsdf.inputs["Roughness"])
        links.new(split.outputs["Blue"], bsdf.inputs["Metallic"])
        bsdf.inputs["Roughness"].default_value = 1.0
        bsdf.inputs["Metallic"].default_value = 0.0
        group = nodes.new("ShaderNodeGroup")
        group.node_tree = occlusion_group(bpy)
        links.new(tex.outputs["Color"], group.inputs["Occlusion"])
    if normal_image is not None:
        tex = nodes.new("ShaderNodeTexImage")
        tex.image = normal_image
        normal_map = nodes.new("ShaderNodeNormalMap")
        links.new(tex.outputs["Color"], normal_map.inputs["Color"])
        links.new(normal_map.outputs["Normal"], bsdf.inputs["Normal"])


def bake_piece(bpy, op, out_path):
    src = os.path.join(ROOT, op["src"])
    # Blender resolves an image load against the (unsaved) blend file, not the
    # cwd, so every written map is named absolute from here on.
    out_path = os.path.abspath(out_path)
    size = int(op.get("size", 256))
    want = {name: bool(op.get(name, False)) for name in BAKE_MAPS}
    started = time.time()

    bpy.ops.import_scene.gltf(filepath=src)
    meshes = [ob for ob in bpy.context.scene.objects if ob.type == "MESH"]
    if not meshes:
        die("recipe.no_mesh: %s" % src, 4)
    for ob in meshes:
        if ob.data.uv_layers.active is None:
            die('recipe.no_uv: object "%s" has no TEXCOORD_0' % ob.name, 4)
        if not any(mat is not None for mat in ob.data.materials):
            die('recipe.no_material: object "%s" has no material' % ob.name, 4)
    materials = mesh_materials(meshes)
    repack_uvs(bpy, meshes, size)

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = BAKE_SAMPLES
    scene.cycles.use_denoising = False
    scene.cycles.seed = 0
    scene.render.threads_mode = "FIXED"
    scene.render.threads = 1
    scene.render.bake.margin = BAKE_MARGIN

    ao_pixels = None
    normal_pixels = None
    pointiness = None

    if want["ao"] or want["grime"]:
        image = new_image(bpy, "e2040_ao", size, (1.0, 1.0, 1.0, 1.0))
        targets = add_target(bpy, materials, image)
        select_meshes(bpy, meshes)
        bpy.ops.object.bake(type="AO", use_clear=False, margin=BAKE_MARGIN)
        ao_pixels = image_pixels(image)
        drop_targets(targets)
        bpy.data.images.remove(image)

    if want["normal"]:
        image = new_image(bpy, "e2040_normal", size, (0.5, 0.5, 1.0, 1.0))
        targets = add_target(bpy, materials, image)
        bevels = add_bevel_nodes(materials, float(op.get("bevel", BAKE_BEVEL_RADIUS)))
        select_meshes(bpy, meshes)
        bpy.ops.object.bake(type="NORMAL", use_clear=False, margin=BAKE_MARGIN)
        drop_bevel_nodes(bevels)
        normal_pixels = image_pixels(image)
        drop_targets(targets)
        bpy.data.images.remove(image)

    if want["grime"]:
        image = new_image(bpy, "e2040_grime", size, (GRIME_FLAT + 0.04,) * 3 + (1.0,))
        targets = add_target(bpy, materials, image)
        saved = swap_emit_pointiness(materials)
        scene.cycles.samples = 1
        select_meshes(bpy, meshes)
        bpy.ops.object.bake(type="EMIT", use_clear=False, margin=BAKE_MARGIN)
        scene.cycles.samples = BAKE_SAMPLES
        restore_surface(saved)
        pointiness = image_pixels(image)[0::4]
        drop_targets(targets)
        bpy.data.images.remove(image)

    stem = os.path.splitext(os.path.basename(out_path))[0]
    out_dir = os.path.dirname(out_path) or "."
    written = {}
    ao_bytes = None
    grime_bytes = None
    if ao_pixels is not None:
        ao_bytes = to_bytes(ao_pixels[0::4])
        if want["ao"]:
            path = os.path.join(out_dir, stem + "_ao.png")
            write_png(path, size, size, gray_rgba(ao_bytes))
            written["ao"] = path
    if want["grime"]:
        grime_rgba_bytes = grime_rgba(pointiness, ao_pixels[0::4])
        grime_bytes = grime_rgba_bytes[0::4]
        path = os.path.join(out_dir, stem + "_grime.png")
        write_png(path, size, size, grime_rgba_bytes)
        written["grime"] = path
    if want["normal"]:
        path = os.path.join(out_dir, stem + "_normal.png")
        write_png(path, size, size, to_bytes(normal_pixels))
        written["normal"] = path

    orm_image = None
    normal_image = None
    if ao_bytes is not None:
        path = os.path.join(out_dir, stem + "_orm.png")
        if grime_bytes is None:
            grime_bytes = bytes(len(ao_bytes))
        rough, metal = authored_surface(materials)
        write_png(path, size, size, orm_rgba(ao_bytes, grime_bytes, rough, metal))
        written["orm"] = path
        orm_image = bpy.data.images.load(path, check_existing=False)
        orm_image.colorspace_settings.name = "Non-Color"
    if want["normal"]:
        normal_image = bpy.data.images.load(written["normal"], check_existing=False)
        normal_image.colorspace_settings.name = "Non-Color"
    for mat in materials:
        wire_material(bpy, mat, orm_image, normal_image)

    bpy.ops.export_scene.gltf(
        filepath=out_path,
        export_format="GLB",
        export_yup=True,
        export_apply=True,
        export_extras=False,
    )
    for name, path in written.items():
        sys.stdout.write("recipe.map %s %s\n" % (name, path))
    sys.stdout.write(
        "recipe.baked %s objects=%d size=%d wall=%.2f\n"
        % (out_path, len(meshes), size, time.time() - started)
    )


def main():
    recipe_path, out_path = args_after_separator(sys.argv[1:])
    ops = recipe_ops(recipe_path)
    import bpy

    fresh_scene(bpy)
    if ops[0]["op"] == "bake":
        bake_piece(bpy, ops[0], out_path)
    else:
        apply_ops(bpy, ops)
        bpy.context.view_layer.update()
        bpy.ops.export_scene.gltf(
            filepath=out_path,
            export_format="GLB",
            export_yup=True,
            export_apply=True,
            export_extras=False,
        )
    sys.stdout.write("recipe.cooked %s\n" % out_path)


main()
