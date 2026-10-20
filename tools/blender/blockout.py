"""Parametric blockout generator: JSON spec in, three PNGs out.

Builds a humanoid blockout from primitives only (no imported assets), so the
20 class blockouts come from data instead of 20 hand-made scripts. Renders the
two silhouette tests from content/design/faction-war-style.md plus one grey
studio view.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/blockout.py -- <spec.json> <outdir>

Writes into <outdir>:
  silhouette_big.png   flat black on white, orthographic 3/4 front
  silhouette_30m.png   same camera, 30 m field, the figure about 90 px tall
  grey_studio.png      studio light, mid grey, 3/4 view

and prints one JSON line on stdout with the measured silhouette metrics:
  {"name":..., "height_px":..., "shoulder_px":..., "bbox_fill":...}

Spec keys (all optional, defaults are a 1.8 m 7-head human):
  name, height_m, heads, shoulder_span, torso_depth, chest_w, waist_w,
  hip_w, leg_len, arm_len, hand, head_size, bulk, stance_deg, attachments

Attachment kinds:
  arm_cannon {side, length, radius, angle_deg}  barrel over the shoulder
  shield     {side}                             plate on the forearm
  staff      {side}                             pole in the hand
  bow        {side}                             arc and string in the hand
  quiver                                       tube of arrows on the back
  cape                                        cloth behind the figure

Side "r" is the figure's right hand. The figure faces -Y, so its right side
is world -X, which is also the side the camera looks from.

Set BLOCKOUT_FORCE_TRANSPARENT=1 to take the film_transparent fallback path
even when the world background already works. It is used to test that path.
"""

import json
import math
import os
import sys
import warnings

import bpy
from mathutils import Matrix, Vector
from bpy_extras.object_utils import world_to_camera_view

try:
    import numpy as np
except ImportError:  # pragma: no cover - Blender ships numpy
    np = None

EXIT_USAGE = 2
EXIT_FAIL = 3

SIL_BIG = "silhouette_big.png"
SIL_30M = "silhouette_30m.png"
GREY_STUDIO = "grey_studio.png"

RES_BIG = (800, 1000)          # portrait: the big silhouette
TARGET_30M_PX = 90.0           # the figure is about this tall in the 30 m test
FIELD_30M_M = 30.0             # the 30 m read test frame is this wide
RES_30M_FALLBACK = (1280, 720)  # used when the figure is too small/large

CAM_AZ_DEG = -35.0             # 3/4 front, looking at the figure's right side
CAM_EL_DEG = 12.0              # a little above the middle
CAM_DIST = 6.0
MARGIN = 0.06                  # framing margin around the figure

INK_SRGB = 0.5                 # a pixel is ink when it is darker than this

DEFAULTS = {
    "name": "unnamed",
    "height_m": 1.80,
    "heads": 7.0,
    "bulk": 1.0,
    "stance_deg": 8.0,
    "attachments": [],
}

# The body numbers fall back to these fractions of height_m, so a spec that
# only names the class still gets a body that stands up.
BODY_FRACTION = {
    "shoulder_span": 0.40,
    "torso_depth": 0.155,
    "chest_w": 0.24,
    "waist_w": 0.17,
    "hip_w": 0.19,
    "leg_len": 0.47,
    "arm_len": 0.38,
    "hand": 0.105,
    "head_size": 0.128,
}


class BlockoutError(Exception):
    """A blockout that could not be built or rendered."""


# ---------------------------------------------------------------- spec input


def num(spec, key, default):
    """A spec number, with the default when it is missing or unusable."""
    value = spec.get(key, default)
    try:
        return float(value)
    except (TypeError, ValueError):
        return float(default)


