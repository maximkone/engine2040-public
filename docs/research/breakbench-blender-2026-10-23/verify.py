#!/usr/bin/env python3
"""BX25 bake-bytes-x2 gate: two Blender column records must agree.

Run: python verify.py run1.json run2.json

Timings are allowed to differ; the fixture hash, cell count and canonical
bake bytes/SHA must not. Exit 0 is PASS, 1 is FAIL.
"""

import json
import sys


# Every field that must be byte-for-byte stable across two independent runs.
STABLE_KEYS = (
    "format",
    "status",
    "blender",
    "addon",
    "addon_version",
    "sites_file_sha256",
    "site_count",
    "expected_cells",
    "cell_count",
    "cell_count_match",
    "jitter_seed",
    "canonical_quantum_m",
    "bake_bytes",
    "bake_sha256",
    "vertex_count",
    "face_count",
)


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: verify.py run1.json run2.json", file=sys.stderr)
        return 2

    with open(sys.argv[1], encoding="utf-8") as handle:
        first = json.load(handle)
    with open(sys.argv[2], encoding="utf-8") as handle:
        second = json.load(handle)

    errors = []
    for key in STABLE_KEYS:
        if first.get(key) != second.get(key):
            errors.append("%s: %r != %r" % (key, first.get(key), second.get(key)))
    for name, record in (("run1", first), ("run2", second)):
        if record.get("status") != "PASS":
            errors.append("%s status is %r" % (name, record.get("status")))
        if record.get("cell_count") != record.get("expected_cells"):
            errors.append(
                "%s cells %r != expected %r"
                % (name, record.get("cell_count"), record.get("expected_cells"))
            )
        if not isinstance(record.get("bake_bytes"), int) or record["bake_bytes"] <= 0:
            errors.append("%s bake_bytes is not positive" % name)

    if errors:
        print("FAIL " + "; ".join(errors))
        return 1
    print(
        "PASS cells=%d bake_bytes=%d sha256=%s operator_ms=%s/%s"
        % (
            first["cell_count"],
            first["bake_bytes"],
            first["bake_sha256"],
            first["operator_ms"],
            second["operator_ms"],
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
