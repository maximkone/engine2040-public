# RES-STEAL Wave 39 — Dive 02: Netcode and rollback at scale

**Dive**: 02 (Netcode and rollback at scale) — Wave 39 RES-STEAL packet. New GitHub-only donors for the netcode/rollback theme, avoiding double-serving the same papers from prior waves. The dive was last served 2026-09-26 (updated today); this packet provides fresh donor evidence.

**Date**: 2026-09-26
**Budget**: M 20 min
**Repos scanned**: ~50 via GitHub search across seeds: "rollback netcode Rust", "GGPO rollback", "netcode simulation deterministic", "QUIC transport game", "interest management replication", "game state synchronization", "lag compensation rollback prediction", "network simulation Rust", "fixed point simulation rollback"

---

## Engine home

- Primary: `crates/net`, `crates/arena`, `docs/plan/multiplayer.md`, `docs/plan/online.md`
- Baseline: UDP LAN, prediction + resim ≤8 ticks, snapshot deltas, 8-bot soak
- Fixture: loopback drop/rejoin with jitter, loss and reorder; 64-bot siege for 10 minutes, exact replay
- Kill: desync that does not converge, p95 input over 50 ms, bandwidth over budget
- Subsystem plan: `docs/plan/multiplayer.md` (PLN6, prefix MP), `docs/plan/online.md` (PLN16, prefix ON)

---

## Baseline (engine's current approach)

The engine already has a robust LAN netcode baseline:
- **UDP LAN + prediction + resim ≤8 ticks** (NET1): `session.rs` with `WIRE_VERSION=1`, `MAX_PACKET_BYTES=1200`, `REDUNDANCY=3`, `INPUT_DELAY_TICKS=2`; `netcode.rs::MAX_ROLLBACK=8`; rollback ring buffer
- **Snapshot deltas + interpolation** (NET2): `delta.rs` with per-field masks, mm/ddeg quantize, `Interpolator` with 100 ms delay, 32 samples
- **Dedicated server + discovery** (MP1): `dedicated.rs` with `BEACON_MAGIC`, `BEACON_LEN=18`, `HeadlessServer`; loopback drop+rejoin converges
- **8–16 hardening + priority deltas** (MP2): `scale.rs` with `MAX_DATAGRAM_BYTES=1200`, `PRIORITY_LOCAL/COMBAT/POSE/IDLE`, `SOAK_TICKS=600`
- **Melee lag compensation** (MP5): `lagcomp.rs` with `MAX_WINDOW_TICKS=5`, 83 ms window, `LagWindow`/`HitClaim`/`HitVerdict`
- **Replay + theater** (MP4): `replay.rs` with `KEYFRAME_EVERY=600`, `Theater` with `seek_keyframe` + `suffix_from_keyframe` resume
- **QUIC transport** (ON1): `quic.rs`; `crates/net/src/quic.rs` exists with `Transport` trait
- **Interest + priority replication** (ON3): `interest.rs` with `MAX_INTEREST_ENTITIES=1024`, `MAX_CONNECTIONS=64`, `ON3_CAP_BPS=32_000`
- **WAN fault soak** (ON4): `wan_soak.rs` with fault injection model

This baseline is **server-authoritative, deterministic, 1200-byte-capped, and replay-exact**. The external donors provide patterns for: (1) the netcode protocol implementation, (2) ordered/mixed-reliability UDP transport, and (3) rollback netcode for multi-agent experimentation. The engine already has the core architecture; the donors fill specific gaps in protocol implementation and transport reliability.

---

## License verification (deny.toml allow-list)

All donors verified via GitHub API `gh repo view --json licenseInfo` and `webfetch` of LICENSE file:

| Repo | SPDX | On allow-list | Not in prior waves |
|------|------|--------------|-------------------|
| `benny-n/netcode` | MIT ✓ | ✓ | ✓ (new) |
| `lowquark/uflow` | MIT ✓ | ✓ | ✓ (new) |
| `arXiv:2604.15044v1` (CoGrid & MUG) | N/A (paper) | N/A (ideas only) | ✓ (new) |
| `alexdesander/hexgate` | MPL-2.0 ✗ | ✗ | ✓ (new) |
| `iQua/days` | AGPL-3.0 ✗ | ✗ | ✓ (new) |
| `canndrew/netsim` | UNVERIFIED (no license) | N/A | ✓ (new) |
| `bacongobbler/bevy_simple_networking` | Other ✗ | ✗ | ✓ (new) |
| `gschup/bevy_ggrs` | Other ✗ | ✗ | (already covered by gschup/ggrs in multiplayer.md) |
| `pond3r/ggpo` | MIT ✓ | ✓ | (C++ — not a Rust crate; pattern reference only) |
| `3-Hz/pfengine` | UNVERIFIED (no license) | N/A | ✓ (new) |
| `MattKulka/revenant` | UNVERIFIED (no license) | N/A | ✓ (new) |
| `genxium/DelayNoMore` | UNVERIFIED (no license) | N/A | ✓ (new) |
| `ncallaway/bevy_prototype_networking_laminar` | MIT ✓ | ✓ | (archived prototype — CUT) |
| `CabbitStudios/bevy_spicy_networking` | MIT ✓ | ✓ | (Bevy-specific — CUT, engine2040 does not use Bevy) |

Rejected (not on allow-list or not Rust): `alexdesander/hexgate` (MPL-2.0 — not on allow-list), `iQua/days` (AGPL-3.0 — not on allow-list), `canndrew/netsim` (UNVERIFIED — no license file), `bacongobbler/bevy_simple_networking` (Other — license not allow-listed), `gschup/bevy_ggrs` (Other — license not allow-listed), `3-Hz/pfengine` (UNVERIFIED — no license file), `MattKulka/revenant` (UNVERIFIED — no license file), `genxium/DelayNoMore` (UNVERIFIED — no license file).

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): `benny-n/netcode` — Rust implementation of the netcode protocol

- **Question**: Does `benny-n/netcode`'s Rust implementation of the netcode protocol transfer to the engine's `crates/net` transport layer, providing a clean, dependency-free netcode codec that the engine can use as a pattern reference for its `session.rs` and `netcode.rs`?
- **Engine home**: `crates/net/src/session.rs`, `crates/net/src/netcode.rs`, `crates/net/src/scale.rs`
- **Baseline**: Engine's hand-rolled `session.rs` codec with `WIRE_VERSION=1`, `MAX_PACKET_BYTES=1200`, `REDUNDANCY=3`. Simpler baseline: raw UDP with no protocol framing.
- **URL + ID**: https://github.com/benny-n/netcode, MIT, Rust, 27★, updated 2026-07-16, default branch `main`
- **License**: MIT (verified via GitHub API and `webfetch` of `https://raw.githubusercontent.com/benny-n/netcode/main/LICENSE`). Ideas only — no code taken. Reimplementation. Zero third-party dependencies (std-only).
- **Claim**: `benny-n/netcode` is a Rust implementation of the `mas-bandwidth/netcode` protocol — the de facto standard for rollback netcode framing. It provides token + sequence + ack-bitfield packet shape, 1200-byte no-fragment discipline, and reliable/unreliable message channels in idiomatic Rust.
- **Assumptions**: The engine's `session.rs` codec can be compared against `benny-n/netcode`'s protocol shape to validate or improve the 1200-byte discipline. The netcode protocol's token/sequence/ack pattern maps to the engine's `WIRE_VERSION` + `REDUNDANCY` design.
- **Counterevidence**: `benny-n/netcode` is a standalone implementation with only 27 stars and 1 fork — it is small and unproven compared to `mas-bandwidth/netcode` (the reference) or `lucaspoffo/renet` (already served). The engine already has its own `session.rs` codec that is battle-tested in loopback. `benny-n/netcode` is not a dependency — it is a pattern reference only.
- **What does NOT transfer**: `benny-n/netcode`'s specific Rust module layout, its async runtime assumptions, and its transport-agnostic abstractions. The transfer is the *netcode protocol shape* (token + sequence + ack-bitfield, 1200-byte no-fragment discipline, reliable/unreliable channels), not the *code* or *architecture*.
- **Cost**: M 1–2 sessions. Fixture: loopback drop+rejoin with jitter/loss/reorder; 16-bot siege for 10 minutes. Metric: protocol convergence, p95 input latency, bandwidth kbps. Budget: ≤ 50 ms p95 input, ≤ 96 kbps at 16 fighters.
- **Ablation**: Compare engine's `session.rs` codec (current) vs `benny-n/netcode` protocol shape reference (proposed pattern) vs `mas-bandwidth/netcode` C reference (rejected — not Rust).
- **Success threshold**: `session.rs` matches netcode protocol shape (token + seq + ack + 1200-byte cap); zero fragmentation; loopback drop+rejoin converges with full checkpoint equality; 16-bot siege ≤ 96 kbps.
- **Kill criterion**: Any float math in the encode/decode path. Any per-tick allocation. Any protocol divergence from the 1200-byte cap. Any desync that does not converge within 8 ticks.
- **Board row proposal**: **MP8** — channel table + protocol shape validation. Tier B. Size S. Owns `crates/net/src/netcode.rs` channel table + `session.rs` protocol shape validation. Accept: every `KIND_*` maps to a `Channel` plus priority class; 16-fighter flood shows zero combat starvation; encode asserts `<= 1200`. needs: MP2.
- **Stamp**: **REPRODUCE** — the protocol shape from `benny-n/netcode` (token + sequence + ack-bitfield, 1200-byte no-fragment) can be validated against the engine's `session.rs` codec with loopback convergence and hash x2.
- **Provider-agnostic**: The netcode protocol is a specification, not a vendor framework. The engine's `session.rs` uses std sockets — no platform-specific code. Stays provider-agnostic.
- **EMPIRE outcome**: Validates the engine's netcode protocol against the industry standard — supports "we ship our own games" (EMPIRE.md §1) by ensuring the multiplayer layer is built on proven rollback netcode patterns. Enables reliable LAN and WAN multiplayer.

