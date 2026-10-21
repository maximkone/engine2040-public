# Asset pipeline decision — 2026-10-21

One local lane, one cloud lane, one rig. No fallbacks, no paid services.

## Concept lane — 2D, free

The concept image is a reference, never a shipped asset, so it can be
generated anywhere, including anonymously.

- **OVH AI Endpoints, SDXL**: genuinely $0 per request, no monthly quota, no
  account needed at 2 requests a minute per IP; with a key, 400 a minute per
  project (a key needs a project with a payment method). 1024x1024, OpenRAIL++-M
  licence, served from FR/EU only. FLUX is not on this endpoint.
- **Local SDXL-Turbo** is the obvious offline option on a 6 GB card, OpenRAIL++-M
  licence, seconds per image. Verify the licence text before we rely on it.
- **Forge's kaggle-flux lane** burns the free Kaggle quota and is Apache-2.0,
  so it is the lane to use when the concept has to be shippable-adjacent.

## Install log, this PC, 2026-10-21

What actually happened, so nobody repeats it:

- `pip install torch` **without** `--only-binary=:all:` tries to build torch
  from source here and dies. Always wheels only.
- `download.pytorch.org` is unreliable from this machine: a 2.4 GB wheel
  arrived with a wrong SHA-256 and pip backtracked through four versions
  before giving up, and the CPU index timed out. `files.pythonhosted.org`
  (plain PyPI) is the route that works.
- Python 3.11 is not installed. The uv-managed **3.10.20** works and
  `gpytoolbox 0.2.0` has a cp310 wheel. The venv lives at
  `C:\forge-media\models\stable-fast-3d\.venv`.
- The two C++ extensions need torch at build time, so pip's build isolation
  hides it: install them with `--no-build-isolation`, `USE_NATIVE_ARCH=0`
  and `CPPFLAGS=-include utility`. VS Build Tools are installed, LLVM is not.
- Hugging Face gated access works: the token in
  `~/.cache/huggingface/token` reads `stabilityai/stable-fast-3d` fine.
- The Kaggle CLI authenticates and works, and the account has no datasets
  yet. Kaggle's notebook image already has torch, which is why the cloud
  track is a real alternative and not a plan on paper.

## The Kaggle track

`tools/asset-gen/kaggle-sf3d/sf3d.ipynb` with its `kernel-metadata.json`.
The weights travel as a **private Kaggle dataset**, so no Hugging Face token
ever reaches the cloud; the notebook points at the dataset file with
`--pretrained-model` and patches `torch.bfloat16` to `torch.float16`
because the T4 has no bf16. Push with `kaggle kernels push -p <dir>`, then
`kernels status`, `kernels logs`, `kernels output -p <dir>`.

## Base mesh — local: SF3D

`tools/asset-gen/sf3d.mjs` runs Stability AI's SF3D on this PC. It is the only
local generator. Weights are gated (accept the Community Licence, then
`huggingface-cli login`): 4.0 GB, about 6 GB VRAM, and it returns a GLB with
UVs and material maps, which is exactly what the paint pass needs. Python 3.11
in a venv, because gpytoolbox has no 3.12+ wheel. Licence: commercial at or
under $1M revenue, with "Powered by Stability AI" in the credits.

On a 6 GB card, texture resolution 512. The VRAM floor is about 6 GB.

## Base mesh — cloud: Kaggle, for hero assets only

Same code, same weights, run on a Kaggle GPU: `kaggle kernels push`, then
`kernels status`, `kernels logs`, `kernels output -p DIR`. Thirty GPU hours a
week, resets weekly, phone verification required. The agent-facing door is
`datalayer/jupyter-mcp-server` with `SANDBOX_VARIANT=kaggle`, which runs a
kernel through the Kaggle job API so a fleet can upload, run and fetch with
no human click. `googlecolab/google-colab-cli` is the second free pool.

Kaggle's accelerator list changed in September 2026: T4, A100, L4, H100 and
RTX Pro 6000. P100 is gone, so plan on T4 15 GB and check what your account
offers. A100 40 GB would fit TRELLIS.2 (MIT, 24 GB) for hero assets.

