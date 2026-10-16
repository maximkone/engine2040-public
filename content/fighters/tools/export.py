"""CT7 pass 2: assign the atlas material, inset UVs, export the body GLB."""
import bpy
import sys

TILES_UV = {  # role -> (ox, oy, sc) in UV space (matches assemble.py)
    "armor": (0.0, 0.5, 0.5),
    "body": (0.5, 0.75, 0.25),
    "head": (0.75, 0.75, 0.25),
    "hair": (0.5, 0.5, 0.25),
    "hands": (0.75, 0.5, 0.25),
    "feet": (0.5, 0.25, 0.25),
    "boots": (0.75, 0.25, 0.25),
    "gloves": (0.0, 0.0, 0.25),
    "helmet": (0.25, 0.0, 0.25),
    # CT19: grafted weapons live in the Q2 free strip (atlas px x[0,512)
    # y[1024,1536)); they join quadrant 2, keeping 4 materials per character.
    "weapon": (0.0, 0.25, 0.25),
}
INSET = 1.0 / 2048.0  # one atlas texel toward the tile center


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
    arm = bpy.data.objects["human_male"]
    mesh_ob = bpy.data.objects["BODY_fighter"]
    me = mesh_ob.data

    # Inset UVs one texel toward their tile center (mip bleed guard).
    rects = [(ox, oy, ox + sc, oy + sc) for (ox, oy, sc) in TILES_UV.values()]
    uv = me.uv_layers.get("UV_bake") or me.uv_layers.active
    moved = 0
    for loop in me.loops:
        u, v = uv.data[loop.index].uv
        best = None
        for (x0, y0, x1, y1) in rects:
            if x0 - 1e-6 <= u <= x1 + 1e-6 and y0 - 1e-6 <= v <= y1 + 1e-6:
                cx, cy = (x0 + x1) * 0.5, (y0 + y1) * 0.5
                d = abs(u - cx) + abs(v - cy)
                if best is None or d < best[0]:
                    best = (d, x0, y0, x1, y1)
        if best is None:
            raise RuntimeError("UV outside all tiles: %s" % str((u, v)))
        _, x0, y0, x1, y1 = best
        nu = min(max(u, x0 + INSET), x1 - INSET)
        nv = min(max(v, y0 + INSET), y1 - INSET)
        uv.data[loop.index].uv = (nu, nv)
        moved += 1

    # Quadrant of each tile: armor->Q0, body/head/hair/hands->Q1,
    # gloves/helmet->Q2, feet/boots->Q3. Quadrant-local UVs + one material
    # per quadrant (4 materials per character: the content budget).
    QUAD_OF = {
        "armor": 0, "body": 1, "head": 1, "hair": 1, "hands": 1,
        "feet": 3, "boots": 3, "gloves": 2, "helmet": 2, "weapon": 2,
    }
    QUAD_ORG = {0: (0.0, 0.5), 1: (0.5, 0.5), 2: (0.0, 0.0), 3: (0.5, 0.0)}
    tile_items = list(TILES_UV.items())
    quad_mats = []
    for qi in range(4):
        img = bpy.data.images.load(args["atlas"].replace("_atlas.png", "_atlas_q%d.png" % qi))
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
        # Centroid of the face loops decides the tile, hence the quadrant.
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
            uv.data[li].uv = (2.0 * (u - qx), 2.0 * (v - qy))

    # Export the body (armature + joined mesh), no animations.
    bpy.ops.object.select_all(action="DESELECT")
    arm.select_set(True)
    mesh_ob.select_set(True)
    bpy.context.view_layer.objects.active = arm
    bpy.ops.export_scene.gltf(
        filepath=args["bodyglb"],
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=False,
        export_animations=False,
        export_skins=True,
        export_morph=False,
        export_materials="EXPORT",
        export_image_format="AUTO",
    )
    tris = 0
    for p in me.polygons:
        tris += len(p.vertices) - 2
    print("CT7-EXPORTED %s verts=%d tris=%d uvs=%d" % (variant, len(me.vertices), tris, moved))


main()
