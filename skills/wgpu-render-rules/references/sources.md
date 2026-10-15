# Sources

This skill is a derived work of the learn-wgpu tutorial. The rules are condensed and reworded; a few short code lines are kept from the pages.

- Origin: https://github.com/sotrh/learn-wgpu at commit d90b30088fceae214c53603207464c874a08b6d8 (branch master). It teaches wgpu 30.0 with winit 0.30 (`wgpu = "30.0"` in docs/beginner/tutorial1-window/README.md). The manifest agrees (verified 2026-10-11): the workspace Cargo.toml at the pin, https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/Cargo.toml , has `wgpu = "30.0"`, `winit = "0.30"`, `pollster = "0.3"` and `env_logger = "0.10"`.
- Licence: MIT, https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/LICENSE ("Copyright (c) 2020 Benjamin Hansen"), verified 2026-10-11 from the repository LICENSE file. The repository holds no separate licence for its docs folder; the 50 text files fetched carry no other notice except a third-party MIT note inside the terrain page, which this skill does not use.
- Attribution: rules and short code derived from the learn-wgpu tutorial by Benjamin Hansen, MIT licence. The MIT permission notice is kept in `THIRD_PARTY_NOTICES.md`. Changes: condensed into rules, reworded, short code lines kept. No endorsement by the author or by the wgpu project is implied.
- Supply copy: the pages below are kept in git-ignored `work/wgpu-render-rules/src/docs/` with a `MANIFEST.json` that holds the pinned commit and a size per file. `tests/test_wgpu_render_rules.py` checks every API name a rule cites against the page the rule names (live test).

## Pages used (all at the pinned commit)

- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/beginner/tutorial1-window/README.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/beginner/tutorial2-surface/README.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/beginner/tutorial3-pipeline/README.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/beginner/tutorial4-buffer/README.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/beginner/tutorial5-textures/README.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/beginner/tutorial6-uniforms/README.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/beginner/tutorial7-instancing/README.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/beginner/tutorial8-depth/README.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/compute/introduction/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/showcase/alignment/README.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/showcase/gifs/README.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/showcase/windowless/README.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/0.12/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/0.13/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/0.14/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/0.15/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/0.16/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/22.0/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/24.0/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/25.0/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/27.0/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/28.0/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/29.0/readme.md
- https://github.com/sotrh/learn-wgpu/blob/d90b30088fceae214c53603207464c874a08b6d8/docs/news/30.0/readme.md

## Measured

The skill was built because a bare-model baseline probe on the 12-task sheet `evals/wgpu-render-rules_trials.jsonl` failed the topic (see `research/BASELINE-PROBES.md`). Its lift with and without the skill, from live model runs, is in `references/trial-proof.json`.
