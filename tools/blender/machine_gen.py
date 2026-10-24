"""The machine generator: seeded siege engines, cranes, carts and frames.

The algorithm is ported from ldo/blender_spaceship_generator (MIT License,
Copyright (c) 2016 Michael Davies; https://github.com/ldo/blender_spaceship_generator,
spaceship_generator.py). That generator starts from a random box, extrudes
the faces by seeded rules (segments, scale, sideways shift, tilt, ribbed
sections, asymmetric blocks), then sorts the faces by the way they point and
adds detail to each group (grids, pipes, discs, sockets, spars).

What is ours:

- The mesh maths is plain Python (no bpy). A seed gives the same numbers on
  any machine, and the tests run without Blender. Blender is only used to
  write the GLB.
- Seeds use splitmix64, not Python's random module, so the stream does not
  depend on the Python build.
- Output is a low-poly prop: at most three material slots (wood, iron, tarp)
  and a triangle budget that is checked before anything is written. Over
  budget is refused, not thinned.
- The presets (ballista, crane, cart, machine) are data: a base box, which
  faces extrude, and the weight of each detail kind per face direction. They
  are seeded silhouettes to judge in the rack render, not reconstructions of
  historical engines.
- Known simplifications against upstream: the zero-length ring steps of the
  ribbed section are dropped (the corrugation stays), and faces are not
  welded, so shared edges can carry T-junctions. Both are cosmetic at prop
  scale.

Units are metres, Z up. The base sits on z = 0.

Run in Blender (writes the GLB and a <name>.machine.json sidecar):
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/machine_gen.py -- <out.glb> [--preset cart] [--seed 7] [--max-tris N]

Plain Python, numbers only (no Blender needed):
  python tools/blender/machine_gen.py --stats [--preset cart] [--seed 7]
  python tools/blender/machine_gen.py --list
"""

import hashlib
import json
import math
import os
import sys

EXIT_USAGE = 2
EXIT_FAIL = 3

MASK = (1 << 64) - 1
GOLDEN = 0x9E3779B97F4A7C15
DEG = math.pi / 180.0

# Material slots. Three is the cap for a machine prop.
WOOD, IRON, TARP = 0, 1, 2
SLOTS = (
    ("machine_wood", (0.42, 0.29, 0.17)),
    ("machine_iron", (0.27, 0.27, 0.28)),
    ("machine_tarp", (0.66, 0.60, 0.46)),
)
SLOT_NAMES = {WOOD: "wood", IRON: "iron", TARP: "tarp"}


class MachineError(Exception):
    """A machine that could not be built, or is over its budget."""


# ------------------------------------------------------------------ numbers


def vadd(a, b):
    return (a[0] + b[0], a[1] + b[1], a[2] + b[2])


def vsub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def vmul(a, k):
    return (a[0] * k, a[1] * k, a[2] * k)


def vdot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def vcross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def vlen(a):
    return math.sqrt(vdot(a, a))


def vnorm(a):
    length = vlen(a)
    if length < 1e-12:
        return (0.0, 0.0, 0.0)
    return vmul(a, 1.0 / length)


def vlerp(a, b, k):
    return (a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k)


class Rng:
    """splitmix64: the same stream on every Python build and machine.

    A string seed is hashed with sha256, so "bridge-3" and 3 are both fine.
    """

    def __init__(self, seed):
        if isinstance(seed, str):
            seed = int(hashlib.sha256(seed.encode("utf-8")).hexdigest()[:16], 16)
        self.state = int(seed) & MASK

    def _next(self):
        self.state = (self.state + GOLDEN) & MASK
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)

    def random(self):
        return (self._next() >> 11) / float(1 << 53)

    def uniform(self, low, high):
        return low + (high - low) * self.random()

    def randint(self, low, high):
        """Inclusive at both ends, like random.randint."""
        if high < low:
            raise MachineError(f"randint: {high} < {low}")
        return low + min(high - low, int(self.random() * (high - low + 1)))

    def pick(self, weighted):
        """weighted is a list of (item, weight). Weight 0 is never picked."""
        total = sum(w for _, w in weighted if w > 0)
        if total <= 0:
            return None
        mark = self.random() * total
        last = None
        for item, weight in weighted:
            if weight <= 0:
                continue
            last = item
            mark -= weight
            if mark < 0:
                return item
        return last


