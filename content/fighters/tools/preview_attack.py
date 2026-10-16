"""CT7 eyes (attack): pose the staged assembly with its native action."""
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
    variant = args["variant"]
    outdir = args["outdir"]
    os.makedirs(outdir, exist_ok=True)
    arm = bpy.data.objects["human_male"]
    mesh_ob = bpy.data.objects["BODY_fighter"]
    me = mesh_ob.data

    # Same quadrant assignment as the export path (staged faces all sit on
    # slot 0; per-face slots + quadrant-local UVs are assigned on export).
    TILES_UV = {
        "armor": (0.0, 0.5, 0.5), "body": (0.5, 0.75, 0.25),
        "head": (0.75, 0.75, 0.25), "hair": (0.5, 0.5, 0.25),
        "hands": (0.75, 0.5, 0.25), "feet": (0.5, 0.25, 0.25),
        "boots": (0.75, 0.25, 0.25), "gloves": (0.0, 0.0, 0.25),
        "helmet": (0.25, 0.0, 0.25), "weapon": (0.0, 0.25, 0.25),
    }
    QUAD_OF = {"armor": 0, "body": 1, "head": 1, "hair": 1, "hands": 1,
               "feet": 3, "boots": 3, "gloves": 2, "helmet": 2, "weapon": 2}
    QUAD_ORG = {0: (0.0, 0.5), 1: (0.5, 0.5), 2: (0.0, 0.0), 3: (0.5, 0.0)}
    INSET = 1.0 / 2048.0
    tile_items = list(TILES_UV.items())
    uv = me.uv_layers.get("UV_bake") or me.uv_layers.active
    quad_mats = []
    for qi in range(4):
        img = bpy.data.images.load(
            os.path.join(args["atlasdir"], "%s_atlas_q%d.png" % (variant, qi)))
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
    me.materials.clear()
    for m in quad_mats:
        me.materials.append(m)
    for poly in me.polygons:
        cu = cv = 0.0
        for li in poly.loop_indices:
            u, v = uv.data[li].uv
            cu += u
            cv += v
        cu /= len(poly.loop_indices)
        cv /= len(poly.loop_indices)
        quad = None
        for role, (ox, oy, sc) in tile_items:
            if ox - 1e-6 <= cu <= ox + sc + 1e-6 and oy - 1e-6 <= cv <= oy + sc + 1e-6:
                quad = QUAD_OF[role]
                qx, qy = QUAD_ORG[quad]
                break
        if quad is None:
            raise RuntimeError("face centroid outside all tiles")
        poly.material_index = quad
        for li in poly.loop_indices:
            u, v = uv.data[li].uv
            uu = min(max(u, 0.0), 1.0)
            vv = min(max(v, 0.0), 1.0)
            uv.data[li].uv = (2.0 * (uu - qx), 2.0 * (vv - qy))

    # CT19 scale pole: 1.8 m striped pole left of the fighter. It reads as a
    # measuring pole from every angle, never as a weapon.
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

    if arm.animation_data is None:
        arm.animation_data_create()
    arm.animation_data.action = bpy.data.actions["human_male_cast_main_hand_melee"]
    scene.frame_set(5)
    scene.render.filepath = os.path.join(outdir, variant + "_attack.png")
    bpy.ops.render.render(write_still=True)
    print("CT7-PREVIEW-ATTACK " + variant)


main()