def load_spec(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            spec = json.load(handle)
    except OSError as err:
        raise BlockoutError(f"cannot read spec {path}: {err}")
    except ValueError as err:
        raise BlockoutError(f"spec {path} is not valid JSON: {err}")
    if not isinstance(spec, dict):
        raise BlockoutError(f"spec {path} must be a JSON object")
    return spec


# --------------------------------------------------------------- proportions


class Proportions:
    """Every number the body builder needs, derived from the spec.

    The vertical stack is fixed by the spec: legs on the floor, then
    pelvis/waist/chest, then a head of height/heads on top. height_m is
    therefore the real height of the figure.
    """

    def __init__(self, spec):
        self.name = str(spec.get("name", DEFAULTS["name"]))
        self.h = max(num(spec, "height_m", DEFAULTS["height_m"]), 0.2)
        self.heads = max(num(spec, "heads", DEFAULTS["heads"]), 1.0)
        self.head_h = self.h / self.heads
        self.shoulder_span = max(num(spec, "shoulder_span", BODY_FRACTION["shoulder_span"] * self.h), 0.05)
        self.torso_depth = max(num(spec, "torso_depth", BODY_FRACTION["torso_depth"] * self.h), 0.05)
        self.chest_w = max(num(spec, "chest_w", BODY_FRACTION["chest_w"] * self.h), 0.05)
        self.waist_w = max(num(spec, "waist_w", BODY_FRACTION["waist_w"] * self.h), 0.05)
        self.hip_w = max(num(spec, "hip_w", BODY_FRACTION["hip_w"] * self.h), 0.05)
        self.leg_len = max(num(spec, "leg_len", BODY_FRACTION["leg_len"] * self.h), 0.1)
        self.arm_len = max(num(spec, "arm_len", BODY_FRACTION["arm_len"] * self.h), 0.1)
        self.hand = max(num(spec, "hand", BODY_FRACTION["hand"] * self.h), 0.02)
        self.head_size = max(num(spec, "head_size", BODY_FRACTION["head_size"] * self.h), 0.05)
        self.bulk = max(num(spec, "bulk", DEFAULTS["bulk"]), 0.2)
        self.stance = math.radians(num(spec, "stance_deg", DEFAULTS["stance_deg"]))
        attachments = spec.get("attachments", DEFAULTS["attachments"])
        self.attachments = attachments if isinstance(attachments, list) else []

        # Vertical stack. z_hip is the hip joint height, which is the leg length.
        self.z_hip = self.leg_len
        self.torso_h = max(self.h - self.leg_len - self.head_h, self.h * 0.2)
        self.hip_h = 0.28 * self.torso_h
        self.waist_h = 0.16 * self.torso_h
        self.chest_h = self.torso_h - self.hip_h - self.waist_h
        self.z_waist = self.z_hip + self.hip_h
        self.z_chest = self.z_waist + self.waist_h
        self.z_neck = self.z_chest + self.chest_h  # the head starts here
        self.z_shoulder = self.z_neck - 0.06 * self.h

        # Limbs.
        self.upper_len = 0.47 * self.arm_len
        self.fore_len = 0.53 * self.arm_len
        self.thigh_len = 0.50 * self.leg_len
        self.shin_len = 0.42 * self.leg_len
        self.hip_x = self.hip_w * 0.42
        self.pad_w = 0.26 * self.bulk
        self.pad_h = 0.22 * self.bulk


def side_sign(side):
    """World sign for a side name. The figure faces -Y, so "r" is world -X."""
    text = str(side or "r").strip().lower()
    return -1.0 if text.startswith("r") else 1.0


# ------------------------------------------------------------ mesh helpers


def align(direction):
    """A quaternion that turns local +Z into direction."""
    vec = Vector(direction)
    if vec.length < 1e-9:
        vec = Vector((0.0, 0.0, 1.0))
    return Vector((0.0, 0.0, 1.0)).rotation_difference(vec.normalized())


def box(name, size, loc, material=None, quat=None):
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=tuple(loc))
    ob = bpy.context.object
    ob.name = name
    ob.scale = size
    if material:
        ob.data.materials.append(material)
    if quat is not None:
        ob.rotation_mode = "QUATERNION"
        ob.rotation_quaternion = quat
    return ob


def cyl(name, radius, depth, loc, material=None, quat=None, verts=16):
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=verts, radius=radius, depth=depth, location=tuple(loc)
    )
    ob = bpy.context.object
    ob.name = name
    if material:
        ob.data.materials.append(material)
    if quat is not None:
        ob.rotation_mode = "QUATERNION"
        ob.rotation_quaternion = quat
    return ob


def limb(name, length, width, depth, base, direction, material):
    """A box that runs from base along direction for length."""
    vec = Vector(direction).normalized()
    center = Vector(base) + vec * (length * 0.5)
    return box(name, (width, depth, length), center, material, quat=align(vec))


def segment(name, a, b, radius, material, verts=10):
    """A cylinder from point a to point b."""
    start, end = Vector(a), Vector(b)
    delta = end - start
    if delta.length < 1e-6:
        return None
    return cyl(name, radius, delta.length, (start + end) * 0.5, material,
               quat=align(delta), verts=verts)


def basis_quat(x_axis, z_axis):
    """A quaternion from a right handed basis (x, y, z) built from x and z."""
    zx = Vector(z_axis).normalized()
    xx = Vector(x_axis).normalized()
    yy = zx.cross(xx).normalized()
    xx = yy.cross(zx).normalized()
    return Matrix((xx, yy, zx)).transposed().to_quaternion()


# ----------------------------------------------------------------- materials


def make_material(name, rgb):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (rgb[0], rgb[1], rgb[2], 1.0)
    try:
        # Blender 5.x deprecates use_nodes; the node tree may already be there.
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            if getattr(mat, "node_tree", None) is None and hasattr(mat, "use_nodes"):
                mat.use_nodes = True
        tree = getattr(mat, "node_tree", None)
        if tree:
            bsdf = tree.nodes.get("Principled BSDF")
            if bsdf:
                bsdf.inputs["Base Color"].default_value = (rgb[0], rgb[1], rgb[2], 1.0)
                bsdf.inputs["Roughness"].default_value = 0.7
    except Exception:
        pass
    return mat


