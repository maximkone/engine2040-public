"""CT7 eyes (rest + wire): render the shipped body GLB bytes."""
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


def assign_quadrant_materials(variant, atlasdir):
    # CT19: bind the cooked quadrant PNGs explicitly (the body GLB UVs are
    # already quadrant-local from export), so the rest render can never show
    # flat untextured grey even if embedding failed.
    quad_mats = []
    for qi in range(4):
        img = bpy.data.images.load(
            os.path.join(atlasdir, "%s_atlas_q%d.png" % (variant, qi)))
        img.colorspace_settings.name = "sRGB"
        mat = bpy.data.materials.new("MAT_%s_q%d" % (variant, qi))
        mat.use_nodes = True
        bsdf = mat.node_tree.nodes["Principled BSDF"]
        tex = mat.node_tree.nodes.new("ShaderNodeTexImage")
        tex.image = img
        mat.node_tree.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
        bsdf.inputs["Roughness"].default_value = 0.8
        bsdf.inputs["Metallic"].default_value = 0.0
        quad_mats.append(mat)
    n = 0
    for ob in bpy.data.objects:
        if ob.type != "MESH" or not ob.name.startswith("BODY"):
            continue
        for i in range(min(len(ob.data.materials), 4)):
            ob.data.materials[i] = quad_mats[i]
            n += 1
    if n == 0:
        # Fallback: assign by slot order on every imported mesh.
        for ob in bpy.data.objects:
            if ob.type != "MESH":
                continue
            for i in range(min(len(ob.data.materials), 4)):
                ob.data.materials[i] = quad_mats[i]
                n += 1
    print("CT19-REST-MATS slots=%d" % n)


def scale_pole():
    # CT19 scale pole: 1.8 m striped pole left of the fighter.
    pole_w = bpy.data.materials.new("POLE_W")
    pole_w.use_nodes = True
    pole_w.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.9, 0.9, 0.9, 1.0)
    pole_b = bpy.data.materials.new("POLE_B")
    pole_b.use_nodes = True
    pole_b.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.05, 0.05, 0.05, 1.0)
    for i in range(18):
        bpy.ops.mesh.primitive_cube_add(size=1.0, location=(-1.3, 0.0, 0.05 + 0.1 * i))
        band = bpy.context.view_layer.objects.active
        band.scale = (0.03, 0.03, 0.05)
        bpy.ops.object.transform_apply(scale=True)
        band.data.materials.append(pole_w if i % 2 == 0 else pole_b)
    bpy.ops.mesh.primitive_cylinder_add(radius=0.09, depth=0.02, location=(-1.3, 0.0, 0.01))
    bpy.context.view_layer.objects.active.data.materials.append(pole_b)


def main():
    args = parse_args()
    variant = args["variant"]
    outdir = args["outdir"]
    os.makedirs(outdir, exist_ok=True)

    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    bpy.ops.import_scene.gltf(filepath=args["bodyglb"])
    assign_quadrant_materials(variant, args["atlasdir"])
    scale_pole()

    bpy.ops.object.camera_add(location=(2.7, -3.3, 1.7))
    cam = bpy.context.view_layer.objects.active
    bpy.context.scene.camera = cam
    import mathutils
    d = mathutils.Vector((0.0, 0.0, 0.95)) - cam.location
    cam.rotation_euler = d.to_track_quat("-Z", "Y").to_euler()
    bpy.ops.object.light_add(type="SUN", location=(3, -2, 5))
    bpy.ops.object.light_add(type="AREA", location=(-2.5, -2.5, 3.0))
    bpy.context.view_layer.objects.active.data.energy = 400.0
    bpy.ops.object.light_add(type="SUN", location=(-3, 3, 4))
    bpy.context.view_layer.objects.active.data.energy = 1.2

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 64
    scene.render.resolution_x = 1024
    scene.render.resolution_y = 1024
    world = bpy.data.worlds["World"]
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs["Strength"].default_value = 1.2

    scene.render.filepath = os.path.join(outdir, variant + "_rest.png")
    bpy.ops.render.render(write_still=True)

    # Freestyle edge lines over the shaded render: real topology evidence.
    scene.render.use_freestyle = True
    lineset = scene.view_layers[0].freestyle_settings.linesets.active
    lineset.select_silhouette = True
    lineset.select_border = True
    lineset.select_crease = True
    lineset.select_edge_mark = False
    lineset.linestyle.thickness = 1.0
    lineset.linestyle.color = (0.0, 0.0, 0.0)
    scene.render.filepath = os.path.join(outdir, variant + "_wire.png")
    bpy.ops.render.render(write_still=True)
    print("CT7-PREVIEW-REST " + variant)


main()
