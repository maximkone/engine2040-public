"""ART0 - render isometric sprite frames from a Blender scene, headless.

One camera ring, one light rig, transparent film, RA2-look numbers from
docs/plan/kam2-style-lock.md. No add-ons, no downloads, no paid rig.

Run from the repo root (the `--` separates our flags from Blender's):

  blender --background --factory-startup --python tools/kam2/render_frames.py -- --scene content/kam2/unit.swordsman.glb --out C:/tmp/kam2/swordsman --frames 8 --size 64
  blender --background --factory-startup --python tools/kam2/render_frames.py -- --self-test --out C:/tmp/kam2/selftest

Output: <out>/frame_NN.png (+ mask_NN.png with --mask), <out>/frames.json.
Frame NN is compass heading NN of the style-lock order S, SW, W, NW, N, NE, E, SE:
the model stands still and the camera walks the ring, so one PNG per facing.

Exit: 0 wrote the set - 1 render failed (one `E_ART ...` line on stderr) - 2 bad flags.
Blender exits 0 after a script error, so every failure ends in an explicit sys.exit.
"""

import argparse
import json
import math
import os
import sys
import tempfile

import bpy
from mathutils import Vector

# Style lock, docs/plan/kam2-style-lock.md. Change there first.
FACINGS = ["S", "SW", "W", "NW", "N", "NE", "E", "SE"]
PITCH_DEG = math.degrees(math.atan(0.5))  # 26.565, the 2:1 diamond
KEY_COLOUR = "#FFE8C4"
KEY_STRENGTH = 3.2
FILL_COLOUR = "#6E86A8"
FILL_STRENGTH = 0.55  # lock weight 0.35, lifted so the shade side reads instead of crushing
FILL_ELEVATION_DEG = 25.0
FILL_AZIMUTH_OFFSET_DEG = 100.0  # front-left, away from the key's front-right
AMBIENT_COLOUR = "#3A4250"
AMBIENT_STRENGTH = 0.5  # lock weight 0.15, lifted for a uniform-environment renderer
SUN_ELEVATION_DEG = 50.0
# 40 deg from camera right toward the camera: the key lights the faces the player sees.
# The lock writes this as -40 deg from camera right; the sign there assumes the opposite
# zero, and a key behind the unit leaves every front face in fill light only.
SUN_AZIMUTH_DEG = 40.0
SHADOW_COLOUR = "#1B1F24"
SHADOW_ALPHA = 0.35
FLOOR_EPSILON = 1.0e-4


def hex_to_linear(value):
    """#RRGGBB (sRGB) to Blender's linear RGBA tuple."""
    text = value.lstrip("#")
    rgb = []
    for i in (0, 2, 4):
        c = int(text[i : i + 2], 16) / 255.0
        rgb.append(c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4)
    return (rgb[0], rgb[1], rgb[2], 1.0)


def fail(message):
    sys.stderr.write("E_ART %s\n" % message)
    raise SystemExit(1)


def warn(message):
    sys.stderr.write("W_ART %s\n" % message)


