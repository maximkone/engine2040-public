#!/usr/bin/env python3
"""Generate one Kimodo combat clip from a cached text embedding.

Run it with the Kimodo venv python. clip.mjs does this for you:

    <KIMODO_HOME>/venv/Scripts/python.exe tools/motion/kimodo_gen.py \
        --prompt "A person raises a sword and brings it down in a strong overhead strike." \
        --seed 71235 --seconds 2.0 --out-dir target/motion/slash_over --stem slash_over

The text encoder (LLM2Vec on Meta-Llama-3-8B) never loads here. The prompt must be
in the embedding cache (emb.npz: texts [N], emb [N,4096]) and is matched by exact
text. A prompt that is not cached exits 2 and prints the Kaggle step that adds it.

--samples N draws N candidates from one seed (the trial's batch) and --pick K keeps
candidate K. The shipped slash_over was made with --samples 4 --pick 1: a batch draw,
not the single-sample clip (the single sample is candidate 0 of the batch).

Writes into --out-dir:
  <stem>.bvh                   SOMA-77 BVH, Kimodo's own export (the teacher's file)
  <stem>.npz                   the bake source: the BVH converted back to the Kimodo
                               NPZ layout, rewritten with C-contiguous arrays
  <stem>.raw.npz               Kimodo's direct output, kept for the record
  <stem>.bvh.provenance.json   prompt, seed, model, licences, hashes

Exit codes: 0 ok, 2 prompt not in the cache, 3 generation or conversion failed,
4 setup (no cache, no checkpoint, kimodo does not import).

Ported from the sidecar (C:\\forge\\sidecars\\kimodo\\generate_once.py) and the
trial scripts (empire card kimodo-trial-2026-10-20/scripts/trial.py). Same sampler
settings: 100 steps, post-processing on, 5 transition frames, one sample.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import time

MODEL_NAME = "Kimodo-SOMA-RP-v1.2"
MODEL_ID = "nvidia/Kimodo-SOMA-RP-v1.2"
EMB_WIDTH = 4096
FPS_FALLBACK = 30.0
MIN_SECONDS = 0.5
MAX_SECONDS = 10.0
MAX_PROMPT = 500
DEFAULT_STEPS = 100
VRAM_FRACTION = 0.7  # share of the card this process may allocate
CONVERT_TIMEOUT_S = 600

EXIT_OK = 0
EXIT_MISS = 2
EXIT_GEN = 3
EXIT_SETUP = 4

# Keys whose leading batch axis of 1 is dropped before saving one sample.
EXPECTED_NDIM = {
    "local_rot_mats": 4,
    "global_rot_mats": 4,
    "posed_joints": 3,
    "root_positions": 2,
    "smooth_root_pos": 2,
    "foot_contacts": 2,
    "global_root_heading": 2,
}

MISS_HELP = """\
prompt not in the encoder cache: {prompt!r}
The text encoder never runs on this PC, so a new prompt needs a one-time encode:
  1. Add the prompt as a new line in PROMPTS in tools/motion/kaggle_encode.py
     (keep the existing lines; the cache is rebuilt from the whole list).
  2. Run that file on a Kaggle GPU notebook (T4). Its header says how.
  3. Copy the kernel's emb.npz and emb.provenance.json to {cache_dir}
     (the cache file is {cache}).
