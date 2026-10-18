STEAL CARD: Oteryn/Oteryn-Game (BLOCK 10, position 10, GitHub topic game-engine?l=rust&o=desc&s=updated). Read live 2026-10-18, read-only. No bytes vendored.
Donor head read: 94288ef41a3dece278b30eedf82b6c2d0ea7bc09 (branch main).

What it is: the canonical native-Rust game repository for a Tibia-like MMO (migrated from blakinio/Oteryn-v2): authoritative multichannel game server, native client, its own gameplay protocol and native world/content model. It is server/platform-first, not a renderer: the README is a set of ADRs (identity/gateway/admission, PostgreSQL ownership, world format + studio, dual TCP-default/QUIC-opt-in transport, recovery capacity). Workspace: protocol-oteryn (wire codecs), session-tcp, session, world-bundle, simulation-determinism, ntp-clock, input-actions, renderer, client-runtime, identity, diagnostics, foundation, error-codes and more. 0 stars. Early; acceptance does not imply implementation (their own status model).

Licence: MPL-2.0 (read live 2026-10-18: LICENSE = "Mozilla Public License Version 2.0"; GitHub-detected SPDX: MPL-2.0). Creative assets are reserved separately (LICENSE-ASSETS.md) — code only. File-level copyleft: adapt into our own files, keep notices on copied files.

Engine2040 anchors it lands on:
- HASH-LAW (golden d57b5c8ae3096614) and crates/arena/src/sync.rs (SystemHashes).
- crates/net wire: delta.rs / snapshot.rs / versioning (WIRE_VERSION); HS2.
- crates/game input contract (recorded inputs) and the lobby/session wiring in crates/net.

=== STEALS (ranked by value per hour) ===

1. A named determinism profile + domain-separated state hash over bounded canonical state | Oteryn/Oteryn-Game@94288ef:crates/simulation-determinism/src/lib.rs:12-19 (DECISION_DOMAIN_V1 / STATE_HASH_DOMAIN_V1 domain separators; explicit bounds MAX_CANONICAL_STATE_ENTRIES 4096, KEY_BYTES 128, VALUE_BYTES 16384, MAX_FIXED_SCALE_DECIMALS 18) and :87-92 (ACTIVE_PROFILE names the derivation and hash ids) | MPL-2.0 | SIM (proof) | lands docs/HASH-LAW.md text + crates/arena/src/sync.rs | S-M
   Idea: a versioned profile (revision + profile_id) names *which* decision derivation and *which* state hash are in force, and the hash is domain-separated (a fixed prefix byte string per purpose) over a canonical, size-bounded key/value state; SHA-256 so it is specified across toolchains. Unknown versions are errors, not silent behaviour change.
   Maps: engine2040 has one golden hash and a scope doc. Steal the *named* profile + domain separation + explicit bounds: every hashed quantity gets a documented domain, a version, and a maximum size, so a future hash change is a new named version and not an accidental golden move. Cheap, doc-only first step.
   Kill rule: kill if adding a profile id ever changes the current golden digest (it must be a rename of the existing derivation, not a re-hash), or if a bound is set below a real arena quantity (then it becomes a silent failure).

2. Bounded frame length-prefixing + numeric error codes + capability negotiation on the wire | Oteryn/Oteryn-Game@94288ef:crates/protocol-oteryn/src/lib.rs:1-11 (FND-02 wire boundary: bounded frame length-prefixing, envelope types with encode/decode, ALPN id, typed per-domain payload codecs) and :18-40 (FoundationProtocolError with reserved numeric codes 1001..1041: MalformedFrame, FrameTooLarge, ProtocolMajorMismatch, TransportProfileMismatch, CapabilityMismatch, ...) | MPL-2.0 | SIM-adjacent (wire) | lands crates/net versioning (WIRE_VERSION) + reconnect paths | M
   Idea: every frame carries a bounded length prefix; a mismatch is a *typed numeric* error the peer can branch on (protocol-major, transport-profile, capability), not a string; a rejected frame changes nothing. Capabilities are negotiated, and a payload codec is per domain (spatial vs object) so a new domain does not bump the whole protocol.
   Maps: engine2040 has WIRE_VERSION and delta/snapshot codecs. Steal the discipline: a numeric code per refusal kind (so a client can tell "you are old" from "you sent junk" without text), bounded lengths on every read, and per-domain codecs so a siege/social feature does not renumber the duel wire.
   Kill rule: kill if a code duplicates an existing WIRE_VERSION refusal or if any length bound is not enforced before allocation (that reintroduces an unbounded read).

3. Fail-closed synchronized clock read carrying its own error bound | Oteryn/Oteryn-Game@94288ef:crates/ntp-clock/src/lib.rs:1-25 (one read-only adjtimex; returns clock *and* max error in microseconds; TIME_ERROR / STA_UNSYNC / bad reading / non-Linux -> None; nothing cached; the crate is the workspace's only unsafe) | MPL-2.0 | SIM-adjacent (clock, off-tick) | lands crates/net session start / WAN ping + crates/net/src/tick_budget.rs | S
   Idea: ask the kernel for the realtime clock and its maximum error together, and treat "not synchronized" as an honest None rather than a wrong time; isolate the single unsafe FFI call in one crate so everything else forbids unsafe. Consumers decide whether an error bound is good enough.
   Maps: engine2040 measures RTT/Ping in crates/net and paces off the wall clock. Steal the "clock + error bound, fail closed" API shape for the session start (and any future NTP on the door), and the one-crate unsafe containment as a pattern for a single platform read.
   Kill rule: kill it if the platform is not Linux (this read is Linux-only; Windows needs a different primitive) or if any sim code reads it.

4. Semantic input ids with a validated charset and bounded chords | Oteryn/Oteryn-Game@94288ef:crates/input-actions/src/semantic.rs:6-9 (MAX_IDENTIFIER_BYTES 64, MAX_CHORD_INPUTS 4) and :10-48 (validate_identifier: non-empty, length-bounded, ASCII alphanumeric or `. _ - : /`; ActionId/ContextId are validated newtypes) | MPL-2.0 | PRESENTATION/GAME (input config) | lands crates/game input contract + content | S
   Idea: an action is a validated semantic id (charset and length pinned) and a chord is at most four inputs; identifiers are framework-neutral, so config files and the wire carry the same validated token. Bad config is a typed error at parse time.
   Maps: engine2040 binds actions in the window/contract. Steal the validated-id newtype and the chord cap so a rebind GUI, a config file and a replay agree on one canonical spelling (folds into the Byte-Engine action-card idea, steal 3 there).
   Kill rule: kill if a valid existing binding string fails the charset (then widen the charset first) or if the validation runs inside the fixed tick.

=== NOT WORTH STEALING ===
- The renderer, client-runtime and world-bundle crates: the project is a 2D tile MMO server stack; nothing in a 3D fighter's frame path.
- Protocol payload codecs themselves (spatial/object/quest/spell wire): they are game-specific schemas; take the framing rules (steal 2), not the payloads.

DONE 4 steals, 6 donor files read.
