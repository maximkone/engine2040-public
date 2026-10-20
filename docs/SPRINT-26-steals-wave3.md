### Steals wave 3, verified (owner 2026-10-20)

Boarded 2026-10-20 after the question "can we do a big wave of steals now".
Method: every candidate was grepped in the tree first, and the six rows below are
the ones with no owner today. The audit that removed the other candidates, with
file evidence, is `docs/research/steals-2026-10-20/wave-3.md`. Donors were read
live on 2026-10-20 unless the row says otherwise.

The wave is six small rows, not twenty. Previous waves proved the failure mode:
a module lands, nothing calls it, and the finish line does not move. Each row
below names its consumer and a number.

- [ ] GS43 render: contact shadows under a foot — the shadow map loses the last 10 cm where a body touches the sand. March the depth buffer 16 steps over 0.3 m, assume 0.1 m surface thickness, and darken ambient only. Donor bevyengine/bevy@main path crates/bevy_pbr/src/contact_shadows.rs, read live 2026-10-20 (linear_steps 16, thickness 0.1, length 0.3, depth prepass required; zero bytes vendored). Diff: good. Tier A. Size S. Owns `crates/render/src/contact_shadow.rs`, `crates/render/src/lib.rs`.
  Accept: `cargo test -p engine-render --lib contact_shadow` prints 5 passed; the live arena capture darkens the 0.3 m box under both feet by 8% mean luma or more and changes 0 pixels when the pass is off; the capture is opened; golden d57b5c8ae3096614 x2.
  F2P: no contact-shadow module and 0 hits for contact_shadow in the render crate.
  P2P: golden x2, `node tools/finish.mjs A4` unchanged.
  needs: none.
- [ ] GS44 render: automatic exposure from a 64-bin histogram — a night arena lit by 250 torches clips without it. Build log-luma bins, aim the frame at 0.18 mean over 30 frames, clamp to plus or minus 4 stops, and stop per-frame pumping. Donor bevyengine/bevy@main directory crates/bevy_post_process/src/auto_exposure/, read live 2026-10-20 (auto_exposure.wesl, compensation_curve.rs, settings.rs; method only). Diff: good. Tier A. Size S. Owns `crates/render/src/auto_exposure.rs`, `crates/render/src/lib.rs`.
  Accept: `cargo test -p engine-render --lib auto_exposure` prints 5 passed; a synthetic ramp at -4 EV and +4 EV reaches 0.18 plus or minus 0.02 within 30 steps and stays inside plus or minus 0.01 for the next 90; two runs byte-equal; the night capture does not clip torch cores; golden x2.
  F2P: 0 hits for auto_exposure in the render crate.
  P2P: golden x2, present tests green.
  needs: none.
- [ ] GS45 render: a pass report that names every pass — today a pass can be skipped and the frame looks merely wrong. Each pass declares reads and writes; the report lists ran, culled or failed with a reason; a read with no writer fails the plan. Donor uinosoft/zenfg render graph plus Themaister/Granite renderer/render_graph.cpp validate_passes, read 2026-10-11 in the engine-landscape dive row 1. Diff: hard. Tier A. Size M. Owns `crates/render/src/report.rs`, `crates/render/src/graph.rs`.
  Accept: `cargo test -p engine-render --lib pass_report` prints 6 passed; the arena plan names sky, IBL, sun shadow, GTAO, bloom and TAA as ran and one disabled pass as culled; the JSON is byte-equal x2; a pass reading a texture with no writer fails with the pass and the texture named.
  F2P: no pass report, and a culled pass prints nothing today.
  P2P: golden x2, `node tools/finish.mjs A4 A5` unchanged.
  needs: none.
- [ ] GS46 door: world.query with the BRP filter shape — agents trained on Bevy already know with, without, option, has and strict. Add the read-only query verb over our scene snapshot with the same names, so a Bevy-shaped request runs here unchanged. Donor bevyengine/bevy@main path crates/bevy_remote/src/lib.rs, read live 2026-10-20. Diff: regular. Tier A. Size S. Owns `crates/endpoint/src/scene_query.rs`.
  Accept: `cargo test -p engine-endpoint --lib world_query` prints 5 passed; a 3-object fixture filters by with and without; strict true on a missing component returns missing_component naming it; the same request twice is byte-equal; 0 write verbs added.
  F2P: 0 hits for world.query in the endpoint crate.
  P2P: door transcript tests green, golden x2.
  needs: none.
- [x] GS47 proof: one digest on Windows, Linux and wasm — NOOP, closed 2026-10-20: the work already exists. `.github/workflows/golden-matrix.yml` records the golden tape, the corpus row, the ragdoll row and the arena row on five targets (linux-x86_64, windows-x86_64, linux-aarch64, wasm32-wasip1, wasm32-wasip1-simd128) and the compare job runs `tools/golden-matrix.mjs`, which names the first divergent tick, plus the physics lib tests native against wasm. A missing artifact fails the job by name. Landed by HS2/WEB1/WEB3/DSN2; the row asked for nothing they do not do. Landed 21aa0838 (audit) + this close.
- [ ] GS48 ai: a decision inspector with show and diff — when two fights diverge nobody can say which choice differed. Record every AI decision in an off-by-default ring (tick, fighter, scores per action, chosen) and serve ai.decisions show and diff; diff prints the first tick two tapes disagree and the score gap. Donor lazyxeon/AstraWeave@main decision ring and aw_decision issue 287, README read live 2026-10-20. Diff: good. Tier A. Size S. Owns `crates/ai/src/decision_log.rs`, `crates/endpoint/src/packs/play.rs`.
  Accept: `cargo test -p engine-ai --lib decision_log` prints 5 passed; two tapes with one changed input print the first divergent tick and both score vectors; the ring is off by default with 0 allocations; x2 byte-equal.
  F2P: 0 hits for decision_log in the ai crate.
  P2P: golden x2, door transcript tests green.
  needs: none.