# ---------------------------------------------------------------- body build


def build_body(P, mats):
    """The humanoid blockout. Returns the parts and the body landmarks."""
    parts = []
    body = mats["body"]
    gear = mats["gear"]
    b = P.bulk
    tag = P.name

    # torso: pelvis, waist, chest, then a chest plate for the armour layer
    parts.append(box(f"{tag}_pelvis", (P.hip_w * b, P.torso_depth * 0.85 * b, P.hip_h),
                     (0.0, 0.0, P.z_hip + P.hip_h * 0.5), body))
    parts.append(box(f"{tag}_waist", (P.waist_w * b, P.torso_depth * 0.78 * b, P.waist_h),
                     (0.0, 0.0, P.z_waist + P.waist_h * 0.5), body))
    parts.append(box(f"{tag}_chest", (P.chest_w * b, P.torso_depth * b, P.chest_h),
                     (0.0, 0.0, P.z_chest + P.chest_h * 0.5), body))
    parts.append(box(f"{tag}_chest_plate",
                     (P.chest_w * b * 1.04, P.torso_depth * b * 1.07, P.chest_h * 0.52),
                     (0.0, -P.torso_depth * 0.02, P.z_chest + P.chest_h * 0.68), body))

    # neck and head. The head is height/heads tall, so it tops the figure.
    parts.append(box(f"{tag}_neck", (P.head_size * 0.44, P.head_size * 0.44, P.head_h * 0.2),
                     (0.0, 0.0, P.z_neck + P.head_h * 0.04), body))
    parts.append(box(f"{tag}_head", (P.head_size * b, P.head_size * 0.86 * b, P.head_h),
                     (0.0, 0.0, P.z_neck + P.head_h * 0.5), body))
    parts.append(box(f"{tag}_jaw", (P.head_size * 0.72 * b, P.head_size * 0.78 * b, P.head_h * 0.26),
                     (0.0, -P.head_size * 0.1, P.z_neck + P.head_h * 0.16), body))

    # shoulders: the span is the outer width of both pauldrons
    land = {"shoulder": {}, "elbow": {}, "wrist": {}, "hand": {}, "fore_dir": {}}
    for side, sgn in (("r", -1.0), ("l", 1.0)):
        # The pauldron reaches exactly to shoulder_span/2, and the joint sits
        # under its outer half, so the arm hangs clear of the chest.
        pad_cx = sgn * (P.shoulder_span * 0.5 - P.pad_w * 0.5)
        joint_x = sgn * (P.shoulder_span * 0.5 - P.pad_w * 0.50)
        parts.append(box(f"{tag}_pauldron_{side}", (P.pad_w, P.torso_depth * 0.98 * b, P.pad_h),
                         (pad_cx, 0.0, P.z_shoulder), body))
        joint = Vector((joint_x, 0.0, P.z_shoulder - 0.02))
        land["shoulder"][side] = joint

        # arms hang down and a little out, with a small forward bend
        up_dir = Vector((sgn * math.sin(math.radians(6.0)), 0.0, -math.cos(math.radians(6.0))))
        elbow = joint + up_dir * P.upper_len
        fore_dir = Vector((sgn * 0.035, -0.075, -1.0)).normalized()
        wrist = elbow + fore_dir * P.fore_len
        hand_pos = wrist + fore_dir * (P.hand * 0.5)

        parts.append(limb(f"{tag}_upperarm_{side}", P.upper_len, 0.17 * b, 0.17 * b,
                          joint, up_dir, body))
        parts.append(limb(f"{tag}_forearm_{side}", P.fore_len, 0.15 * b, 0.15 * b,
                          elbow, fore_dir, body))
        parts.append(limb(f"{tag}_hand_{side}", P.hand, 0.16 * b, 0.12 * b,
                          wrist, fore_dir, body))

        land["elbow"][side] = elbow
        land["wrist"][side] = wrist
        land["hand"][side] = hand_pos
        land["fore_dir"][side] = fore_dir

    # legs: planted on the floor, splayed by stance_deg
    for side, sgn in (("r", -1.0), ("l", 1.0)):
        hip = Vector((sgn * P.hip_x, 0.0, P.z_hip))
        thigh_dir = Vector((sgn * math.sin(P.stance), 0.0, -math.cos(P.stance)))
        knee = hip + thigh_dir * P.thigh_len
        shin_dir = Vector((sgn * math.sin(P.stance * 0.7), 0.0, -math.cos(P.stance * 0.7)))
        ankle = knee + shin_dir * P.shin_len
        parts.append(limb(f"{tag}_thigh_{side}", P.thigh_len, 0.23 * b, 0.26 * b,
                          hip, thigh_dir, body))
        parts.append(limb(f"{tag}_shin_{side}", P.shin_len, 0.19 * b, 0.21 * b,
                          knee, shin_dir, body))
        parts.append(box(f"{tag}_boot_{side}", (0.24 * b, 0.34 * b, 0.18 * b),
                         (ankle.x, ankle.y - 0.06, 0.09), body))

    return parts, land


