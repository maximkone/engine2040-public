#!/usr/bin/env python3
"""BX25 fixture: seeded Voronoi sites for the Blender cell-fracture column.

The wall is the engine's fixture wall in `crates/destruction/src/fracture.rs`
(`fn wall`): 4 x 3 x 0.4 m, hit at [0.3, 1.4, 0.2], spec(24, 7). Engine space
is +Y up; Blender space is +Z up, so the wall maps as

    Blender (x, y, z) = (engine x, engine z_depth, engine y_up)

and occupies x [-2, 2], y [-0.2, 0.2], z [0, 3].

Sites are drawn with scipy's seeded Latin-hypercube sampler and mapped into
the wall interior. The first seed from --seed (default 7) whose 24 sites keep
at least --min-sep metres apart is pinned; the printed JSON is the fixture, so
the headless Blender run needs no scipy.

The engine's BX14 rule (impact energy -> cell count) is not landed. This
fixture pins the count at 24, the engine wall spec's cell count; the BX12 stone
row (toughness 8.0, brittleness 0.85, shard rule few-large) names the wall
material. The column proves the tool and the count, not a BX14 rule.

Run: python sites.py --out sites-24.json
"""

import argparse
import json
import sys

from scipy.stats import qmc
import scipy


WALL_MIN = (-2.0, -0.2, 0.0)
WALL_MAX = (2.0, 0.2, 3.0)
# Keep every site off the wall skin so every Voronoi cell has volume.
MARGIN = (0.10, 0.04, 0.15)


def interior(seed: int, count: int) -> list[tuple[float, float, float]]:
    """Draw `count` seeded points strictly inside the wall, Blender space."""
    unit = qmc.LatinHypercube(d=3, seed=seed).random(count)
    points = []
    for row in unit:
        point = []
        for axis in range(3):
            lo = WALL_MIN[axis] + MARGIN[axis]
            hi = WALL_MAX[axis] - MARGIN[axis]
            point.append(round(lo + float(row[axis]) * (hi - lo), 6))
        points.append(tuple(point))
    return points


def min_separation(points: list[tuple[float, float, float]]) -> float:
    """Smallest pairwise 3D distance, metres."""
    best = float("inf")
    for i, a in enumerate(points):
        for b in points[i + 1:]:
            d2 = sum((a[k] - b[k]) ** 2 for k in range(3))
            best = min(best, d2)
    return best ** 0.5


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, help="fixture JSON path")
    parser.add_argument("--count", type=int, default=24)
    parser.add_argument("--seed", type=int, default=7, help="first scipy seed to try")
    parser.add_argument("--min-sep", type=float, default=0.03,
                        help="minimum 3D site separation, metres")
    args = parser.parse_args()

    if args.count < 2:
        print("count must be at least 2", file=sys.stderr)
        return 2

    chosen_seed = None
    points = []
    separation = 0.0
    # A seed whose draw is too clustered makes slivers; take the next drawn
    # seed instead. The chosen seed is recorded and the JSON is pinned.
    for seed in range(args.seed, args.seed + 200):
        candidate = interior(seed, args.count)
        separation = min_separation(candidate)
        if separation >= args.min_sep:
            chosen_seed = seed
            points = candidate
            break
    if chosen_seed is None:
        print("no seed within 200 kept the minimum separation", file=sys.stderr)
        return 1

    fixture = {
        "format": "bx25-sites-v1",
        "purpose": "BX25 Blender-bpy fracture column for BreakBench",
        "space": "blender_xyz_m",
        "wall_m": {"min": list(WALL_MIN), "max": list(WALL_MAX)},
        "engine": {
            "wall": "crates/destruction/src/fracture.rs::wall",
            "hit": [0.3, 1.4, 0.2],
            "spec": {"cells": args.count, "seed": 7},
            "material": {
                "id": "stone",
                "toughness_q16": 524288,
                "brittleness_q16": 55706,
                "shard": "few_large",
                "source": "crates/destruction/src/material.rs SIEGE_ROWS[0]",
            },
            "bx14": "impact energy -> cell count is not landed; the spec pins 24",
        },
        "sampler": {
            "library": "scipy.stats.qmc",
            "method": "LatinHypercube",
            "scipy_version": scipy.__version__,
            "first_seed": args.seed,
            "seed": chosen_seed,
            "count": args.count,
            "min_separation_m": round(separation, 6),
        },
        "sites": [list(p) for p in points],
    }
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(fixture, handle, indent=2)
        handle.write("\n")
    print(
        "wrote %s: %d sites, scipy seed %d, min separation %.3f m"
        % (args.out, len(points), chosen_seed, separation)
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