# ------------------------------------------------------------------ mesh


class Face:
    __slots__ = ("v", "mat", "live")

    def __init__(self, verts, mat):
        self.v = list(verts)
        self.mat = mat
        self.live = True


class Mesh:
    """Vertices and faces. Faces index into self.verts and are wound CCW from outside."""

    def __init__(self):
        self.verts = []
        self.faces = []

    def vert(self, p):
        self.verts.append((float(p[0]), float(p[1]), float(p[2])))
        return len(self.verts) - 1

    def face(self, indices, mat):
        self.faces.append(Face(indices, mat))
        return len(self.faces) - 1

    def corners(self, fi):
        return [self.verts[i] for i in self.faces[fi].v]

    def centroid(self, fi):
        pts = self.corners(fi)
        count = float(len(pts))
        return tuple(sum(p[k] for p in pts) / count for k in range(3))

    def normal(self, fi):
        """Newell's method: the unit normal of a planar polygon, right-hand rule."""
        pts = self.corners(fi)
        nx = ny = nz = 0.0
        count = len(pts)
        for k in range(count):
            a = pts[k]
            b = pts[(k + 1) % count]
            nx += (a[1] - b[1]) * (a[2] + b[2])
            ny += (a[2] - b[2]) * (a[0] + b[0])
            nz += (a[0] - b[0]) * (a[1] + b[1])
        return vnorm((nx, ny, nz))

    def kill(self, fi):
        self.faces[fi].live = False

    def block(self, centre, axes, half, mat):
        """A box on three unit axes (u, w, n) with half sizes. Every face points out."""
        au, aw, an = axes
        hu, hw, hn = half
        corner = {}
        for su in (-1, 1):
            for sw in (-1, 1):
                for sn in (-1, 1):
                    p = vadd(centre, vadd(vmul(au, su * hu), vadd(vmul(aw, sw * hw), vmul(an, sn * hn))))
                    corner[(su, sw, sn)] = self.vert(p)
        sides = (
            ((1, -1, -1), (1, 1, -1), (1, 1, 1), (1, -1, 1), au),
            ((-1, -1, -1), (-1, 1, -1), (-1, 1, 1), (-1, -1, 1), vmul(au, -1)),
            ((-1, 1, -1), (1, 1, -1), (1, 1, 1), (-1, 1, 1), aw),
            ((-1, -1, -1), (1, -1, -1), (1, -1, 1), (-1, -1, 1), vmul(aw, -1)),
            ((-1, -1, 1), (1, -1, 1), (1, 1, 1), (-1, 1, 1), an),
            ((-1, -1, -1), (-1, 1, -1), (1, 1, -1), (1, -1, -1), vmul(an, -1)),
        )
        out = []
        for a, b, c, d, outward in sides:
            fi = self.face([corner[a], corner[b], corner[c], corner[d]], mat)
            if vdot(self.normal(fi), outward) < 0:
                self.faces[fi].v.reverse()
            out.append(fi)
        return out

    def extrude(self, fi, distance):
        """Push a face out along its normal, with the side walls. Returns (top, sides).

        A negative distance pushes it in, which makes a recess.
        """
        face = self.faces[fi]
        n = self.normal(fi)
        base = list(face.v)
        top_verts = [self.vert(vadd(self.verts[i], vmul(n, distance))) for i in base]
        count = len(base)
        sides = []
        for j in range(count):
            m = (j + 1) % count
            sides.append(self.face([base[j], base[m], top_verts[m], top_verts[j]], face.mat))
        face.live = False
        top = self.face(top_verts, face.mat)
        return top, sides

    def scale(self, fi, su, sw):
        """Scale a face in its own plane about its centre: u along edge 0, w across."""
        pts = self.corners(fi)
        n = self.normal(fi)
        u = vnorm(vsub(pts[1], pts[0]))
        w = vcross(n, u)
        c = self.centroid(fi)
        for i in self.faces[fi].v:
            d = vsub(self.verts[i], c)
            self.verts[i] = vadd(c, vadd(vmul(u, vdot(d, u) * su),
                                         vadd(vmul(w, vdot(d, w) * sw), vmul(n, vdot(d, n)))))

    def translate(self, fi, vec):
        for i in self.faces[fi].v:
            self.verts[i] = vadd(self.verts[i], vec)

    def rotate_about_y(self, fi, angle):
        c, s = math.cos(angle), math.sin(angle)
        for i in self.faces[fi].v:
            x, y, z = self.verts[i]
            self.verts[i] = (x * c + z * s, y, -x * s + z * c)

    def subdivide(self, fi, nu, nv):
        """Split a quad into nu x nv quads. Returns the new face indices."""
        face = self.faces[fi]
        if len(face.v) != 4 or nu < 1 or nv < 1:
            return [fi]
        v0, v1, v2, v3 = face.v
        p0, p1, p2, p3 = (self.verts[i] for i in (v0, v1, v2, v3))
        grid = []
        for i in range(nu + 1):
            s = i / nu
            column = []
            for j in range(nv + 1):
                t = j / nv
                if (i, j) == (0, 0):
                    column.append(v0)
                elif (i, j) == (nu, 0):
                    column.append(v1)
                elif (i, j) == (nu, nv):
                    column.append(v2)
                elif (i, j) == (0, nv):
                    column.append(v3)
                else:
                    a = vlerp(p0, p1, s)
                    b = vlerp(p3, p2, s)
                    column.append(self.vert(vlerp(a, b, t)))
            grid.append(column)
        face.live = False
        out = []
        for i in range(nu):
            for j in range(nv):
                out.append(self.face([grid[i][j], grid[i + 1][j], grid[i + 1][j + 1], grid[i][j + 1]],
                                     face.mat))
        return out

    def prism(self, base, axis, radius, depth, segments, mat):
        """A closed cylinder rising from base along unit axis. The bottom is hidden in the face."""
        ref = (1.0, 0.0, 0.0) if abs(axis[0]) < 0.9 else (0.0, 1.0, 0.0)
        u = vnorm(vcross(axis, ref))
        w = vcross(axis, u)
        bottom = []
        top = []
        for k in range(segments):
            t = 2.0 * math.pi * k / segments
            ring = vmul(vadd(vmul(u, math.cos(t)), vmul(w, math.sin(t))), radius)
            b = vadd(base, ring)
            bottom.append(self.vert(b))
            top.append(self.vert(vadd(b, vmul(axis, depth))))
        for k in range(segments):
            m = (k + 1) % segments
            self.face([bottom[k], bottom[m], top[m], top[k]], mat)
        self.face(top, mat)

    def tri_count(self):
        return sum(len(f.v) - 2 for f in self.faces if f.live)

    def arrays(self):
        """Live faces with compact vertex indices: (verts, faces, material slots)."""
        remap = {}
        verts = []
        faces = []
        mats = []
        for face in self.faces:
            if not face.live:
                continue
            idx = []
            for i in face.v:
                if i not in remap:
                    remap[i] = len(verts)
                    verts.append(self.verts[i])
                idx.append(remap[i])
            faces.append(tuple(idx))
            mats.append(face.mat)
        return verts, faces, mats