Then run this command again.
"""


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def drop_namespace_shadow() -> list:
    """Remove sys.path entries that shadow the real kimodo package.

    A directory named "kimodo" with no __init__.py on sys.path (the machine-wide
    PYTHONPATH carries C:\\forge\\sidecars) imports as a namespace package, which
    wins over the venv's editable install and breaks load_model. Returns the
    entries removed.
    """
    removed = []
    keep = []
    for entry in sys.path:
        try:
            cand = os.path.join(entry or ".", "kimodo")
            if os.path.isdir(cand) and not os.path.isfile(os.path.join(cand, "__init__.py")):
                removed.append(entry)
                continue
        except OSError:
            pass
        keep.append(entry)
    if removed:
        sys.path[:] = keep
    return removed


def pick_sample(key: str, array, samples: int, pick: int):
    """Keep candidate `pick` of a batch of `samples` model outputs."""
    want = EXPECTED_NDIM.get(key)
    if want is not None and array.ndim == want + 1 and array.shape[0] == samples:
        return array[pick]
    return array


def load_cache(path: str, prompt: str):
    """(texts, emb) from the cache, or (None, None, message)."""
    import numpy as np

    if not os.path.isfile(path):
        return None, None, f"embedding cache missing: {path}. Copy emb.npz there (see tools/motion/kaggle_encode.py)."
    try:
        z = np.load(path, allow_pickle=False)
    except Exception as exc:  # loud, names the file
        return None, None, f"embedding cache unreadable: {path} ({type(exc).__name__}: {exc})"
    if "texts" not in z.files or "emb" not in z.files:
        return None, None, f"embedding cache malformed: {path} needs 'texts' and 'emb'"
    texts = [str(t) for t in z["texts"]]
    emb = z["emb"]
    if emb.ndim != 2 or emb.shape[1] != EMB_WIDTH:
        return None, None, f"embedding cache malformed: emb shape {emb.shape}, want [N,{EMB_WIDTH}]"
    if emb.shape[0] != len(texts):
        return None, None, f"embedding cache malformed: {len(texts)} texts vs {emb.shape[0]} emb rows"
    if prompt not in texts:
        return None, None, "miss"
    return texts, emb, ""


def resolve_checkpoint(raw: str):
    """(model_dir, checkpoint_root) or (None, None, message)."""
    root = os.path.abspath(raw)
    if os.path.isfile(os.path.join(root, "config.yaml")):
        model_dir = root
    elif os.path.isfile(os.path.join(root, MODEL_NAME, "config.yaml")):
        model_dir = os.path.join(root, MODEL_NAME)
    else:
        return None, None, f"checkpoint missing: no config.yaml in {root} or {root}{os.sep}{MODEL_NAME}. Fix: set KIMODO_HOME to the folder that holds ckpt\\{MODEL_NAME}."
    if os.path.basename(model_dir) != MODEL_NAME:
        return None, None, f"checkpoint folder {model_dir} must be named {MODEL_NAME}"
    if not os.path.isfile(os.path.join(model_dir, "model.safetensors")):
        return None, None, f"checkpoint incomplete: model.safetensors missing in {model_dir}"
    return model_dir, os.path.dirname(model_dir), ""


class CacheEncoder:
    """Stands in for the text encoder: returns the cached vector for the exact prompt."""

    def __init__(self, texts, emb, prompt: str):
        self.map = {t: emb[i] for i, t in enumerate(texts)}
        self.prompt = prompt

    def to(self, *args, **kwargs):
        return self

    def __call__(self, texts):
        import numpy as np
        import torch

        if isinstance(texts, str):
            texts = [texts]
        rows = [self.map[t] for t in texts]  # KeyError = prompt not cached
        return torch.from_numpy(np.stack(rows).astype(np.float32))[:, None, :], [1] * len(rows)


def run_convert(src: str, dst: str, *args: str) -> None:
    """Run kimodo's motion_convert in this venv. Raises RuntimeError with the tail of its output."""
    cmd = [sys.executable, "-m", "kimodo.scripts.motion_convert", src, dst, *args]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=CONVERT_TIMEOUT_S)
    if proc.returncode != 0:
        tail = (proc.stderr or proc.stdout or "").strip()[-600:]
        raise RuntimeError(f"motion_convert exited {proc.returncode} ({' '.join(args)}): {tail or 'no output'}")


def check_bvh(path: str) -> int:
    with open(path, "rb") as fh:
        data = fh.read()
    if not data.lstrip().startswith(b"HIERARCHY") or b"MOTION" not in data:
        raise RuntimeError(f"{path} is not a BVH (needs HIERARCHY and MOTION blocks)")
    return len(data)


def rewrite_contiguous(src: str, dst: str) -> int:
    """Copy every array of an NPZ into dst with C-contiguous layout. Returns the array count."""
    import numpy as np

    with np.load(src, allow_pickle=False) as z:
        arrays = {k: np.ascontiguousarray(z[k]) for k in z.files}
    np.savez_compressed(dst, **arrays)
    return len(arrays)


