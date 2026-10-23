"""BX25: headless Blender cell-fracture column for BreakBench.

Run under Blender 5.2 (the official Cell Fracture 0.2.1 extension):

    blender --background --factory-startup --python-exit-code 2 \
        --python fracture_column.py -- \
        --sites sites-24.json --out run1.json --seed 7 --expect-cells 24

The fixture is one wall (the engine's 4 x 3 x 0.4 m wall from
`crates/destruction/src/fracture.rs::wall`) and 24 Voronoi sites drawn by
scipy (`sites.py`). The add-on cuts the wall by the sites; the script writes
one JSON record: cell count, canonical bake bytes and SHA-256, timings.

Determinism: the add-on adds 1 mm of `random.random()` jitter to every hull
vertex, so the script seeds Python's RNG with --seed before the operator. The
canonical bytes quantise world vertices to 10 um, deduplicate them, map faces
to those vertices, sort faces and cell records, then hash the whole stream.
Two runs with the same sites, seed and Blender version must agree on the SHA.

Extension: Cell Fracture is not bundled in Blender 5.2; it is the official
extension `bl_ext.user_default.cell_fracture` (0.2.1, Blender 4.2+,
GPL-3.0-or-later, https://extensions.blender.org/add-ons/cell-fracture/). Install it once
with `blender --command extension install-file -r user_default -e <zip>` or
the UI; the script enables it per run, so --factory-startup still works.
"""

import argparse
import hashlib
import json
import os
import random
import struct
import sys
import time
import tomllib

import bpy


FORMAT = "bx25-blender-column-v1"
BAKE_MAGIC = b"BX25BAKE1"
QUANTUM = 100_000  # 10 um in metres
WALL_MIN = (-2.0, -0.2, 0.0)
WALL_MAX = (2.0, 0.2, 3.0)
CELL_COLLECTION = "BX25Cells"


def parse_args() -> argparse.Namespace:
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    parser = argparse.ArgumentParser(description="BX25 Blender fracture column")
    parser.add_argument("--sites", required=True, help="fixture JSON from sites.py")
    parser.add_argument("--out", required=True, help="result JSON path")
    parser.add_argument("--seed", type=int, default=7,
                        help="Python RNG seed for the add-on hull jitter")
    parser.add_argument("--expect-cells", type=int, default=24)
    return parser.parse_args(argv)


def ensure_fracture_addon() -> str:
    """Return the enabled module name, or raise with the install command."""
    modules = ("bl_ext.user_default.cell_fracture", "object_fracture_cell")
    for module in modules:
        try:
            bpy.ops.preferences.addon_enable(module=module)
        except Exception:
            continue
        try:
            bpy.ops.object.add_fracture_cell_objects.get_rna_type()
            return module
        except Exception:
            continue
    raise RuntimeError(
        "Cell Fracture add-on missing: install the official 0.2.1 extension "
        "from https://extensions.blender.org/add-ons/cell-fracture/ then rerun"
    )


def addon_version(module: str) -> str:
    """Read the extension manifest version when the module is installed."""
    try:
        parts = module.split(".")
        if len(parts) >= 3 and parts[0] == "bl_ext":
            root = bpy.utils.user_resource("EXTENSIONS")
            manifest = os.path.join(root, parts[1], parts[2], "blender_manifest.toml")
            with open(manifest, "rb") as handle:
                return str(tomllib.load(handle).get("version", "unknown"))
    except Exception:
        pass
    return "unknown"


def wall_object() -> bpy.types.Object:
    """The engine fixture wall in Blender space, no object transform."""
    x0, x1 = WALL_MIN[0], WALL_MAX[0]
    y0, y1 = WALL_MIN[1], WALL_MAX[1]
    z0, z1 = WALL_MIN[2], WALL_MAX[2]
    verts = [
        (x0, y0, z0), (x1, y0, z0), (x1, y1, z0), (x0, y1, z0),
        (x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1),
    ]
    faces = [
        (0, 3, 2, 1),  # bottom, -Z
        (4, 5, 6, 7),  # top, +Z
        (0, 1, 5, 4),  # front, -Y
        (3, 7, 6, 2),  # back, +Y
        (0, 4, 7, 3),  # left, -X
        (1, 2, 6, 5),  # right, +X
    ]
    mesh = bpy.data.meshes.new("BX25Wall")
    mesh.from_pydata(verts, [], faces)
    mesh.update()
    obj = bpy.data.objects.new("BX25Wall", mesh)
    bpy.context.scene.collection.objects.link(obj)
    return obj


def sites_object(points: list[tuple[float, float, float]]) -> bpy.types.Object:
    mesh = bpy.data.meshes.new("BX25Sites")
    mesh.from_pydata(points, [], [])
    mesh.update()
    obj = bpy.data.objects.new("BX25Sites", mesh)
    bpy.context.scene.collection.objects.link(obj)
    return obj


def quantize(value: float) -> int:
    return int(round(value * QUANTUM))


def cell_bytes(obj: bpy.types.Object) -> bytes:
    """Canonical bake bytes for one cell: sorted vertices, sorted faces."""
    matrix = obj.matrix_world.copy()
    mesh = obj.data
    coords = sorted({
        (quantize((matrix @ v.co).x), quantize((matrix @ v.co).y),
         quantize((matrix @ v.co).z))
        for v in mesh.vertices
    })
    if not coords:
        raise RuntimeError("empty cell %s" % obj.name)
    index = {coord: i for i, coord in enumerate(coords)}
    faces = []
    for poly in mesh.polygons:
        loop = []
        for vertex in poly.vertices:
            co = matrix @ mesh.vertices[vertex].co
            loop.append(index[(quantize(co.x), quantize(co.y), quantize(co.z))])
        if not loop:
            continue
        rotations = [tuple(loop[k:] + loop[:k]) for k in range(len(loop))]
        faces.append(min(rotations))
    faces.sort()
    out = bytearray()
    out += struct.pack("<I", len(coords))
    for coord in coords:
        out += struct.pack("<iii", *coord)
    out += struct.pack("<I", len(faces))
    for face in faces:
        out += struct.pack("<I", len(face))
        for vertex in face:
            out += struct.pack("<I", vertex)
    return bytes(out)


