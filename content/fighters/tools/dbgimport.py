import bpy
import sys
argv = sys.argv[sys.argv.index("--") + 1:]
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
bpy.ops.import_scene.gltf(filepath=argv[0])
for o in bpy.data.objects:
    print("CT7-OBJ", o.type, o.name)
print("CT7-ACTIONS", [a.name for a in bpy.data.actions])
