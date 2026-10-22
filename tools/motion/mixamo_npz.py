"""A Mixamo FBX clip as a Kimodo NPZ, so motion_bake bakes it like a teacher clip.

Run (Blender, headless):
  blender -b --factory-startup --python-exit-code 1 -P tools/motion/mixamo_npz.py -- \
      <clip.fbx> <out.npz> --offsets <soma77_offsets.npy>

Why it is sound: Kimodo's SOMA-77 data is strict forward kinematics with
identity rest rotations (a joint's offset from its parent, read in the
parent's frame, is constant across every frame to 4e-7, and each global
rotation is its parent's times its local to 1e-16). Its rest pose is a T-pose,
Y up, facing +Z, the left arm along +X. A Mixamo FBX downloaded "with skin, T-pose"
has the same rest shape once Blender's Z-up, facing -Y frame is turned to Y-up
(x, y, z) -> (x, z, -y). So each mapped bone's world rotation relative to its
own rest, D = W(t) W0^-1, is the world rotation the SOMA bone takes; a SOMA
joint with no Mixamo bone follows its parent. Positions come from forward
kinematics on the true SOMA offsets (passed in, measured from a Kimodo clip),
so rotations and positions agree by construction, and the root travels as the
Mixamo hips do, scaled by the leg-length ratio.

Writes local_rot_mats (N,77,3,3), global_rot_mats, posed_joints (N,77,3),
root_positions (N,3), fps, in SOMA-77 order (engine_assets::soma::SOMA77).
"""

import os
import sys

import bpy
import numpy as np
from mathutils import Matrix

EXIT_USAGE = 2
EXIT_FAIL = 3

# SOMA-77 in the engine's order, with parents (crates/assets/src/soma.rs).
SOMA77 = [
    ("hips", None), ("spine1", "hips"), ("spine2", "spine1"), ("chest", "spine2"),
    ("neck1", "chest"), ("neck2", "neck1"), ("head", "neck2"), ("headend", "head"),
    ("jaw", "head"), ("lefteye", "head"), ("righteye", "head"),
]
for side in ("left", "right"):
    SOMA77 += [(f"{side}shoulder", "chest"), (f"{side}arm", f"{side}shoulder"),
               (f"{side}forearm", f"{side}arm"), (f"{side}hand", f"{side}forearm")]
    SOMA77 += [(f"{side}handthumb1", f"{side}hand"), (f"{side}handthumb2", f"{side}handthumb1"),
               (f"{side}handthumb3", f"{side}handthumb2"), (f"{side}handthumbend", f"{side}handthumb3")]
    for finger in ("index", "middle", "ring", "pinky"):
        base = f"{side}hand{finger}"
        SOMA77 += [(f"{base}1", f"{side}hand"), (f"{base}2", f"{base}1"), (f"{base}3", f"{base}2"),
                   (f"{base}4", f"{base}3"), (f"{base}end", f"{base}4")]
for side in ("left", "right"):
    SOMA77 += [(f"{side}leg", "hips"), (f"{side}shin", f"{side}leg"), (f"{side}foot", f"{side}shin"),
               (f"{side}toebase", f"{side}foot"), (f"{side}toeend", f"{side}toebase")]
assert len(SOMA77) == 77

# Mixamo bone (prefix stripped) -> SOMA joint. The Mixamo finger 1 is the
# proximal phalanx at the knuckle; SOMA's finger 1 sits in the palm, so the
# phalanges take 2-4. Mixamo's thumb 1 starts in the palm like SOMA's.
MIXAMO_TO_SOMA = {"Hips": "hips", "Spine": "spine1", "Spine1": "spine2", "Spine2": "chest",
                  "Neck": "neck1", "Head": "head"}
for mx, so in (("Left", "left"), ("Right", "right")):
    MIXAMO_TO_SOMA.update({f"{mx}Shoulder": f"{so}shoulder", f"{mx}Arm": f"{so}arm",
                           f"{mx}ForeArm": f"{so}forearm", f"{mx}Hand": f"{so}hand",
                           f"{mx}UpLeg": f"{so}leg", f"{mx}Leg": f"{so}shin",
                           f"{mx}Foot": f"{so}foot", f"{mx}ToeBase": f"{so}toebase"})
    for i in (1, 2, 3):
        MIXAMO_TO_SOMA[f"{mx}HandThumb{i}"] = f"{so}handthumb{i}"
        for finger in ("Index", "Middle", "Ring", "Pinky"):
            MIXAMO_TO_SOMA[f"{mx}Hand{finger}{i}"] = f"{so}hand{finger.lower()}{i + 1}"

# Blender Z-up, facing -Y  ->  Kimodo Y-up, facing +Z.
TO_YUP = np.array([[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]])


def arg(name, fallback=None):
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if name in argv and argv.index(name) + 1 < len(argv):
        return argv[argv.index(name) + 1]
    return fallback