def parse_args(argv):
    ours = list(argv)
    if "--" in ours:
        ours = ours[ours.index("--") + 1 :]
    parser = argparse.ArgumentParser(
        prog="render_frames.py",
        description="Render KAM2 isometric sprite frames from a .blend or .glb scene.",
    )
    parser.add_argument("--scene", help=".blend, .glb or .gltf source scene")
    parser.add_argument("--self-test", action="store_true", help="build a cube+cone scene and render it")
    parser.add_argument("--out", help="output folder (default: %%TEMP%%/kam2-art/<name>)")
    parser.add_argument("--frames", type=int, default=8, help="frames around the ring (default 8)")
    parser.add_argument("--size", type=int, default=64, help="square cell size in px (default 64)")
    parser.add_argument("--pitch", type=float, default=PITCH_DEG, help="camera pitch in degrees")
    parser.add_argument("--margin", type=float, default=0.06, help="framing margin, fraction of the span")
    parser.add_argument("--prefix", default="frame", help="frame file prefix (default frame)")
    parser.add_argument("--samples", type=int, default=48, help="Cycles samples (default 48)")
    parser.add_argument("--engine", choices=["cycles", "eevee", "workbench"], default="cycles")
    parser.add_argument("--mask", action="store_true", help="also render the team-colour mask pass")
    parser.add_argument("--no-shadow", action="store_true", help="skip the contact-shadow ellipse")
    parser.add_argument("--keep-lights", action="store_true", help="keep lights of a loaded scene")
    parser.add_argument("--frame", type=int, default=None, help="scene frame to render")
    parser.add_argument("--action", help="animation clip to play (glTF action name, e.g. walk)")
    parser.add_argument("--height-px", type=int, default=None, help="fit the upright body to N px")
    parser.add_argument("--span", type=float, default=None, help="fixed ortho span in world units")
    parser.add_argument("--ground-px", type=int, default=None, help="cell row where the ground pivot lands")
    parser.add_argument("--floor-z", type=float, default=None, help="ground plane z (default: pose lowest point)")
    parser.add_argument("--yaw", type=float, default=None, help="one fixed yaw instead of a ring, e.g. 135")
    parser.add_argument("--pivot-x", type=float, default=None, help="world x of the ground pivot")
    parser.add_argument("--pivot-y", type=float, default=None, help="world y of the ground pivot")
    args = parser.parse_args(ours)
    if not args.self_test and not args.scene:
        parser.error("--scene or --self-test is required")
    if args.frames < 1:
        parser.error("--frames must be >= 1")
    if args.size < 8:
        parser.error("--size must be >= 8")
    if args.height_px is not None and args.height_px >= args.size:
        parser.error("--height-px must be smaller than --size (the cell needs room for the shadow)")
    if args.span is not None and args.span <= 0:
        parser.error("--span must be > 0")
    if args.ground_px is not None and not 0 <= args.ground_px < args.size:
        parser.error("--ground-px must be inside the cell")
    if (args.pivot_x is None) != (args.pivot_y is None):
        parser.error("--pivot-x and --pivot-y go together")
    if args.yaw is not None:
        if not 0.0 <= args.yaw < 360.0:
            parser.error("--yaw must be in [0, 360)")
        args.frames = 1
    return args


def reset_empty():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def apply_material(obj, colour, metallic=0.0, roughness=0.5):
    mat = bpy.data.materials.new("%s mat" % obj.name)
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    if bsdf is not None:
        bsdf.inputs["Base Color"].default_value = hex_to_linear(colour)
        bsdf.inputs["Metallic"].default_value = metallic
        bsdf.inputs["Roughness"].default_value = roughness
    obj.data.materials.append(mat)
    return mat


def build_self_test():
    """A coloured cube hull and a cone roof: enough to prove ring, light and alpha."""
    reset_empty()
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=(0.0, 0.0, 0.3))
    hull = bpy.context.object
    hull.name = "KAM2 selftest hull"
    hull.scale = (1.2, 0.9, 0.6)
    apply_material(hull, "#9AA3AB", metallic=0.35, roughness=0.4)

    bpy.ops.mesh.primitive_cone_add(
        vertices=16, radius1=0.5, radius2=0.0, depth=0.65, location=(0.0, 0.0, 0.925)
    )
    roof = bpy.context.object
    roof.name = "KAM2 selftest roof"
    apply_material(roof, "#A33B2E", metallic=0.0, roughness=0.6)


def load_scene(path):
    ext = os.path.splitext(path)[1].lower()
    if ext == ".blend":
        bpy.ops.wm.open_mainfile(filepath=path)
    elif ext in (".glb", ".gltf"):
        reset_empty()
        bpy.ops.import_scene.gltf(filepath=path)
    else:
        fail("scene must be .blend, .glb or .gltf: %s" % path)


def hide_foreign_rig(scene, keep_lights):
    """Our camera and sun own the look; nothing of a loaded file may outshine them."""
    for ob in list(bpy.data.objects):
        if ob.type == "CAMERA":
            ob.hide_render = True
        elif ob.type == "LIGHT" and not keep_lights:
            ob.hide_render = True


def pick_action(name):
    """Find a clip by name; glTF may prefix it (Armature|walk)."""
    exact = [a for a in bpy.data.actions if a.name == name]
    if exact:
        return exact[0]
    tail = [a for a in bpy.data.actions if a.name.rsplit("|", 1)[-1] == name]
    if len(tail) == 1:
        return tail[0]
    known = sorted(a.name for a in bpy.data.actions)
    fail("action %r not found; clips: %s" % (name, ", ".join(known) or "none"))