# ------------------------------------------------------------------ face helpers


def face_frame(mesh, fi):
    """Centre, in-plane axes u (along edge 0) and w (towards corner 3), normal, and sizes."""
    pts = mesh.corners(fi)
    n = mesh.normal(fi)
    e0 = vsub(pts[1], pts[0])
    u = vnorm(e0)
    w = vcross(n, u)
    return mesh.centroid(fi), u, w, n, vlen(e0), vlen(vsub(pts[3], pts[0]))


def face_aspect(mesh, fi):
    pts = mesh.corners(fi)
    a = vlen(vsub(pts[1], pts[0]))
    b = vlen(vsub(pts[2], pts[1]))
    if a <= 1e-9 or b <= 1e-9:
        return float("inf")
    ratio = a / b
    return max(ratio, 1.0 / ratio)


def face_category(n):
    """Which way the face points, as the upstream face sort (x is fore, z is up)."""
    if n[2] > 0.9:
        return "up"
    if n[2] < -0.9:
        return "down"
    if n[0] > 0.9:
        return "fore"
    if n[0] < -0.9:
        return "aft"
    if abs(n[1]) > 0.9:
        return "side"
    return None


def on_face(pts, s, t):
    """A point on a quad given as corners v0..v3, s along v0->v1 and t across to v3."""
    return vlerp(vlerp(pts[0], pts[1], s), vlerp(pts[3], pts[2], s), t)