def parse_args(argv=None):
    ap = argparse.ArgumentParser(description="Generate one Kimodo combat clip (BVH + bake-ready NPZ).")
    ap.add_argument("--prompt", required=True, help="the motion in words; must be in the embedding cache")
    ap.add_argument("--seed", type=int, required=True)
    ap.add_argument("--seconds", type=float, default=2.0)
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--stem", default="clip", help="file stem inside --out-dir")
    ap.add_argument("--steps", type=int, default=DEFAULT_STEPS)
    ap.add_argument("--samples", type=int, default=1, help="candidates drawn from the seed (a batch)")
    ap.add_argument("--pick", type=int, default=0, help="which candidate of the batch to keep (0-based)")
    ap.add_argument("--cache", default=os.environ.get("KIMODO_CACHE", ""), help="emb.npz (default KIMODO_HOME/emb.npz)")
    ap.add_argument("--ckpt-dir", default=os.environ.get("KIMODO_CKPT", ""), help="default KIMODO_HOME/ckpt")
    return ap.parse_args(argv)


def validate(args) -> str:
    """Return '' when the arguments are fine, else the reason."""
    if not args.prompt.strip():
        return "prompt is empty; describe the motion in words"
    if len(args.prompt) > MAX_PROMPT:
        return f"prompt is {len(args.prompt)} characters, past the {MAX_PROMPT} cap"
    if not (MIN_SECONDS <= args.seconds <= MAX_SECONDS):
        return f"seconds {args.seconds} outside [{MIN_SECONDS}, {MAX_SECONDS}]"
    if not (1 <= args.steps <= 500):
        return f"steps {args.steps} outside [1, 500]"
    if not (1 <= args.samples <= 16):
        return f"samples {args.samples} outside [1, 16]"
    if not (0 <= args.pick < args.samples):
        return f"pick {args.pick} outside [0, {args.samples - 1}] for --samples {args.samples}"
    if not args.stem or any(c in args.stem for c in "/\\:"):
        return f"stem {args.stem!r} must be a plain file name"
    return ""