def activate_action(scene, action, frame):
    """Bind one clip to every object it drives and park the rest of the NLA stack.

    A glTF action carries one slot per animated node; an object takes the slot named
    after itself. Objects without a slot lose their action, so no other clip leaks in.
    """
    driven = 0
    for ob in bpy.data.objects:
        data = ob.animation_data
        if data is None:
            continue
        for track in data.nla_tracks:
            track.mute = True
        slot = None
        for candidate in action.slots:
            if candidate.name_display == ob.name:
                slot = candidate
                break
        if slot is None:
            data.action = None
            continue
        data.action = action
        try:
            data.action_slot = slot
        except Exception as err:
            warn("slot %s on %s: %s" % (slot.name_display, ob.name, err))
            continue
        driven += 1
    if driven == 0:
        fail("action %r drives no object" % action.name)
    scene.frame_set(frame)


def visible_bounds(scene):
    """World-space bounds of every renderable mesh, evaluated (modifiers, shape keys)."""
    deps = bpy.context.evaluated_depsgraph_get()
    lo = Vector((math.inf, math.inf, math.inf))
    hi = Vector((-math.inf, -math.inf, -math.inf))
    points = 0
    for ob in scene.objects:
        if ob.hide_render or ob.type not in {"MESH", "CURVE", "SURFACE", "META", "FONT"}:
            continue
        try:
            evaluated = ob.evaluated_get(deps)
            mesh = evaluated.to_mesh()
        except Exception as err:  # a broken object must not hide the rest
            warn("skip %s: %s" % (ob.name, err))
            continue
        if mesh is None:
            continue
        matrix = evaluated.matrix_world
        for vertex in mesh.vertices:
            p = matrix @ vertex.co
            lo.x = min(lo.x, p.x)
            lo.y = min(lo.y, p.y)
            lo.z = min(lo.z, p.z)
            hi.x = max(hi.x, p.x)
            hi.y = max(hi.y, p.y)
            hi.z = max(hi.z, p.z)
            points += 1
        evaluated.to_mesh_clear()
    if points == 0:
        fail("scene has no visible mesh geometry")
    return lo, hi


def camera_basis(yaw_deg, pitch_deg):
    """Unit vectors: scene->camera, view forward, screen right, screen up."""
    yaw = math.radians(yaw_deg)
    pitch = math.radians(pitch_deg)
    to_cam = Vector(
        (
            math.sin(yaw) * math.cos(pitch),
            math.cos(yaw) * math.cos(pitch),
            math.sin(pitch),
        )
    )
    forward = -to_cam
    right = forward.cross(Vector((0.0, 0.0, 1.0))).normalized()
    up = right.cross(forward).normalized()
    return to_cam, forward, right, up


def frame_plan(lo, hi, frames, pitch, size, margin, height_px=None, span=None, ground_px=None, floor_z=None, pivot_x=None, pivot_y=None, yaw=None):
    """One span and one pivot pixel shared by every facing, so frames stack exactly.

    Heading 0 is south (toward the camera). The model never turns; the camera walks
    the ring, which gives the same picture as a fixed camera and a turning unit.
    """
    floor = lo.z if floor_z is None else floor_z
    pivot = Vector(
        (
            (lo.x + hi.x) * 0.5 if pivot_x is None else pivot_x,
            (lo.y + hi.y) * 0.5 if pivot_y is None else pivot_y,
            floor,
        )
    )
    corners = [
        Vector((x, y, z))
        for x in (lo.x, hi.x)
        for y in (lo.y, hi.y)
        for z in (min(lo.z, floor), hi.z)
    ]
    us, vs = [], []
    for i in range(frames):
        yaw_i = yaw if yaw is not None else 180.0 - i * 360.0 / frames
        _, _, right, up = camera_basis(yaw_i, pitch)
        for corner in corners:
            rel = corner - pivot
            us.append(rel.dot(right))
            vs.append(rel.dot(up))
    u_mid = (min(us) + max(us)) * 0.5
    v_mid = (min(vs) + max(vs)) * 0.5
    if pivot_x is not None:
        # An explicit pivot is a contract: the camera aims at it, not at the alpha box,
        # so every state of one asset lands on the same pivot pixel.
        u_mid = 0.0
        v_mid = 0.0
    u_span = max(max(us) - min(us), 1.0e-4)
    v_span = max(max(vs) - min(vs), 1.0e-4)
    if span is not None:
        span_used = span
    elif height_px is not None:
        # Sprite height: the upright body (floor to crown) at this pitch fills height_px.
        body = (hi.z - floor) * math.cos(math.radians(pitch))
        span_used = max(body, 1.0e-4) * size / height_px
    else:
        span_used = max(u_span, v_span) * (1.0 + margin)
    # Pixel of the ground pivot; constant for every facing by construction.
    pixel = [size * 0.5 - u_mid / span_used * size, size * 0.5 + v_mid / span_used * size]
    v_aim = v_mid
    if ground_px is not None:
        # Put the ground pivot on ground_px, whatever the pose does above it.
        v_aim = (ground_px - size * 0.5) * span_used / size
        pixel[1] = float(ground_px)
    return {
        "pivot": pivot,
        "span": span_used,
        "u_mid": u_mid,
        "v_mid": v_mid,
        "v_aim": v_aim,
        "pixel": tuple(pixel),
        "lo": lo,
        "hi": hi,
    }