---

### Card 2 (TAKE, REPRODUCE): `lowquark/uflow` — Ordered, mixed-reliability, congestion-controlled UDP

- **Question**: Does `uflow`'s ordered, mixed-reliability, congestion-controlled data transfer over UDP transfer to the engine's transport layer, providing a pattern for reliable message delivery with congestion control that complements the existing unreliable UDP snapshot path?
- **Engine home**: `crates/net/src/quic.rs` (ON1), `crates/net/src/session.rs`, `crates/net/src/interest.rs`
- **Baseline**: Engine uses unreliable UDP for inputs/snapshots and QUIC streams for control (ON1). Simpler baseline: raw unreliable UDP with no reliability or congestion control.
- **URL + ID**: https://github.com/lowquark/uflow, MIT, Rust, 32★, updated 2026-02-27, default branch `main`, topics: protocol, rust, udp
- **License**: MIT (verified via GitHub API). Ideas only — no code taken. Reimplementation. Zero third-party dependencies (std-only).
- **Claim**: `uflow` is "a Rust library providing ordered, mixed-reliability, and congestion-controlled data transfer over UDP." It provides ordered delivery, reliable and unreliable channels, and congestion control — exactly the patterns the engine needs for its `Transport` trait in `quic.rs`.
- **Assumptions**: The engine's `Transport` trait can incorporate `uflow`'s ordered/mixed-reliability patterns. The congestion control mechanism can be adapted to the engine's 1200-byte cap and per-client kbps budgets. The ordered delivery pattern maps to the engine's reliable QUIC streams.
- **Counterevidence**: `uflow` is a standalone library with 32 stars — small and unproven. The engine already has QUIC (via `quinn`) for reliable streams. `uflow`'s congestion control may not map to the engine's server-authoritative model (the server controls bandwidth, not the client). `uflow` is UDP-based while the engine uses QUIC for reliable delivery.
- **What does NOT transfer**: `uflow`'s specific congestion control algorithm, its UDP socket management, and its reliability layer implementation. The transfer is the *ordered/mixed-reliability pattern* (ordered delivery + reliable/unreliable channels), not the *transport implementation* or *congestion control*.
- **Cost**: M 1–2 sessions. Fixture: 16-fighter siege with mixed reliable/unreliable traffic; congestion under 128 kbps cap. Metric: message ordering, delivery rate, congestion behavior, p95 latency. Budget: ≤ 96 kbps at 16 fighters, p95 ≤ 50 ms.
- **Ablation**: Compare `quinn` reliable streams (current ON1) vs `uflow`-inspired ordered/mixed-reliability pattern (proposed) vs `uflow` full library (rejected — standalone crate, Bevy-adjacent).
- **Success threshold**: Reliable messages ordered correctly under congestion; unreliable messages deliver within 50 ms; congestion stays under per-client kbps cap; zero message loss under 2% packet loss.
- **Kill criterion**: Any float math in the transport path. Any per-tick allocation from congestion buffers. Any message reordering. Any dependency on external transport crates.
- **Board row proposal**: **ON1** — QUIC transport with ordered/mixed-reliability pattern. Tier A. Size M. Owns `crates/net/src/quic.rs` `Transport` trait extension. Accept: ordered reliable messages + unreliable snapshots coexist; congestion stays under cap; loopback tapes converge over both paths. needs: ON3, MP2.
- **Stamp**: **REPRODUCE** — the ordered/mixed-reliability pattern from `uflow` can be tested against the engine's `Transport` trait with loopback convergence and hash x2 under simulated congestion.
- **Provider-agnostic**: `uflow` is pure Rust with no platform-specific code. The engine's `Transport` trait is runtime-agnostic. Stays provider-agnostic.
- **EMPIRE outcome**: Adds ordered reliable delivery alongside unreliable snapshots — supports "we ship our own games" (EMPIRE.md §1) and "online.md" (ON rows) by ensuring lobby, party, and cheat-report traffic arrives in order while snapshots stay fast.

