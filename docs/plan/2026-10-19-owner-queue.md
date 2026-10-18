# Owner queue 2026-10-19

Rows and decisions only Maxim can close. One line each: the exact command or
the exact question. Everything else on the board is agent work.

## Human / machine runs

- ON30 (docs/SPRINT-11-north-star.md): two humans + 6 AI over two real
  networks. Host: `target\release\net_relay.exe` on a reachable host, then
  `target\release\arena.exe --scene arena --fighters 8 --spear --host`; guest:
  `target\release\arena.exe --scene arena --join HOST:7043 CODE`. Paste the
  loss/latency lines and both seat hashes into the row.
- SG3 (docs/SPRINT-08-hard-stuff.md): one hand-played keyboard run that lands
  a tow. `play-arena.bat`, then hold W and press Q once at the rival; keep the
  log line `arena spear: thrown=... hits=... tethers=1` and paste it here.
- OPT31 (docs/SPRINT-12-tier-2040.md): reference-GPU frame report over the fixed
  arena camera path, x2 within 5%.
  `node tools/frame-report.mjs --frames 600 --out target/evidence/build/` (or the
  OPT2 harness line in the row), then paste adapter + driver + both medians.
- OPT33 (docs/SPRINT-12-tier-2040.md): DX12 vs Vulkan A/B on the reference GPU
  with the OPT2 harness, 600 frames, x2 within 5%; paste both reports under
  `target/evidence/backend/` and say which backend becomes the default.
- CT45 (docs/SPRINT-07-accepted-plan-rows.md): measured VRAM of the 213 MiB
  bound set on the reference GPU. The report table is already generated
  (f2effc78); only the measured VRAM column and the reference adapter name are
  missing.

## Owner decisions

- NS31c1 (docs/SPRINT-01-rome-first.md): the Accept line says the coated
  material record has "strictly more bindings", but coat adds no texture slot.
  Keep the wording or change it to what the code does?
- Close the 32 obsolete rows in `docs/plan/2026-10-17-obsolete-rows.md`
  (E4 E6 E12 E17 E21 E22 E26 E5 E8 E11 E18 E19 E20 E24 E25 E28 E33 E34 E35
  E36 E41 E45 E47 E53 E54 E60 E83 E84 E85 E88 E89 E94): tick each with
  `Landed obsolete.` or delete it. E54's premise is wrong (the root package
  keeps those deps on purpose) and E22/E54 have no replacement.
- Deleted net code stays deleted until a caller exists: `replicate.rs`,
  `server.rs` (BX9), `steam_relay.rs`, `nat.rs`; lag compensation is a hold
  window, not a view rewind. Confirm, or name the caller that should come back.
- Brotli or gzip for the web budget (`crates/web` budget leg)?
- SP50 Accept wording: the digest `4aae94af` came from a JS spike and cannot be
  reproduced in Rust. Reword the Accept or drop the digest?
- GP33 contradicts the tier ladder (docs/SPRINT-12-tier-2040.md). Which one
  wins?
- API2 leftovers: `missing_docs` in 38 crates, a zero-warning `cargo doc` and
  the doctest leg. Which of the three is the bar?
- ARC13: report-only receipt instead of auto-revert? Confirm.
- 14 ticked rows whose code is gone (listed in
  `docs/plan/2026-10-18-deleted-code-triage.md`, "Ticks with no code"):
  untick them, or accept them as closed by replacement?
