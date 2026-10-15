"""Scene code the MCP client sends as an `execute_blender_code` step.

Builds `CT4_Crate`: a 0.5 m beveled crate prop with one PBR material,
smart-UV-projected and triangulated, selected for GLB export. Engine
space follows docs/plan/content.md: meters, +Y up on export (the
exporter flag handles the Blender +Z-up conversion).

Deterministic: fixed dimensions, fixed bevel segments, no randomness,
so the same step gives the same mesh bytes on every run. Safe-mode
clean: imports bpy only, no open/os/network.
"""
import bpy

# Wipe the default scene (cube, light, camera): name-based, order-free.
for name in ("Cube", "Light", "Camera"):
    obj = bpy.data.objects.get(name)
    if obj is not None:
        bpy.data.objects.remove(obj, do_unlink=True)

bpy.ops.mesh.primitive_cube_add(size=1.0, location=(0.0, 0.0, 0.25))
crate = bpy.context.active_object
crate.name = "CT4_Crate"
crate.scale = (0.5, 0.5, 0.5)
bpy.ops.object.transform_apply(scale=True)

bevel = crate.modifiers.new(name="CT4_Bevel", type="BEVEL")
bevel.width = 0.02
bevel.segments = 2
bpy.ops.object.modifier_apply(modifier=bevel.name)

bpy.ops.object.mode_set(mode="EDIT")
bpy.ops.mesh.select_all(action="SELECT")
bpy.ops.uv.smart_project(angle_limit=66.0, island_margin=0.02)
bpy.ops.mesh.quads_convert_to_tris()
bpy.ops.object.mode_set(mode="OBJECT")

mat = bpy.data.materials.new(name="CT4_Mat")
mat.use_nodes = True
bsdf = mat.node_tree.nodes.get("Principled BSDF")
bsdf.inputs["Base Color"].default_value = (0.55, 0.36, 0.2, 1.0)
bsdf.inputs["Roughness"].default_value = 0.8
crate.data.materials.append(mat)

bpy.ops.object.select_all(action="DESELECT")
crate.select_set(True)
bpy.context.view_layer.objects.active = crate

mesh = crate.data
print(
    f"built {crate.name}: verts={len(mesh.vertices)} "
    f"loops={len(mesh.loops)} polys={len(mesh.polygons)} "
    f"mats={len(mesh.materials)} selected={crate.select_get()}"
)