def make_camera(scene, plan, distance):
    data = bpy.data.cameras.new("KAM2 camera")
    data.type = "ORTHO"
    data.ortho_scale = plan["span"]
    data.clip_start = 0.01
    data.clip_end = distance * 4.0
    ob = bpy.data.objects.new("KAM2 camera", data)
    scene.collection.objects.link(ob)
    scene.camera = ob
    return ob


def place_camera(cam, plan, yaw_deg, pitch_deg, distance):
    to_cam, forward, right, up = camera_basis(yaw_deg, pitch_deg)
    aim = plan["pivot"] + right * plan["u_mid"] + up * plan["v_aim"]
    cam.location = aim + to_cam * distance
    cam.rotation_mode = "QUATERNION"
    cam.rotation_quaternion = forward.to_track_quat("-Z", "Y")


def make_sun(scene, name, colour, strength, angle):
    data = bpy.data.lights.new(name, "SUN")
    data.color = hex_to_linear(colour)[:3]
    data.energy = strength
    data.angle = angle
    ob = bpy.data.objects.new(name, data)
    scene.collection.objects.link(ob)
    return ob


def aim_sun(sun, yaw_deg, pitch_deg, colour, strength, elevation_deg, azimuth_deg):
    """Aim from a screen-space azimuth, so shading stays put while the camera walks."""
    to_cam, _, right, _ = camera_basis(yaw_deg, pitch_deg)
    horizontal = Vector((to_cam.x, to_cam.y, 0.0))
    if horizontal.length < 1.0e-6:
        horizontal = Vector((0.0, -1.0, 0.0))
    horizontal.normalize()
    azimuth = math.radians(azimuth_deg)
    elevation = math.radians(elevation_deg)
    to_sun = (
        right * math.cos(azimuth) + horizontal * math.sin(azimuth)
    ) * math.cos(elevation) + Vector((0.0, 0.0, 1.0)) * math.sin(elevation)
    sun.data.color = hex_to_linear(colour)[:3]
    sun.data.energy = strength
    sun.rotation_mode = "QUATERNION"
    sun.rotation_quaternion = (-to_sun.normalized()).to_track_quat("-Z", "Y")


def add_contact_shadow(scene, plan):
    """Soft black ellipse at 35% under the model, the style-lock contact shadow."""
    try:
        width = max(plan["hi"].x - plan["lo"].x, 1.0e-3)
        height = max(plan["hi"].y - plan["lo"].y, 1.0e-3)
        z = plan["pivot"].z + max(plan["hi"].z - plan["lo"].z, 1.0e-3) * FLOOR_EPSILON
        bpy.ops.mesh.primitive_circle_add(
            vertices=48,
            radius=1.0,
            fill_type="NGON",
            location=(plan["pivot"].x, plan["pivot"].y, z),
        )
        ob = bpy.context.object
        ob.name = "KAM2 contact shadow"
        ob.scale = (width * 0.62 + 0.02, height * 0.62 + 0.02, 1.0)

        mat = bpy.data.materials.new("KAM2 contact shadow")
        tree = mat.node_tree
        tree.nodes.clear()
        out = tree.nodes.new("ShaderNodeOutputMaterial")
        mix = tree.nodes.new("ShaderNodeMixShader")
        transparent = tree.nodes.new("ShaderNodeBsdfTransparent")
        emit = tree.nodes.new("ShaderNodeEmission")
        emit.inputs["Color"].default_value = hex_to_linear(SHADOW_COLOUR)
        emit.inputs["Strength"].default_value = 1.0
        coord = tree.nodes.new("ShaderNodeTexCoord")
        gradient = tree.nodes.new("ShaderNodeTexGradient")
        gradient.gradient_type = "SPHERICAL"
        mul = tree.nodes.new("ShaderNodeMath")
        mul.operation = "MULTIPLY"
        mul.inputs[1].default_value = SHADOW_ALPHA
        tree.links.new(coord.outputs["Object"], gradient.inputs["Vector"])
        tree.links.new(gradient.outputs["Fac"], mul.inputs[0])
        tree.links.new(mul.outputs[0], mix.inputs["Fac"])
        tree.links.new(transparent.outputs[0], mix.inputs[1])
        tree.links.new(emit.outputs[0], mix.inputs[2])
        tree.links.new(mix.outputs[0], out.inputs["Surface"])
        ob.data.materials.append(mat)
        ob.visible_shadow = False
        return ob
    except Exception as err:
        warn("contact shadow skipped: %s" % err)
        return None