# --------------------------------------------------------------- attachments


def add_arm_cannon(name, spec, P, mats, land, side, sgn):
    """A barrel over the shoulder. angle_deg is the tilt from vertical,
    the same convention gunner_blockout.py used for its barrel."""
    length = num(spec, "length", 1.05 * P.h)
    radius = num(spec, "radius", 0.05 * P.h)
    angle = num(spec, "angle_deg", 66.0)
    elev = math.radians(max(min(90.0 - angle, 89.0), -89.0))
    yaw = math.radians(10.0)

    direction = Vector((
        sgn * math.sin(yaw) * math.cos(elev),
        -math.cos(yaw) * math.cos(elev),
        math.sin(elev),
    )).normalized()

    mount = land["shoulder"][side] + Vector((sgn * 0.07, -0.11, 0.03))
    parts = [
        cyl(f"{name}_cannon_breech", radius * 1.5, radius * 2.4, mount, mats["gear"],
            quat=align(direction)),
        cyl(f"{name}_cannon_barrel", radius, length, mount + direction * (length * 0.5),
            mats["gear"], quat=align(direction)),
        cyl(f"{name}_cannon_muzzle", radius * 1.3, radius * 1.6,
            mount + direction * (length - radius * 0.8), mats["gear"], quat=align(direction)),
    ]
    # the collar sits where the barrel meets the shoulder
    parts.append(box(f"{name}_cannon_mount", (radius * 2.6, radius * 3.0, radius * 2.2),
                     mount - direction * (radius * 1.6), mats["gear"], quat=align(direction)))
    return parts


def add_shield(name, spec, P, mats, land, side, sgn):
    """A plate on the outer side of the forearm."""
    fore_dir = land["fore_dir"][side]
    mid = land["elbow"][side] + fore_dir * (P.fore_len * 0.55)
    thickness = 0.055 * P.bulk
    outward = Vector((sgn, 0.0, 0.0))
    quat = basis_quat(outward, fore_dir)
    center = mid + outward * (0.09 * P.bulk + thickness * 0.5)
    size = (thickness, 0.42 * P.arm_len, 0.60 * P.arm_len)
    parts = [box(f"{name}_shield", size, center, mats["gear"], quat=quat)]
    parts.append(cyl(f"{name}_shield_boss", thickness * 1.6, thickness * 1.2,
                      center + outward * (thickness * 0.6), mats["gear"],
                      quat=align(outward), verts=12))
    return parts


def add_staff(name, spec, P, mats, land, side, sgn):
    """A pole held in the hand, its foot near the ground."""
    length = num(spec, "length", 1.1 * P.h)
    radius = num(spec, "radius", 0.018 * P.h)
    hand = land["hand"][side]
    tilt = math.radians(4.0)
    direction = Vector((sgn * math.sin(tilt), 0.05 * math.sin(tilt), math.cos(tilt))).normalized()
    foot = Vector((hand.x, hand.y, 0.02))
    top = foot + direction * length
    parts = [
        cyl(f"{name}_staff", radius, length, (foot + top) * 0.5, mats["gear"],
            quat=align(direction), verts=12),
        cyl(f"{name}_staff_knob", radius * 1.9, radius * 2.6, top, mats["gear"],
            quat=align(direction), verts=12),
    ]
    return parts


def add_bow(name, spec, P, mats, land, side, sgn):
    """An arc of limbs in the plane that faces forward, with a string."""
    hand = land["hand"][side]
    radius = num(spec, "radius", 0.42 * P.arm_len)
    half = math.radians(num(spec, "arc_deg", 62.0))
    center = hand + Vector((sgn * 0.03, -0.10, 0.0))
    steps = 10
    pts = []
    for i in range(steps + 1):
        t = -half + 2.0 * half * i / steps
        pts.append(center + Vector((0.0, math.cos(t) * radius, math.sin(t) * radius)))
    parts = []
    for i in range(steps):
        limb_seg = segment(f"{name}_bow_limb_{i}", pts[i], pts[i + 1],
                            0.016 * P.bulk, mats["gear"], verts=8)
        if limb_seg:
            parts.append(limb_seg)
    string = segment(f"{name}_bow_string", pts[0], pts[-1], 0.007 * P.bulk,
                     mats["gear"], verts=6)
    if string:
        parts.append(string)
    return parts


