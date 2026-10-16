# STEAL Dive 12 — Trust and security for AI edits and MCP, serving EMPIRE outcome 4 (2026-09-26)

**Dive**: 12 (Trust and security for AI edits and MCP). Home: `crates/endpoint` (sandbox, tools), `crates/provider`, `crates/net` (server authority), `docs/EDITOR_AI_CONTROL.md`, `docs/plan/ai-native.md` (AX, MCP, EA, EV).
**EMPIRE outcome 4** (AI makes whole games): every card ends in a brief→game consumer (AB1–AB4, EA1, VK1, EV2, RV1). Security that cannot serve the brief→game loop is rejected.
**Baseline**: typed `#[e2_tool]` tools (MCP9), EXPLAIN refusal caps (MCP10), journaled undo via WAL (ED8), recorded provider fixtures (PV2/PV4), server authority (ON4), RV1 red-team tape, plus wave-36 takes RV2 (`nab` pattern filter), MCP11 (capability-scoped permissions), EA2 (`wirken` audit pattern), CT16 (`munal-os` sandbox boundary), MCP12 (wasmtime tool sandbox).
**Fixture**: poisoned brief + malicious asset name + out-of-scope tool call each refused and logged; modified client cannot move authoritative state (per DEEP-DIVES.md §12).
**Kill (dive)**: any write outside the command service; any secret in a log.
**Method**: GitHub repo search (seeds: `mcp server audit log rust`, `prompt injection detector rust`, `wasm sandbox wasmtime capability rust`, `server authoritative anti-cheat rollback rust`) + `github_get_file_contents` listings + `webfetch` LICENSE reads live 2026-09-26. No donor repeat: wave-36 (`nab`, `AegisForge-MCP`, `wirken`, `munal-os`, `boxagnts`), wave-28 sandbox/auth donors, Dive-11 donors (`dolly`, `smooth-bevy-cameras`, `bevy_trauma_shake`, `saltmire-juice`, `Quad-Fighter-2`, `game-feel-hitstop-juice`).
**deny.toml allow-list**: MIT, Apache-2.0, Zlib, BSD-3-Clause, BSD-2-Clause, ISC, Unicode-DFS-2016, Unicode-3.0, CC0-1.0, Unlicense, 0BSD, MIT-0.
**Access date**: 2026-09-26 (all URLs + LICENSE reads).

## Ranked cards (best-first, 5) + 1 moonshot BET

### Card 1 — TAKE — `yfedoseev/jailguard` — embedded pure-Rust injection classifier as offline import gate (RV3)