# ------------------------------------------------------------------ detail kinds


def detail_panel(mesh, rng, fi):
    """A grid of raised cells: the hull plating."""
    for sub in mesh.subdivide(fi, rng.randint(2, 3), rng.randint(2, 3)):
        top, sides = mesh.extrude(sub, rng.uniform(0.02, 0.05))
        for s in sides:
            mesh.faces[s].mat = WOOD
        mesh.faces[top].mat = TARP
        mesh.scale(top, 0.8, 0.8)


def detail_pipes(mesh, rng, fi):
    """A few short cylinders on a grid across the face."""
    pts = mesh.corners(fi)
    _, _, _, n, fu, fw = face_frame(mesh, fi)
    hs = rng.randint(1, 2)
    vs = rng.randint(1, 2)
    m = min(fu / (hs + 2), fw / (vs + 2))
    radius = 0.65 * m
    segments = rng.randint(6, 8)
    for h in range(hs):
        for v in range(vs):
            base = on_face(pts, (h + 1) / (hs + 1), (v + 1) / (vs + 1))
            mesh.prism(base, n, radius, 1.3 * m * rng.uniform(1.0, 2.0), segments, IRON)


def detail_bolts(mesh, rng, fi):
    """Rivet heads on a small grid: the greeble layer."""
    pts = mesh.corners(fi)
    n = face_frame(mesh, fi)[3]
    hs = rng.randint(2, 3)
    vs = rng.randint(2, 3)
    radius = rng.uniform(0.012, 0.02)
    for h in range(hs):
        for v in range(vs):
            base = on_face(pts, (h + 1) / (hs + 1), (v + 1) / (vs + 1))
            mesh.prism(base, n, radius, radius * 0.8, 6, IRON)


def detail_ribs(mesh, rng, fi):
    """Wooden spars sitting across the face, the frame's ribs."""
    c, u, w, n, fu, fw = face_frame(mesh, fi)
    count = rng.randint(1, 3)
    half_w = rng.uniform(0.02, 0.04)
    half_h = rng.uniform(0.03, 0.06)
    for k in range(count):
        offset = -fw / 2.0 + fw * (k + 1) / (count + 1)
        centre = vadd(c, vadd(vmul(w, offset), vmul(n, half_h)))
        mesh.block(centre, (u, w, n), (fu * 0.45, half_w, half_h), WOOD)


def detail_disc(mesh, rng, fi):
    """A flat round boss, a wheel hub on a side wall or a pad under the frame."""
    c, _, _, n, fu, fw = face_frame(mesh, fi)
    small = min(fu, fw)
    depth = 0.125 * small
    radius = min(depth * rng.uniform(3.0, 4.0), 0.45 * small)
    mesh.prism(c, n, radius, depth, 12, WOOD)
    mesh.prism(vadd(c, vmul(n, depth * 0.9)), n, radius * 0.3, depth * 0.4, 8, IRON)


