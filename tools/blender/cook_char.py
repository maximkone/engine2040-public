# cook_char.py - headless Blender cook: BVH, FBX or glTF in, glTF out.
# Run by tools/cook-char.mjs, one Blender process per clip (Blender is heavy):
#   blender -b -P tools/blender/cook_char.py -- <in> <out.glb|out.gltf>
# Exit 0 wrote the file. Exit 2 bad arguments. Exit 3 the cook failed; the
# reason is one `E_COOK_*` line on stderr. Blender itself exits 0 after a
# script error, so every failure here ends in an explicit sys.exit.
# Never linked into the engine and never in the tick: cook-time only.
import os
import sys

try:
    import bpy
except ImportError:  # plain python (argument checks only)
    bpy = None

IMPORT_EXTS = (".bvh", ".fbx", ".gltf", ".glb")
EXPORT_EXTS = (".glb", ".gltf")
EXIT_BAD_ARGS = 2
EXIT_COOK_FAILED = 3


class CookError(Exception):
    """A cook failure with a stable code (the E_COOK_* token)."""

    def __init__(self, code, detail):
        # One line: Blender operator errors carry a whole traceback.
        flat = " ".join(str(detail).split())
        flat = flat if len(flat) <= 300 else "..." + flat[-297:]  # the error is at the tail
        super().__init__(f"{code} {flat}")
        self.code = code


def args_after_dashes(argv):
    # Blender keeps its own flags before `--`; ours come after it.
    return argv[argv.index("--") + 1:] if "--" in argv else []


def check_paths(args, exists=os.path.exists):
    # Returns (input, output) or raises CookError. Pure: no bpy needed.
    if len(args) != 2:
        raise CookError("E_COOK_USAGE", "need exactly: <in> <out.glb|out.gltf> after `--`")
    src, dst = args
    if not exists(src):
        raise CookError("E_COOK_INPUT", f"input not found: {src}")
    if os.path.splitext(src)[1].lower() not in IMPORT_EXTS:
        raise CookError("E_COOK_FORMAT", f"input must be one of {', '.join(IMPORT_EXTS)}: {src}")
    if os.path.splitext(dst)[1].lower() not in EXPORT_EXTS:
        raise CookError("E_COOK_FORMAT", f"output must be one of {', '.join(EXPORT_EXTS)}: {dst}")
    return src, dst


def import_clip(src):
    ext = os.path.splitext(src)[1].lower()
    if ext == ".bvh":
        # Blender takes the BVH frame time as the scene rate, so the glTF key
        # times match the file; the skeleton arrives as an armature plus action.
        bpy.ops.import_anim.bvh(
            filepath=src,
            rotate_mode="NATIVE",
            update_scene_fps=True,
            update_scene_duration=True,
        )
    elif ext == ".fbx":
        bpy.ops.import_scene.fbx(filepath=src)
    else:
        bpy.ops.import_scene.gltf(filepath=src)


def export_clip(dst):
    fmt = "GLB" if dst.lower().endswith(".glb") else "GLTF_SEPARATE"
    bpy.ops.export_scene.gltf(
        filepath=dst,
        check_existing=False,
        export_format=fmt,
        export_animations=True,
        export_cameras=False,
        export_lights=False,
    )


def cook(src, dst):
    # Empty factory scene so the default cube, camera and light never ship.
    bpy.ops.wm.read_factory_settings(use_empty=True)
    try:
        import_clip(src)
    except Exception as err:  # operators raise RuntimeError on a bad file
        raise CookError("E_COOK_IMPORT", f"{src}: {err}")
    objects = len(bpy.data.objects)
    if objects == 0:
        raise CookError("E_COOK_EMPTY", f"{src} imported no objects")
    actions = len(bpy.data.actions)
    parent = os.path.dirname(os.path.abspath(dst))
    os.makedirs(parent, exist_ok=True)
    try:
        export_clip(dst)
    except Exception as err:
        raise CookError("E_COOK_EXPORT", f"{dst}: {err}")
    if not os.path.exists(dst) or os.path.getsize(dst) == 0:
        raise CookError("E_COOK_NO_OUTPUT", f"export wrote nothing: {dst}")
    if dst.lower().endswith(".glb"):
        with open(dst, "rb") as handle:
            if handle.read(4) != b"glTF":
                raise CookError("E_COOK_NO_OUTPUT", f"not a GLB: {dst}")
    return objects, actions, os.path.getsize(dst)


def main(argv):
    try:
        src, dst = check_paths(args_after_dashes(argv))
    except CookError as err:
        sys.stderr.write(f"{err}\n")
        return EXIT_BAD_ARGS
    if bpy is None:
        sys.stderr.write("E_COOK_NO_BPY run this script with blender -b -P\n")
        return EXIT_COOK_FAILED
    try:
        objects, actions, size = cook(src, dst)
    except CookError as err:
        sys.stderr.write(f"{err}\n")
        return EXIT_COOK_FAILED
    sys.stdout.write(f"cook_char: objects={objects} actions={actions} bytes={size} out={dst}\n")
    return 0


if __name__ == "__main__":
    code = main(sys.argv)
    if code != 0:
        sys.exit(code)
