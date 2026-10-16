# ARXIV Wave 47 — Dive 14: hard-problem queue (version-gated receipts, record-path policy, vault secret injection UX, bundle-cook determinism)

**Dive**: 14 (hard-problem queue: EV12 version-gated golden receipts, EV13 record-path policy for the proof CLI, MCP14 vault-backed one-child secret injection, CT12/AS1 bundle-cook determinism). Wave 47 RES-ARXIV packet.
**Date**: 2026-09-26
**Budget**: M 20 min
**Donor rotation**: FootsiesGym (2607.06514, wave 46), NovaFabric (2609.12582, wave 46), Sello/Notarized-Agents (2606.04193, wave 46), HTEC (2309.10089, wave 46), DDGI-Resampling (2108.05263, wave 44), COSA-GS (wave 43), allocation-sampling profiler (2506.16883, wave 45), coordinated rename (2601.00482, wave 45), change-point ensembles (2606.18377 + 2408.08148, waves 44–45) explicitly NOT repeated — no new evidence on any of them. Split-RC stays verdict-LATER, cited as context only.

---

## Papers (donors, in own words)

**arXiv:2104.06020v1** — *Reproducible Builds: Increasing the Integrity of Software Supply Chains* (Lamb & Zacchiroli, Apr 2021). cs.SE. https://arxiv.org/abs/2104.06020 — License UNVERIFIED on arXiv page. Ideas only, reimplemented never copied.
**Reproduced claim (paper's, in own words)**: trusting source is not trusting binaries — third-party-built executables can diverge from reviewed source with severe consequences; the fix is builds that produce bit-for-bit identical bytes every time, plus the hard-won lesson from making Debian reproducible that the fight against nondeterminism (timestamps, paths, ordering, environment leakage) doubles as quality assurance.

**arXiv:2509.09036v1** — *It Should Be Easy but... New Users Experiences and Challenges with Secret Management Tools* (Neil, Mungara, Williams, Acar, Reaves, Sep 2025, extended version). cs.HC. https://arxiv.org/abs/2509.09036 — License UNVERIFIED. Ideas only.
**Reproduced claim (paper's, in own words)**: in a two-step lab study, 21 new users did secret storage-and-access then secret injection with Vault/Infisical-class tools plus public help resources; even when users found the functionality and liked interface flexibility, remediating a hard-coded secret got hard the moment docs felt insufficient — users then left the official docs for secondary sources or workarounds. Reported pain clusters: documentation content quality, navigation (both docs and web UI) when hunting for the helpful page, and missing supportive tool features.

**arXiv:2409.05014v2** — *Analyzing Challenges in Deployment of the SLSA Framework for Software Supply Chain Security* (Tamanna et al., Sep 2024). cs.CE/cs.SE. https://arxiv.org/abs/2409.05014 — License UNVERIFIED. Ideas only.
**Reproduced claim (paper's, in own words)**: after the reported 200% rise in supply-chain attacks, SLSA adoption still lags — so the authors thematically coded 1,523 SLSA-related issues across 233 GitHub repos (LDA-assisted) and found four adoption blockers headed by complex implementation and unclear communication, with five mitigation strategies headed by streamlining provenance generation, improving the verification step, and writing specific detailed documentation. Some strategies mitigate several blockers at once; some blockers need future tooling.

**arXiv:2605.08363v1** — *Kettle: Attested builds for verifiable software provenance* (Asad & Arko, May 2026). cs.CR/cs.SE. https://arxiv.org/abs/2605.08363 — License UNVERIFIED. Ideas only. Counterevidence / second take for the receipts cards, not a second cook donor.
**Reproduced claim (paper's, in own words)**: instead of re-running the build to check it, build inside a measured confidential VM that records source commit + dependency set + toolchain + environment + output digests into a provenance document, commit that document's SHA-256 into the TEE attestation report-data field, and let verification collapse to one signature check against the vendor root plus digest comparisons — removing build infra, operators and distribution from the verifier's trust surface, with optional pre-attestation and confidential source delivery.

---

## Engine home

- Primary: `docs/plan/quality.md` (PLN14/PLN29: EV11 verify-evidence open, EV12 version-gated receipts open, EV13 record-path policy open), `docs/plan/ai-native.md` (PLN9: MCP14 vault-backed one-child injection open, MCP15 version-pinned registry open, EA3 digest-chained receipts open), `docs/plan/assets.md` (PLN11: AS1 bundle landed, CT12 bake manifest open), `docs/plan/shipping.md` (PLN13: GM3 packaged exe + bundle, operator clean-machine run)
- Consumers: `tools/proof.mjs` legacy golden gate + tape schema (EV12/EV13), `src/bin/e2040-mcp.rs` launcher (MCP14), `crates/assets/src/bundle.rs` + future `manifest.rs` (CT12/AS1), GM3 clean-machine run
- Subsystem plans: `docs/plan/quality.md`, `docs/plan/ai-native.md`, `docs/plan/assets.md`, `docs/plan/shipping.md`
- EVIDENCE home: `docs/EVIDENCE.md` (one row per method adoption)
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (wave 47 section)
- SPRINT: EV11 (line 575), EV12 (line 601), EV13 (line 607), MCP14 (ai-native line 327), MCP15 (line 332), EA3 (line 323), CT12 (assets, no `manifest.rs` on disk), AS1 (landed, `bundle.rs` blake3)

---

## Simpler baseline (what the engine already has)

- **Version gate PROPOSED, not built**: EV12 is `[ ]` open — a stale-version receipt with the right hash passes the legacy golden gate today; the tape carries no build field. EV11 (digest+seal beside a6/a7/a9 writers, tamper demos fail typed) is the adjacent landed discipline; the version assert is the missing half.
- **Path policy PROPOSED, not built**: EV13 is `[ ]` open — a `.lanes/`-containing record path succeeds silently today. No allow/deny path rule exists in `crates/proof`; the RV1 red-team tape and `mcp-auth --allowed-dirs` prior art are patterns, not ports.
- **Vault injection PROPOSED, not built**: MCP14 is `[ ]` open — `e2040-mcp` spawns servers with no one-child injection today. The privacy gates (canary tests, key-names-only audit, headless keyfile path in acceptance) are specified but unwired; RV2/MCP8/MCP11 are the neighboring trust rows.
- **Cook deterministic, manifest missing**: AS1 bundle + blake3 digests landed; CT1 retarget and CT2 mesh-cook prove byte-identical x2; cook pins thread count 1 and posts encoder version in the receipt. CT12 (bake manifest) is open — no `manifest.rs` on disk — so there is no single manifest row that a 20-of-20 byte-equal rebuild gate can point at.

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): reproducible-cook discipline as the CT12 manifest acceptance (timestamps, paths, order, environment)

- **Question**: does the 2104.06020 discipline — enumerate every nondeterminism source (embedded timestamps, absolute paths, unordered iteration, environment/toolchain leakage), kill each at the cook layer, and treat the resulting byte-equality as QA signal — reproduce as CT12's acceptance: 20 of 20 rebuilds byte-equal with a manifest (source bytes + settings + importer version + target profile → artifact bytes + blake3 digest) that names the pinned inputs?
- **Engine home**: `docs/plan/assets.md` CT12 (new `manifest.rs`, needs CT2) over AS1 `bundle.rs`; consumers GM3 packaged bundle, CT6 museum registry, Tier B 20-of-20 gate
- **Baseline**: byte-identical x2 on CT1/CT2 spot checks (landed). Proposed: manifest-pinned full-bundle rebuild matrix (20 rebuilds, two lane machines or two checkouts) with per-source nondeterminism hunts.
- **URL + ID**: https://arxiv.org/abs/2104.06020, arXiv:2104.06020v1 (source-elimination discipline + reproducibility-as-QA only)
- **License**: UNVERIFIED. Ideas only — no Debian tooling, no new dependency, std-only manifest.
- **Paper claim**: bit-for-bit reproducible builds decide whether binaries match source; Debian experience shows reproducibility work surfaces QA bugs (their distro, their builds — NOT our numbers).
- **Reproduced result**: NONE YET — no rebuild matrix exists. Thresholds below are bars, not forecasts.
- **Assumptions**: thread count 1 stays pinned during the spike (assets-plan constraint); blake3 stays the digest (no new hash); manifest is presentation/provenance, never tick input (HASH-LAW untouched).
- **Counterevidence**: Debian-scale reproducibility took years of distro-wide effort — the engine take scopes to *one* bundle (arena + fighters) with a closed input set, and any source that needs per-asset tuning to pass is a demo, not a gate (kill rule). Kettle (Card 4's donor) argues attestation can substitute for rebuildability — rejected here as the *primary* answer (needs TEE hardware + vendor root the engine must not require); kept only as the challenger ablation.
- **What does NOT transfer**: any Debian build tooling, any distro-scale process claim, any paper timeline/effort number as an engine estimate.
- **Experiment sketch (S, 1 session)**: write the CT12 manifest shape (inputs + digests + encoder/importer versions); run the arena-bundle cook 20× across two checkouts; diff first divergence byte-first; fix-and-relist each nondeterminism source found.
- **Benchmark/ablation**: manifest-pinned vs unpinned rebuild agreement (20/20 vs today's x2); per-source ablation (timestamp strip on/off, path relativize on/off, sort-order pin on/off — which one actually moved the needle); Kettle-style attest-without-rebuild as the challenger (must lose on hardware-independence to stay rejected).
- **Success threshold**: 20/20 byte-equal across checkouts; manifest names every pinned input; each fixed source has a regression test; golden unchanged x2.
- **Kill criterion**: any source needs per-asset hand-tuning (demo, not gate); divergences that move with wall-clock or machine identity twice with the same fingerprint → BLOCKERS.md, x2 spot checks stay the answer; any manifest byte leaking into the tick or hash.
- **Board row proposal**: none — experiment sketch for existing CT12. EVIDENCE row on the manifest if it lands.
- **Stamp**: REPRODUCE (proposed).
- **EMPIRE outcome 4**: generated games ship as bundles — a manifest-pinned reproducible cook is what lets brief→game outputs be re-cooked by the factory without "it built on my lane."

### Card 2 (TAKE, REPRODUCE): version-gated golden receipts as the SLSA-provenance answer at engine scale (EV12)

- **Question**: does the 2409.05014 strategy pair — streamline provenance generation (build field rides the tape by construction) + improve the verification step (gate refuses stale-version receipts loudly) — reproduce as EV12: `engine_version` enforced in the legacy golden gate, stale-version receipt fails loudly, current passes, golden unchanged?
- **Engine home**: `tools/proof.mjs` version assert (+ tape build field) — EV12, needs none; neighbor EV11 (digest+seal writers) stays frozen
- **Baseline**: stale-version receipt with the right hash passes silently today (ADV46 finding). Proposed: version assert as the refusal rule with a named error.
- **URL + ID**: https://arxiv.org/abs/2409.05014, arXiv:2409.05014v2 (adoption strategies only: streamlined provenance + tighter verification + specific docs)
- **License**: UNVERIFIED. Ideas only — no SLSA tooling, no new dependency.
- **Paper claims**: complex implementation + unclear communication block SLSA; streamlined generation + better verification + detailed docs mitigate (their 1,523-issue corpus — quoted, never adopted as engine numbers).
- **Reproduced result**: NONE YET. Our metric is seeded stale-version refusal, not issue counts.
- **Assumptions**: tape build field is additive (old tapes without it fail loudly with the named error, never silently pass); version compare is exact string equality first (no semver-range cleverness until the exact gate is green).
- **Counterevidence**: the paper's own corpus warns verification complexity *is* the adoption blocker — so the take keeps the gate to one assert + one named error, and Kettle (Card 4) warns a signature-only check without version binding attests the wrong thing well. Any proposal to range-accept versions or auto-migrate receipts is rejected at review (slim rule: receipts append-only, never edited to fit).
- **What does NOT transfer**: SLSA levels/threat-model as acceptance, any SLSA generator/verifier code, issue-count statistics as engine bars.
- **Experiment sketch (S)**: add the build field + version assert; seed a stale-version receipt with the correct hash (must fail loudly, naming the version skew) and a current receipt (must pass); run golden x2.
- **Benchmark/ablation**: gate on vs off (seeded stale passes/fails); exact-match vs range-match (range must be shown to admit a stale receipt before it is refused as a design).
- **Success threshold**: every seeded stale-version receipt refused with the version named; current receipts pass; golden unchanged; error text names the recovery (re-record on current build).
- **Kill criterion**: any stale receipt passes silently twice (same fingerprint → BLOCKERS.md, EV11 digests stay the answer); version compare needs per-release tuning (demo, not gate).
- **Board row proposal**: none — experiment sketch for existing EV12. EVIDENCE row on the assert if it lands.
- **Stamp**: REPRODUCE (proposed).
- **EMPIRE outcome 4**: brief→game proof rots the moment old receipts pass new gates — version binding is what keeps the factory's sold claims checkable after the engine moves.

### Card 3 (TAKE, REPRODUCE): supportive-tool UX for vault secret injection (MCP14 launcher, docs-first per the lab study)

- **Question**: does the 2509.09036 finding — new users fail at secret *injection* specifically when docs are insufficient, then defect to secondary sources or workarounds — reproduce as MCP14's acceptance shape: one-child injection whose headless keyfile path + error texts + key-names-only audit are usable enough that a new lane operator completes 20 launches with 0 canary bytes outside the child and never needs a workaround?
- **Engine home**: `src/bin/e2040-mcp.rs` launcher hunks (door frozen) — MCP14, needs MCP5 + MCP7 (both ticked); consumers every MCP-served lane, AB4 brief-to-game runs
- **Baseline**: no one-child injection today (missing). Proposed: env/file→child-only injection + canary suite + audit-holds-key-names-only + headless keyfile path, with the study's three pain clusters as the checklist.
- **URL + ID**: https://arxiv.org/abs/2509.09036, arXiv:2509.09036v1 (two-task lab method + three pain clusters only)
- **License**: UNVERIFIED. Ideas only — no Vault/Infisical code, no new dependency.
- **Paper claims**: 21-user lab: injection is where new users stall; doc quality + navigation + supportive features decide whether they stay on the rails (their users, their tools — NOT our numbers).
- **Reproduced result**: NONE YET. Our metric is canary-clean launches + workaround-free onboarding on the lane, not lab task times.
- **Assumptions**: secrets never enter diagnostics by construction (privacy-gate law); audit holds key names only; child-only means the parent process cannot log, print, or persist the bytes (canary tests assert this, they do not merely document it).
- **Counterevidence**: the study is narrow (21 new users, lab setting, authors' own caveat it is a proxy for early-career adopters) — so the engine run is its own 20-launch fixture plus a fresh-operator walkthrough, and any step that only passes after the operator reads a secondary source counts as a docs failure, not a pass. Vault-class server features (rotation, policies, audit backends) do NOT transfer — one-child injection only.
- **What does NOT transfer**: any Vault/Infisical server code or config schema, any lab timing/success number as acceptance, any server-side secret-management scope.
- **Experiment sketch (S)**: build injection + 20-launch canary fixture (seeded fake secrets, parent-log + env-dump + audit-trail scans for bytes); fresh-operator walkthrough using only in-repo docs; log every workaround/defection.
- **Benchmark/ablation**: injection on vs today's spawn (canary bytes found/not-found); docs-as-shipped vs docs-with-study-checklist (content quality + navigation + supportive errors — defection count); keyfile path vs env path (headless parity).
- **Success threshold**: 20/20 launches canary-clean; audit shows key names only; walkthrough completes with zero secondary-source defections; golden unchanged.
- **Kill criterion**: any secret byte outside the child (reject immediately, BLOCKERS.md); walkthrough needs a workaround twice with the same fingerprint → docs fail, MCP14 stays open; any secret in git.
- **Board row proposal**: none — experiment sketch for existing MCP14. EVIDENCE row on the launcher if it lands.
- **Stamp**: REPRODUCE (proposed).
- **EMPIRE outcome 4**: 24/7 brief→game loops hand provider keys to child servers every run — injection UX that fails new operators is what leaks the keys that pay for the fleet.

### Card 4 (TAKE, REPRODUCE with counterevidence built in): record-path policy + Kettle-shaped attestation as the challenger that must lose on hardware-independence (EV13 + receipts)

- **Question**: can EV13's record-path policy — `--record`/`--replay` refuse lane-dir/traversal paths loudly with a named error — land as the SLSA "unclear communication" fix (the refusal *names* the recovery), while the Kettle attestation shape (provenance doc + one-check verify without rebuild) is scored head-to-head against the engine's journal+FNV receipts and rejected unless it wins without TEE hardware?
- **Engine home**: `crates/proof` path policy + negative test (EV13, needs none) vs `tools/proof.mjs` + ED7 journal + EV11 digests (incumbent receipts); consumer Q1/PQ3/proof CLI every wave
- **Baselines**: `.lanes/`-containing record path succeeds silently today (ADV46); receipts are FNV/hash + journal seal, std-only, no vendor root. Proposed: explicit allow/deny path rule with named errors; Kettle challenger scored on the same fixtures.
- **URLs + IDs**: https://arxiv.org/abs/2409.05014 (verification + docs strategies for EV13) + https://arxiv.org/abs/2605.08363, arXiv:2605.08363v1 (attested-build challenger: provenance doc + single-check verify)
- **License**: both UNVERIFIED. Ideas only — no TEE SDK, no vendor attestation code, no new dependency; Kettle's *shape* (inputs+digests doc, one-check verify) is the only thing scored.
- **Paper claims**: SLSA verification/documentation strategies mitigate adoption blockers (theirs); Kettle collapses verification to one signature check + digest comparisons with no rebuild (theirs — quoted, never adopted).
- **Reproduced result**: NONE YET. Our metrics are refusal loudness + verify-without-rebuild on lane fixtures.
- **Assumptions**: path policy is a pure allow/deny function over the record path (no filesystem writes during the check, no network); Kettle challenger runs with *software* signatures only in the spike (no TEE hardware on the lane) — which is exactly why it is expected to lose on the hardware-independence bar.
- **Counterevidence**: Kettle's trust chains to the TEE manufacturer's root, not to the engine — adopting it would trade the journal (which the engine owns) for a vendor root (which it must not require per provider-neutral law). The paper's confidential-source-delivery half has no engine consumer (sources are open, MIT). EV13's own risk per the SLSA study: a refusal that does not name the recovery *is* the "unclear communication" blocker — so the acceptance requires the named error + the passing alternative in one line.
- **What does NOT transfer**: any TEE/CVM code, any vendor-root trust, Kettle's confidentiality half, any SLSA verifier binary.
- **Experiment sketch (S)**: EV13 — seed lane-dir, traversal (`../`), and absolute-escape record paths (must refuse loudly with the named error + the legal alternative); normal paths unaffected. Challenger — write a Kettle-shaped provenance doc for one bundle cook, verify-by-doc-only vs verify-by-rebuild; score hardware needed, bytes compared, failure localisation.
- **Benchmark/ablation**: policy on vs off (seeded bad paths refused/silent); named-error vs bare-refusal (fresh-operator recovery without secondary sources); doc-verify vs rebuild-verify (agreement + hardware bill).
- **Success threshold**: all seeded bad paths refused with the error naming the fix; normal paths pass; doc-verify agrees with rebuild-verify on the fixture AND the challenger is refused as a dependency (software-only signature proves nothing a journal seal does not already prove cheaper).
- **Kill criterion**: any bad path succeeds silently twice (same fingerprint → BLOCKERS.md); named error missing the recovery (then it is the blocker the paper names); challenger needs TEE/vendor account to pass (rejected immediately per provider-neutral law).
- **Board row proposal**: none — experiment sketch for existing EV13 (+ scored challenger for the receipts line). EVIDENCE row only if the policy lands.
- **Stamp**: REPRODUCE (proposed).
- **EMPIRE outcome 4**: proof that silently records into lane dirs corrupts every later replay — and proof that needs vendor hardware cannot be sold with an MIT engine.

### Card 5 (REJECT): lifts of any donor apparatus as-is

- **Question**: lift Debian's reproducible-builds toolchain, a Vault/Infisical server deployment, the SLSA generator/verifier stack, or a Kettle TEE attestation pipeline straight into the engine?
- **Verdict**: REJECT all four lifts; Cards 1–4 take the disciplines through engine gates instead.
- **Reasons (one line each)**: Debian tooling is distro-scale process, not one-bundle cook code — discipline only; Vault/Infisical servers are operated infrastructure with a trust + license surface the lane must not carry — one-child injection only; SLSA generators/verifiers reintroduce the implementation complexity the paper itself reports as the blocker — one assert + one named error instead; Kettle TEE attestation chains trust to a vendor root and needs hardware the reference lane does not have — shape scored, dependency refused.
- **Revisit trigger**: Card 1/2/3/4 spikes beat baselines with receipts on engine fixtures — then the combination is already what Cards 1–4 describe. No new evidence needed beyond that.

---

## What does NOT transfer (explicit)

- Any donor source bytes (all UNVERIFIED; reimplement ideas only, std-only, no new dependency).
- Any Debian/SLSA/lab/Kettle number as engine acceptance (their distros/corpora/users/hardware, never ours).
- Any TEE hardware, vendor attestation root, or confidential-compute dependency (provider-neutral law).
- Any secret-management server, policy engine, or rotation infrastructure (MCP14 is one-child injection only).
- Any semver-range acceptance or receipt auto-migration (receipts append-only; stale fails loudly).
- Any manifest, statistics, or attestation math leaking into the tick or hash (HASH-LAW; all four takes are cook/proof/launcher-side by construction).
- A second cooker, second registry, second harness, or per-asset-tuned gate (CUT / kill rules above).

---

## Integration homes (per take)

| Take | Subsystem plan | EVIDENCE home | Consumer |
|---|---|---|---|
| Card 1 reproducible-cook manifest | `docs/plan/assets.md` CT12 (+AS1, CT6) | `docs/EVIDENCE.md` manifest row (only if lands) | GM3 bundle, Tier B 20-of-20 gate |
| Card 2 version-gated receipts | `docs/plan/quality.md` EV12 (+EV11 context) | `docs/EVIDENCE.md` assert row (only if lands) | proof golden gate, factory claim checks |
| Card 3 vault injection UX | `docs/plan/ai-native.md` MCP14 | `docs/EVIDENCE.md` launcher row (only if lands) | MCP lanes, AB4 brief-to-game runs |
| Card 4 path policy + attestation challenger | `docs/plan/quality.md` EV13 (+ receipts line) | `docs/EVIDENCE.md` policy row (only if lands) | proof CLI every wave |
| Card 5 reject | — | — | — |

## Paper claim vs reproduced result vs integrated gain (kept distinct)

- **Paper claims**: 2104.06020 bit-for-bit reproducibility as supply-chain integrity + reproducibility-as-QA from Debian (theirs); 2509.09036 injection-is-where-new-users-stall with doc/navigation/supportive-feature pain clusters over 21 lab users (theirs); 2409.05014 four SLSA blockers (complex implementation, unclear communication first) + five strategies over 1,523 issues / 233 repos (theirs); 2605.08363 one-signature + digest-comparison verification with no rebuild inside TEE attestation (theirs). All theirs — quoted, never adopted as engine numbers.
- **Reproduced results**: NONE — no spike has run for any card. Cards 1–4 propose reproductions with engine fixtures and kill rules.
- **Integrated gains**: NONE — no CT12 manifest, no EV12 assert, no MCP14 injection, no EV13 policy has landed. Any future gain is measured in rebuild agreement counts, seeded-refusal loudness, canary-clean launch counts, and defection-free walkthroughs — never quoted from a paper.

---

## Budget and next step

Budget M 20 min: four query families (supply-chain hit, secret-UX hit-rich, SLSA hit-rich, asset-pipeline mismatch logged) + four full-abstract reads + plan/board cross-checks (EV12/EV13 SPRINT lines, MCP14/MCP15/EA3 ai-native rows, AS1/CT12 assets rows) + this note. At budget stop: takes + next step reported, no code touched.

Next step (orchestrator): file Cards 1–4 as experiment sketches on existing rows (CT12, EV12, MCP14, EV13 — no new rows); log the Dive-14 donor set (2104.06020, 2509.09036, 2409.05014, 2605.08363) so future waves rotate past them without new evidence.

## Source traceability

- ArXiv queries 2026-09-26: `reproducible builds determinism bundle provenance supply chain` → 2104.06020 (+4 supply-chain-blockchain mismatches); `secret management vault injection usability developer experience` → 2509.09036 (+4 fuzzy-vault/password-manager mismatches, domain-correctly rejected); `SLSA provenance attestation in-toto versioned receipts software supply chain` → 2409.05014, 2605.08363 (+3 adjacent); `deterministic asset pipeline content-addressable build cache hermetic` → cache/CAM mismatches (logged, no donor taken).
- Abstracts verified live via arxiv tools 2026-09-26 (2104.06020, 2509.09036, 2409.05014, 2605.08363). No PDFs fetched; citation graphs not needed for method cards.
- Code cross-checks: `docs/SPRINT.md` EV12 (line 601) / EV13 (line 607) open Tier A S; `docs/plan/ai-native.md` MCP14 (line 327) / MCP15 (line 332) / EA3 (line 323) open; `docs/plan/assets.md` AS1 landed (`bundle.rs` blake3) / CT12 open (no `manifest.rs`); `docs/plan/shipping.md` GM3 open (operator clean-machine run).
- Donor dedup: FootsiesGym/NovaFabric/Sello/HTEC (wave 46), DDGI-Resampling (wave 44), COSA-GS (wave 43), sampling-profiler + rename + CPD ensembles (wave 45), Split-RC (LATER) — none re-served.
- EMPIRE: outcome 4 ("AI makes whole games" — brief→game needs re-cookable bundles, checkable receipts, leak-free key handling, corruption-proof proof paths).