def detail_socket(mesh, rng, fi):
    """An exhaust-style socket: each cell pushed out, shrunk, then pushed back in."""
    for sub in mesh.subdivide(fi, 2, 2):
        length = rng.uniform(0.08, 0.14)
        top, sides = mesh.extrude(sub, length)
        for s in sides:
            mesh.faces[s].mat = IRON
        k = 1.0 / rng.uniform(1.3, 1.6)
        mesh.scale(top, k, k)
        inner_top, inner = mesh.extrude(top, -length * 0.9)
        for s in inner:
            mesh.faces[s].mat = IRON
        k = 1.0 / rng.uniform(1.05, 1.1)
        mesh.scale(inner_top, k, k)


DETAIL = {
    "panel": detail_panel,
    "pipes": detail_pipes,
    "bolts": detail_bolts,
    "ribs": detail_ribs,
    "disc": detail_disc,
    "socket": detail_socket,
}


# ------------------------------------------------------------------ presets

COMMON = {
    "title": "",
    "dims": (1.0, 1.0, 1.0),
    "jitter": 0.2,
    "extrude": "any",
    "chance": 0.5,
    "segments": (1, 3),
    "segment_len": (0.3, 0.7),
    "p_rib": 0.1,
    "p_scale": 0.5,
    "p_translate": 0.5,
    "p_tilt": 0.5,
    "p_asym": 0.15,
    "asym_segments": (1, 3),
    "mirror": False,
    "budget": 5000,
    "detail": {
        "up": (("panel", 3), ("ribs", 2), ("none", 2)),
        "down": (("bolts", 2), ("disc", 1), ("none", 3)),
        "fore": (("panel", 2), ("socket", 1), ("none", 2)),
        "aft": (("socket", 2), ("pipes", 2), ("none", 2)),
        "side": (("pipes", 2), ("ribs", 2), ("bolts", 1), ("none", 2)),
    },
}


def _preset(**over):
    spec = dict(COMMON)
    spec.update(over)
    return spec


PRESETS = {
    "machine": _preset(
        title="generic machine: boxes extruded on random faces, panels, pipes, ribs, bolts, sockets",
        dims=(1.6, 1.0, 1.0),
    ),
    "ballista": _preset(
        title="ballista frame: a long low stock extruded fore and aft, mirrored across Y",
        dims=(1.4, 0.5, 0.22),
        jitter=0.1,
        extrude="x",
        chance=1.0,
        segments=(1, 2),
        segment_len=(0.25, 0.5),
        mirror=True,
        detail={
            "up": (("ribs", 3), ("none", 1)),
            "down": (("bolts", 2), ("none", 3)),
            "fore": (("socket", 2), ("ribs", 2), ("none", 1)),
            "aft": (("pipes", 1), ("none", 3)),
            "side": (("ribs", 3), ("bolts", 2), ("none", 1)),
        },
    ),
    "crane": _preset(
        title="crane: a tall mast on a base, extruded upward, mirrored across Y",
        dims=(0.6, 0.6, 1.2),
        jitter=0.1,
        extrude="up",
        chance=1.0,
        segments=(2, 4),
        segment_len=(0.35, 0.6),
        mirror=True,
        detail={
            "up": (("disc", 2), ("panel", 1), ("none", 1)),
            "down": (("bolts", 2), ("none", 2)),
            "fore": (("panel", 2), ("none", 1)),
            "aft": (("pipes", 2), ("none", 1)),
            "side": (("ribs", 3), ("pipes", 1), ("bolts", 1)),
        },
    ),
    "cart": _preset(
        title="cart: a flat bed with wheel discs on its sides, mirrored across Y",
        dims=(1.5, 0.9, 0.7),
        jitter=0.1,
        extrude="any",
        chance=0.3,
        mirror=True,
        budget=4000,
        detail={
            "up": (("panel", 2), ("ribs", 2), ("none", 1)),
            "down": (("bolts", 1), ("none", 3)),
            "fore": (("ribs", 1), ("none", 2)),
            "aft": (("ribs", 1), ("none", 2)),
            "side": (("disc", 3), ("ribs", 2), ("none", 1)),
        },
    ),
}


# ------------------------------------------------------------------ generator


