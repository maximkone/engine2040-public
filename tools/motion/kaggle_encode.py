# Kaggle: Kimodo text embeddings for the combat prompts -> emb.npz (encoder only).
#
# How to run: make a Kaggle script kernel (or a notebook) with a T4 GPU and the
# HF_TOKEN user secret set to a token that can read the Llama 3 gate. Paste this
# file in and run it. It writes /kaggle/working/emb.npz and emb.provenance.json:
# copy both to <KIMODO_HOME>\emb.npz and <KIMODO_HOME>\emb.provenance.json.
#
# The prompts come from one library file, content/motion/combat_prompts.txt (one
# "id<TAB>prompt" per line). Upload it as a Kaggle dataset named kimodo-combat-prompts
# (or pass --library <file> / set PROMPT_LIBRARY), then run this kernel. The whole file
# is encoded in one run, and the cache is rebuilt from all of it, so keep every line.
# Check the file on this PC first, no GPU:
#   python tools/motion/kaggle_encode.py --check --library content/motion/combat_prompts.txt
# tools/motion/kimodo_gen.py matches a prompt by exact text.
#
# Why this is not a plain encode(): the 8B encoder does not fit on a 16 GB card
# with the Kimodo copy of it, so the weights are split over GPU and CPU
# (device_map + offload), then the same steps run by hand: _convert_to_str,
# prepare_for_tokenization, tokenize, forward. Same maths as Kimodo's encode().
# The pip pins match the Kimodo venv that made the reference vectors.
#
# The Kaggle image cannot build the kimodo wheel (MotionCorrection C++ fails), so
# the vendored llm2vec folder is copied out of the Kimodo source and used alone.

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time

import numpy as np

# Searched in order when no --library and no PROMPT_LIBRARY are given.
LIBRARY_CANDIDATES = [
    "/kaggle/input/kimodo-combat-prompts/combat_prompts.txt",
    "/kaggle/working/combat_prompts.txt",
]
ID_RE = re.compile(r"[a-z0-9_]{1,60}")
PROMPT_MAX = 500  # same cap as tools/motion/clip.mjs and kimodo_gen.py


def read_library(path):
    """(ids, prompts) from an id<TAB>prompt file. Refuses the whole file on any bad line.

    The same rules as tools/motion/library.mjs parseLibrary: comments (#) and blank lines
    skipped, ids lower case letters, digits, underscore; no repeated id or prompt text.
    """
    ids, prompts, errors = [], [], []
    seen_ids, seen_prompts = {}, {}
    with open(path, encoding="utf-8") as fh:
        for n, raw in enumerate(fh, 1):
            line = raw.rstrip("\r\n")
            if not line.strip() or line.strip().startswith("#"):
                continue
            if "\t" not in line:
                errors.append(f"line {n}: want id<TAB>prompt")
                continue
            pid, prompt = (s.strip() for s in line.split("\t", 1))
            if not ID_RE.fullmatch(pid):
                errors.append(f"line {n}: id {pid!r} must be lower case letters, digits or underscore (1..60)")
            elif not prompt:
                errors.append(f"line {n}: id {pid} has an empty prompt")
            elif len(prompt) > PROMPT_MAX:
                errors.append(f"line {n}: id {pid} prompt is {len(prompt)} characters, past the {PROMPT_MAX} cap")
            elif pid in seen_ids:
                errors.append(f"line {n}: id {pid} repeats line {seen_ids[pid]}")
            elif prompt in seen_prompts:
                errors.append(f"line {n}: id {pid} has the same prompt text as line {seen_prompts[prompt]}")
            else:
                seen_ids[pid] = n
                seen_prompts[prompt] = n
                ids.append(pid)
                prompts.append(prompt)
    if errors:
        sys.exit("kaggle_encode: " + path + " is not a valid prompt library:\n  " + "\n  ".join(errors))
    if not prompts:
        sys.exit(f"kaggle_encode: {path} has no prompts")
    return ids, prompts

BASE = "McGill-NLP/LLM2Vec-Meta-Llama-3-8B-Instruct-mntp"
SUP = "McGill-NLP/LLM2Vec-Meta-Llama-3-8B-Instruct-mntp-supervised"
LLAMA = "meta-llama/Meta-Llama-3-8B-Instruct"
OFFLOAD = "/kaggle/temp/offload"
SRC = "/kaggle/working/kimodo-src"
PKG = "/kaggle/working/llm2vec-pkg"
OUT = "/kaggle/working/emb.npz"
PROV = "/kaggle/working/emb.provenance.json"


