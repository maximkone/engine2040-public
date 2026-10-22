"""The style pass on the body: a caricature drawn over a real anatomy.

This is what separates a 2005 character from a real person. The 2005 games
did not use realistic bodies: they used chunky ones, with a head bigger than
life, shoulders wider than life, a waist pulled in, short thick legs, big
hands and big boots, and a painted face on top. A real anatomy can carry
that drawing. The anatomy is the base; the drawing is the style.

So: the body comes from MakeHuman (CC0) and this script bends it. Every
vertex knows how high it is, and the height picks a band out of
content/design/proportions.json, and the band says how much wider or longer
that part of the body is. The head is a separate lump scaled about the neck,
so the face keeps its shape while the head grows.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/proportion_pass.py -- <in.glb> <out.glb> --faction wardens
"""

import json
import os
import sys

import bpy

EXIT_USAGE = 2
EXIT_FAIL = 3

HEAD_TOP = 0.88
# A hand is about a tenth of the figure's height, from the wrist to the
# tip of the middle finger.
HAND_LENGTH = 0.19

NECK = 0.86


class ProportionError(Exception):
    """A proportion pass that could not be finished."""


def arg(name, fallback=None):
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if name in argv:
        i = argv.index(name)
        if i + 1 < len(argv):
            return argv[i + 1]
    return fallback