def _extrudes(rule, n):
    """Which faces the preset lets grow: x or z (both ways), up only, or any face."""
    if rule == "x":
        return abs(n[0]) > 0.5
    if rule == "z":
        return abs(n[2]) > 0.5
    if rule == "up":
        return n[2] > 0.5
    return True


def _ribbed(mesh, rng, fi, length):
    """A corrugated run: the upstream ribbed extrude, minus its zero-length steps."""
    rib_scale = rng.uniform(0.75, 0.95)
    ribs = rng.randint(2, 4)
    per = length / ribs
    for _ in range(ribs):
        fi = mesh.extrude(fi, per * 0.25)[0]
        mesh.scale(fi, rib_scale, rib_scale)
        fi = mesh.extrude(fi, per * 0.5)[0]
        mesh.scale(fi, 1.0 / rib_scale, 1.0 / rib_scale)
        fi = mesh.extrude(fi, per * 0.25)[0]
    return fi


def _hull_run(mesh, rng, fi, spec):
    length = rng.uniform(*spec["segment_len"])
    count = rng.randint(*spec["segments"])
    for i in range(count):
        last = i == count - 1
        if rng.random() < spec["p_rib"]:
            fi = _ribbed(mesh, rng, fi, length)
            continue
        fi = mesh.extrude(fi, length)[0]
        if rng.random() < 0.25:
            fi = mesh.extrude(fi, length * 0.25)[0]
        if rng.random() < spec["p_scale"]:
            a = rng.uniform(1.2, 1.5)
            b = rng.uniform(1.2, 1.5)
            if last or rng.random() > 0.5:
                a = 1.0 / a
                b = 1.0 / b
            mesh.scale(fi, a, b)
        if rng.random() < spec["p_translate"]:
            _, _, w, _, _, _ = face_frame(mesh, fi)
            amount = rng.uniform(0.1, 0.4) * length
            if rng.random() > 0.5:
                amount = -amount
            mesh.translate(fi, vmul(w, amount))
        if rng.random() < spec["p_tilt"]:
            angle = 5.0 * DEG
            if rng.random() > 0.5:
                angle = -angle
            mesh.rotate_about_y(fi, angle)
    return fi


def _mirror_y(mesh):
    """Keep the +Y half and mirror it across Y, so the machine is symmetric."""
    eps = 1e-6
    src = []
    for fi, face in enumerate(mesh.faces):
        if not face.live:
            continue
        y = mesh.centroid(fi)[1]
        if y < -eps:
            face.live = False
        elif y > eps:
            src.append(fi)
    for fi in src:
        face = mesh.faces[fi]
        idx = []
        for i in reversed(face.v):
            x, y, z = mesh.verts[i]
            idx.append(mesh.vert((x, -y, z)))
        mesh.face(idx, face.mat)


def _ground(mesh):
    """Stand the machine on z = 0: the lowest live vertex goes to the floor."""
    used = [i for face in mesh.faces if face.live for i in face.v]
    if not used:
        return
    low = min(mesh.verts[i][2] for i in used)
    if abs(low) > 1e-12:
        mesh.verts = [(x, y, z - low) for x, y, z in mesh.verts]


