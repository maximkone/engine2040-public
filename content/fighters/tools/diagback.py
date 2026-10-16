import bpy
import os
import sys


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1:]
    out = {}
    i = 0
    while i < len(argv):
        out[argv[i].lstrip("-").replace("-", "_")] = argv[i + 1]
        i += 2
    return out


def main():
    args = parse_args()
    outdir = args["outdir"]
    os.makedirs(outdir, exist_ok=True)
    arm = bpy.data.objects["human_male"]

    bpy.ops.object.camera_add(location=(-2.7, 3.3, 1.7))
    cam = bpy.context.view_layer.objects.active
    bpy.context.scene.camera = cam
    import mathutils
    d = mathutils.Vector((0.0, 0.0, 0.95)) - cam.location
    cam.rotation_euler = d.to_track_quat("-Z", "Y").to_euler()

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 64
    scene.render.resolution_x = 1024
    scene.render.resolution_y = 1024
    if arm.animation_data is None:
        arm.animation_data_create()
    arm.animation_data.action = bpy.data.actions["human_male_cast_main_hand_melee"]
    scene.frame_set(5)
    scene.render.filepath = os.path.join(outdir, "diag_back.png")
    bpy.ops.render.render(write_still=True)
    print("CT7-DIAG done")


main()
