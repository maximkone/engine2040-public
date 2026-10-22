"""The Roman kit as game meshes: a gladius and a scutum, real proportions.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/roman_kit.py -- <out-dir>

Writes <out-dir>/gladius.glb and <out-dir>/scutum.glb, metres, Y up after
export. Each part carries a small embedded base-colour image (the game's body
importer wants one per part) and its own roughness and metal.

Gladius (Pompeii pattern, 1st c. AD): blade 0.50 m, parallel edges, short
triangular point, 5 cm wide; bone guard, ribbed wooden grip, round pommel.
Grip axis is +Z in Blender with the grip centre at the origin, so a hand
socket holds it by the origin and the blade runs along +Z.
Scutum (curved rectangular legionary shield): 1.00 m tall, 0.64 m across the
chord, curved on a 0.45 m radius, plywood board in red with a bronze rim and
an iron boss. The grip sits at the origin on the inside (+Y is the face).
"""

import math
import os
import sys

import bmesh
import bpy

EXIT_USAGE = 2


def material(name, rgb, metallic, roughness):
    """A material whose base colour is a tiny embedded image of one colour."""
    img = bpy.data.images.new(f"{name}_img", width=4, height=4)
    img.pixels = [rgb[0], rgb[1], rgb[2], 1.0] * 16
    img.pack()
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = nt.nodes["Principled BSDF"]
    tex = nt.nodes.new("ShaderNodeTexImage")
    tex.image = img
    nt.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    bsdf.inputs["Metallic"].default_value = metallic
    bsdf.inputs["Roughness"].default_value = roughness
    return mat


def mesh_object(name, bm, mat):
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    me.materials.append(mat)
    for poly in me.polygons:
        poly.use_smooth = False
    return ob


def lathe(name, profile, mat, segments=16):
    """A body of revolution about Z from (radius, z) pairs."""
    bm = bmesh.new()
    rings = []
    for r, z in profile:
        ring = [bm.verts.new((r * math.cos(2 * math.pi * k / segments),
                              r * math.sin(2 * math.pi * k / segments), z)) for k in range(segments)]
        rings.append(ring)
    for a, b in zip(rings, rings[1:]):
        for k in range(segments):
            bm.faces.new((a[k], a[(k + 1) % segments], b[(k + 1) % segments], b[k]))
    bm.faces.new(list(reversed(rings[0])))
    bm.faces.new(rings[-1])
    return mesh_object(name, bm, mat)


def gladius():
    steel = material("gladius_blade", (0.62, 0.64, 0.66), 1.0, 0.32)
    bone = material("gladius_guard", (0.80, 0.74, 0.60), 0.0, 0.55)
    wood = material("gladius_grip", (0.36, 0.22, 0.12), 0.0, 0.7)
    brass = material("gladius_pommel", (0.72, 0.55, 0.24), 0.9, 0.4)

    # Blade: a flat diamond section, parallel edges, short point.
    bm = bmesh.new()
    width, thick, base, length, point = 0.05, 0.007, 0.075, 0.50, 0.09
    stations = [(base, width), (base + length - point, width * 0.92), (base + length, 0.0)]
    rings = []
    for z, w in stations:
        hw = w / 2
        ring = [bm.verts.new(v) for v in ((hw, 0, z), (0, thick / 2, z), (-hw, 0, z), (0, -thick / 2, z))]
        rings.append(ring)
    for a, b in zip(rings, rings[1:]):
        for k in range(4):
            bm.faces.new((a[k], a[(k + 1) % 4], b[(k + 1) % 4], b[k]))
    bm.faces.new(list(reversed(rings[0])))
    parts = [mesh_object("gladius_blade", bm, steel)]
    # Guard: a rounded bone block under the blade.
    parts.append(lathe("gladius_guard", [(0.0, 0.050), (0.040, 0.052), (0.044, 0.062),
                                         (0.040, 0.074), (0.0, 0.076)], bone, 12))
    parts[-1].scale = (1.0, 0.55, 1.0)
    # Grip: ribbed wood, the hand's centre at the origin.
    ribs = []
    for k in range(9):
        z = -0.045 + k * 0.012
        ribs.append((0.016 if k % 2 else 0.0185, z))
    parts.append(lathe("gladius_grip", [(0.0, -0.05)] + ribs + [(0.0, 0.05)], wood, 12))
    # Pommel: a flattened ball.
    prof = [(0.03 * math.sin(math.pi * k / 8), -0.05 - 0.035 * (1 - math.cos(math.pi * k / 8)) / 2)
            for k in range(9)]
    parts.append(lathe("gladius_pommel", prof, brass, 14))
    parts[-1].scale = (1.0, 0.6, 1.0)
    return parts