def setup_render(scene, args):
    if args.engine == "cycles":
        scene.render.engine = "CYCLES"
        scene.cycles.device = "CPU"
        scene.cycles.samples = args.samples
        scene.cycles.use_denoising = True
        scene.cycles.use_adaptive_sampling = True
        # Fixed seed: a rebuild of the same model must write the same bytes.
        scene.cycles.seed = 0
        scene.cycles.use_animated_seed = False
    elif args.engine == "eevee":
        scene.render.engine = "BLENDER_EEVEE"
    else:
        scene.render.engine = "BLENDER_WORKBENCH"

    scene.render.resolution_x = args.size
    scene.render.resolution_y = args.size
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = True
    scene.render.dither_intensity = 0.0
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.render.image_settings.color_depth = "8"
    scene.render.image_settings.compression = 15
    try:
        scene.view_settings.view_transform = "Standard"  # flat, saturated sprite colours
        scene.view_settings.look = "None"
        scene.view_settings.exposure = 0.0
        scene.view_settings.gamma = 1.0
    except Exception as err:
        warn("colour management: %s" % err)


def setup_world(scene):
    world = bpy.data.worlds.new("KAM2 ambient")
    background = world.node_tree.nodes.get("Background")
    if background is not None:
        background.inputs["Color"].default_value = hex_to_linear(AMBIENT_COLOUR)
        background.inputs["Strength"].default_value = AMBIENT_STRENGTH
    scene.world = world


def make_mask_material():
    mat = bpy.data.materials.new("KAM2 team mask")
    tree = mat.node_tree
    tree.nodes.clear()
    out = tree.nodes.new("ShaderNodeOutputMaterial")
    emit = tree.nodes.new("ShaderNodeEmission")
    emit.inputs["Color"].default_value = (1.0, 1.0, 1.0, 1.0)
    emit.inputs["Strength"].default_value = 1.0
    tree.links.new(emit.outputs[0], out.inputs["Surface"])
    return mat


def render_set(scene, cam, plan, key, fill, args, out_dir, shadow):
    names, mask_names = [], []
    mask_mat = make_mask_material() if args.mask else None
    view_layer = scene.view_layers[0]
    distance = plan["span"] * 8.0
    for i in range(args.frames):
        yaw = args.yaw if args.yaw is not None else 180.0 - i * 360.0 / args.frames
        _, _, right, _ = camera_basis(yaw, args.pitch)
        place_camera(cam, plan, yaw, args.pitch, distance)
        if shadow is not None:
            # 2 px along the sun in screen space, the style-lock contact-shadow nudge.
            nudge = right * (2.0 / args.size * plan["span"])
            shadow.location = plan["pivot"] + nudge
            shadow.location.z = plan["pivot"].z + max(plan["hi"].z - plan["lo"].z, 1.0e-3) * FLOOR_EPSILON
        aim_sun(key, yaw, args.pitch, KEY_COLOUR, KEY_STRENGTH, SUN_ELEVATION_DEG, SUN_AZIMUTH_DEG)
        aim_sun(fill, yaw, args.pitch, FILL_COLOUR, FILL_STRENGTH, FILL_ELEVATION_DEG, SUN_AZIMUTH_DEG + FILL_AZIMUTH_OFFSET_DEG)

        name = "%s_%02d.png" % (args.prefix, i)
        scene.render.filepath = os.path.join(out_dir, name)
        bpy.ops.render.render(write_still=True)
        names.append(name)

        if mask_mat is not None:
            view_layer.material_override = mask_mat
            if shadow is not None:
                shadow.hide_render = True
            mask_name = "mask_%02d.png" % i
            scene.render.filepath = os.path.join(out_dir, mask_name)
            bpy.ops.render.render(write_still=True)
            if shadow is not None:
                shadow.hide_render = False
            view_layer.material_override = None
            mask_names.append(mask_name)

        sys.stdout.write("KAM2 ART frame %d/%d %s\n" % (i + 1, args.frames, name))
        sys.stdout.flush()
    return names, mask_names