## Not used, and why

- **Tripo hosted free** — Tripo keeps all rights, non-commercial, and the
  gallery publishes our design.
- **TripoSR** — MIT and free, but 2024 quality with vertex colours only. We
  would rebuild the UVs anyway. One local lane means one implementation.
- **Hunyuan3D-2.1 and 2-mini** — best free quality, but the licence does not
  apply in the EU, UK or South Korea. Dev only, never in a build.
- **OVH AI Notebooks** — no free tier, strictly pay-as-you-go. OVH AI
  Endpoints serve SDXL and FLUX for concepts only, which is not a 3D lane.
- **Meshy free** — CC BY 4.0 and commercial, but the gallery publishes our
  design. Throwaway props only.
- **ModelScope and Tianchi free notebooks** — real free GPU hours, but the
  capacity is unpredictable and the session caps are 8 hours.

## Rig — ours, on CPU

UniRig is not the plan: Linux or WSL only, its released weights are a
humanoid-only prior, so an armoured creature with a mechanical arm has that
arm folded into the forearm chain, and the released checkpoint is not the
paper weights. Known failures: missing limbs, twisting skin, flipped merges.

Our characters are stylised and armoured, so the rig is a template we author:
22 bones, fixed, humanoid with a left arm that can be a mechanical limb.
Forge's pinocchio lane already matches a fixed 22-bone template on CPU in
under a minute, with no weights and no GPU. Our Blender pass then paints the
512 atlas, adds the LODs and exports for the engine cook.

## The chain, once

    concept image  ->  SF3D base mesh  ->  our retopo + palette paint
                   ->  22-bone rig  ->  4 LODs  ->  engine cook
                   ->  provenance row

The 30 m silhouette test runs on the base mesh, before anything is painted.
A base that fails dies as a grey shape, which costs minutes.

## Surviving a bad network

The weights are 4,024,289,892 bytes and this machine's link drops. The fetch
is built for that, not around it:

- `tools/asset-gen/fetch-weights.mjs` resumes with an HTTP Range, tries
  three sources in order (Hugging Face, hf-mirror, ModelScope), retries with
  a backoff that caps at 60 s, and writes a state file so a re-run picks up
  where the last one stopped.
- A DNS failure is repaired on the spot: the host is resolved over DoH and
  pinned in the Windows hosts file, then the request is retried.
- A lock file keeps one writer on the `.part` file. Two writers would
  corrupt it, and the scheduler, the watcher and a hand-run command can all
  reach the fetcher.
- `tools/asset-gen/fetch-weights-watch.mjs` restarts the fetcher until the
  weights are complete. The Windows task `empire-sf3d-weights` runs it every
  ten minutes, so a dead network, a killed process or a rebooted machine
  recovers without a human.

Observed: 2,462,688,959 of 4,024,289,892 bytes after 101 failed attempts,
with no byte lost to a single failure.

## The shared tree deletes untracked work

A clean in the shared tree removed `crates/game/src/class_sheet.rs` (2050
lines) and both fetch tools while they were untracked. They came back byte
exact from the object database, and they are committed now, so a clean
cannot take them again.

Two habits follow: commit by path as soon as a file is written, and keep the
proof of what a file is in its commit body, because the object database is
the only copy that survives a stray clean.

## What we took from other people

Every lead below was checked for licence before it was used. The rule: no
paid service, no paid tier, no licence that asks anything of the game.

**MakeHuman / MPFB2 — CC0 core assets.** <https://static.makehumancommunity.org/about/license.html>
says the core assets are "shared under Creative Commons, CC0... you are
free to do as you see fit with the asset or derivates of the assets. You do
not need to pay anything, include any copyright notices nor give
attribution", and the MakeHuman FAQ answers the commercial closed source
question with a yes. MPFB2 is a live Blender extension (2.0.18, October
2026) that needs Blender 4.2 or newer, and it makes a rigged, textured
human with clothes. The tool's own code is GPL and MakeHuman's is AGPL, but
the assets are CC0 and the output is ours to shape, armour and paint. This
is the answer to "our characters are lumps": a real body, a real rig, then
our proportions, our armour and our palette on top.

