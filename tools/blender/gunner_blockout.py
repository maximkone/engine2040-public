"""Blockout one heavy gunner silhouette for the 30 m read test.

Primitives only. No imported assets. Outputs a flat black-on-white
silhouette and a grey three-quarter view.
Run: blender -b --factory-startup --python-exit-code 1 -P gunner_blockout.py -- <outdir>
"""
import math
import os
import sys

import bpy

OUT = sys.argv[sys.argv.index("--") + 1] if "--" in sys.argv else os.getcwd()
os.makedirs(OUT, exist_ok=True)

# ---- clean scene -----------------------------------------------------------
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
for block in (bpy.data.meshes, bpy.data.materials, bpy.data.cameras, bpy.data.lights):
    for item in list(block):
        if item.users == 0:
            block.remove(item)


def mat(name, rgb):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = m.node_tree.nodes.get("Principled BSDF")
    if bsdf:
        bsdf.inputs["Base Color"].default_value = (*rgb, 1.0)
        bsdf.inputs["Roughness"].default_value = 0.7
    m.diffuse_color = (*rgb, 1.0)
    return m


BLACK = mat("silhouette", (0.0, 0.0, 0.0))
GREY = mat("grey", (0.55, 0.55, 0.55))


def box(name, size, loc, rot=(0, 0, 0), material=None):
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=loc, rotation=rot)
    ob = bpy.context.object
    ob.name = name
    ob.scale = size
    if material:
        ob.data.materials.append(material)
    return ob


def cyl(name, radius, depth, loc, rot=(0, 0, 0), material=None, verts=12):
    bpy.ops.mesh.primitive_cylinder_add(vertices=verts, radius=radius, depth=depth, location=loc, rotation=rot)
    ob = bpy.context.object
    ob.name = name
    if material:
        ob.data.materials.append(material)
    return ob


# ---- proportions -----------------------------------------------------------
# 2.0 m tall, 6 heads: head 0.333. Shoulders ~3 head widths (head w 0.26 -> 0.78).
H = 0.333
parts = []
parts.append(box("head", (0.24, 0.26, 0.30), (0, 0, 1.84)))
parts.append(box("jaw", (0.20, 0.22, 0.10), (0, -0.03, 1.70)))
parts.append(box("neck", (0.14, 0.14, 0.10), (0, 0, 1.63)))
parts.append(box("torso", (0.62, 0.36, 0.52), (0, 0, 1.38)))
parts.append(box("chest_plate", (0.66, 0.40, 0.22), (0, -0.01, 1.52)))
parts.append(box("pelvis", (0.44, 0.30, 0.22), (0, 0, 1.08)))
parts.append(box("skirt", (0.52, 0.38, 0.26), (0, 0, 0.94)))

# shoulders: 0.78 span, big pauldrons
parts.append(box("pauldron_l", (0.26, 0.30, 0.24), (-0.40, 0, 1.58)))
parts.append(box("pauldron_r", (0.28, 0.32, 0.26), (0.40, 0, 1.58)))

# left arm: deliberately smaller than the cannon side
parts.append(box("upperarm_l", (0.17, 0.17, 0.40), (-0.42, 0, 1.30), rot=(0, math.radians(8), 0)))
parts.append(box("forearm_l", (0.15, 0.15, 0.38), (-0.46, -0.04, 0.94)))
parts.append(box("hand_l", (0.17, 0.13, 0.20), (-0.47, -0.06, 0.70)))

# right arm replaced by the cannon: one unbroken volume from pauldron to muzzle
parts.append(cyl("cannon_body", 0.125, 1.15, (0.58, -0.30, 1.34),
                 rot=(math.radians(66), 0, math.radians(-26))))
parts.append(cyl("cannon_collar", 0.170, 0.20, (0.44, 0.00, 1.56),
                 rot=(math.radians(66), 0, math.radians(-26))))
parts.append(cyl("muzzle", 0.165, 0.18, (0.75, -0.62, 0.90),
                 rot=(math.radians(66), 0, math.radians(-26))))
parts.append(box("vent", (0.34, 0.14, 0.30), (0, 0.24, 1.46)))

# legs: thick, planted wide
for side, x in (("l", -0.17), ("r", 0.17)):
    parts.append(box(f"thigh_{side}", (0.22, 0.26, 0.50), (x, 0, 0.66)))
    parts.append(box(f"shin_{side}", (0.19, 0.22, 0.48), (x, 0, 0.24)))
    parts.append(box(f"boot_{side}", (0.22, 0.34, 0.14), (x, -0.05, 0.05)))

# ---- camera ---------------------------------------------------------------
target = bpy.data.objects.new("target", None)
bpy.context.collection.objects.link(target)
target.location = (0, 0, 1.0)

cam_data = bpy.data.cameras.new("cam")
cam_data.type = "ORTHO"
cam_data.ortho_scale = 2.35
cam = bpy.data.objects.new("cam", cam_data)
bpy.context.collection.objects.link(cam)
cam.location = (2.2, -3.4, 1.75)
con = cam.constraints.new("TRACK_TO")
con.target = target
con.track_axis = "TRACK_NEGATIVE_Z"
con.up_axis = "UP_Y"
bpy.context.scene.camera = cam

scene = bpy.context.scene
scene.render.engine = "BLENDER_WORKBENCH"
scene.render.resolution_x = 512
scene.render.resolution_y = 640
scene.render.film_transparent = False

world = bpy.data.worlds.new("w") if not bpy.data.worlds else bpy.data.worlds[0]
scene.world = world
world.use_nodes = True
world.node_tree.nodes["Background"].inputs[0].default_value = (1, 1, 1, 1)

shading = scene.display.shading


def render(path, size, bg, light, single=None, ortho=2.35):
    scene.render.resolution_x, scene.render.resolution_y = size
    cam_data.ortho_scale = ortho
    scene.display.shading.background_type = "WORLD"
    scene.display.shading.background_color = bg
    world.node_tree.nodes["Background"].inputs[0].default_value = (*bg, 1.0)
    scene.display.shading.light = light
    scene.display.shading.color_type = "SINGLE" if single else "MATERIAL"
    if single is not None:
        scene.display.shading.single_color = single
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)


render(os.path.join(OUT, "gunner_silhouette.png"), (512, 640), (1, 1, 1), "FLAT", single=(0, 0, 0))
render(os.path.join(OUT, "gunner_silhouette_30m.png"), (512, 640), (1, 1, 1), "FLAT", single=(0, 0, 0), ortho=13.0)
render(os.path.join(OUT, "gunner_grey.png"), (512, 640), (0.12, 0.12, 0.12), "STUDIO")

print("WROTE", OUT)