def generate(preset, seed, max_tris=None):
    """Build one machine. The same preset and seed always give the same mesh."""
    if preset not in PRESETS:
        raise MachineError(f"no preset named {preset}; try one of {', '.join(sorted(PRESETS))}")
    spec = PRESETS[preset]
    budget = spec["budget"] if max_tris is None else max_tris
    rng = Rng(seed)
    mesh = Mesh()

    jit = spec["jitter"]
    dims = [d * rng.uniform(1.0 - jit, 1.0 + jit) for d in spec["dims"]]
    axes = ((1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0))
    mesh.block((0.0, 0.0, dims[2] / 2.0), axes, (dims[0] / 2.0, dims[1] / 2.0, dims[2] / 2.0), WOOD)

    # The hull: each face that the rule allows grows a run of segments.
    for fi in range(len(mesh.faces)):
        if not mesh.faces[fi].live:
            continue
        if not _extrudes(spec["extrude"], mesh.normal(fi)):
            continue
        if rng.random() >= spec["chance"]:
            continue
        _hull_run(mesh, rng, fi, spec)

    # Large asymmetric blocks that stick out of the hull.
    if spec["p_asym"] > 0.0:
        for fi in range(len(mesh.faces)):
            if not mesh.faces[fi].live or face_aspect(mesh, fi) > 4.0:
                continue
            if rng.random() < spec["p_asym"]:
                length = rng.uniform(0.1, 0.4)
                for _ in range(rng.randint(*spec["asym_segments"])):
                    fi = mesh.extrude(fi, length)[0]
                    if rng.random() > 0.25:
                        s = 1.0 / rng.uniform(1.1, 1.5)
                        mesh.scale(fi, s, s)

    # Sort the faces by the way they point, then add detail to each group.
    plan = []
    for fi in range(len(mesh.faces)):
        if not mesh.faces[fi].live or face_aspect(mesh, fi) > 3.0:
            continue
        category = face_category(mesh.normal(fi))
        if category is None:
            continue
        kind = rng.pick(spec["detail"].get(category, ()))
        if kind and kind != "none":
            plan.append((fi, kind))
    for fi, kind in plan:
        DETAIL[kind](mesh, rng, fi)

    if spec["mirror"]:
        _mirror_y(mesh)
    _ground(mesh)

    tris = mesh.tri_count()
    if budget and tris > budget:
        raise MachineError(f"{preset} seed={seed}: {tris} tris is over the {budget} budget")
    return mesh


def summary(mesh, preset, seed, budget):
    verts, faces, mats = mesh.arrays()
    tris = sum(len(f) - 2 for f in faces)
    lo = [min(v[k] for v in verts) for k in range(3)] if verts else [0.0, 0.0, 0.0]
    hi = [max(v[k] for v in verts) for k in range(3)] if verts else [0.0, 0.0, 0.0]
    body = {
        "v": [[round(c, 9) for c in p] for p in verts],
        "f": [list(f) for f in faces],
        "m": mats,
    }
    digest = hashlib.sha256(json.dumps(body, separators=(",", ":"), sort_keys=True).encode("utf-8")).hexdigest()
    return {
        "preset": preset,
        "seed": str(seed),
        "verts": len(verts),
        "faces": len(faces),
        "tris": tris,
        "budget": budget,
        "materials": sorted({SLOT_NAMES[m] for m in mats}),
        "bounds_m": {"min": [round(c, 4) for c in lo], "max": [round(c, 4) for c in hi]},
        "sha256": digest,
    }


# ------------------------------------------------------------------ Blender output


# The one export setting set. `prop_gen` exports machines through this too, so
# the standalone cook and the props run write the same bytes.
GLTF_ARGS = {
    "export_format": "GLB",
    "export_apply": True,
    "export_texcoords": False,
    "export_normals": True,
    "use_selection": True,
}


def build_object(verts, faces, mats, name):
    """The Blender object for one baked machine: three flat slots, flat faces.

    Needs Blender's bpy but never resets the scene, so a caller that already
    has one (`prop_gen`) can add a machine to it.
    """
    import bpy  # deferred: the numbers above run without Blender

    slots = []
    for slot_name, rgb in SLOTS:
        mat = bpy.data.materials.new(slot_name)
        mat.diffuse_color = (rgb[0], rgb[1], rgb[2], 1.0)
        slots.append(mat)
    me = bpy.data.meshes.new(name)
    me.from_pydata(verts, [], faces)
    me.update()
    for mat in slots:
        me.materials.append(mat)
    for poly, slot in zip(me.polygons, mats):
        poly.material_index = slot
        poly.use_smooth = False
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    return ob


def export_object(ob, path):
    """Write one object as a GLB and refuse an empty file. Returns its tris."""
    import bpy

    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    bpy.ops.export_scene.gltf(filepath=path, **GLTF_ARGS)
    if not os.path.exists(path) or os.path.getsize(path) == 0:
        raise MachineError(f"could not write {path}")
    return sum(len(p.vertices) - 2 for p in ob.data.polygons)


