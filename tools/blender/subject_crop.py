"""Crop the subject out of a concept.

A concept sheet is the best design reference and the worst mesh input: the
model answers a sheet with four figures and gear studies, and SF3D reads one
object. So the subject has to be cut out before the model sees it.

The cut is found by projection, not by a model: the background is the flat
colour of the border, the mask is everything that differs from it, and the
subject is the widest unbroken run of columns that carry the figure. A
single full body figure is one run; a sheet is several, and the central
figure is the widest.

Run:
  blender -b --factory-startup --python-exit-code 1 -P tools/blender/subject_crop.py -- <in.jpg> <out.png> [--margin 0.06]

Prints one JSON line: the crop box, the size, and how much of the frame the
subject took.
"""

import json
import os
import sys

import bpy

try:
    import numpy as np
except ImportError:  # pragma: no cover - Blender ships numpy
    np = None

EXIT_USAGE = 2
EXIT_FAIL = 3

BG_TOLERANCE = 0.06
COLUMN_FLOOR = 0.02   # a column with less than this share of the height is a gap


class CropError(Exception):
    """A crop that could not be made."""


def arg(name, fallback=None):
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if name in argv:
        i = argv.index(name)
        if i + 1 < len(argv):
            return argv[i + 1]
    return fallback


def longest_run(flags):
    """The longest run of True in a boolean array, as (start, end)."""
    best = (0, -1)
    start = None
    for index, flag in enumerate(flags):
        if flag and start is None:
            start = index
        elif not flag and start is not None:
            if index - start > best[1] - best[0]:
                best = (start, index - 1)
            start = None
    if start is not None and len(flags) - start > best[1] - best[0]:
        best = (start, len(flags) - 1)
    return best


def crop(src, dest, margin):
    image = bpy.data.images.load(src, check_existing=False)
    width, height = image.size
    if np is None:
        raise CropError("numpy is required")
    buf = np.empty(width * height * 4, dtype=np.float32)
    image.pixels.foreach_get(buf)
    pixels = buf.reshape(height, width, 4)

    rgb = pixels[:, :, :3]
    alpha = pixels[:, :, 3]
    # The background is a gradient, not a flat colour, so the background of a
    # row is the median of that row's edges. A sheet has studies at the sides
    # and a figure in the middle: the edges are the background, the studies
    # and the figure are not.
    edge = max(4, int(width * 0.04))
    row_bg = np.median(np.concatenate([rgb[:, :edge, :], rgb[:, -edge:, :]], axis=1), axis=1)
    diff = np.abs(rgb - row_bg[:, None, :]).max(axis=2)
    mask = (diff > BG_TOLERANCE) & (alpha > 0.5)
    if not mask.any():
        raise CropError("the image is all background")
    background = np.median(row_bg, axis=0)

    columns = mask.sum(axis=0)
    keep = columns > (COLUMN_FLOOR * height)
    x0, x1 = longest_run(keep)
    if x1 < x0:
        raise CropError("no column of the image holds the subject")

    rows = mask[:, x0:x1 + 1].sum(axis=1)
    ys = np.where(rows > 0)[0]
    if ys.size == 0:
        raise CropError("the subject columns hold nothing")
    y0, y1 = int(ys[0]), int(ys[-1])

    # The pixel rows come bottom up: row 0 is the bottom of the image.
    top = height - 1 - y1
    bottom = height - 1 - y0

    x0 = max(0, int(x0) - int(margin * width))
    x1 = min(width - 1, int(x1) + int(margin * width))
    top = max(0, top - int(margin * height))
    bottom = min(height - 1, bottom + int(margin * height))

    subject_w = x1 - x0 + 1
    subject_h = bottom - top + 1
    side = max(subject_w, subject_h)
    side = int(side * 1.02)
    # The window can never be larger than the picture itself.
    side = max(8, min(side, width, height))
    cx = (x0 + x1) // 2
    cy = (top + bottom) // 2
    sx0 = max(0, min(width - side, cx - side // 2))
    sy0 = max(0, min(height - side, cy - side // 2))
    sx1 = min(width - 1, sx0 + side - 1)
    sy1 = min(height - 1, sy0 + side - 1)

    out = bpy.data.images.new("crop", width=side, height=side, alpha=False)
    out_buf = np.empty(side * side * 4, dtype=np.float32)
    for row in range(side):
        source_row = min(height - 1, sy0 + row)
        target_row = side - 1 - row
        start = target_row * side * 4
        for channel in range(3):
            out_buf[start + channel:start + side * 4:4] = rgb[source_row, sx0:sx0 + side, channel]
        out_buf[start + 3:start + side * 4:4] = 1.0
    out.pixels.foreach_set(out_buf)
    out.file_format = "PNG"
    out.filepath_raw = dest
    out.save()

    metrics = {
        "source": os.path.abspath(src),
        "out": os.path.abspath(dest),
        "source_size": [width, height],
        "subject_box": [x0, top, x1, bottom],
        "subject_px": [subject_w, subject_h],
        "crop": [sx0, sy0, sx1, sy1],
        "crop_size": side,
        "background_rgb": [round(float(v), 4) for v in background],
        "fill_of_frame": round(float(mask.sum()) / float(width * height), 4),
    }
    print(json.dumps(metrics, sort_keys=True))
    return 0


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(args) < 2:
        sys.stderr.write("usage: blender -b --factory-startup --python-exit-code 1 "
                         "-P subject_crop.py -- <in.jpg> <out.png> [--margin 0.06]\n")
        return EXIT_USAGE
    margin = float(arg("--margin", "0.06"))
    return crop(args[0], args[1], margin)


if __name__ == "__main__":
    try:
        code = main()
    except CropError as err:
        sys.stderr.write(f"E_CROP {err}\n")
        code = EXIT_FAIL
    except Exception as err:
        sys.stderr.write(f"E_CROP {type(err).__name__}: {err}\n")
        code = EXIT_FAIL
    if code != 0:
        sys.exit(code)
