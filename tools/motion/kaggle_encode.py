# Kaggle: Kimodo text embeddings for the combat prompts -> emb.npz (encoder only).
#
# How to run: make a Kaggle script kernel (or a notebook) with a T4 GPU and the
# HF_TOKEN user secret set to a token that can read the Llama 3 gate. Paste this
# file in and run it. It writes /kaggle/working/emb.npz and emb.provenance.json:
# copy both to <KIMODO_HOME>\emb.npz and <KIMODO_HOME>\emb.provenance.json.
#
# To add a prompt: append it to PROMPTS below (keep the old lines, the cache is
# rebuilt from this whole list), then rerun. tools/motion/kimodo_gen.py matches a
# prompt by exact text.
#
# Why this is not a plain encode(): the 8B encoder does not fit on a 16 GB card
# with the Kimodo copy of it, so the weights are split over GPU and CPU
# (device_map + offload), then the same steps run by hand: _convert_to_str,
# prepare_for_tokenization, tokenize, forward. Same maths as Kimodo's encode().
# The pip pins match the Kimodo venv that made the reference vectors.
#
# The Kaggle image cannot build the kimodo wheel (MotionCorrection C++ fails), so
# the vendored llm2vec folder is copied out of the Kimodo source and used alone.

import json
import os
import shutil
import subprocess
import sys
import time

import numpy as np

PROMPTS = [
    "A person swings a sword in a fast horizontal slash.",
    "A person raises a sword and brings it down in a strong overhead strike.",
    "A person raises a shield to block an incoming attack and braces behind it.",
    "A person lunges forward and thrusts a spear.",
    "A person parries an attack with a sword and then strikes back.",
    "A person is hit in the chest and staggers backward.",
    "A person is struck and collapses to the ground.",
    "A person delivers a strong front kick.",
    "A person draws a bow, aims, and releases the arrow.",
    "A person draws a sword from the hip and settles into a fighting stance.",
    "A person raises both arms in victory and cheers.",
    "A person stands in a ready fighting stance, shifting their weight slightly.",
    "A person dodges sideways and steps back.",
    "A person walks forward.",
    "A person runs forward at a fast pace.",
    "A person turns left on the spot.",
]

BASE = "McGill-NLP/LLM2Vec-Meta-Llama-3-8B-Instruct-mntp"
SUP = "McGill-NLP/LLM2Vec-Meta-Llama-3-8B-Instruct-mntp-supervised"
LLAMA = "meta-llama/Meta-Llama-3-8B-Instruct"
OFFLOAD = "/kaggle/temp/offload"
SRC = "/kaggle/working/kimodo-src"
PKG = "/kaggle/working/llm2vec-pkg"
OUT = "/kaggle/working/emb.npz"
PROV = "/kaggle/working/emb.provenance.json"


def main():
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

    texts = sorted(set(PROMPTS))
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