- **Question**: Does an embedded 1.5 MB MLP prompt-injection classifier (claimed 98.40% accuracy, p50 14 ms CPU) transfer as an *offline, import-time-only* screen over briefs, asset names and scene text, stacked behind RV1 keywords + RV2 pattern filter?
- **Engine home**: `crates/endpoint/src/injection/` (behind RV2). **Simpler baseline**: RV1 keyword refusal + RV2 `nab`-pattern filter.
- **URL + rev + access**: https://github.com/yfedoseev/jailguard @ `53d2d47` (main, 11★, updated 2026-09-25), accessed 2026-09-26.
- **Licenses**: code MIT + Apache-2.0 (dual `LICENSE-MIT` read live + `LICENSE-APACHE` listed at root — both on allow-list). Weights/data: embedded 1.5 MB MLP under `models/` — license not read → weights **UNVERIFIED**. `PORT(yfedoseev/jailguard@53d2d47:src/)` (pattern + gate discipline only) + EVIDENCE row below.
- **Exact claim**: pure-Rust detector with embedded MLP classifier; 98.40% accuracy, p50 14 ms CPU inference; bindings for Python/JS/Go; positioned as Apache-2.0/MIT alternative to Rebuff (archived) and Lakera Guard. Repo ships `benches/`, `tests/`, `data/`, `models/`, `BENCHMARKS.md`.
- **Assumptions**: brief/asset-name/scene-text injection vectors overlap the donor's text-attack distribution enough that a score gate adds catches beyond keywords + patterns; 14 ms is payable at import (never per-tick, never per-tool-call).
- **Counterevidence**: 11★, donor numbers are self-reported (BENCHMARKS.md not independently reproduced here); donor threat model is web/LLM text, ours is briefs/asset names/tool results — partial overlap; MLP is float math, so any tick or per-call use violates HASH-LAW; embedded weights cannot enter git without a license + SHA-pinned manifest (per RESEARCH-VERDICT DT5 rule).
- **What does NOT transfer**: Python/JS/Go bindings, any Rebuff-compat server wiring, donor thresholds/tuning, the weights file itself until its license is read. Only the *gate discipline* transfers (score → refuse-with-receipt at import, deterministic fallback to RV1+RV2 when the gate is absent).
- **Cost / fixture / metric / budget**: S (1 session). Fixture: RV1 red-team tape + 20 poisoned briefs + malicious asset names; metric: extra catch rate over RV1+RV2, false-positive rate on 20 legit briefs, ms per import. Budget: import-time only, 0 per-tick allocs, 0 float in tick.
- **Ablation/comparator**: RV1+RV2 (baseline) vs RV1+RV2+score-gate (candidate) vs score-gate-alone (must be worse — proves stacking matters). Success threshold: ≥ 20% extra catches over baseline with 0 FP on legit briefs.
- **Kill criterion**: any FP blocking a legitimate brief; any float inference in the tick or per-tool-call path; weights committed without a license + manifest; per-call p95 +10%.
- **Board row**: `[ ] RV3 endpoint: stacked injection score-gate at import. Tier A. Size S. Owns gate hunks beside crates/endpoint/src/injection/ (RV1/RV2 frozen). Accept: +20% catches over RV1+RV2 on the tape; 0 FP on 20 legit briefs; import-time only; sim bytes x2. needs: RV1, RV2.`
- **Falsifiable experiment + consumer**: reimplement score-gate wiring with a mock scorer first (deterministic thresholds), run tape x2; consumer is the AB1 brief-intake gate of the brief→game pipeline. Killed if mock-gate shows no headroom over RV1+RV2 (then the donor's 98.4% adds nothing here).
- **EMPIRE outcome 4**: poisoned briefs are the #1 way a brief→game run ships an adversarial game — the AB1 intake must refuse them with a receipt.

### Card 2 — TAKE — `quipu-log/Quipu-Log` — Merkle-chained tamper-evident audit log for brief→game provenance (EA3)

- **Question**: Does an append-only Merkle history-tree audit log with per-field encryption and time-travel queries transfer as the tamper-evident receipt chain behind ED8 journals and LW6/AB provenance records?
- **Engine home**: `crates/endpoint/src/audit/` + ED8 WAL receipts. **Simpler baseline**: ED8 WAL (atomic writes, CRC32 recovery) with plain log lines.
- **URL + rev + access**: https://github.com/quipu-log/Quipu-Log @ `c2a974f` (main, 1★, updated 2026-06-14), accessed 2026-09-26.
- **Licenses**: code Apache-2.0 (`LICENSE` read live — on allow-list). Data/weights: none (log contents are operator-owned). `PORT(quipu-log/Quipu-Log@c2a974f:crates/)` (receipt-chain discipline only) + EVIDENCE row below.
- **Exact claim**: tamper-evident audit logging for Rust — append-only Merkle history tree, per-field encryption, time-travel queries; usable embedded, as a server, or via MCP.
- **Assumptions**: a hash-chained receipt log can ride the existing ED8 journal (each entry commits to the previous digest) with bounded overhead; time-travel queries serve the AB4 demo audit ("show me every edit that built this game").
- **Counterevidence**: 1★, ~3 months old, no community proof; per-field encryption invites hand-rolled-crypto risk (banned — use existing vetted primitives only); Merkle hashing per edit costs cycles; the donor's server/MCP transport is not wanted (engine already has the MCP door).
- **What does NOT transfer**: the donor's server/MCP transport, its crypto choices, its storage engine. Only the *receipt-chain discipline* transfers (append-only + digest-chained + bounded retention).
- **Cost / fixture / metric / budget**: S. Fixture: 100-session tamper test (modified client rewrites history); metric: tamper detection 100%, overhead per edit. Budget: ≤ 0.5 ms per edit, log rotation capped (size + age, cf. prism's 5 MiB/30 d pattern).
- **Ablation**: plain ED8 log (baseline) vs digest-chained log (candidate) vs chained-without-rotation (must grow without bound — proves the cap matters). Threshold: 100% tamper detection, cap holds.
- **Kill criterion**: log grows without bound; any custom crypto in the engine; any network requirement for verification; overhead > 0.5 ms.
- **Board row**: `[ ] EA3 endpoint: digest-chained audit receipts over ED8. Tier B. Size S. Owns audit hunks beside crates/editor/src/durable.rs (journal frozen). Accept: 100-session tamper test 100% detected; rotation cap holds; overhead ≤ 0.5 ms. needs: ED8, RV1.`
- **Falsifiable experiment + consumer**: chain digests over 100 journaled edits, tamper one byte, verify detection x2; consumer is the AB4 demo audit view. Killed if detection < 100% or the cap fails.
- **EMPIRE outcome 4**: "AI makes whole games" needs proof of *how* the game was made — chained receipts make the brief→game build auditable.

### Card 3 — TAKE — `Codexia-afk/WasmVault` — capability manifests + static import scan + wasmtime sandbox for tool execution (MCP13)

- **Question**: Does the donor's three-layer boundary — `plugin.toml` capability manifest → `wasmparser` static import scan with 0–100 risk score → wasmtime-24 WASI sandbox with live syscall interception — transfer as the execution boundary for MCP9 tool calls and the CT16 content sandbox?
- **Engine home**: `crates/endpoint/src/sandbox/` (MCP9 tool path, CT16). **Simpler baseline**: MCP11 capability-scoped refusals with no execution isolation.
- **URL + rev + access**: https://github.com/Codexia-afk/WasmVault @ `main` (v0.2.0, 6★, updated 2026-09-23), accessed 2026-09-26.
- **Licenses**: code MIT (`LICENSE` read live — on allow-list). Data/weights: none. `PORT(Codexia-afk/WasmVault@main:src/)` (manifest + scan + sandbox discipline only) + EVIDENCE row below.
- **Exact claim**: security-first WASM package manager + runtime sandbox; every plugin ships a capability manifest, undergoes static import scanning (catches stealth `sock_open` imports), runs under real-time host interception; < 5 ms startup, < 16 MB per plugin; 3-second selftest proves scoped-filesystem isolation, network-interceptor block, and 16 MB allocation trap; CLI: `selftest / inspect / permissions / verify / run`.
- **Assumptions**: tool/plugin capabilities can be declared as manifests the EXPLAIN gate (MCP10) already resembles; static import-vs-manifest diff catches over-scoped tools before they run; wasmtime is an acceptable new dependency (needs DEPENDENCIES row + `cargo deny`).
- **Counterevidence**: 6★, Apple-Silicon-first claims, self-reported numbers; dependency weight is real (`wasmtime 24`, `wasmparser`, `tokio full`, `ratatui`, `ed25519-dalek` per its Cargo.toml); overlaps MCP12 (wasmtime sandbox) — the new falsifiable gap is *manifest + static scan before run*, not the sandbox itself.
- **What does NOT transfer**: the package-manager CLI, TUI, Ed25519 trust PKI, tuning of risk bands. Only *declare → scan → refuse-or-run* transfers.
- **Cost / fixture / metric / budget**: M (2 sessions). Fixture: 12 hostile tool payloads (network, fs-escape, fork-bomb per donor suite) + Dive-12 fixture; metric: containment 12/12, startup ms, per-call overhead. Budget: ≤ 0.5 ms per tool call, sandbox cold start ≤ 5 ms.
- **Ablation**: MCP11 refusal-only (baseline) vs manifest+scan+refusal (candidate, no sandbox) vs full sandbox (candidate+). Threshold: scan layer alone refuses ≥ 80% of over-scoped calls before any sandbox is needed.
- **Kill criterion**: any sandbox escape; any manifest-declared capability bypassed by undeclared imports at runtime; new dependency without DEPENDENCIES row + deny check; overhead over budget.
- **Board row**: `[ ] MCP13 endpoint: capability manifests + static import scan before tool run. Tier B. Size M. Owns manifest/scan hunks beside crates/endpoint/src/sandbox/ (MCP9/MCP11 frozen). Accept: 12/12 hostile payloads contained; scan refuses ≥ 80% pre-run; overhead ≤ 0.5 ms. needs: MCP9, MCP11.`
- **Falsifiable experiment + consumer**: implement manifest-vs-import diff on the 20 adversarial briefs from MCP11's fixture x2; consumer is the AB2-tested tool runner in the brief→game loop. Killed if the scan adds no refusals beyond MCP11 (then fold into MCP11, no new row).
- **EMPIRE outcome 4**: generated games invoke tools hundreds of times per run — declared capabilities with pre-run refusal is what keeps the brief→game loop from exfiltrating or wrecking the machine.

### Card 4 — TAKE — `numikel/kprun` — vault-backed per-process secret injection for MCP servers (MCP14)

- **Question**: Does per-process secret injection from an encrypted KeePass vault (nothing in `mcp.json`, nothing on stdout, audit log with key names only) transfer as the engine's MCP-server secret discipline, directly satisfying the Dive-12 kill rule "any secret in a log"?
- **Engine home**: `e2040-mcp` launcher + MCP door server-spawn path. **Simpler baseline**: tokens pasted in client config files / shell exports.
- **URL + rev + access**: https://github.com/numikel/kprun @ `main` (v0.7.2, 9★, updated 2026-09-25), accessed 2026-09-26.
- **Licenses**: code MIT (`LICENSE` read live, `license = "MIT"` in Cargo.toml — on allow-list). Data/weights: none (vault contents are operator-owned). `PORT(numikel/kprun@main:crates/kprun-core/)` (spawn + audit discipline only) + EVIDENCE row below.
- **Exact claim**: secrets live in a KeePass vault, unlock via keyfile → OS keyring → prompt, inject as env into *one child process only*; `run` prints nothing on stdout (child owns stdio); audit log is JSON lines with entry/key names, never values, never the vault path; cross-platform; CI matrix fmt/clippy/tests on ubuntu/windows/macos.
- **Assumptions**: engine-launched MCP servers can be spawned through the same one-child-only injection; operator unlock (keyfile/keyring) is acceptable for dev + CI; the pattern can be a runbook + launcher change, not a crate dependency.
- **Counterevidence**: 9★; new deps if vendored (`keepass`, `keyring`, `rust-argon2` — Argon2id noted slow in dev); vault-unlock UX friction for 24/7 loops (needs headless Story: keyfile); partial overlap with prism's keychain rule (Card 6) — this card is the headless/CI answer, prism is the tray answer.
- **What does NOT transfer**: the KeePass tooling itself, install scripts, release assets. Only *inject-into-one-child + never-log-values* transfers.
- **Cost / fixture / metric / budget**: S. Fixture: launch an MCP server with a canary token through the launcher; grep stdout/logs/config for the canary; metric: 0 secret bytes anywhere + unlock latency. Budget: unlock ≤ 2 s interactive, headless via keyfile.
- **Ablation**: config-file token (baseline, must leak into the file by construction) vs injected (candidate) vs injected-with-full-env-inherit (must leak — proves the allowlist matters). Threshold: 0 canary bytes outside the child across 20 launches.
- **Kill criterion**: any secret byte in stdout, logs, `mcp.json`, or audit; any shell-wide export required; headless unlock impossible.
- **Board row**: `[ ] MCP14 mcp: vault-backed one-child secret injection for launched servers. Tier A. Size S. Owns launcher hunks beside e2040-mcp (door frozen). Accept: 20 launches, 0 canary bytes outside the child; audit holds key-names-only; headless keyfile path works. needs: MCP5, MCP7.`
- **Falsifiable experiment + consumer**: canary-token launch test x2 with log grep; consumer is the operator's own brief→game rig (which today pastes tokens into configs). Killed if any canary escapes.
- **EMPIRE outcome 4**: the brief→game loop spawns provider-backed MCP servers constantly — leaked keys in logs/configs would poison the money machine before it starts.

### Card 5 — TAKE — `maverickkhan/rust-agent-sandbox` — versioned agent-tool registry with capability + audit envelopes (MCP15)

- **Question**: Does a *versioned* WASM tool runtime (tool versions pinned per run, capability controls + audit logging) transfer as the version-pinned tool registry behind the brief→game replay story — so a replay pins the exact tool versions the original run used?
- **Engine home**: `crates/endpoint` tool registry (MCP9) + replay tapes. **Simpler baseline**: unversioned tool allow-list (current MCP9).
- **URL + rev + access**: https://github.com/maverickkhan/rust-agent-sandbox @ `169c0cd` (main, 0★, updated 2026-06-18), accessed 2026-09-26.
- **Licenses**: code MIT (`LICENSE` read live — on allow-list). Data/weights: none. `PORT(maverickkhan/rust-agent-sandbox@169c0cd:crates/)` (versioned-registry + envelope discipline only) + EVIDENCE row below.
- **Exact claim**: WebAssembly-based runtime for securely executing *versioned* AI-agent tools with capability controls and audit logging; ships ARCHITECTURE.md, crates layout, `deny.toml`, MCP-tagged topics.
- **Assumptions**: tool identity := (name, version, capability set); the replay tape records tool versions; replays refuse-or-pin when versions drift; capability envelopes compose with MCP11.
- **Counterevidence**: 0★, only root listing read (implementation depth UNVERIFIED beyond layout/docs); overlaps WasmVault (Card 3) — the distinct falsifiable gap is *versioning for replay*, not sandboxing; stale (June) vs the fresher WasmVault.
- **What does NOT transfer**: the runtime code itself (unread), tokio/wasi wiring, any versioning scheme details. Only *pin-versions-in-the-tape* transfers.
- **Cost / fixture / metric / budget**: S. Fixture: two tool versions (v1 permissive, v2 restricted), one recorded run on v1; metric: replay on v1 identical, replay-against-v2 refuses-or-repins with receipt. Budget: registry lookup ≤ 0.1 ms.
- **Ablation**: unversioned replay (baseline, silently uses newest — must diverge by construction) vs pinned replay (candidate). Threshold: pinned replay byte-identical x2; drift detected 100%.
- **Kill criterion**: version pin bypassable; pinned replay diverges; no new gap beyond Card 3 (then fold into MCP13, no new row).
- **Board row**: `[ ] MCP15 endpoint: version-pinned tool registry for replays. Tier B. Size S. Owns registry hunks beside the MCP9 tool table (tool schemas frozen). Accept: v1 replay x2 identical; v2 drift refused with receipt; lookup ≤ 0.1 ms. needs: MCP9, MCP11.`
- **Falsifiable experiment + consumer**: two-version replay test x2; consumer is EV2 brief→game eval (which must reproduce a build exactly). Killed if unversioned replay already suffices (then versions are vacuous).
- **EMPIRE outcome 4**: reproducible brief→game builds are the product promise — unpinned tools make "same brief, same game" a lie.

### Moonshot BET — `1broseidon/prism` — human-in-the-loop MCP gateway with postures, held calls and redacted audit (BET-MCP16)

- **Bet**: a local MCP gateway where every agent connects to one endpoint, tools aggregate as `{server}__{tool}`, calls resolve through posture (Supervised / First-use / Guided / Trusted) + glob rules (deny > ask > allow), held calls wait on a 2-minute operator card, and the audit keeps agent/tool/timestamp/verdict only — can be reimplemented headless as the operator-consent gate for brief→game writes (AB4 demo with a human approve step).
- **Baseline**: unattended MCP door (token-as-door, no per-call consent).
- **Metric**: (a) 100% of write-class tool calls held-or-ruled before execution; (b) audit holds 0 argument/result bytes; (c) held-call overhead on allow-listed reads ≈ 0; (d) unattended mode degrades to deny-by-default with receipt.
- **URL + rev + access**: https://github.com/1broseidon/prism @ `main` (1★ + 1 fork, updated 2026-09-11, Rust + Tauri), accessed 2026-09-26.
- **Licenses**: code MIT (`LICENSE` read live — on allow-list). Data/weights: none. `PORT(1broseidon/prism@main:crates/prism-core/)` (posture/rule/hold/audit discipline only) + EVIDENCE row below.
- **Claim / assumptions / counterevidence / no-transfer**: claim — one endpoint (`127.0.0.1:9086/mcp`), held-call cards (allow once / 30 min / always / server-wide), OAuth 2.1 agent auth with loopback consent, secrets in OS keychain, audit capped (5 MiB + 3 archives, 30 d) with args/results never persisted, lexical risky-action watch-list, loopback-only with Host checks. Assumptions: consent can be expressed headless (policy file = pre-answered cards; timeout = deny). Counterevidence: a Tauri tray app needing D-Bus/keychain/AppIndicator is the opposite of headless CI; human latency breaks 24/7 autonomy; donor's own words — "does not sandbox the servers it launches"; 1★. No-transfer: tray UI, Tauri shell, OAuth browser flows, harness hook installers — only posture/rule/hold/audit semantics transfer.
- **Kill rule**: headless run cannot pre-answer holds (CI blocks forever); any write auto-allowed without a rule; any argument/result bytes persisted; hold timeout defaults to allow.
- **Board row (BET)**: `[ ] BET-MCP16 endpoint (MOONSHOT): consent-gated MCP door for brief→game writes. Tier C. Size M. Owns policy hunks beside the MCP door (door frozen). Accept: 100% writes held-or-ruled; 0 arg bytes in audit; unattended = deny-by-default; 20-run AB4 demo all consented. Kill: CI block, auto-allow, arg persistence, default-allow. needs: MCP5, MCP10, MCP11.`
- **Falsifiable experiment + consumer**: policy-file pre-answers + deny-on-timeout on 20 scripted write calls x2; consumer is the AB4 operator-supervised demo. Killed by the rule above.
- **EMPIRE outcome 4**: the bold version of "AI makes whole games" — a game the operator watched being approved, action by action.

## Rejects (one-line reasons)

- `harsha-moparthy/capbox` (0★, fuel-metering numbers claimed): REJECT — no LICENSE file at root (verified by full listing 2026-09-26) → UNVERIFIED, never a code donor.
- `misteral/piguard` (MIT verified live): REJECT — ONNX Runtime + DeBERTa-v3 weight stack with ~300 ms cold start vs jailguard's 14 ms embedded gate; heavier, needs a weight license row, no new falsifiable gap.
- `DrakoTrogdor/InjectorDetector` (MIT verified live, 0★): REJECT — single SAFE/NOT-SAFE verdict CI gate for git repos; coarser than the RV1+RV2+score stack and overlaps RV2's pattern layer with no new gap.
- Dive-11 donors (`h3r2tic/dolly`, `bonsairobo/smooth-bevy-cameras`, `johanhelsing/bevy_trauma_shake`, `saltmire/saltmire-juice`, `ValorZard/Quad-Fighter-2`, `m2-md/game-feel-hitstop-juice`): not re-swept — no new evidence.
- Wave-36 Dive-12 donors (`MikkoParkkola/nab`, `sunilgentyala/AegisForge-MCP`, `gebruder/wirken`, `Askannz/munal-os`, `guyoung/boxagnts`): not repeated — RV2/MCP11/EA2/CT16/MCP12 already cover them.

## EVIDENCE rows (proposed)

| Idea | Source | Verdict |
|---|---|---|
| Score-gated import screen stacked over RV1+RV2 (import-time only, mock scorer first) | `PORT(yfedoseev/jailguard@53d2d47:src/)` MIT+Apache-2.0, https://raw.githubusercontent.com/yfedoseev/jailguard/main/LICENSE-MIT, verified 2026-09-26 (weights UNVERIFIED) | IN — RV3 |
| Digest-chained audit receipts with bounded rotation | `PORT(quipu-log/Quipu-Log@c2a974f:crates/)` Apache-2.0, https://raw.githubusercontent.com/quipu-log/Quipu-Log/main/LICENSE, verified 2026-09-26 | IN — EA3 |
| Capability manifests + static import-vs-manifest scan before tool run | `PORT(Codexia-afk/WasmVault@main:src/)` MIT, https://raw.githubusercontent.com/Codexia-afk/WasmVault/main/LICENSE, verified 2026-09-26 | IN — MCP13 |
| Vault-backed one-child secret injection, key-names-only audit | `PORT(numikel/kprun@main:crates/kprun-core/)` MIT, https://raw.githubusercontent.com/numikel/kprun/main/LICENSE, verified 2026-09-26 | IN — MCP14 |
| Version-pinned tool registry for replay-exact brief→game builds | `PORT(maverickkhan/rust-agent-sandbox@169c0cd:crates/)` MIT, https://raw.githubusercontent.com/maverickkhan/rust-agent-sandbox/main/LICENSE, verified 2026-09-26 | IN — MCP15 |
| Consent-gated MCP door (postures, held calls, redacted audit) | `PORT(1broseidon/prism@main:crates/prism-core/)` MIT, https://raw.githubusercontent.com/1broseidon/prism/main/LICENSE, verified 2026-09-26 | BET — MCP16 moonshot |

## Unverified

- No code cloned or run; all takes are pattern reimplementations (Rust, offline/headless, integer-tick-safe) — REPRODUCE stamps require the listed experiments.
- Donor numbers (jailguard 98.40%/14 ms, WasmVault <5 ms/<16 MB, capbox-style hostile counts) are donor claims, not engine measurements; engine values come from the Dive-12 fixture, not donor READMEs.
- `jailguard` embedded weights are UNVERIFIED: no weight file enters git without a license read + SHA-pinned manifest; the RV3 experiment starts with a mock scorer so the row holds even if weights are refused.
- `rust-agent-sandbox` internals beyond the root listing + description are UNVERIFIED; MCP15 survives on the version-pinning discipline alone.
- WasmVault/kprun/prism PORT revs pin `main` (commit SHAs not captured this pass); re-pin to SHAs before any port.
- wasmtime (Cards 3, 5) needs a DEPENDENCIES row + `cargo deny check` before any dependency lands; the experiments are designed to fail closed to MCP11 if the dep is refused.