def cell_counts(record: bytes) -> tuple[int, int]:
    """(vertices, faces) of one canonical cell record."""
    vertices = struct.unpack_from("<I", record, 0)[0]
    faces = struct.unpack_from("<I", record, 4 + vertices * 12)[0]
    return vertices, faces


def validate_fixture(fixture: dict, expected: int) -> list[tuple[float, float, float]]:
    if fixture.get("format") != "bx25-sites-v1":
        raise RuntimeError("bad sites format %r" % fixture.get("format"))
    wall = fixture.get("wall_m", {})
    if tuple(wall.get("min", ())) != WALL_MIN or tuple(wall.get("max", ())) != WALL_MAX:
        raise RuntimeError("sites wall %r is not the engine wall" % (wall,))
    spec_cells = fixture.get("engine", {}).get("spec", {}).get("cells")
    if spec_cells != expected:
        raise RuntimeError("fixture spec has %r cells, expected %d" % (spec_cells, expected))
    raw = fixture.get("sites", [])
    if len(raw) != expected:
        raise RuntimeError("fixture has %d sites, expected %d" % (len(raw), expected))
    points = []
    for point in raw:
        if len(point) != 3:
            raise RuntimeError("site %r is not 3D" % (point,))
        x, y, z = (float(v) for v in point)
        if not (WALL_MIN[0] < x < WALL_MAX[0]
                and WALL_MIN[1] < y < WALL_MAX[1]
                and WALL_MIN[2] < z < WALL_MAX[2]):
            raise RuntimeError("site %r is outside the wall" % (point,))
        points.append((x, y, z))
    return points


def main() -> int:
    args = parse_args()
    started = time.perf_counter()
    with open(args.sites, "rb") as handle:
        sites_raw = handle.read()
    fixture = json.loads(sites_raw.decode("utf-8"))
    points = validate_fixture(fixture, args.expect_cells)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    module = ensure_fracture_addon()

    wall = wall_object()
    sites = sites_object(points)
    sites.parent = wall
    bpy.context.view_layer.update()

    bpy.ops.object.select_all(action="DESELECT")
    wall.select_set(True)
    bpy.context.view_layer.objects.active = wall

    # The add-on jitters every hull vertex with the global RNG. Reset it right
    # before the operator so two processes consume the same sequence.
    random.seed(args.seed)
    op_started = time.perf_counter()
    result = bpy.ops.object.add_fracture_cell_objects(
        source={"VERT_CHILD"},
        source_limit=0,
        source_noise=0.0,
        cell_scale=(1.0, 1.0, 1.0),
        recursion=0,
        use_smooth_faces=False,
        use_sharp_edges=False,
        use_sharp_edges_apply=False,
        use_data_match=False,
        use_island_split=False,
        margin=0.0,
        material_index=0,
        use_interior_vgroup=False,
        use_debug_bool=False,
        use_debug_points=False,
        use_debug_redraw=False,
        use_recenter=False,
        use_remove_original=True,
        collection_name=CELL_COLLECTION,
        mass_mode="VOLUME",
        mass=1.0,
    )
    operator_ms = (time.perf_counter() - op_started) * 1000.0
    if result != {"FINISHED"}:
        raise RuntimeError("cell fracture operator returned %r" % (result,))

    collection = bpy.data.collections.get(CELL_COLLECTION)
    if collection is None:
        raise RuntimeError("cell collection %r missing" % CELL_COLLECTION)
    cells = [obj for obj in collection.objects if obj.type == "MESH"]
    records = [cell_bytes(obj) for obj in cells]
    records.sort()
    blob = BAKE_MAGIC + struct.pack("<Q", len(records))
    for record in records:
        blob += struct.pack("<Q", len(record)) + record
    digest = hashlib.sha256(blob).hexdigest()

    counts = [cell_counts(record) for record in records]
    vertex_count = sum(vertices for vertices, _ in counts)
    face_count = sum(faces for _, faces in counts)

    passed = len(records) == args.expect_cells
    out = {
        "format": FORMAT,
        "status": "PASS" if passed else "FAIL",
        "blender": bpy.app.version_string,
        "addon": module,
        "addon_version": addon_version(module),
        "wall_m": {"min": list(WALL_MIN), "max": list(WALL_MAX)},
        "sites_file_sha256": hashlib.sha256(sites_raw).hexdigest(),
        "site_count": len(points),
        "expected_cells": args.expect_cells,
        "cell_count": len(records),
        "cell_count_match": passed,
        "jitter_seed": args.seed,
        "canonical_quantum_m": 1.0 / QUANTUM,
        "bake_bytes": len(blob),
        "bake_sha256": digest,
        "vertex_count": vertex_count,
        "face_count": face_count,
        "operator_ms": round(operator_ms, 3),
        "total_ms": round((time.perf_counter() - started) * 1000.0, 3),
    }
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(out, handle, indent=2)
        handle.write("\n")
    print("BX25_RESULT " + json.dumps(out, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