def main():
    args = parse_args(sys.argv[1:])
    out_dir = os.path.abspath(
        args.out
        or os.path.join(
            tempfile.gettempdir(), "kam2-art", "selftest" if args.self_test else "frames"
        )
    )

    if args.self_test:
        build_self_test()
        source = "self-test"
    else:
        scene_path = os.path.abspath(args.scene)
        if not os.path.exists(scene_path):
            fail("scene not found: %s" % scene_path)
        load_scene(scene_path)
        source = scene_path

    scene = bpy.context.scene
    if args.action:
        activate_action(scene, pick_action(args.action), args.frame if args.frame is not None else 0)
    elif args.frame is not None:
        scene.frame_set(args.frame)
    os.makedirs(out_dir, exist_ok=True)

    setup_render(scene, args)
    setup_world(scene)
    hide_foreign_rig(scene, args.keep_lights)

    lo, hi = visible_bounds(scene)
    plan = frame_plan(
        lo,
        hi,
        args.frames,
        args.pitch,
        args.size,
        args.margin,
        height_px=args.height_px,
        span=args.span,
        ground_px=args.ground_px,
        floor_z=args.floor_z,
        pivot_x=args.pivot_x,
        pivot_y=args.pivot_y,
        yaw=args.yaw,
    )
    shadow = None if args.no_shadow else add_contact_shadow(scene, plan)

    cam = make_camera(scene, plan, plan["span"] * 8.0)
    key = make_sun(scene, "KAM2 key", KEY_COLOUR, KEY_STRENGTH, 0.009)
    fill = make_sun(scene, "KAM2 fill", FILL_COLOUR, FILL_STRENGTH, 1.0)

    names, mask_names = render_set(scene, cam, plan, key, fill, args, out_dir, shadow)

    pivot_px = (round(plan["pixel"][0], 3), round(plan["pixel"][1], 3))
    meta = {
        "generator": "tools/kam2/render_frames.py",
        "source": source,
        "engine": args.engine,
        "samples": args.samples if args.engine == "cycles" else None,
        "action": args.action,
        "scene_frame": scene.frame_current,
        "height_px": args.height_px,
        "ground_px": args.ground_px,
        "floor_z": args.floor_z,
        "frames": names,
        "count": len(names),
        "mask_frames": mask_names,
        "width": args.size,
        "height": args.size,
        "pitch_deg": round(args.pitch, 4),
        "yaw_deg": None if args.yaw is None else round(args.yaw, 4),
        "yaw_step_deg": None if args.yaw is not None else round(360.0 / args.frames, 4),
        "facings": (
            FACINGS
            if args.frames == len(FACINGS)
            else ["yaw_%03d" % round(args.yaw)]
            if args.yaw is not None
            else ["yaw_%02d" % i for i in range(args.frames)]
        ),
        "pivot": [int(round(pivot_px[0])), int(round(pivot_px[1]))],
        "pivot_px": list(pivot_px),
        "pivot_world": [round(plan["pivot"].x, 6), round(plan["pivot"].y, 6)],
        "ortho_scale": round(plan["span"], 6),
        "bounds": {
            "min": [round(v, 6) for v in (lo.x, lo.y, lo.z)],
            "max": [round(v, 6) for v in (hi.x, hi.y, hi.z)],
        },
    }
    meta_path = os.path.join(out_dir, "frames.json")
    with open(meta_path, "w", encoding="utf-8") as handle:
        json.dump(meta, handle, indent=2)
        handle.write("\n")

    sys.stdout.write("KAM2 ART OK frames=%d out=%s\n" % (len(names), out_dir))
    sys.stdout.flush()


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception:
        import traceback

        traceback.print_exc()
        sys.stderr.write("E_ART render failed\n")
        sys.exit(1)