def main():
    ap = argparse.ArgumentParser(description="Encode the Kimodo prompt library into emb.npz (Kaggle GPU).")
    ap.add_argument("--library", default="", help="id<TAB>prompt file to encode in one run")
    ap.add_argument("--check", action="store_true", help="read and check the library, then stop")
    args, _ = ap.parse_known_args()  # Kaggle may add its own argv
    lib = args.library or os.environ.get("PROMPT_LIBRARY", "")
    if not lib:
        lib = next((p for p in LIBRARY_CANDIDATES if os.path.isfile(p)), "")
    if not lib or not os.path.isfile(lib):
        print("STOP: no prompt library. Upload content/motion/combat_prompts.txt as the Kaggle dataset "
              "kimodo-combat-prompts, or pass --library <file>.", flush=True)
        sys.exit(8)
    ids, lib_prompts = read_library(lib)
    print(f"library {os.path.basename(lib)}: {len(ids)} prompts", flush=True)
    if args.check:
        print("check ok", flush=True)
        return

    t0 = time.time()
    from kaggle_secrets import UserSecretsClient  # Kaggle only

    os.environ["HF_TOKEN"] = UserSecretsClient().get_secret("HF_TOKEN")
    os.makedirs(OFFLOAD, exist_ok=True)

    if not os.path.isdir(SRC):
        subprocess.run(
            ["git", "clone", "--depth", "1", "https://github.com/nv-tlabs/kimodo.git", SRC],
            check=True,
        )

    pkg = os.path.join(PKG, "llm2vec")
    if os.path.isdir(pkg):
        shutil.rmtree(pkg)
    os.makedirs(PKG, exist_ok=True)
    shutil.copytree(os.path.join(SRC, "kimodo", "model", "llm2vec"), pkg)
    sys.path.insert(0, PKG)

    subprocess.run(
        [sys.executable, "-m", "pip", "install", "-q",
         "transformers==5.1.0", "peft==0.21.2", "accelerate==1.15.0"],
        check=True,
    )
    # Kaggle preinstalls torchao 0.10, which peft 0.21 refuses at import. The encoder
    # does not quantize, so drop it rather than pull a new torch stack.
    subprocess.run([sys.executable, "-m", "pip", "uninstall", "-y", "torchao"], check=False)

    import torch
    from llm2vec.llm2vec import LLM2Vec, batch_to_device

    print("GPU:", torch.cuda.get_device_name(0), flush=True)
    m = LLM2Vec.from_pretrained(
        base_model_name_or_path=BASE,
        peft_model_name_or_path=SUP,
        torch_dtype=torch.bfloat16,
        device_map="auto",
        max_memory={0: "13GiB", "cpu": "3GiB"},
        offload_folder=OFFLOAD,
        low_cpu_mem_usage=True,
    )
    m.eval()
    name = m.model.config._name_or_path
    print("config._name_or_path =", name, flush=True)
    if name != LLAMA:
        print("STOP: the adapter did not resolve to Llama-3-8B-Instruct. Do not use this cache.", flush=True)
        sys.exit(8)

    texts = sorted(set(lib_prompts))
    rows = []
    for i, text in enumerate(texts):
        t1 = time.time()
        s = m._convert_to_str("", text)  # same as encode(): empty instruction, then the marker
        feats = m.tokenize([m.prepare_for_tokenization(s)])
        feats = batch_to_device(feats, "cuda:0")
        with torch.no_grad():
            e = m.forward(feats).detach().cpu().to(torch.float32)
        rows.append(e[0].numpy())
        print(f"{i + 1}/{len(texts)} {round(time.time() - t1, 1)} s  {text[:60]}", flush=True)
        torch.cuda.empty_cache()

    emb = np.stack(rows).astype(np.float32)
    assert emb.shape == (len(texts), 4096), emb.shape
    n = emb / np.linalg.norm(emb, axis=1, keepdims=True)
    sim = n @ n.T
    off = sim[~np.eye(len(texts), dtype=bool)]
    print(f"sanity: cosine between different prompts min {off.min():.3f} max {off.max():.3f}", flush=True)
    if len(texts) > 1 and off.max() > 0.9999:
        print("STOP: two different prompts gave the same vector. Do not use this cache.", flush=True)
        sys.exit(8)

    np.savez(OUT, texts=np.array(texts), emb=emb)
    prov = {
        "texts": len(texts),
        "library": {"file": os.path.basename(lib), "ids": len(ids)},
        "encoder": "LLM2Vec on Meta-Llama-3-8B-Instruct",
        "adapters": [BASE, SUP],
        "base": LLAMA,
        "dtype": "bfloat16",
        "gpu": torch.cuda.get_device_name(0),
        "seconds": round(time.time() - t0),
        "licence_note": "Built with Meta Llama 3. Meta Llama 3 Community License applies to the encoder step.",
        "kimodo_wheel": "not installed; vendored llm2vec only (MotionCorrection C++ build fails on this image)",
        "pins": {"transformers": "5.1.0", "peft": "0.21.2", "accelerate": "1.15.0"},
    }
    with open(PROV, "w", encoding="utf-8") as fh:
        json.dump(prov, fh, indent=1)
    print(f"saved {OUT} {emb.shape} {os.path.getsize(OUT)} bytes", flush=True)


if __name__ == "__main__":
    main()
