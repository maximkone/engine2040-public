# engine2040

Rust-first game engine aiming for Flax-class depth in third-person action
games — strong 3D visuals, a practical editor, physics, destruction,
multiplayer — with provider-agnostic AI in gameplay and authoring. The proof
game is a multiplayer Roman arena on Windows (`docs/FOCUS.md`).

> **Status: experimental, work in progress.** This engine is written almost
> entirely by AI coding agents running 24/7 on free models, with one human
> (Maxim) setting the direction and the bars. It is not a finished product and
> the API changes daily.
> Issues and ideas are welcome; pull requests may be closed while the agents'
> queue owns the code. Not affiliated with Flax Engine: "Flax" is named only as
> a comparison target. Licensed under Apache-2.0
> ([LICENSE-APACHE](LICENSE-APACHE)).

**Current state:** 34 workspace packages, about 550k lines and 6,625 tests, all
headless: deterministic arena match, LAN rollback netcode, AI-safe command
service with an MCP door, provider adapters, spatial services, asset
import/cook, animation and ragdoll. Not yet: a real window, the editor app,
the game exe, GPU lighting and post passes, content. Start with
[the review](docs/PLAN-REVIEW.md) and [the board](docs/SPRINT.md).

## Run

```powershell
node tools/proof.mjs
cargo run --locked --release -- --help
cargo run --locked --release --example window --features window -- 10000 5
```

`node tools/proof.mjs` runs every local gate. The default application runs 600
fixed ticks with seed 2040 and must report `final_hash=d57b5c8ae3096614`. The
window example needs a display/GPU and draws sprites. See
[CONTRIBUTING.md](CONTRIBUTING.md) for complete gates and limitations.

## Sprint with agents (OpenCode)

```powershell
node tools/sprint-loop.mjs --dry-run      # what is ready now
node tools/sprint-loop.mjs               # autonomous waves, 20 wide (free models only)
New-Item target/sprint.halt               # stop after the current wave
```

Each wave is a fresh orchestrator session that dispatches planners and coders
in parallel, gates once, commits every green task and pushes the branch you
are on (`--safe` never pushes). Progress: `target/sprint-loop.log` and
`target/sprint-handoff.md`. Inside OpenCode, `/sprint` does the same in one
session.

## Design and plans

- [Vision](docs/VISION.md) · [architecture](docs/ARCHITECTURE.md) · [code law](docs/CODELAW.md)
- [Milestone order](docs/ROADMAP.md) · [finish line](docs/FINISH-RICH.md) · [visual targets](docs/VISUAL-TARGET.md)
- [AI/editor contracts](docs/EDITOR_AI_CONTROL.md) · [hash scope](docs/HASH-LAW.md)
- [Subsystem plans](docs/plan/README.md) · [content sources](docs/plan/content.md)
- [Documentation index](docs/INDEX.md) · [agent skills](skills/SKILLS.md)

Crates follow explicit acyclic dependencies. Provider SDKs, GPU resources and
blocking IO stay outside `engine-core` and the authoritative tick. Live AI
proposes validated actions; replay consumes the recorded actions without models.

Project license: Apache-2.0. Dependency licenses follow `deny.toml` and provenance
policy; visual references and researched algorithms do not imply copied code.