def load_spec():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
    path = os.path.join(root, "content", "design", "proportions.json")
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def band_for(t, bands):
    for band in bands:
        if band["from"] <= t < band["to"]:
            return band
    return bands[-1]


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(args) < 2:
        sys.stderr.write("usage: blender -b --factory-startup --python-exit-code 1 "
                         "-P proportion_pass.py -- <in.glb> <out.glb> --faction wardens\n")
        return EXIT_USAGE
    src, dest = args[0], args[1]
    faction = str(arg("--faction", "wardens")).lower()
    class_id = arg("--class")

    spec = load_spec()
    if class_id:
        faction = spec.get("classes", {}).get(class_id, faction)
    profile = spec.get("factions", {}).get(faction)
    if profile is None:
        raise ProportionError(f"no proportions for {faction}")
    bands = profile["bands"]

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=src)
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    if not meshes:
        raise ProportionError(f"no mesh in {src}")
    bpy.ops.object.select_all(action="DESELECT")
    for ob in meshes:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    if len(meshes) > 1:
        bpy.ops.object.join()
    ob = bpy.context.view_layer.objects.active
    ob.name = "body"
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    if ob.data.shape_keys is not None:
        bpy.ops.object.convert(target="MESH")
        ob = bpy.context.view_layer.objects.active
        ob.name = "body"

    me = ob.data
    zs = [v.co.z for v in me.vertices]
    if not zs:
        raise ProportionError("the mesh has no vertices")
    z_min, z_max = min(zs), max(zs)
    height = max(z_max - z_min, 1e-6)
    centre_x = sum(v.co.x for v in me.vertices) / len(me.vertices)
    span = max(v.co.x for v in me.vertices) - min(v.co.x for v in me.vertices)
    # The torso's own width, measured at chest height where there are no
    # arms. The arms' reach is measured against the torso and not
    # against the figure, because the figure is as wide as the arms are
    # spread: measured against itself, an arm at the figure's edge was
    # pulled by nothing at all.
    chest_lo = z_min + height * 0.58
    chest_hi = z_min + height * 0.74
    # The innermost vertex at each height is the body's own edge: the
    # arm is outboard of it. So the body's half width is the median of
    # the innermost vertices across the chest band. The old measure took
    # the median of every vertex there, which is the arm's width and not
    # the body's, so the reference was too big and the pull was measured
    # against itself and the hands stayed out.
    edges = []
    steps = 12
    for step in range(steps):
        lo = chest_lo + (chest_hi - chest_lo) * step / steps
        hi = chest_lo + (chest_hi - chest_lo) * (step + 1) / steps
        row = sorted(abs(v.co.x - centre_x) for v in me.vertices
                     if lo <= v.co.z < hi)
        if row:
            edges.append(row[0])
    edges.sort()
    torso = (edges[len(edges) // 2] * 2.0) if edges else span * 0.5
    centre_y = sum(v.co.y for v in me.vertices) / len(me.vertices)

    # How far the hands come in, and how far the arms reach. Both are
    # shares of the torso's own width.
    hands_in = float(profile.get("hands", {}).get("in", 1.0))
    head = profile.get("head", {})
    head_scale = float(head.get("scale", 1.0))
    head_x = float(head.get("x", 1.0))
    head_y = float(head.get("y", 1.0))
    neck_z = z_min + NECK * height

    # The rig is the authority on what a vertex is. MPFB's bone names say
    # what a vertex belongs to, so the arms are never mistaken for the
    # shoulders and the hands can be made into mitts at the wrist.
    group_names = {g.index: g.name.lower() for g in ob.vertex_groups}
    armatures = [o for o in bpy.context.scene.objects if o.type == "ARMATURE"]
    rig = armatures[0] if armatures else None

    def bone_part(name):
        # The fingers are weighted to their own bones, so "hand" alone misses
        # them, and a finger that falls through to the body bands gets
        # stretched into a claw.
        # The rig has no "hand" group: the hands are weighted to
        # "wrist.r", "metacarpal1.r" and the fingers' own groups. The
        # rule that looked for a name starting with "hand" matched
        # nothing, so the hands fell through to the body bands: they
        # were never made into mitts and never brought in.
        for part in ("hand", "finger", "thumb", "metacarpal", "wrist",
                     "pinky", "ring", "middle", "index"):
            if part in name and name[-1:] in ("l", "r"):
                return "hand"
        # The bone names are "upperarm01.r", "lowerarm01.r" and
        # "shoulder01.r", and the rule used to ask for a name that
        # started with "arm", so it matched nothing: no vertex was ever
        # an arm, and the arms were never touched. The name contains the
        # part, and the side is the last letter.
        for part in ("upperarm", "lowerarm", "forearm", "arm", "shoulder",
                     "clavicle"):
            if part in name and name[-1:] in ("l", "r"):
                return "arm"
        return ""

    def wrist_for(name):
        if rig is None:
            return None, None
        for candidate in rig.data.bones:
            cname = candidate.name.lower()
            if "hand" in cname and not any(x in cname for x in ("thumb", "finger", "index", "middle", "ring", "pinky")):
                head = rig.matrix_world @ candidate.head_local
                tail = rig.matrix_world @ candidate.tail_local
                return head, (tail - head).normalized()
        return None, None

    def classify(vertex):
        best = None
        for entry in vertex.groups:
            if best is None or entry.weight > best.weight:
                best = entry
        if best is None:
            return ""
        return group_names.get(best.group, "")

    for vertex in me.vertices:
        co = vertex.co
        t = (co.z - z_min) / height
        name = classify(vertex)
        part = bone_part(name)
        if "toe" in name or "foot" in name:
            part = "foot"
        elif "leg" in name:
            part = "leg"
        if "head" in name or "neck" in name:
            part = "head"

        if part in ("hand",):
            # A mitt: the fingers come in and the fist gets fat, at the
            # wrist, on the hand bone's own axis. The sheet says how fat.
            #
            # The fingers are shortened and fattened, both measured from
            # the wrist, and the palm keeps its own size. The old rule
            # scaled the whole distance from the wrist along the axis, so
            # the fingertips were pulled back to the wrist and the fingers
            # stretched into spikes.
            mitt = float(profile.get("hands", {}).get("scale", 1.0))
            wrist, axis = wrist_for(name)
            if wrist is not None:
                offset = co - wrist
                along = offset.dot(axis)
                across = offset - axis * along
                # The palm is the first third of the hand's length; the
                # fingers are the rest.
                length = HAND_LENGTH
                palm = min(1.0, along / length) if length > 0 else 1.0
                # How much of this vertex is in the fingers.
                finger = max(0.0, (along - length * 0.33) / max(length * 0.67, 1e-6))
                # The fingers get shorter and fatter; the palm only gets
                # fatter, a little.
                shorten = 1.0 - 0.34 * finger
                fatten = 1.0 + (mitt - 1.0) * (0.35 + 0.65 * finger)
                co = (wrist + axis * (along * shorten)
                      + across * fatten)
            # The hand comes in towards the body. The hands were the
            # widest part of the figure, and a figure is as wide as its
            # hands' spread, so the box was wide and the body inside it
            # was thin. A character stands with its hands in beside it.
            if hands_in < 1.0:
                out = abs(co.x - centre_x)
                limit = max(torso * 0.5, 1e-6)
                pull = 1.0 - (1.0 - hands_in) * min(1.0, out / limit)
                co.x = centre_x + (co.x - centre_x) * pull
            continue
        if part in ("arm", "upperarm", "forearm", "shoulder"):
            # An arm is not ours to draw by height, but it is ours to
            # draw by mass and by where it hangs.
            #
            # The arms are the widest part of a figure's silhouette, so a
            # character with heavy shoulders and thin arms reads as a
            # stick. But the arms were spread out to the sides: the
            # measured figure was 1.23 m wide, with 82% of that width at
            # chest height and 28% at the thighs, so the box was wide
            # and the body inside it was thin. A character with its arms
            # out reads as a scarecrow.
            #
            # So the arm is brought in towards the body, by how far out
            # it is, and kept thick. An arm that already hangs down is
            # left alone, and an arm that reaches out is pulled in. The
            # sheet says how heavy it is.
            arm = float(profile.get("arms", {}).get("scale", 1.0))
            reach = float(profile.get("arms", {}).get("reach", 1.0))
            # How far out this vertex is, as a share of the torso's own
            # half width. The shoulder sits at the torso's edge and the
            # arm comes down from there, so the reach is a share of the
            # torso and not of the figure.
            out = abs(co.x - centre_x)
            limit = max(torso * 0.5, 1e-6)
            pull = 1.0 - (1.0 - reach) * min(1.0, out / limit)
            co.x = centre_x + (co.x - centre_x) * pull * arm
            co.y = centre_y + (co.y - centre_y) * (1.0 + (arm - 1.0) * 0.6)
            co.z = z_min + (co.z - z_min) * 1.04
            continue
        if part == "head":
            co.z = neck_z + (co.z - neck_z) * head_scale
            co.x = centre_x + (co.x - centre_x) * (head_x * head_scale)
            co.y = centre_y + (co.y - centre_y) * (head_y * head_scale)
            continue
        if part == "leg" or part == "foot":
            band = band_for(t, bands)
            co.x = centre_x + (co.x - centre_x) * float(band["x"])
            co.y = centre_y + (co.y - centre_y) * float(band["y"])
            co.z = z_min + (co.z - z_min) * float(band["z"])
            if part == "foot":
                # The boot is its own shape: the sheet says how big.
                boot = float(profile.get("boots", {}).get("scale", 1.0))
                co.x = centre_x + (co.x - centre_x) * boot
                co.y = centre_y + (co.y - centre_y) * boot
            continue
        if t >= HEAD_TOP:
            co.z = neck_z + (co.z - neck_z) * head_scale
            co.x = centre_x + (co.x - centre_x) * (head_x * head_scale)
            co.y = centre_y + (co.y - centre_y) * (head_y * head_scale)
            continue
        band = band_for(t, bands)
        co.x = centre_x + (co.x - centre_x) * float(band["x"])
        co.y = centre_y + (co.y - centre_y) * float(band["y"])
        co.z = z_min + (co.z - z_min) * float(band["z"])
    me.update()

    os.makedirs(os.path.dirname(os.path.abspath(dest)) or ".", exist_ok=True)

    # Every mesh goes into the file, each with its own material. Exporting
    # one object dropped the garment, and the body arrived with no
    # materials at all, so the cook had nothing to paint.
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    def label_for(source, material):
        """What a material is: the skin, or the cloth.

        The object name is the authority, because the objects still have
        their own names here: the body is "Human" and everything else is
        the garment. The material's own node tree cannot be trusted, for
        the exporter rewrites it, and the body's slot comes back looking
        as if it carried a texture.

        A material with no image at all is the skin whatever it is
        called: MPFB's skin is procedural and has no image to write, and
        that is the one case the cook cannot get wrong.
        """
        if material is None:
            return "skin"
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                tree = getattr(material, "node_tree", None)
                if tree:
                    has_image = any(node.type == "TEX_IMAGE" and node.image
                                    for node in tree.nodes)
                    if not has_image:
                        return "skin"
        except Exception:
            pass
        return "skin" if source.name.lower() == "human" else "cloth"

    for source in meshes:
        if not source.material_slots:
            # The import hands back the body with no slots at all, not with
            # a slot that has nothing in it, so the slot is made here. A
            # body with no material arrives at the cook as one flat colour
            # for the skin, the hair and the cloth alike.
            material = bpy.data.materials.new(name="skin")
            material.use_nodes = True
            source.data.materials.append(material)
            for poly in source.data.polygons:
                poly.material_index = 0
            continue
        for slot in source.material_slots:
            label = label_for(source, slot.material)
            if slot.material is None:
                # A slot with nothing in it: the skin, whose material was
                # procedural and could not be written.
                material = bpy.data.materials.new(name=label)
                material.use_nodes = True
                slot.material = material
            else:
                slot.material.name = label
    bpy.ops.object.select_all(action="DESELECT")
    for source in meshes:
        source.select_set(True)
    bpy.context.view_layer.objects.active = ob
    try:
        bpy.ops.export_scene.gltf(filepath=dest, export_format="GLB", export_apply=True,
                                  export_texcoords=True, export_normals=True, use_selection=True)
    except TypeError:
        bpy.ops.export_scene.gltf(filepath=dest, export_format="GLB")
    if not os.path.exists(dest) or os.path.getsize(dest) == 0:
        raise ProportionError(f"export wrote nothing: {dest}")

    zs = [v.co.z for v in me.vertices]
    metrics = {
        "source": os.path.abspath(src),
        "out": os.path.abspath(dest),
        "faction": faction,
        "class": class_id,
        "vertices": len(me.vertices),
        "height_m": round(max(zs) - min(zs), 4),
        "head_scale": head_scale,
        "bands": len(bands),
    }
    print(json.dumps(metrics, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except ProportionError as err:
        sys.stderr.write(f"E_PROP {err}\n")
        code = EXIT_FAIL
    except Exception as err:
        sys.stderr.write(f"E_PROP {type(err).__name__}: {err}\n")
        code = EXIT_FAIL
    if code != 0:
        sys.exit(code)
