# 0003 Netcode path for the duel

- Status: Proposed
- Context: The live path is server-authoritative in `crates/net/src/session.rs` (Client predicts, Server never rolls back). `crates/net/src/rollback.rs` (`P2pSession`, `MAX_ROLLBACK` 128) has no caller outside `net` tests. A third model in `crates/net/src/netcode.rs` (`MAX_ROLLBACK` 8) feeds only soak receipts. See `docs/plan/BOOK-GAPS-2026-10-12.md:187-197`.
- Decision: Propose the lead picks one model for the duel (keep server-authoritative with client resim and retire `rollback.rs`, or wire `P2pSession` and retire the server path), then fixes the rollback cap and makes the soak test the chosen path. The architect picks no model.
- Consequences: One played path. Dead code retired. No hash change until wired.
- Compliance: none yet.
- Advice: Lead, via inbox `--ref ADR-0003 --by architect`.