def add_quiver(name, spec, P, mats, land, side, sgn):
    """A tube of arrows on the back."""
    length = num(spec, "length", 0.34 * P.h)
    radius = num(spec, "radius", 0.045 * P.h)
    base = Vector((P.chest_w * 0.22, P.torso_depth * 0.5 + radius + 0.03,
                   P.z_chest + P.chest_h * 0.30))
    direction = Vector((0.30, 0.16, 1.0)).normalized()
    top = base + direction * length
    parts = [
        cyl(f"{name}_quiver", radius, length, (base + top) * 0.5, mats["gear"],
            quat=align(direction), verts=12),
    ]
    for i, spread in enumerate((-0.5, 0.0, 0.5)):
        arrow_dir = (direction + Vector((spread * 0.22, 0.0, 0.0))).normalized()
        arrow = segment(f"{name}_quiver_arrow_{i}", top - arrow_dir * 0.05,
                        top + arrow_dir * (0.10 * P.h), 0.008 * P.bulk, mats["gear"], verts=6)
        if arrow:
            parts.append(arrow)
    return parts


def add_cape(name, spec, P, mats, land, side, sgn):
    """Cloth behind the figure: a hanging slab plus a shoulder mantle."""
    top_z = P.z_neck - 0.02
    bottom_z = P.h * 0.20
    height = max(top_z - bottom_z, 0.05)
    center_z = (top_z + bottom_z) * 0.5
    back_y = P.torso_depth * 0.5 + 0.05
    quat = basis_quat(Vector((1.0, 0.0, 0.0)), Vector((0.0, math.sin(math.radians(10.0)),
                                                         -math.cos(math.radians(10.0)))))
    parts = [
        box(f"{name}_cape", (P.shoulder_span * 0.92 * P.bulk, 0.055 * P.bulk, height),
            (0.0, back_y, center_z), mats["gear"], quat=quat),
        box(f"{name}_cape_mantle", (P.shoulder_span * 1.02 * P.bulk, P.torso_depth * 0.72 * P.bulk,
                                    0.15 * P.bulk),
            (0.0, P.torso_depth * 0.34, top_z - 0.05), mats["gear"]),
    ]
    return parts


ATTACHMENTS = {
    "arm_cannon": add_arm_cannon,
    "cannon": add_arm_cannon,
    "shield": add_shield,
    "staff": add_staff,
    "bow": add_bow,
    "quiver": add_quiver,
    "cape": add_cape,
}


def build_attachments(P, mats, land):
    """Apply every attachment the spec asks for. Returns the parts and a count
    per kind, so the metrics line shows that each kind really built."""
    parts = []
    counts = {}
    for index, item in enumerate(P.attachments):
        if not isinstance(item, dict):
            continue
        kind = str(item.get("kind", "")).strip().lower()
        builder = ATTACHMENTS.get(kind)
        if builder is None:
            print(f"WARN unknown attachment kind ignored: {kind}", file=sys.stderr)
            continue
        side = str(item.get("side", "r")).strip().lower()
        if side not in ("r", "l"):
            side = "r"
        sgn = side_sign(side)
        name = f"{P.name}_{kind}_{side}_{index}"
        built = builder(name, item, P, mats, land, side, sgn)
        counts[kind] = counts.get(kind, 0) + len(built)
        parts.extend(built)
    return parts, counts


# ------------------------------------------------------------------- camera


def world_bbox(objects):
    low = Vector((1e9, 1e9, 1e9))
    high = Vector((-1e9, -1e9, -1e9))
    for ob in objects:
        if ob.type != "MESH":
            continue
        matrix = ob.matrix_world
        for corner in ob.bound_box:
            point = matrix @ Vector(corner)
            for axis in range(3):
                low[axis] = min(low[axis], point[axis])
                high[axis] = max(high[axis], point[axis])
    return low, high


def make_camera(scene, target_obj, low, high):
    data = bpy.data.cameras.new("blockout_cam")
    data.type = "ORTHO"
    data.ortho_scale = 1.0
    data.clip_start = 0.05
    data.clip_end = 200.0
    cam = bpy.data.objects.new("blockout_cam", data)
    bpy.context.collection.objects.link(cam)

    center = (low + high) * 0.5
    target_obj.location = center
    az = math.radians(CAM_AZ_DEG)
    el = math.radians(CAM_EL_DEG)
    direction = Vector((
        math.sin(az) * math.cos(el),
        -math.cos(az) * math.cos(el),
        math.sin(el),
    ))
    cam.location = center + direction * CAM_DIST
    con = cam.constraints.new("TRACK_TO")
    con.target = target_obj
    con.track_axis = "TRACK_NEGATIVE_Z"
    con.up_axis = "UP_Y"
    scene.camera = cam
    bpy.context.view_layer.update()
    return cam, data


def set_resolution(scene, resolution):
    """world_to_camera_view normalises by the scene resolution, so the scene
    must already have the render resolution when the camera is framed."""
    scene.render.resolution_x, scene.render.resolution_y = resolution
    scene.render.resolution_percentage = 100