def main(argv=None) -> int:
    args = parse_args(argv)
    why = validate(args)
    if why:
        print(f"kimodo_gen: {why}", file=sys.stderr)
        return EXIT_SETUP

    home = os.environ.get("KIMODO_HOME", r"C:\forge-media\kimodo")
    cache = args.cache or os.path.join(home, "emb.npz")
    ckpt_dir = args.ckpt_dir or os.path.join(home, "ckpt")

    # Offline: this process never reaches the network.
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["TRANSFORMERS_OFFLINE"] = "1"

    texts, emb, why = load_cache(cache, args.prompt)
    if texts is None:
        if why == "miss":
            print(MISS_HELP.format(prompt=args.prompt, cache_dir=os.path.dirname(cache) or ".", cache=cache), file=sys.stderr)
            print(json.dumps({"ok": False, "status": "missing_prompt", "prompt": args.prompt}), flush=True)
            return EXIT_MISS
        print(f"kimodo_gen: {why}", file=sys.stderr)
        return EXIT_SETUP

    model_dir, ckpt_root, why = resolve_checkpoint(ckpt_dir)
    if model_dir is None:
        print(f"kimodo_gen: {why}", file=sys.stderr)
        return EXIT_SETUP
    os.environ["CHECKPOINT_DIR"] = ckpt_root

    if drop_namespace_shadow():
        print("kimodo_gen: dropped a sys.path entry that shadows the kimodo package", file=sys.stderr)

    try:
        import numpy as np
        import torch
        from kimodo.model.load_model import load_model
    except Exception as exc:
        print(
            f"kimodo_gen: kimodo or torch does not import under {sys.executable} "
            f"({type(exc).__name__}: {exc}). Fix: run with <KIMODO_HOME>\\venv\\Scripts\\python.exe "
            f"and PYTHONPATH=<KIMODO_HOME>\\src.",
            file=sys.stderr,
        )
        return EXIT_SETUP

    device = "cuda:0" if torch.cuda.is_available() else "cpu"
    if device == "cpu":
        print("kimodo_gen: no CUDA device, generating on CPU (slow)", file=sys.stderr)
    else:
        torch.cuda.set_per_process_memory_fraction(VRAM_FRACTION)

    t0 = time.time()
    enc = CacheEncoder(texts, emb, args.prompt)
    model, resolved = load_model(
        MODEL_NAME, device=device, default_family="Kimodo", return_resolved_name=True, text_encoder=enc
    )
    fps = float(getattr(model, "fps", FPS_FALLBACK))
    frames = int(round(fps * args.seconds))
    if frames < 1:
        print(f"kimodo_gen: {args.seconds} s gives {frames} frames", file=sys.stderr)
        return EXIT_SETUP

    out_dir = os.path.abspath(args.out_dir)
    os.makedirs(out_dir, exist_ok=True)
    stem = os.path.join(out_dir, args.stem)
    raw_path = stem + ".raw.npz"
    bvh_path = stem + ".bvh"
    npz_path = stem + ".npz"
    conv_tmp = stem + ".conv.npz"
    prov_path = bvh_path + ".provenance.json"

    try:
        torch.manual_seed(int(args.seed))
        if device != "cpu":
            torch.cuda.manual_seed_all(int(args.seed))
        output = model(
            [args.prompt],
            [frames],
            constraint_lst=[],
            num_denoising_steps=int(args.steps),
            num_samples=int(args.samples),
            multi_prompt=True,
            num_transition_frames=5,
            post_processing=True,
            return_numpy=True,
        )
    except Exception as exc:
        print(f"kimodo_gen: generation failed ({type(exc).__name__}: {exc})", file=sys.stderr)
        return EXIT_GEN
    gen_s = round(time.time() - t0, 1)

    arrays = {k: pick_sample(k, v, args.samples, args.pick) for k, v in output.items() if isinstance(v, np.ndarray)}
    for key in ("local_rot_mats", "root_positions"):
        if key not in arrays:
            print(f"kimodo_gen: model output has no {key}; nothing to convert", file=sys.stderr)
            return EXIT_GEN

    try:
        # Numeric keys only: motion_convert hands every key to torch.from_numpy.
        np.savez_compressed(
            raw_path,
            fps=np.float32(fps),
            seed=np.int64(args.seed),
            steps=np.int64(args.steps),
            post=np.bool_(True),
            **arrays,
        )
        run_convert(raw_path, bvh_path, "--from", "kimodo", "--to", "soma-bvh", "--source-fps", f"{fps:g}")
        bvh_bytes = check_bvh(bvh_path)
        # The teacher's own round trip: BVH back to the Kimodo NPZ layout, then C order.
        run_convert(bvh_path, conv_tmp, "--from", "soma-bvh", "--to", "kimodo")
        n_arrays = rewrite_contiguous(conv_tmp, npz_path)
        os.remove(conv_tmp)
    except (RuntimeError, OSError, ValueError) as exc:
        print(f"kimodo_gen: conversion failed: {exc}", file=sys.stderr)
        return EXIT_GEN

    bvh_sha = sha256_file(bvh_path)
    npz_sha = sha256_file(npz_path)
    prov = {
        "file": os.path.basename(bvh_path),
        "sha256": bvh_sha,
        "bytes": bvh_bytes,
        "npz": os.path.basename(npz_path),
        "npz_sha256": npz_sha,
        "npz_arrays": n_arrays,
        "raw_npz_sha256": sha256_file(raw_path),
        "model": MODEL_ID,
        "resolved": resolved,
        "prompt": args.prompt,
        "seed": int(args.seed),
        "seconds": float(args.seconds),
        "steps": int(args.steps),
        "samples": int(args.samples),
        "batch_index": int(args.pick),
        "post_processing": True,
        "frames": frames,
        "fps": fps,
        "keyposes": None,
        "text_encoder": "pre-computed cache, LLM2Vec on Meta-Llama-3-8B-Instruct (Built with Meta Llama 3)",
        "licence": "Weights: NVIDIA Open Model License. Text encoder: Meta Llama 3 Community License (Built with Meta Llama 3).",
        "generated_by": "AI model (Kimodo), not hand-keyed",
        "device": device,
        "gen_seconds": gen_s,
        "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    with open(prov_path, "w", encoding="utf-8") as fh:
        json.dump(prov, fh, indent=1)
        fh.write("\n")

    result = {
        "ok": True,
        "status": "generated",
        "bvh": bvh_path,
        "npz": npz_path,
        "provenance": prov_path,
        "bvh_sha256": bvh_sha,
        "npz_sha256": npz_sha,
        "frames": frames,
        "seed": int(args.seed),
        "seconds": float(args.seconds),
        "steps": int(args.steps),
        "samples": int(args.samples),
        "pick": int(args.pick),
        "prompt": args.prompt,
    }
    print(json.dumps(result), flush=True)
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
