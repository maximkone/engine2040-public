# Owner queue 2026-10-20 — only Maxim can decide or do these

Each line is one decision or one human-only run. Nothing here can be landed by an agent alone.

## Board shape (decide first)
- The board has **702 open rows** and 88 of them carried a dated status line. An agent session can land a handful per pass; the rest is a backlog. Question: do you want the old shards (SPRINT-01 Rome-first, SPRINT-02 done, SPRINT-09 empire, the 2026-10-02 "res" rows) archived into `docs/SPRINT-ARCHIVE.md` so the open board shows only live work, or kept as-is?
- `docs/plan/2026-10-17-obsolete-rows.md` asks you to close E4 E6 E12 E17 E21 E22 E26 plus the 25 the E-triage added. Say yes and they go to the archive.

## Wording and API decisions (agents must not guess)
- NS31c1: the Accept says the coated material record has "strictly more bindings", but coat adds no texture slot. Keep that line or change it to what the code does?
- SP50: the Accept names digest `4aae94af`, which came from a JS spike and cannot be reproduced in Rust. Replace the digest with a reproducible one, or drop the numeric Accept?
- GP33 contradicts the tier ladder (it asks for a quality the web tier cannot hold). Which one wins?
- ARC13: report-only receipt instead of the automatic revert the row names?
- API2 leftovers: `missing_docs` in 38 crates, a zero-warning `cargo doc` leg and the doctest leg — scope them or drop them?
- Brotli vs gzip for the web budget: page bytes are gzip today (`build.json` gzipBytes); switch the budget to brotli or keep gzip?
- Deleted net code (`replicate.rs`, `server.rs`, `steam_relay.rs`, `nat.rs`) stays deleted until a caller exists — confirm, or name the caller you want.

## Human-only proofs (two people, two machines, hands)
- Two humans on real keyboards, one LAN: `arena.exe --scene arena --fighters 2 --spear --ticks 100000 --host` and `arena.exe --scene arena --join HOST:7043 CODE` (or `play-arena-host.bat` / `play-arena-join.bat HOST:7043 CODE`). Closes ON30 / EG44 / MP25 / MP26 honesty.
- Two real networks: run `net_relay` on a reachable host, then `on30-serve` and `on30-join` from two different networks, and keep the two logs.
- SG3 needs one hand-played tow: `arena.exe --scene arena --fighters 2 --spear --play` and land a throw (Q aims at the nearest rival) with the window open; the wind-up capture and the release binary check are already in the row.
- The reference-GPU fills that need a quiet desktop, not a laptop session: PF19 and PQ18's reference leg.

## Installs and downloads (agent stops here by rule)
- GS2: `cargo install cargo-llvm-cov` (or a prebuilt release) so `target/reports/wiring-cov.json` can hold a real run instead of the canned fixture. Say yes and it is a five-minute job next session.