def view_extents(ortho_scale, resolution):
    """The world size of the frame. The larger image side is the ortho scale."""
    width, height = resolution
    if width >= height:
        return ortho_scale, ortho_scale * height / width
    return ortho_scale * width / height, ortho_scale


def projected_span(scene, cam, corners):
    us, vs = [], []
    for point in corners:
        co = world_to_camera_view(scene, cam, point)
        us.append(co.x)
        vs.append(co.y)
    return min(us), max(us), min(vs), max(vs)


def frame_camera(scene, cam, target_obj, corners, margin=MARGIN):
    """Fit the figure in the frame: ortho scale plus a shift so it is centred."""
    data = cam.data
    resolution = (scene.render.resolution_x, scene.render.resolution_y)
    data.ortho_scale = 1.0
    bpy.context.view_layer.update()
    u0, u1, v0, v1 = projected_span(scene, cam, corners)
    du, dv = u1 - u0, v1 - v0
    usable = 1.0 - 2.0 * margin
    scale = max(du / usable, dv / usable)
    data.ortho_scale = scale
    bpy.context.view_layer.update()

    # Centre the projected figure: move camera and target together in the
    # camera plane. Moving the camera right slides the image content left,
    # so the shift carries (uc - 0.5), not the other way round.
    uc, vc = (u0 + u1) * 0.5, (v0 + v1) * 0.5
    view_w, view_h = view_extents(data.ortho_scale, resolution)
    basis = cam.matrix_world.to_3x3()
    shift = basis @ Vector((1.0, 0.0, 0.0)) * ((uc - 0.5) * view_w)
    shift += basis @ Vector((0.0, 1.0, 0.0)) * ((vc - 0.5) * view_h)
    cam.location = cam.location + shift
    target_obj.location = target_obj.location + shift
    bpy.context.view_layer.update()
    return data.ortho_scale


# ------------------------------------------------------------------ renders


def make_world(scene, color):
    """Set the background colour for the Workbench render.

    Measured on Blender 5.2: with scene.display.shading.background_type of
    WORLD or THEME the Workbench background is world.color, the world
    viewport display colour. The world Background node is ignored by
    Workbench, so it is set as well but nothing depends on it.
    """
    scene.display.shading.background_type = "WORLD"
    world = bpy.data.worlds.new("blockout_world")
    scene.world = world
    world.color = (color[0], color[1], color[2])
    tree = None
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            if getattr(world, "node_tree", None) is None and hasattr(world, "use_nodes"):
                world.use_nodes = True
        tree = getattr(world, "node_tree", None)
    except Exception:
        tree = None
    if tree:
        try:
            node = tree.nodes.get("Background")
            if node:
                node.inputs[0].default_value = (color[0], color[1], color[2], 1.0)
        except Exception:
            pass
    return world


def render_to(scene, cam, path, resolution, ortho_scale):
    set_resolution(scene, resolution)
    cam.data.ortho_scale = ortho_scale
    scene.render.filepath = path
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    bpy.ops.render.render(write_still=True)
    if not os.path.exists(path) or os.path.getsize(path) == 0:
        raise BlockoutError(f"render wrote nothing: {path}")


def read_pixels(path):
    """Return (pixels, width, height) with rows bottom up, as numpy or lists."""
    image = bpy.data.images.load(path, check_existing=False)
    width, height = image.size
    if np is not None:
        buf = np.empty(width * height * 4, dtype=np.float32)
        image.pixels.foreach_get(buf)
        return buf.reshape(height, width, 4), width, height, image
    flat = list(image.pixels)
    rows = [[flat[(y * width + x) * 4: (y * width + x) * 4 + 4] for x in range(width)]
            for y in range(height)]
    return rows, width, height, image


def srgb(linear):
    if linear <= 0.0031308:
        return linear * 12.92
    return 1.055 * (linear ** (1.0 / 2.4)) - 0.055


def ink_mask(pixels, width, height):
    """True where the figure is. Works with an opaque or a transparent
    background: a transparent corner means the alpha decides, otherwise the
    brightness does."""
    if np is not None:
        alpha = pixels[:, :, 3]
        if alpha[0, 0] < 0.5:
            return alpha > 0.5
        lum = pixels[:, :, :3].mean(axis=2)
        mask = np.empty_like(lum, dtype=bool)
        for i in range(lum.shape[0]):
            for j in range(lum.shape[1]):
                mask[i, j] = srgb(float(lum[i, j])) < INK_SRGB
        return mask
    alpha_bg = pixels[0][0][3] < 0.5
    mask = []
    for row in pixels:
        mask.append([
            (px[3] > 0.5) if alpha_bg else (srgb((px[0] + px[1] + px[2]) / 3.0) < INK_SRGB)
            for px in row
        ])
    return mask