def rot3(m4):
    """The rotation of a 4x4 world matrix, scale removed (Mixamo rigs carry 0.01)."""
    m = np.array(m4.to_3x3())
    for c in range(3):
        m[:, c] /= max(np.linalg.norm(m[:, c]), 1e-12)
    u, _, vt = np.linalg.svd(m)
    r = u @ vt
    if np.linalg.det(r) < 0:
        u[:, -1] *= -1
        r = u @ vt
    return r


def strip(name):
    return name.split(":")[-1]


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(args) < 2:
        sys.stderr.write("usage: mixamo_npz.py -- <clip.fbx> <out.npz> --offsets <soma77_offsets.npy>\n")
        return EXIT_USAGE
    src, out = args[0], args[1]
    offsets = np.load(arg("--offsets", r"C:\forge-media\kimodo\soma77_offsets.npy"))
    if offsets.shape != (77, 3):
        raise RuntimeError(f"offsets must be (77, 3), got {offsets.shape}")

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.fbx(filepath=src, automatic_bone_orientation=False)
    arms = [o for o in bpy.context.scene.objects if o.type == "ARMATURE"]
    if len(arms) != 1:
        raise RuntimeError(f"expected one armature, found {len(arms)}")
    arm = arms[0]
    action = arm.animation_data.action if arm.animation_data else None
    if action is None:
        raise RuntimeError("the FBX carries no animation")
    first, last = (int(round(v)) for v in action.frame_range)
    scene = bpy.context.scene
    fps = scene.render.fps / scene.render.fps_base

    bones = {strip(b.name): b.name for b in arm.data.bones}
    missing = [m for m in MIXAMO_TO_SOMA if m not in bones]
    core = [m for m in missing if "Hand" not in m or m.endswith("Hand")]
    if core:
        raise RuntimeError(f"Mixamo bones missing: {core}")

    names = [n for n, _ in SOMA77]
    index = {n: i for i, n in enumerate(names)}
    parent = [index[p] if p else -1 for _, p in SOMA77]
    driver = {so: bones[mx] for mx, so in MIXAMO_TO_SOMA.items() if mx in bones}

    world = arm.matrix_world
    rest = {so: rot3(world @ arm.data.bones[b].matrix_local) for so, b in driver.items()}

    # Root scale: SOMA's leg length over Mixamo's, so the hips ride at SOMA height.
    def chain(points):
        return sum(np.linalg.norm(points[i + 1] - points[i]) for i in range(len(points) - 1))
    mx_leg = chain([np.array(world @ arm.data.bones[bones[b]].head_local)
                    for b in ("LeftUpLeg", "LeftLeg", "LeftFoot")])
    so_leg = np.linalg.norm(offsets[index["leftshin"]]) + np.linalg.norm(offsets[index["leftfoot"]])
    scale = so_leg / max(mx_leg, 1e-9)

    frames = list(range(first, last + 1))
    n = len(frames)
    local = np.zeros((n, 77, 3, 3))
    glob = np.zeros((n, 77, 3, 3))
    posed = np.zeros((n, 77, 3))
    roots = np.zeros((n, 3))
    hips_bone = bones["Hips"]
    for k, frame in enumerate(frames):
        scene.frame_set(frame)
        for j in range(77):
            so = names[j]
            if so in driver:
                w = rot3(world @ arm.pose.bones[driver[so]].matrix)
                d = w @ rest[so].T
                g = TO_YUP @ d @ TO_YUP.T
            else:
                g = glob[k, parent[j]] if parent[j] >= 0 else np.eye(3)
            glob[k, j] = g
            local[k, j] = g if parent[j] < 0 else glob[k, parent[j]].T @ g
        hip = np.array(world @ arm.pose.bones[hips_bone].head)
        roots[k] = TO_YUP @ hip * scale
        for j in range(77):
            if parent[j] < 0:
                posed[k, j] = roots[k]
            else:
                posed[k, j] = posed[k, parent[j]] + glob[k, parent[j]] @ offsets[j]

    # The feet stand on y = 0 at the first frame, like a Kimodo clip.
    feet = [index[f] for f in ("lefttoeend", "righttoeend", "leftfoot", "rightfoot")]
    lift = posed[0, feet, 1].min()
    posed[:, :, 1] -= lift
    roots[:, 1] -= lift

    np.savez(out, local_rot_mats=local, global_rot_mats=glob, posed_joints=posed,
             root_positions=roots, fps=np.array(float(fps)))
    print(f"MIXAMO_NPZ frames={n} fps={fps:g} scale={scale:.4f} driven={len(driver)} "
          f"skipped_fingers={len(missing)} out={out}")
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except Exception as err:  # noqa: BLE001 - one line for the driver
        sys.stderr.write(f"E_MIXAMO {type(err).__name__}: {err}\n")
        code = EXIT_FAIL
    if code:
        sys.exit(code)