def write_glb(verts, faces, mats, path, name):
    """Write the mesh as a GLB with three flat materials. Needs Blender's bpy."""
    import bpy  # deferred: the numbers above run without Blender

    bpy.ops.wm.read_factory_settings(use_empty=True)
    ob = build_object(verts, faces, mats, name)
    return export_object(ob, path)


# ------------------------------------------------------------------ CLI


class UsageError(Exception):
    pass


def parse(argv):
    opts = {"out": None, "preset": "machine", "seed": "0", "max_tris": None, "stats": False, "list": False}
    i = 0
    while i < len(argv):
        arg = argv[i]
        if arg == "--stats":
            opts["stats"] = True
        elif arg == "--list":
            opts["list"] = True
        elif arg in ("--preset", "--seed", "--max-tris"):
            if i + 1 >= len(argv):
                raise UsageError(f"{arg} needs a value")
            value = argv[i + 1]
            i += 1
            if arg == "--preset":
                opts["preset"] = value
            elif arg == "--seed":
                opts["seed"] = value
            else:
                try:
                    opts["max_tris"] = int(value)
                except ValueError:
                    raise UsageError(f"--max-tris needs a whole number, got {value}")
        elif arg.startswith("--"):
            raise UsageError(f"unknown flag {arg}")
        elif opts["out"] is None:
            opts["out"] = arg
        else:
            raise UsageError(f"extra argument {arg}")
        i += 1
    return opts


def seed_value(text):
    return int(text) if text.isdigit() else text


def main(argv):
    try:
        opts = parse(argv)
    except UsageError as err:
        sys.stderr.write(f"E_USAGE {err}\n{__doc__}")
        return EXIT_USAGE

    if opts["list"]:
        listing = {name: {"budget": spec["budget"], "title": spec["title"]} for name, spec in PRESETS.items()}
        print(json.dumps({"presets": listing}, indent=2, sort_keys=True))
        return 0

    if opts["preset"] not in PRESETS:
        sys.stderr.write(f"E_USAGE no preset named {opts['preset']}; try one of {', '.join(sorted(PRESETS))}\n")
        return EXIT_USAGE
    seed = seed_value(opts["seed"])
    budget = PRESETS[opts["preset"]]["budget"] if opts["max_tris"] is None else opts["max_tris"]
    try:
        mesh = generate(opts["preset"], seed, opts["max_tris"])
    except MachineError as err:
        sys.stderr.write(f"E_MACHINE {err}\n")
        return EXIT_FAIL

    info = summary(mesh, opts["preset"], opts["seed"], budget)
    if opts["stats"]:
        print(json.dumps(info, indent=2, sort_keys=True))
        return 0
    if opts["out"] is None:
        sys.stderr.write(f"E_USAGE need an output .glb path, or --stats / --list\n{__doc__}")
        return EXIT_USAGE

    verts, faces, mats = mesh.arrays()
    stem = os.path.splitext(os.path.abspath(opts["out"]))[0]
    os.makedirs(os.path.dirname(stem), exist_ok=True)
    try:
        blender_tris = write_glb(verts, faces, mats, stem + ".glb", f"machine_{opts['preset']}")
    except ImportError:
        sys.stderr.write("E_MACHINE the GLB write needs Blender: run with blender -b -P, or use --stats\n")
        return EXIT_FAIL
    except MachineError as err:
        sys.stderr.write(f"E_MACHINE {err}\n")
        return EXIT_FAIL
    if blender_tris != info["tris"]:
        sys.stderr.write(f"E_MACHINE Blender kept {blender_tris} tris, the mesh has {info['tris']}\n")
        return EXIT_FAIL
    with open(stem + ".machine.json", "w", encoding="utf-8") as handle:
        json.dump(info, handle, indent=2, sort_keys=True)
    print(f"MACHINE {opts['preset']} seed={opts['seed']} tris={info['tris']} "
          f"materials={','.join(info['materials'])} -> {stem}.glb")
    return 0


if __name__ == "__main__":
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else sys.argv[1:]
    sys.exit(main(argv))