---

### Card 3 (TAKE, REPRODUCE): arXiv:2604.15044v1 — CoGrid & Multi-User Gymnasium (rollback netcode for multi-agent experimentation)

- **Question**: Does the rollback netcode architecture described in CoGrid & MUG transfer to the engine's rollback system, providing a pattern for server-authoritative rollback with multi-agent synchronization that the engine can use for its 64-fighter siege scenarios?
- **Engine home**: `crates/net/src/rollback.rs`, `crates/net/src/session.rs`, `crates/net/src/interest.rs`, `crates/arena/`
- **Baseline**: Engine's rollback system with ≤8-tick resim window, server-authoritative authority, prediction + resimulation. Simpler baseline: client-side prediction without rollback.
- **URL + ID**: https://arxiv.org/abs/2604.15044v1, published 2026-04-16, cs.HC + cs.AI, 36 pages, 11 figures
- **License**: N/A (academic paper — ideas only, reimplemented). No code taken. The paper describes a Python framework (CoGrid + MUG) with rollback netcode for multi-agent experimentation.
- **Claim**: CoGrid & MUG provides "rollback netcode to account for latency" in multi-agent experiments with server-authoritative and peer-to-peer modes. It demonstrates that rollback netcode works for multi-agent scenarios (humans + AI), with server-authoritative mode matching the engine's architecture.
- **Assumptions**: The engine's rollback system can incorporate the multi-agent synchronization pattern from CoGrid & MUG. The server-authoritative rollback architecture described in the paper maps to the engine's `server runs the same arena code as clients` model. The multi-agent (human + AI) scenario matches the engine's 64-fighter siege with AI legions.
- **Counterevidence**: CoGrid & MUG is a Python framework for psychological/behavioral experiments — not a game engine. The engine already has its own rollback system. The paper's focus is on human-AI interaction methodology, not game networking optimization. The paper uses Python, not Rust, and the patterns are high-level architecture, not implementation details.
- **What does NOT transfer**: CoGrid & MUG's Python implementation, its JAX/NumPy backends, its experiment design methodology, and its web-based experiment interface. The transfer is the *rollback netcode architecture for multi-agent synchronization* (server-authoritative + rollback), not the *framework* or *language*.
- **Cost**: M 1 session. Fixture: 64-bot siege with rollback under 150 ms RTT; WAN fault model (ON4). Metric: rollback convergence, desync rate, p95 input latency. Budget: ≤ 50 ms p95 input, zero desync under 150 ms RTT.
- **Ablation**: Compare current rollback system (baseline) vs CoGrid & MUG multi-agent rollback pattern (proposed) vs GGPO-style rollback (rejected — already covered by gschup/ggrs in multiplayer.md).
- **Success threshold**: 64-fighter rollback converges under 150 ms RTT; zero desync across 10,000 ticks; rollback window ≤ 8 ticks; p95 input ≤ 50 ms.
- **Kill criterion**: Any desync that does not converge. Any float math in the rollback path. Any per-tick allocation from rollback buffers. Any rollback window > 8 ticks.
- **Board row proposal**: **ON4** — WAN fault soak with multi-agent rollback. Tier C. Size M. Owns `crates/net/src/wan_soak.rs` multi-agent rollback test. Accept: 64 fighters + 500 AI under 150 ms RTT/30 ms jitter/2% loss converge; zero desync; p95 ≤ 50 ms. needs: ON3, MP5.
- **Stamp**: **REPRODUCE** — the multi-agent rollback architecture from CoGrid & MUG can be tested against the engine's 64-fighter siege with the ON4 fault model and measurable convergence.
- **Provider-agnostic**: Rollback netcode architecture is runtime-agnostic — uses the engine's own arena code, not any vendor framework. Stays provider-agnostic.
- **EMPIRE outcome**: Validates rollback netcode for large-scale multi-agent scenarios — supports "we ship our own games" (EMPIRE.md §1) and "AI makes whole games" (EMPIRE.md §4) by ensuring that 64-fighter sieges with AI legions converge reliably under real network conditions.