def measure(mask, width, height):
    """The measured silhouette: box in pixels, height, and how much of the
    box the figure fills."""
    if np is not None:
        rows = np.where(mask.any(axis=1))[0]
        cols = np.where(mask.any(axis=0))[0]
        if rows.size == 0:
            raise BlockoutError("the silhouette render is empty")
        r0, r1 = int(rows[0]), int(rows[-1])
        c0, c1 = int(cols[0]), int(cols[-1])
        box_w = c1 - c0 + 1
        box_h = r1 - r0 + 1
        fill = float(mask.sum()) / float(box_w * box_h)
        return {"x0": c0, "x1": c1, "y0": r0, "y1": r1,
                "width_px": box_w, "height_px": box_h, "fill": fill}
    rows = [y for y in range(height) if any(mask[y][x] for x in range(width))]
    cols = [x for x in range(width) if any(mask[y][x] for y in range(height))]
    if not rows:
        raise BlockoutError("the silhouette render is empty")
    box_w = cols[-1] - cols[0] + 1
    box_h = rows[-1] - rows[0] + 1
    ink = sum(1 for y in range(height) for x in range(width) if mask[y][x])
    return {"x0": cols[0], "x1": cols[-1], "y0": rows[0], "y1": rows[-1],
            "width_px": box_w, "height_px": box_h, "fill": ink / float(box_w * box_h)}


def corner_is_white(pixels, width, height):
    if np is not None:
        alpha = float(pixels[0, 0, 3])
        if alpha < 0.5:
            return False
        lum = float(pixels[0, 0, :3].mean())
        return srgb(lum) > 0.92
    px = pixels[0][0]
    if px[3] < 0.5:
        return False
    return srgb((px[0] + px[1] + px[2]) / 3.0) > 0.92


def composite_over_white(path):
    """Flatten a transparent render onto white, so the PNG reads black on
    white like the other silhouette."""
    pixels, width, height, image = read_pixels(path)
    if np is not None:
        flat = pixels.reshape(-1, 4)
        alpha = flat[:, 3:4]
        rgb = flat[:, :3] * alpha + (1.0 - alpha)
        out = np.concatenate([rgb, np.ones_like(alpha)], axis=1).reshape(-1)
        image.pixels.foreach_set(out.astype(np.float32))
    else:
        out = []
        for row in pixels:
            for px in row:
                a = px[3]
                out.extend([
                    px[0] * a + (1.0 - a),
                    px[1] * a + (1.0 - a),
                    px[2] * a + (1.0 - a),
                    1.0,
                ])
        image.pixels[:] = out
    image.file_format = "PNG"
    image.filepath_raw = path
    image.save()
    bpy.data.images.remove(image)


def render_silhouette(scene, cam, path, resolution, ortho_scale):
    """Flat black on white. Returns (mode, measurement).

    The white comes from scene.display.shading.background_type = WORLD plus a
    white world.color. Workbench honours that on Blender 5.2, but the check
    below is cheap: if the frame does not come out white the script falls back
    to film_transparent and flattens the result onto white, which is the same
    picture with one extra step. Set BLOCKOUT_FORCE_TRANSPARENT=1 to take
    that path on purpose.
    """
    scene.display.shading.light = "FLAT"
    scene.display.shading.color_type = "SINGLE"
    scene.display.shading.single_color = (0.0, 0.0, 0.0)
    scene.display.shading.show_shadows = False
    scene.display.shading.show_cavity = False
    scene.render.film_transparent = False

    make_world(scene, (1.0, 1.0, 1.0))
    render_to(scene, cam, path, resolution, ortho_scale)

    pixels, width, height, image = read_pixels(path)
    bpy.data.images.remove(image)
    forced = os.environ.get("BLOCKOUT_FORCE_TRANSPARENT", "") not in ("", "0")
    if corner_is_white(pixels, width, height) and not forced:
        mode = "world_white"
    else:
        # Fallback: transparent film, then flatten onto white.
        scene.render.film_transparent = True
        render_to(scene, cam, path, resolution, ortho_scale)
        pixels, width, height, image = read_pixels(path)
        bpy.data.images.remove(image)
        composite_over_white(path)
        mode = "film_transparent_flattened"

    pixels, width, height, image = read_pixels(path)
    bpy.data.images.remove(image)
    return mode, measure(ink_mask(pixels, width, height), width, height)


def render_studio(scene, cam, path, resolution, ortho_scale):
    """Mid grey figure under studio light, on a neutral background."""
    scene.display.shading.light = "STUDIO"
    scene.display.shading.color_type = "MATERIAL"
    scene.display.shading.show_shadows = True
    scene.display.shading.shadow_intensity = 0.35
    scene.display.shading.show_cavity = True
    try:
        scene.display.shading.cavity_type = "SCREEN"
    except Exception:
        pass
    scene.render.film_transparent = False
    make_world(scene, (0.18, 0.18, 0.18))
    render_to(scene, cam, path, resolution, ortho_scale)


# --------------------------------------------------------------------- main