def scutum():
    red = material("scutum_board", (0.52, 0.08, 0.06), 0.0, 0.75)
    bronze = material("scutum_rim", (0.66, 0.46, 0.20), 0.9, 0.38)
    iron = material("scutum_boss", (0.30, 0.30, 0.31), 1.0, 0.45)

    height, chord, radius, thick = 1.00, 0.64, 0.45, 0.012
    half = math.asin((chord / 2) / radius)
    cols, rows = 12, 6

    def arc(r, k):
        a = -half + 2 * half * k / cols
        return r * math.sin(a), r * math.cos(a) - radius

    bm = bmesh.new()
    front, back = [], []
    for j in range(rows + 1):
        z = -height / 2 + height * j / rows
        front.append([bm.verts.new((*arc(radius, k), z)) for k in range(cols + 1)])
        back.append([bm.verts.new((*arc(radius - thick, k), z)) for k in range(cols + 1)])
    for j in range(rows):
        for k in range(cols):
            bm.faces.new((front[j][k], front[j][k + 1], front[j + 1][k + 1], front[j + 1][k]))
            bm.faces.new((back[j][k + 1], back[j][k], back[j + 1][k], back[j + 1][k + 1]))
    for j in range(rows):
        bm.faces.new((front[j][0], front[j + 1][0], back[j + 1][0], back[j][0]))
        bm.faces.new((back[j][cols], back[j + 1][cols], front[j + 1][cols], front[j][cols]))
    for k in range(cols):
        bm.faces.new((front[0][k + 1], front[0][k], back[0][k], back[0][k + 1]))
        bm.faces.new((front[rows][k], front[rows][k + 1], back[rows][k + 1], back[rows][k]))
    parts = [mesh_object("scutum_board", bm, red)]

    # Rim: a bronze band along the top and bottom edges, a hair proud.
    for zc in (-height / 2, height / 2):
        bm = bmesh.new()
        lo = [bm.verts.new((*arc(radius + 0.006, k), zc - 0.018)) for k in range(cols + 1)]
        hi = [bm.verts.new((*arc(radius + 0.006, k), zc + 0.018)) for k in range(cols + 1)]
        for k in range(cols):
            bm.faces.new((lo[k], lo[k + 1], hi[k + 1], hi[k]))
        parts.append(mesh_object("scutum_rim", bm, bronze))
    # Boss: an iron dome on the face, centred.
    prof = [(0.11 * math.cos(math.pi / 2 * k / 6), 0.06 * math.sin(math.pi / 2 * k / 6)) for k in range(7)]
    boss = lathe("scutum_boss", [(0.11, -0.005)] + prof, iron, 16)
    boss.rotation_euler = (-math.pi / 2, 0.0, 0.0)
    boss.location = (0.0, 0.002, 0.0)
    parts.append(boss)
    return parts


def export(objects, path):
    bpy.ops.object.select_all(action="DESELECT")
    for ob in objects:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB", use_selection=True,
                              export_apply=True)
    tris = sum(len(p.vertices) - 2 for ob in objects for p in ob.data.polygons)
    print(f"ROMAN_KIT {os.path.basename(path)} parts={len(objects)} tris={tris}")


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if not args:
        sys.stderr.write(__doc__)
        return EXIT_USAGE
    out = args[0]
    os.makedirs(out, exist_ok=True)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sword = gladius()
    export(sword, os.path.join(out, "gladius.glb"))
    for ob in sword:
        bpy.data.objects.remove(ob, do_unlink=True)
    export(scutum(), os.path.join(out, "scutum.glb"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