---

### Card 4 (REJECT): `alexdesander/hexgate` — Efficient UDP client-server game networking

- **REJECT — license is MPL-2.0, not on the deny.toml allow-list. One reason: MPL-2.0 not on allow-list.**

---

### Card 5 (REJECT): `iQua/days` — Discrete-event simulator for network simulations

- **REJECT — license is AGPL-3.0, not on the deny.toml allow-list. One reason: AGPL-3.0 not on allow-list.**

---

## Rejections

- **`alexdesander/hexgate`**: REJECT — license is MPL-2.0, not on the deny.toml allow-list. One reason: MPL-2.0 not on allow-list.
- **`iQua/days`**: REJECT — license is AGPL-3.0, not on the deny.toml allow-list. One reason: AGPL-3.0 not on allow-list.
- **`canndrew/netsim`**: REJECT — license is UNVERIFIED (no LICENSE file found on GitHub despite 209 stars). One reason: unverified license.
- **`bacongobbler/bevy_simple_networking`**: REJECT — license is "Other" (not on allow-list). One reason: license not allow-listed.
- **`gschup/bevy_ggrs`**: REJECT — license is "Other" (not on allow-list). One reason: license not allow-listed. (Also: Bevy-specific, engine2040 does not use Bevy.)
- **`3-Hz/pfengine`**: REJECT — license is UNVERIFIED (no LICENSE file found, 0 stars). One reason: unverified license.
- **`MattKulka/revenant`**: REJECT — license is UNVERIFIED (no LICENSE file found, 0 stars). One reason: unverified license.
- **`genxium/DelayNoMore`**: REJECT — license is UNVERIFIED (no LICENSE file found, 162 stars). One reason: unverified license.
- **`benny-n/netcode`** is already a TAKE (Card 1) — not a rejection.
- **`lowquark/uflow`** is already a TAKE (Card 2) — not a rejection.
- **`ncallaway/bevy_prototype_networking_laminar`**: REJECT — MIT license but archived prototype (last push 2024-07). One reason: archived prototype; no active maintenance.
- **`CabbitStudios/bevy_spicy_networking`**: REJECT — MIT license but Bevy-specific; engine2040 does not use Bevy. One reason: Bevy-bound; engine uses std sockets, not Bevy.
- **`pond3r/ggpo`**: REJECT — MIT license but C++ (not a Rust crate). One reason: C++ source; engine requires Rust implementation. (Pattern reference only.)
- **`codename-B/roomielib`**: REJECT — MIT license but 0 stars and no traction. One reason: too small/unproven; no game-specific netcode pattern identified.
- **`ccp-project/portus`**: REJECT — ISC license but a congestion control plane (SIGCOMM '18), not a game netcode library. One reason: wrong domain — network congestion control, not game rollback netcode.
- **`n0-computer/patchbay`**: REJECT — Apache-2.0 license but Linux-namespace-based network simulation, not a game netcode library. One reason: wrong domain — network simulation infrastructure, not game netcode.
- **`smokku/bevy_networking_turbulence`**: REJECT — archived. One reason: archived; no active maintenance.
- **`Ragnyll/ngs`**: REJECT — no license found. One reason: unverified license.
- **`Risgrynsgrot/rustNetGame`**: REJECT — no license found. One reason: unverified license.
- **`Ohkthx/lijk-rs`**: REJECT — no license found. One reason: unverified license.
- **`maryamali27/multiplayer-fps`**: REJECT — no license found. One reason: unverified license.

---

## Counterevidence summary

1. **Engine already has a mature netcode baseline**: The engine already has UDP LAN, prediction + resim ≤8 ticks, snapshot deltas, dedicated server, lobby, replay, lag compensation, heal, and QUIC transport. The donors add pattern references, not missing functionality (Cards 1, 2, 3).
2. **Bevy-specific libraries don't apply**: `bevy_spicy_networking`, `bevy_ggrs`, `bevy_simple_networking`, and `bevy_prototype_networking_laminar` are all Bevy-bound. Engine2040 uses std sockets and its own ECS — no Bevy dependency.
3. **Most small repos are unproven**: `benny-n/netcode` (27★), `lowquark/uflow` (32★), and `genxium/DelayNoMore` have minimal traction. The engine should not depend on unproven libraries. Pattern references are safe; dependency risks are not.
4. **MPL-2.0 and AGPL-3.0 are blocked**: `hexgate` (MPL-2.0) and `days` (AGPL-3.0) cannot be used under the deny.toml allow-list. These are legitimate netcode concepts but the licenses are not permissive.
5. **UNVERIFIED licenses are rejected**: `netsim`, `pfengine`, `revenant`, and `DelayNoMore` have no license file. Under the strict license-first policy, these cannot be taken.
6. **The paper is a pattern reference only**: CoGrid & MUG describes a Python framework for psychological experiments. The rollback netcode architecture is transferable as a design pattern, but the implementation is Python and not game-specific.

---

## What does NOT transfer (explicit)

- All Bevy-specific libraries (`bevy_spicy_networking`, `bevy_ggrs`, `bevy_simple_networking`, `bevy_prototype_networking_laminar`, `smokku/bevy_networking_turbulence`) — engine2040 does not use Bevy
- All C++ source (`pond3r/ggpo`) — the engine requires Rust implementations
- All UNVERIFIED-license repos (`netsim`, `pfengine`, `revenant`, `DelayNoMore`, `Ragnyll/ngs`, `Risgrynsgrot/rustNetGame`, `Ohkthx/lijk-rs`, `maryamali27/multiplayer-fps`) — license-first policy
- All MPL-2.0 and AGPL-3.0 repos (`hexgate`, `days`) — not on deny.toml allow-list
- All "Other" license repos (`bevy_ggrs`, `bevy_simple_networking`) — not on allow-list
- All archived repos (`bevy_prototype_networking_laminar`, `smokku/bevy_networking_turbulence`) — no active maintenance
- All congestion control planes (`portus`, `patchbay`) — wrong domain (network infrastructure, not game netcode)
- All Python implementations (`CoGrid`, `MUG`) — pattern reference only, not code
- Any paper claim that requires live model access or external data in the tick
- Any transport dependency that is not std-only or on the allow-list
- Any Bevy plugin, metrics crate, or async runtime in the tick loop

---

## Integration homes

- **Subsystem plan**: `docs/plan/multiplayer.md` (PLN6, prefix MP), `docs/plan/online.md` (PLN16, prefix ON)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 02 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 02 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows MP8, ON1, ON4)
- **HASH-LAW**: `docs/HASH-LAW.md`
- **Focus**: `docs/FOCUS.md`
- **Multiplayer plan**: `docs/plan/multiplayer.md` §5 (Donors), §7 (Board tasks)
- **Online plan**: `docs/plan/online.md` §5 (Donors), §7 (Board tasks)

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1–3 are adopted, write `docs/research/steal-wave39.md` as the persisted note (this file). Propose MP8, ON1, ON4 as board rows via the orchestrator. Cards 1–3 are TAKE with REPRODUCE stamp. Rejections carry one reason each.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| MP8 (`benny-n/netcode`, REPRODUCE) | Validates the engine's netcode protocol against the industry standard — supports "we ship our own games" (EMPIRE.md §1) by ensuring the multiplayer layer is built on proven rollback netcode patterns. Enables reliable LAN multiplayer. |
| ON1 (`lowquark/uflow`, REPRODUCE) | Adds ordered reliable delivery alongside unreliable snapshots — supports "we ship our own games" (EMPIRE.md §1) and the online plan (ON rows) by ensuring lobby, party, and cheat-report traffic arrives in order while snapshots stay fast. Enables internet multiplayer via QUIC. |
| ON4 (arXiv:2604.15044v1, REPRODUCE) | Validates rollback netcode for large-scale multi-agent scenarios — supports "we ship our own games" (EMPIRE.md §1) and "AI makes whole games" (EMPIRE.md §4) by ensuring that 64-fighter sieges with AI legions converge reliably under real network conditions. |
| `alexdesander/hexgate` (REJECT) | License (MPL-2.0) blocks adoption — maintains license-first discipline per `deny.toml`. No EMPIRE impact. |
| `iQua/days` (REJECT) | License (AGPL-3.0) blocks adoption — maintains license-first discipline per `deny.toml`. No EMPIRE impact. |

