# FINISH-PROOF — aggregate finish proofs

Source: `docs/FINISH-RICH.md` + `node tools/finish.mjs` on tree b219b26, 2026-10-05.
Cheap proofs run now: `node tools/finish.mjs` PASS (exit 0), `node tools/workspace-check.mjs` FAIL (exit 1: engine-playtest look_judge.rs undeclared), `node tools/sprint-loop.mjs --dry-run` PASS.
Rule: PASS = cheap proof green now; SKIP = no single proof cmd; heavy = cargo build/test, GPU, packaged exe, or soak — not run here. Evidence for heavy rows lives in FINISH-RICH status lines.

| Tier | Proof cmd | Status |
|---|---|---|
| A1 | `node tools/proof.mjs` (fixture-only: `node tools/proof.mjs --skip-build`) | heavy |
| A2 | `cargo run --locked -p engine-editor --features ui --bin e2040-editor -- --record-a2 target/evidence/a2` | heavy |
| A3 | `cargo run --locked -p engine-editor --features ui --bin e2040-editor -- --record-a3 target/evidence/a3` | heavy |
| A4 | GX1-GX10 + CT7-CT8 + Q1 captures on RTX 3050 | heavy |
| A5 | `arena.exe` + MP1-MP3 + GP1/GP3 + PX1 + DX1 + GM3 packaged run | heavy |
| A6 | `cargo test -p engine-playtest --lib eval -- --nocapture` | heavy |
| A7 | `cargo test --locked -p engine-playtest seeded_bug` | heavy |
| A8 | GM3 clean-folder packaged run | heavy |
| A9 | `cargo test -p engine-proof --all-features --lib a9` | heavy |
| A10 | `cargo test -p engine-editor --all-features tuning` | heavy |
| B | see Finish B tables (second game UW39, dedicated server, soak, templates) | SKIP |
| C1 | GX1-GX10 live submit on real device | heavy |
| C2 | RT0-RT8 lighting on reference GPU | heavy |
| C3 | VG1-VG4 geometry and crowd | heavy |
| C4 | ON1-ON8 online converge + relay | heavy |
| C5 | WD1-WD6 + CT9 Rome maps stream + siege replay | heavy |
| P1 | GX + RL + RP + RT + VG + VX + Q1 + PQ4 look + panel review | heavy |
| P2 | LM2 + AM2-AM6 + AM9-AM10 + PX4 + PQ1 feel + latency | heavy |
| P3 | `cargo test --locked -p engine-playtest --all-features tournament -- --nocapture --test-threads=1` | heavy |
| P4 | MP2-MP5 + ON1-ON8 64-client soak | heavy |
| P5 | AH1-AH5 + FZ1 + PQ3 packaged soak + fuzz + chaos | heavy |
| P6 | PF1-PF9 speed budgets | heavy |
| P7 | `cargo test --locked -p engine-editor --lib onboard` + eval + brief_eval + build_game + mind + `cargo run --locked --example ai_builds_arena` | heavy |
| P8 | SL1-SL3 janitor + `cargo clippy -- -D warnings` | heavy |
| P9 | WIN1-WIN4 + UIX1-UIX4 + GM3 packaged ship | heavy |
| P10 | `node tools/finish.mjs` | PASS |
