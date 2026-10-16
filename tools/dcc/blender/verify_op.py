# verify_op.py - GEO1 check after each recipe op.
# Own code, ideas only from the donor. No donor bytes pasted.
# Donor: PatrykIti/blender-ai-mcp modeling.py + mesh.py (Apache-2.0).
# Run by Blender as a lone tool. Never linked. Never in the tick.
# Each op must pass object count plus tri count plus a NaN check.
# Fail prints recipe.verify_failed and exits 5.
import hashlib
import json
import math
import sys

# Base 4 from GEO1b plus 4 new toward 12.
ALLOW_OPS = [
    "cube",
    "plane",
    "bevel",
    "extrude",
    "boolean",
    "subdivide",
    "decimate",
    "uv_smart",
]

FAIL_EXIT = 5


def fail(op, check, detail):
    # One line a recipe runner can grep.
    sys.stdout.write(f"recipe.verify_failed op={op} check={check} {detail}\n")
    return FAIL_EXIT


def parse_verts(text):
    # Form: "x,y,z;x,y,z". Empty means no spot check.
    if not text:
        return []
    out = []
    for part in text.split(";"):
        part = part.strip()
        if not part:
            continue
        nums = [float(x) for x in part.split(",")]
        out.append(nums)
    return out


def has_bad_vert(verts):
    # True on NaN, inf, or short rows.
    for v in verts:
        if len(v) != 3:
            return True
        for n in v:
            if not math.isfinite(n):
                return True
    return False


def verify(op, objects, tris, exp_objects, exp_tris, verts):
    # Order is fixed so each miss names one check.
    if op not in ALLOW_OPS:
        return ("unknown_op", f"want one of {','.join(ALLOW_OPS)}")
    if objects != exp_objects:
        return ("object_count", f"got {objects} want {exp_objects}")
    if tris != exp_tris:
        return ("tri_count", f"got {tris} want {exp_tris}")
    if has_bad_vert(verts):
        return ("nan_vertices", "verts hold NaN, inf, or a short row")
    return (None, "ok")


def sidecar_hash(record):
    # Stable hash for the sidecar. Same input gives same hash.
    blob = json.dumps(record, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(blob.encode("utf-8")).hexdigest()


def usage():
    return (
        "usage: verify_op.py --op <name> --objects N --tris N "
        "--exp-objects N --exp-tris N [--verts \"x,y,z;...\"]\n"
        "  --list-ops prints the 8 allowed ops.\n"
        "Fail exits 5 with recipe.verify_failed. Pass prints recipe.verify_ok.\n"
    )


def main(argv):
    if "--help" in argv or "-h" in argv:
        sys.stdout.write(usage())
        sys.stdout.write("exit 5 on any verify miss.\n")
        return 0
    if "--list-ops" in argv:
        sys.stdout.write("\n".join(ALLOW_OPS) + "\n")
        return 0
    try:
        op = argv[argv.index("--op") + 1]
        objects = int(argv[argv.index("--objects") + 1])
        tris = int(argv[argv.index("--tris") + 1])
        exp_objects = int(argv[argv.index("--exp-objects") + 1])
        exp_tris = int(argv[argv.index("--exp-tris") + 1])
    except (ValueError, IndexError):
        sys.stdout.write(usage())
        return 2
    verts_text = ""
    if "--verts" in argv:
        try:
            verts_text = argv[argv.index("--verts") + 1]
        except IndexError:
            verts_text = ""
    try:
        verts = parse_verts(verts_text)
    except ValueError:
        return fail(op if "op" in dir() else "?", "nan_vertices", "verts do not parse")
    check, detail = verify(op, objects, tris, exp_objects, exp_tris, verts)
    if check is not None:
        return fail(op, check, detail)
    sys.stdout.write(f"recipe.verify_ok op={op} objects={objects} tris={tris}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
