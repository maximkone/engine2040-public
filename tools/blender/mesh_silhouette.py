"""The 30 m read test for any mesh: import a GLB, frame it, render a
black-on-white silhouette and a 30 m frame, and print the measured numbers.

Run: blender -b --factory-startup --python-exit-code 1 -P mesh_silhouette.py -- <in.glb> <outdir>
"""
import json
import os
import sys

import bpy
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
if len(argv) < 2:
    raise SystemExit("usage: mesh_silhouette.py <in.glb> <outdir>")
src, out_dir = argv[0], argv[1]
os.makedirs(out_dir, exist_ok=True)

bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
bpy.ops.import_scene.gltf(filepath=src)

meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
if not meshes:
    raise SystemExit("no mesh in the file")

low = Vector((1e9, 1e9, 1e9))
high = Vector((-1e9, -1e9, -1e9))
for ob in meshes:
    for corner in ob.bound_box:
        co = ob.matrix_world @ Vector(corner)
        low = Vector((min(low[i], co[i]) for i in range(3)))
        high = Vector((max(high[i], co[i]) for i in range(3)))
size = high - low
center = (high + low) * 0.5

# normalise to 2 m tall, feet at z = 0
scale = 2.0 / max(size.z, 1e-6)
for ob in meshes:
    ob.scale = (scale, scale, scale)
bpy.context.view_layer.update()
low = Vector((1e9, 1e9, 1e9))
high = Vector((-1e9, -1e9, -1e9))
for ob in meshes:
    for corner in ob.bound_box:
        co = ob.matrix_world @ Vector(corner)
        low = Vector((min(low[i], co[i]) for i in range(3)))
        high = Vector((max(high[i], co[i]) for i in range(3)))
size = high - low
center = (high + low) * 0.5
for ob in meshes:
    ob.location.z -= low.z

target = bpy.data.objects.new("target", None)
bpy.context.collection.objects.link(target)
target.location = (center.x, center.y, 1.0)

cam_data = bpy.data.cameras.new("cam")
cam_data.type = "ORTHO"
cam = bpy.data.objects.new("cam", cam_data)
bpy.context.collection.objects.link(cam)
cam.location = (2.2, -3.4, 1.75)
con = cam.constraints.new("TRACK_TO")
con.target = target
con.track_axis = "TRACK_NEGATIVE_Z"
con.up_axis = "UP_Y"

scene = bpy.context.scene
scene.render.engine = "BLENDER_WORKBENCH"
scene.render.film_transparent = False
world = bpy.data.worlds[0] if bpy.data.worlds else bpy.data.worlds.new("w")
scene.world = world
world.color = (1, 1, 1)


def render(path, res, ortho, bg, light, single=None):
    scene.render.resolution_x, scene.render.resolution_y = res
    cam_data.ortho_scale = ortho
    scene.display.shading.background_type = "WORLD"
    scene.display.shading.background_color = bg
    world.color = bg
    scene.display.shading.light = light
    scene.display.shading.color_type = "SINGLE" if single else "MATERIAL"
    if single is not None:
        scene.display.shading.single_color = single
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)


render(os.path.join(out_dir, "silhouette_big.png"), (800, 1000), 2.4, (1, 1, 1), "FLAT", single=(0, 0, 0))
render(os.path.join(out_dir, "silhouette_30m.png"), (1132, 636), 30.0, (1, 1, 1), "FLAT", single=(0, 0, 0))
render(os.path.join(out_dir, "grey_studio.png"), (800, 1000), 2.4, (0.12, 0.12, 0.12), "STUDIO")

tris = 0
for ob in meshes:
    ob.data.calc_loop_triangles()
    tris += len(ob.data.loop_triangles)

print(json.dumps({
    "source": src,
    "objects": len(meshes),
    "triangles": tris,
    "height_m": round(size.z, 4),
    "width_m": round(size.x, 4),
    "depth_m": round(size.y, 4),
    "note": "silhouette_30m is a 30 m wide frame; read it at 100% and the figure is 90 px tall",
}))