**Quaternius and Kenney — CC0.** <https://quaternius.com/> ships hundreds of
low poly models under CC0, including a Fantasy Props MegaKit of 200-odd
medieval props, game ready and rigged. CC0 means ship them, no credit. Use
them for the world's neutral clutter, the crates, the market stalls, the
furniture: the world is not the faction, and the faction's identity stays
ours. Kenney's kits are CC0 too.

**The painted-light technique.** The 2005 look is baked light, not engine
light. Blender Conference 2024's "Vertex colors: the hidden workhorse of
shaders" and the Polycount Retro 3D Art FAQ both land on the same recipe:
paint the light into the vertex colours, a strong key from the upper front,
one bounce of fill, a rim on the edges, the crevices darkened, and let the
engine's own light be soft. That is now the cook's `LIGHT` layer.

**The inverted-hull outline.** The era's games drew a black rim by drawing
the mesh twice: once normally, once with the normals pushed out and the
front faces culled. It is the cheapest silhouette read ever invented and it
is a shader trick, so it belongs to the render lane: draw the mesh again
with the vertices pushed along their normals by a fixed world distance, back
faces only, in black.

**Armour by shrinkwrap.** The standard way to put armour on a body: model
the plates, then shrinkwrap them onto the base mesh with an offset, then
bone parent them. Our props are already the plates, so the armour pass is
a shrinkwrap and a parent, not a modelling job.

**The input template.** "Lean, arms out from the sides, plain background,
whole body in frame, one figure, no panels" is the wording that makes the
generator give a character with separated limbs. "Hulking" gives a boulder.

## The chain, as one command

    node tools/asset-gen/character.mjs --concept <concept.png> \
      --name ashkin_gunner --faction ashkin --out content/generated/characters/ashkin_gunner

That runs the four tools in order and writes `character.json`: the base mesh
from the one local 3D lane, the style cook, the 30 m gate, the 22-bone rig.
The gate has teeth: a silhouette that does not read stops the chain with the
numbers and the picture, before anything is painted.

The gate band is calibrated on the six blockouts. The two heavies that read
as rectangles scored `bbox_fill` 0.7443 and 0.7519; the four that read
scored 0.5283 to 0.6604. So the band is 0.40 to 0.70, and a base outside it
dies as a grey shape in a minute instead of costing a day of paint.

## What each tool owns

- `tools/asset-gen/sf3d.mjs` — the one local 3D lane. SF3D, local weights,
  device detected, no hub call at run time.
- `tools/blender/style_cook.py` — our palette pushed into one 512 atlas, the
  triangle budget, four LODs, painted AO as vertex colours, everything at the
  2 m canonical height. The hue chooses the palette entry and the luminance
  scales it, so the generator's baked light survives and its colours do not.
- `tools/blender/mesh_silhouette.py` — the 30 m read test, the same picture
  and the same measurements as `blockout.py`, so a generated base and a hand
  blockout compare number for number.
- `tools/blender/rig22.py` — our own 22-bone rig, the engine's
  `HumanoidBone` names and the engine's chain. The proportions are ours
  (hips at half the height, the shoulder line at 0.82); the mesh supplies
  the scale and the reach. No UniRig, no weights, no GPU, no network.

## The image estimator is optional

SF3D's image estimator refines the decoder's roughness and metallic with a
CLIP pass. It wants `open_clip_pytorch_model.bin`, 577 MB, and this machine
cannot be trusted with that download: the hub fetch froze at 190 MB and the
eight-way range fetch still only managed 210 kB/s.

The decoder already estimates roughness and metallic on its own, so
`sf3d/system.py` now treats the image estimator as optional and loads a
checkpoint that carries its weights without it. A run with
`image_estimator_cls: ""` makes the same mesh, one CLIP refinement poorer,
and our cook repaints the material anyway.