---

## Source traceability

- **GitHub search**: `gh search repos "rollback netcode Rust"`, `gh search repos "GGPO"`, `gh search repos "netcode simulation deterministic"`, `gh search repos "QUIC multiplayer"`, `gh search repos "interest management game server"`, `gh search repos "lag compensation rollback prediction"`, `gh search repos "network simulation Rust"`, `gh search repos "fixed point simulation rollback"`, `gh search repos "replication game" --language rust`, `gh search repos "game server framework" --language rust`
- **GitHub API**: All repos verified via `gh repo view <owner>/<repo> --json licenseInfo` and `webfetch` of `https://raw.githubusercontent.com/<owner>/<repo>/<branch>/LICENSE`
- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=all:rollback+AND+all:netcode&max_results=10` → arXiv:2604.15044v1 (CoGrid & MUG); `https://export.arxiv.org/api/query?search_query=all:interest+OR+all:management+AND+all:networked&max_results=5` → no relevant game netcode papers
- **Cross-reference**: DEEP-DIVES.md §02 (dive 02 row), `docs/plan/multiplayer.md` (PLN6), `docs/plan/online.md` (PLN16)
- **EMPIRE.md**: §1 (we ship our own games), §4 (AI makes whole games), Race section (forge + engine2040 both live forever)
- **Donor dedup**: Wave 26 donors (`georust/rstar`, `cooper-tretter/bq-guard`, `dagryazev/bqcost`) excluded; Wave 35 donors (Pichlmair, FootsiesGym, Gajardo) excluded; Wave 36 donors excluded; Wave 37 donors (`madsim-rs/madsim`, `rust-lang/rustc-hash`, `copper-project/copper-rs`, `turbofish-org/orga`, `eldruin/wyhash-rs`) excluded; Wave 38 donors (`AlexanderNyr/AuraLite-Physics-Engine`, `sudokatie/vortex`, `agroudiev/dynamics`, `jim-ec/constraint_solver`) excluded; Plan-file donors (`mas-bandwidth/netcode`, `cBournhonesque/lightyear`, `HouraiTeahouse/backroll-rs`, `gschup/ggrs`, `lucaspoffo/renet`, `quinn-rs/quinn`, `rustls/rustls`, `ValveSoftware/GameNetworkingSockets`, `johanhelsing/matchbox`, `simgine/bevy_replicon`, `Xiph Opus`) excluded from new-take consideration.
- **License allow-list**: `deny.toml` [licenses].allow = [MIT, Apache-2.0, Zlib, BSD-3-Clause, BSD-2-Clause, ISC, Unicode-DFS-2016, Unicode-3.0, CC0-1.0, Unlicense, 0BSD, MIT-0]
