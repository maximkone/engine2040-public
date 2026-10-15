"""CT7 clips: export one GLB per curated action from human_male.blend."""
import bpy
import os
import sys

CLIPS = [
    "human_male_idle",
    "human_male_run",
    "human_male_jump",
    "human_male_death",
    "human_male_idle_combat_main_hand_melee",
    "human_male_idle_combat_unarmed",
    "human_male_cast_main_hand_melee",
    "human_male_cast_main_hand_melee_2",
    "human_male_cast_main_hand_melee_3",
    "human_male_cast_two_handed_melee",
    "human_male_cast_two_handed_melee_2",
    "human_male_cast_off_hand_shield",
    "human_male_cast_dual_wield_melee",
    "human_male_power_up",
]


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
    only = args.get("only")
    wanted = [only] if only else CLIPS
    os.makedirs(outdir, exist_ok=True)
    arm = bpy.data.objects["human_male"]
    # Drop every mesh/light so only the armature exports.
    for ob in list(bpy.data.objects):
        if ob != arm:
            bpy.data.objects.remove(ob, do_unlink=True)
    # Drop non-curated actions (deterministic animation set).
    for act in list(bpy.data.actions):
        if act.name not in wanted:
            bpy.data.actions.remove(act)
    missing = [c for c in wanted if c not in bpy.data.actions]
    if missing:
        raise RuntimeError("missing actions: " + ",".join(missing))
    if arm.animation_data is None:
        arm.animation_data_create()
    bpy.ops.object.select_all(action="DESELECT")
    arm.select_set(True)
    bpy.context.view_layer.objects.active = arm
    for name in sorted(wanted):
        arm.animation_data.action = bpy.data.actions[name]
        # Bake the exact frame range of the action.
        f0, f1 = int(bpy.data.actions[name].frame_range[0]), int(bpy.data.actions[name].frame_range[1])
        bpy.context.scene.frame_start = f0
        bpy.context.scene.frame_end = f1
        bpy.ops.export_scene.gltf(
            filepath=os.path.join(outdir, name + ".glb"),
            export_format="GLB",
            use_selection=True,
            export_yup=True,
            export_apply=False,
            export_animations=True,
            export_frame_range=True,
            export_force_sampling=True,
            export_optimize_animation_size=False,
            export_skins=False,
            export_morph=False,
            export_materials="NONE",
        )
        print("CT7-CLIP " + name + " frames=%d-%d" % (f0, f1))


main()