def build_scene(spec):
    """Clean scene, figure, camera. Returns what the renderer needs."""
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.display.render_aa = "8"
    set_resolution(scene, RES_BIG)
    try:
        scene.view_settings.view_transform = "Standard"
    except Exception:
        pass

    P = Proportions(spec)
    mats = {
        "body": make_material("blockout_body", (0.55, 0.55, 0.55)),
        "gear": make_material("blockout_gear", (0.40, 0.40, 0.40)),
    }
    parts, land = build_body(P, mats)
    gear_parts, kind_counts = build_attachments(P, mats, land)
    parts.extend(gear_parts)
    bpy.context.view_layer.update()

    low, high = world_bbox(parts)
    target = bpy.data.objects.new("blockout_target", None)
    bpy.context.collection.objects.link(target)
    cam, cam_data = make_camera(scene, target, low, high)

    corners = []
    for ob in parts:
        matrix = ob.matrix_world
        for corner in ob.bound_box:
            corners.append(matrix @ Vector(corner))

    scale = frame_camera(scene, cam, target, corners)
    return {
        "P": P, "scene": scene, "cam": cam, "cam_data": cam_data, "parts": parts,
        "corners": corners, "ortho_scale": scale, "kind_counts": kind_counts,
    }


def shoulder_pixels(scene, cam, parts, resolution):
    """The visible shoulder width: the projected width of both pauldrons."""
    widths = []
    for ob in parts:
        if ob.type != "MESH" or "_pauldron_" not in ob.name:
            continue
        matrix = ob.matrix_world
        us = []
        for corner in ob.bound_box:
            co = world_to_camera_view(scene, cam, matrix @ Vector(corner))
            us.append(co.x)
        widths.append((min(us), max(us)))
    if not widths:
        return 0.0
    left = min(w[0] for w in widths)
    right = max(w[1] for w in widths)
    return (right - left) * resolution[0]


def main(argv):
    args = argv[argv.index("--") + 1:] if "--" in argv else []
    if len(args) < 2:
        sys.stderr.write(
            "usage: blender -b --factory-startup --python-exit-code 1 "
            "-P blockout.py -- <spec.json> <outdir>\n"
        )
        return EXIT_USAGE
    spec_path, outdir = args[0], args[1]
    try:
        spec = load_spec(spec_path)
        os.makedirs(outdir, exist_ok=True)
        built = build_scene(spec)
        scene = built["scene"]
        cam = built["cam"]
        P = built["P"]

        big_path = os.path.join(outdir, SIL_BIG)
        mode, big = render_silhouette(scene, cam, big_path, RES_BIG, built["ortho_scale"])

        # The 30 m test: the same camera, a frame 30 m wide, and the figure
        # about 90 px tall in it. The resolution follows from the measured
        # projected height of the figure.
        view_h_big = built["ortho_scale"]
        projected_h = big["height_px"] * view_h_big / float(RES_BIG[1])
        res_x = int(round(TARGET_30M_PX * FIELD_30M_M / max(projected_h, 1e-6) / 2.0)) * 2
        res_x = max(640, min(res_x, 2560))
        res_y = int(round(res_x * 9.0 / 16.0 / 2.0)) * 2
        if res_y >= res_x:
            res_x, res_y = RES_30M_FALLBACK
        path_30m = os.path.join(outdir, SIL_30M)
        mode_30m, small = render_silhouette(scene, cam, path_30m, (res_x, res_y), FIELD_30M_M)

        studio_path = os.path.join(outdir, GREY_STUDIO)
        render_studio(scene, cam, studio_path, RES_BIG, built["ortho_scale"])

        shoulder_px = shoulder_pixels(scene, cam, built["parts"], RES_BIG)
        metrics = {
            "name": P.name,
            "height_px": round(big["height_px"], 2),
            "shoulder_px": round(shoulder_px, 2),
            "bbox_fill": round(big["fill"], 4),
            "bbox_width_px": big["width_px"],
            "figure_height_m": round(projected_h, 4),
            "px_per_m": round(RES_BIG[1] / view_h_big, 2),
            "resolution": list(RES_BIG),
            "ortho_scale_m": round(built["ortho_scale"], 4),
            "height_30m_px": round(small["height_px"], 2),
            "resolution_30m": [res_x, res_y],
            "field_width_m_30m": FIELD_30M_M,
            "background": mode,
            "background_30m": mode_30m,
            "parts": len(built["parts"]),
            "parts_by_kind": built["kind_counts"],
            "spec": os.path.abspath(spec_path),
        }
        print(json.dumps(metrics, sort_keys=True))
        return 0
    except BlockoutError as err:
        sys.stderr.write(f"E_BLOCKOUT {err}\n")
        return EXIT_FAIL
    except Exception as err:  # never leave a half written render behind
        sys.stderr.write(f"E_BLOCKOUT {type(err).__name__}: {err}\n")
        return EXIT_FAIL


if __name__ == "__main__":
    code = main(sys.argv)
    if code != 0:
        sys.exit(code)
