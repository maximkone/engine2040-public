# STAGES - where the game is, and what blocks the next stage

Producer seat (`planner-producer`). One line per run lives in `burndown.tsv`.
This file is the stage table, the top 3 risks and the Rule 4 unblock note.
Base: board slice `node tools/board-slice.mjs --focus --open` = 60 lines
(59 rows + 1 summary line) read again at 2026-10-17T01:10Z, run
planner-producer-r391 (previous run r390, 00:56Z, read the same 59 row ids).
History: the r1-r377 per-run logs were compressed to this shape on 2026-10-16
by r378 so the file stays readable; their full text is in git history
(`git log -p -- docs/design/STAGES.md`).
r391 (2026-10-17T01:10Z) re-read the same slice (59 rows + 1 summary line, the
same 59 row ids). Item 2 FIRED this run: the nearest-earlier line is r390
(00:56Z) - open 60, ready 19, landed 2, overruns 0, returned 0, failed 83,
parked 139 -> this run 60 / 17 / 3 / 0 / 0 / 83 / 139, landed the largest move
at +50.0% (2 -> 3), past the 20% bar - so the stage table, the buffers and the
stage receipts were recomputed and repasted below. Ready also moved -10.5%
(19 -> 17, under the bar) and kept moving while this run was reading: the dir
held 19 files by 01:12Z, so that one field is a moving snapshot and each
number below carries the time it was read. The three landings in the 24 h
window are `0-farm-clippy-nn-20261016` -> d3d21b43 `crates/nn/src/qat.rs`
(08:58Z), `perf-1945z` (11:55Z) and `builder-tester-r317` -> cb20c28c
(2026-10-17T01:03Z, the new one); none is a FOCUS stage row. The receipts read
this run: `node tools/finish.mjs` re-run at 01:09Z (its two warnings and its
summary are pasted in Preproduction), the product report on disk (still
`target/reports/product/latest.json` 2026-10-13T09:21:48Z, 88 h old), the
newest look report `target/reports/look-b46768e0-20261017T0102Z.txt`
(2026-10-17T01:03:28Z, the ARENA frame-split pasted in Vertical Slice) and the
newest G9 receipt `potato-2026-10-17.md` (2026-10-17T00:33:50Z, unchanged since
r389, pasted in Release candidate).

## Stage table

Recomputed r396 from the 59 open FOCUS rows; the live list
`node tools/board-slice.mjs --focus --open` prints `59 row(s)` at 02:16Z and
splits exactly as r385-r395 did: 0 + 11 + 16 + 17 + 0 + 15 = 59 (re-read row
by row this run, same row ids, same stage split, so every number in the table
below is re-derived and unchanged). Buffer = ceil(15% or 25% of the stage's open
rows). A stage never passes without its exit test. Bars never lower.

| Stage | Exit test (one command) | Open FOCUS rows | Buffer | passed |
|---|---|---|---|---|
| Preproduction | `node tools/finish.mjs` | 0 | n/a (no rows) | no |
| First Playable (G1) | `node tools/product-check.mjs` | 11 | 3 (25% of 11) | no |
| Vertical Slice | `node tools/product-check.mjs` | 16 | 4 (25% of 16) | no |
| Alpha | `node tools/proof.mjs` | 17 | 5 (25% of 17) | no |
| Beta | `node tools/product-check.mjs` | 0 | n/a (no rows) | no |
| Release candidate | `node tools/potato-check.mjs` | 15 | 3 (15% of 15) | no |

Buffer recalibration r396: no stage has landed a row against its buffer yet, so
no buffer moves this run. The landings in the 24 h window
(`Select-String target/loop-keeper.log -Pattern ' LANDED '` filtered to
timestamped lines at or after 2026-10-16T02:15Z = 4) are `0-farm-clippy-nn-20261016`
-> d3d21b43 `crates/nn/src/qat.rs` at 08:58Z, `perf-1945z` at 11:55Z (a
ratchet-foreign WARN "engine-render: hot_alloc 128 -> 133" that landed anyway),
`builder-tester-r317` -> cb20c28c at 2026-10-17T01:03Z (a ratchet-foreign WARN
"engine-endpoint: big_files 3 -> 4" that landed anyway) and
`builder-design-systems-r28` -> d0e5dc17 `docs/design/balance-log.md` at
2026-10-17T02:10Z (clean); none is a FOCUS stage row, so no `passed:` field
moves. The newest BLOCKERS row is still 2026-10-17T01:20Z (EG29 link-env x2 on
`rust-lld: error: undefined symbol: anon.f0549b...`, `cargo check -p
engine-game --lib` green) and the 2026-10-17T00:55Z row (engine-perf lib and
seal family red x2 on the arena hash move, `fingerprint: perf-lib-arena-hash-x2`,
`cargo test -p engine-perf --lib` 114 passed 7 failed) still blocks two FOCUS
rows (GS14, EG41) - both are hash-law and queue questions filed by other seats,
not landing receipts, so the estimate half of Sellers ch12 still gets no
matching pair.
Estimate ratio printed for the record: 59 open FOCUS rows / 4 packet landings
in 24 h = 14.8 open rows per landing (19.7 last run). Next run re-derives the
buffers from the same commands.

Item 2 trigger check r391 (2026-10-17T01:10Z): the table IS recomputed this run.
There is still no line seven days back (the file starts 2026-10-12T06:33Z,
4.9 days back, so the first true 7-day comparison is possible from 2026-10-19),
so the comparison falls back to the nearest earlier line (r390, 00:56Z):
open 60 -> 60, ready 19 -> 17 (-10.5%), landed 2 -> 3 (+50.0%, over the bar),
overruns 0 -> 0, returned 0 -> 0, failed 83 -> 83, parked 139 -> 139.
The live list was read row by row this run
(`node tools/board-slice.mjs --focus --open`, 59 rows + 1 summary line) and it
carries the same 59 row ids: First Playable GP27 GP31 GP37 VS1 EG18 EG45 LF10
LF11 LF14 LF17 EG19b; Vertical Slice FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4
EG13 EG14 RL21 FC3 LF16 SP61 SP64; Alpha HS2 HS29 HS30 HS31 HS20 DP1 EG41 EG21
EG17 OPT24 OPT27 GA8 EG25 EG27 EG29 MP24 ON30; Release candidate the 15 GS rows
(GS2 GS4 GS5 GS6 GS10 GS12 GS13 GS14 GS17 GS18 GS19 GS24 GS25 GS30 GS32); 0 rows
sit in Preproduction and Beta - so the 0 + 11 + 16 + 17 + 0 + 15 = 59 split, the
buffers and the six `passed: no` fields below stand (no exit test passes this
run: the Preproduction print is still not clean, the G1/G6 receipts are still
absent or stale, and the G9 receipt is still a loaded run).

Item 2 trigger check r393 (2026-10-17T01:37Z): the stage table above is NOT
recomputed this run. No line seven days back exists yet (the file's oldest line
is 2026-10-12T06:33Z, 4.8 days back; the first true 7-day comparison is possible
from 2026-10-19), so the comparison falls back to the nearest earlier line (r392,
2026-10-17T01:21Z, written by claim `planner-producer-r392`; that run wrote its
burndown line and no STAGES.md section, so the table and the six stage sections
below are still r391's): open 60 -> 60 (0%), ready 17 -> 16 (-5.9%), landed
3 -> 3 (0%), overruns 0 -> 0, returned 0 -> 0, failed 83 -> 83 (0%), parked
140 -> 144 (+2.9%). Nothing moves 20%, so the stage table, the buffers and the
six `passed:` fields stand unchanged from r391; the live FOCUS set was re-read
row by row this run (`node tools/board-slice.mjs --focus --open` = 59 rows + 1
summary line at 01:36Z) and carries the same 59 row ids (the board re-grouped
them; the set is identical), so the 0 + 11 + 16 + 17 + 0 + 15 = 59 split still
holds. No exit test was re-run this run (no stage changed), so no `passed:` field
moves and no bar is lowered.

Item 2 trigger check r394 (2026-10-17T01:49Z): the stage table above is NOT
recomputed this run. No line seven days back exists yet (the file's oldest line
is 2026-10-12T06:33Z, 4.8 days back; the first true 7-day comparison is possible
from 2026-10-19), so the comparison falls back to the nearest earlier line (r393,
2026-10-17T01:37Z, written by claim `planner-producer-r393`): open 60 -> 60
(0%), ready 16 -> 17 (+6.3%), landed 3 -> 3 (0%), overruns 0 -> 0, returned
0 -> 0, failed 83 -> 83 (0%), parked 144 -> 144 (0%). Nothing moves 20%, so the
stage table, the buffers and the six `passed:` fields stand unchanged from r391.
The live FOCUS set was re-read row by row this run (`node tools/board-slice.mjs
--focus --open` = 59 rows + 1 summary line at 01:48Z) and carries the same 59 row
ids, so the 0 + 11 + 16 + 17 + 0 + 15 = 59 split still holds. Two stage receipts
did move on disk since r391 - the newest look report is now
`look-316ee0b5-20261017T0149Z.txt` (01:49Z) and the G9 receipt
`potato-2026-10-17.md` is re-stamped 01:23:54Z - so sections 3 and 6 below are
re-pasted with those fresh reads to stay true; no exit test was re-run and no
`passed:` field moves (the G9 receipt is still stamped LOADED), so no bar is
lowered.

Item 2 trigger check r395 (2026-10-17T02:01Z): the stage table above is NOT
recomputed this run. No line seven days back exists yet (the file's oldest line
is 2026-10-12T06:33Z, 4.8 days back; the first true 7-day comparison is possible
from 2026-10-19), so the comparison falls back to the nearest earlier line (r394,
2026-10-17T01:49Z, written by claim `planner-producer-r394`): open 60 -> 60
(0%), ready 17 -> 17 (0%), landed 3 -> 3 (0%), overruns 0 -> 0, returned 0 -> 0,
failed 83 -> 83 (0%), parked 144 -> 144 (0%). Nothing moves 20%, so the stage
table, the buffers and the six `passed:` fields stand unchanged from r391. The
live FOCUS set was re-read row by row this run (`node tools/board-slice.mjs
--focus --open` = 60 lines = 59 rows + 1 summary at 02:01Z) and carries the same
59 row ids: First Playable GP27 GP31 GP37 VS1 EG18 EG45 LF10 LF11 LF14 LF17
EG19b; Vertical Slice FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 RL21 FC3
LF16 SP61 SP64; Alpha HS2 HS29 HS30 HS31 HS20 DP1 EG41 EG21 EG17 OPT24 OPT27 GA8
EG25 EG27 EG29 MP24 ON30; Release candidate the 15 GS rows (GS2 GS4 GS5 GS6 GS10
GS12 GS13 GS14 GS17 GS18 GS19 GS24 GS25 GS30 GS32); 0 rows in Preproduction and
Beta - so the 0 + 11 + 16 + 17 + 0 + 15 = 59 split, the buffers and the six
`passed: no` fields below stand. One stage receipt moved on disk and was
re-pasted to stay true - the newest look report is now
`look-316ee0b5-20261017T0157Z.txt` (written 2026-10-17T01:57:44Z, present
53.738 ms, cpu p99 418.142) - but no exit test was re-run this run, so no
`passed:` field moves and no bar is lowered.

Item 2 trigger check r396 (2026-10-17T02:15Z): the stage table above IS
recomputed this run. No line seven days back exists yet (the file's oldest line
is 2026-10-12T06:33Z, 4.9 days back; the first true 7-day comparison is possible
from 2026-10-19), so the comparison falls back to the nearest earlier line (r395,
2026-10-17T02:01Z, written by claim `planner-producer-r395`): open 60 -> 60
(0%), ready 17 -> 16 (-5.9%), landed 3 -> 4 (+33.3%, past the 20% bar - the new
landing is `builder-design-systems-r28` -> d0e5dc17 `docs/design/balance-log.md`
at 02:10Z), overruns 0 -> 0, returned 0 -> 0, failed 83 -> 83 (0%), parked
144 -> 144 (0%). The live FOCUS set was re-read row by row this run
(`node tools/board-slice.mjs --focus --open` = 60 lines = 59 rows + 1 summary at
02:16Z) and carries the same 59 row ids: First Playable GP27 GP31 GP37 VS1 EG18
EG45 LF10 LF11 LF14 LF17 EG19b; Vertical Slice FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3
LF4 EG13 EG14 RL21 FC3 LF16 SP61 SP64; Alpha HS2 HS29 HS30 HS31 HS20 DP1 EG41 EG21
EG17 OPT24 OPT27 GA8 EG25 EG27 EG29 MP24 ON30; Release candidate the 15 GS rows
(GS2 GS4 GS5 GS6 GS10 GS12 GS13 GS14 GS17 GS18 GS19 GS24 GS25 GS30 GS32); 0 rows
in Preproduction and Beta - so the 0 + 11 + 16 + 17 + 0 + 15 = 59 split, the
buffers and the six `passed:` fields below re-derive unchanged. Two stage receipts
moved on disk and are re-pasted to stay true - the newest look report is now
`look-5a7631c1-20261017T0205Z.txt` (written 2026-10-17T02:05:48Z, present 74.905
ms, cpu p99 623.748) and the G9 receipt `potato-2026-10-17.md` is unchanged at
01:23:54Z - and the Preproduction exit test `node tools/finish.mjs` was re-run at
02:16Z with the same result. No `passed:` field moves and no bar is lowered.

Item 2 trigger check r397 (2026-10-17T02:31Z, run `planner-producer-r397`): the
stage table above is NOT recomputed this run. No line seven days back exists yet
(the file's oldest line is 2026-10-12T06:33Z, 4.9 days back; the first true 7-day
comparison is possible from 2026-10-19), so the comparison falls back to the
nearest earlier line (r396, 2026-10-17T02:15Z, claim `planner-producer-r396`):
open 60 -> 60 (0%), ready 16 -> 14 (-12.5%), landed 4 -> 4 (0%), overruns 0 -> 0,
returned 0 -> 0, failed 83 -> 83 (0%), parked 144 -> 144 (0%). Nothing moves 20%,
so the stage table, the buffers and the six `passed:` fields stand unchanged from
r391. The live FOCUS set was re-read this run (`node tools/board-slice.mjs --focus
--open` = 60 lines = 59 rows + 1 summary line `59 row(s)` at 02:31Z) and carries
the same 59 row ids: First Playable GP27 GP31 GP37 VS1 EG18 EG45 LF10 LF11 LF14
LF17 EG19b; Vertical Slice FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 RL21
FC3 LF16 SP61 SP64; Alpha HS2 HS29 HS30 HS31 HS20 DP1 EG41 EG21 EG17 OPT24 OPT27
GA8 EG25 EG27 EG29 MP24 ON30; Release candidate the 15 GS rows (GS2 GS4 GS5 GS6
GS10 GS12 GS13 GS14 GS17 GS18 GS19 GS24 GS25 GS30 GS32); 0 rows in Preproduction
and Beta - so the 0 + 11 + 16 + 17 + 0 + 15 = 59 split, the buffers and the six
`passed: no` fields below stand. No stage receipt moved on disk since r396: the
newest look report is still `look-5a7631c1-20261017T0205Z.txt` (written
2026-10-17T02:05:48Z), the product report is still
`target/reports/product/latest.json` (2026-10-13T09:21:48Z) and the G9 receipt
`potato-2026-10-17.md` is still stamped 2026-10-17T01:23:54Z - so no section is
re-pasted, no `passed:` field moves and no bar is lowered. `node
tools/workspace-check.mjs` read 0 FAIL / exit 0 at 02:36Z (re-run after these
edits).

Item 2 trigger check r398 (2026-10-17T02:47Z, run `planner-producer-r398`): the
stage table above is NOT recomputed this run. No line seven days back exists yet
(the file's oldest line is 2026-10-12T06:33Z, 4.9 days back; the first true 7-day
comparison is possible from 2026-10-19), so the comparison falls back to the
nearest earlier line (r397, 2026-10-17T02:31Z, claim `planner-producer-r397`):
open 60 -> 60 (0%), ready 14 -> 16 (+14.3%), landed 4 -> 4 (0%), overruns 0 -> 0,
returned 0 -> 0, failed 83 -> 83 (0%), parked 144 -> 145 (+0.7%, the new park is
the duplicate queue copy `bug-arena-staged-fight-receipt-red-20261017.md` at
02:38:58Z, see Risks item 2). Nothing moves 20%, so the stage table, the buffers
and the six `passed:` fields stand unchanged from r391. The live FOCUS set was
re-read this run (`node tools/board-slice.mjs --focus --open` = 60 lines = 59
rows + 1 summary line `59 row(s)` at 02:44-02:47Z); the 59 ids were re-extracted
and compared as a set against the r397 list this run and match it exactly - the
same 0 + 11 + 16 + 17 + 0 + 15 = 59 split (First Playable GP27 GP31 GP37 VS1
EG18 EG45 LF10 LF11 LF14 LF17 EG19b; Vertical Slice FC1 EG8 SP63 LF1 LF2 EG11
EG12 LF3 LF4 EG13 EG14 RL21 FC3 LF16 SP61 SP64; Alpha HS2 HS29 HS30 HS31 HS20
DP1 EG41 EG21 EG17 OPT24 OPT27 GA8 EG25 EG27 EG29 MP24 ON30; Release candidate
the 15 GS rows GS2 GS4 GS5 GS6 GS10 GS12 GS13 GS14 GS17 GS18 GS19 GS24 GS25 GS30
GS32; 0 rows in Preproduction and Beta). Two stage receipts moved on disk and
sections 3 and 6 below are re-pasted with the fresh reads to stay true - the
newest look report is now `look-c0623232-20261017T0241Z.txt` (written
2026-10-17T02:41:41Z, present 59.736 ms) and the G9 receipt `potato-2026-10-17.md`
is re-stamped 2026-10-17T02:44:48.983Z (floor WITHHELD, box LOADED 22 cargo + 2
rustc) - but no exit test passes, so no `passed:` field moves and no bar is
lowered.

Item 2 trigger check r399 (2026-10-17T03:01Z, run `planner-producer-r399`): the
stage table above is NOT recomputed this run. No line seven days back exists yet
(the file's oldest line is 2026-10-12T06:33Z, 4.9 days back; the first true 7-day
comparison is possible from 2026-10-19), so the comparison falls back to the
nearest earlier line (r398, 2026-10-17T02:47Z, claim `planner-producer-r398`):
open 60 -> 60 (0%), ready 16 -> 15 (-6.3%), landed 4 -> 4 (0%), overruns 0 -> 0,
returned 0 -> 0, failed 83 -> 83 (0%), parked 145 -> 145 (0%). Nothing moves 20%,
so the stage table, the buffers and the six `passed:` fields stand unchanged from
r391. The live FOCUS set was re-read this run (`node tools/board-slice.mjs --focus
--open` = 60 lines = 59 rows + 1 summary line `59 row(s)` at 03:01Z) and its 59 ids
were checked id by id against the r398 list: the same set, so the same
0 + 11 + 16 + 17 + 0 + 15 = 59 split (First Playable GP27 GP31 GP37 VS1 EG18 EG45
LF10 LF11 LF14 LF17 EG19b; Vertical Slice FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4
EG13 EG14 RL21 FC3 LF16 SP61 SP64; Alpha HS2 HS29 HS30 HS31 HS20 DP1 EG41 EG21 EG17
OPT24 OPT27 GA8 EG25 EG27 EG29 MP24 ON30; Release candidate the 15 GS rows GS2 GS4
GS5 GS6 GS10 GS12 GS13 GS14 GS17 GS18 GS19 GS24 GS25 GS30 GS32; 0 rows in
Preproduction and Beta). No stage receipt moved on disk since r398: the newest look
report is still `look-c0623232-20261017T0241Z.txt` (written 2026-10-17T02:41:41Z,
present 59.736 ms), the product report is still
`target/reports/product/latest.json` (2026-10-13T09:21:48Z, 90 h old) and the G9
receipt `potato-2026-10-17.md` is still stamped 2026-10-17T02:44:48Z (`floor:
WITHHELD` on a loaded box) - so no section is re-pasted, no `passed:` field moves
and no bar is lowered.

Item 2 trigger check r400 (2026-10-17T03:14Z, run `planner-producer-r400`): the
stage table above is NOT recomputed this run. No line seven days back exists yet
(the file's oldest line is 2026-10-12T06:33Z, 4.9 days back; the first true 7-day
comparison is possible from 2026-10-19), so the comparison falls back to the
nearest earlier line (r399, 2026-10-17T03:00Z, claim `planner-producer-r399`):
open 60 -> 60 (0%), ready 15 -> 17 (+13.3%), landed 4 -> 4 (0%), overruns 0 -> 0,
returned 0 -> 0, failed 83 -> 83 (0%), parked 145 -> 145 (0%). Nothing moves 20%,
so the stage table, the buffers and the six `passed:` fields stand unchanged from
r391. The live FOCUS set was re-read this run (`node tools/board-slice.mjs --focus
--open` = 60 lines = 59 rows + 1 summary line `59 row(s)` at 03:15Z) and its 59 ids
match the r399 list id by id: the same 0 + 11 + 16 + 17 + 0 + 15 = 59 split (First
Playable GP27 GP31 GP37 VS1 EG18 EG45 LF10 LF11 LF14 LF17 EG19b; Vertical Slice FC1
EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 RL21 FC3 LF16 SP61 SP64; Alpha HS2 HS29
HS30 HS31 HS20 DP1 EG41 EG21 EG17 OPT24 OPT27 GA8 EG25 EG27 EG29 MP24 ON30; Release
candidate the 15 GS rows GS2 GS4 GS5 GS6 GS10 GS12 GS13 GS14 GS17 GS18 GS19 GS24 GS25
GS30 GS32; 0 rows in Preproduction and Beta). One stage receipt moved on disk and
section 3 below is re-pasted with the fresh read to stay true - the newest look
report is now `look-44fd1088-20261017T0312Z.txt` (written 2026-10-17T03:12:40Z,
present 66.057 ms, cpu p99 595.382) - but the product report is still
`target/reports/product/latest.json` (2026-10-13T09:21:48Z) and the G9 receipt
`potato-2026-10-17.md` is still stamped 2026-10-17T02:44:48Z, so no exit test
passes, no `passed:` field moves and no bar is lowered.

Item 2 trigger check r401 (2026-10-17T03:29Z, run `planner-producer-r401`): the
stage table above is NOT recomputed this run. No line seven days back exists yet
(the file's oldest line is 2026-10-12T06:33Z, 4.9 days back; the first true 7-day
comparison is possible from 2026-10-19), so the comparison falls back to the
nearest earlier line (r400, 2026-10-17T03:14Z, claim `planner-producer-r400`):
open 60 -> 60 (0%), ready 17 -> 17 (0%), landed 4 -> 4 (0%), overruns 0 -> 0,
returned 0 -> 0, failed 83 -> 83 (0%), parked 145 -> 146 (+0.7%, the new park is
`builder-fixer-r145.md` at 03:19:47Z, an unlandable PASS whose own work is already
on master `ab4937e1` - see Risks item 3). Nothing moves 20%, so the stage table,
the buffers and the six `passed:` fields stand unchanged from r391. The live FOCUS
set was re-read this run (`node tools/board-slice.mjs --focus --open` = 60 lines =
59 rows + 1 summary line `59 row(s)` at 03:28Z) and its 59 ids match the r400 list
id by id: the same 0 + 11 + 16 + 17 + 0 + 15 = 59 split (First Playable GP27 GP31
GP37 VS1 EG18 EG45 LF10 LF11 LF14 LF17 EG19b; Vertical Slice FC1 EG8 SP63 LF1 LF2
EG11 EG12 LF3 LF4 EG13 EG14 RL21 FC3 LF16 SP61 SP64; Alpha HS2 HS29 HS30 HS31 HS20
DP1 EG41 EG21 EG17 OPT24 OPT27 GA8 EG25 EG27 EG29 MP24 ON30; Release candidate the
15 GS rows GS2 GS4 GS5 GS6 GS10 GS12 GS13 GS14 GS17 GS18 GS19 GS24 GS25 GS30 GS32;
0 rows in Preproduction and Beta). No stage receipt moved on disk since r400: the
newest look report is still `look-44fd1088-20261017T0312Z.txt` (written
2026-10-17T03:12:40Z), the product report is still
`target/reports/product/latest.json` (2026-10-13T09:21:48Z) and the G9 receipt
`potato-2026-10-17.md` is still stamped 2026-10-17T02:44:48Z - so no section is
re-pasted, no `passed:` field moves and no bar is lowered.

Item 2 trigger check r402 (2026-10-17T03:54Z, run `planner-producer-r402`): the
stage table above is NOT recomputed this run. No line seven days back exists yet
(the file's oldest line is 2026-10-12T06:33Z, 4.9 days back; the first true 7-day
comparison is possible from 2026-10-19), so the comparison falls back to the
nearest earlier line (r401, 2026-10-17T03:29Z, claim `planner-producer-r401`):
open 60 -> 60 (0%), ready 17 -> 19 (+11.8%), landed 4 -> 4 (0%), overruns 0 -> 0,
returned 0 -> 0, failed 83 -> 83 (0%), parked 146 -> 147 (+0.7%). Nothing moves
20%, so the stage table, the buffers and the six `passed:` fields stand unchanged
from r396. The live FOCUS set was re-read this run
(`node tools/board-slice.mjs --focus --open` = 60 lines = 59 rows + 1 summary line
`59 row(s)` at 03:54Z): First Playable GP27 GP31 GP37 VS1 EG18 EG45 LF10 LF11 LF14
LF17 EG19b; Vertical Slice FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 RL21
FC3 LF16 SP61 SP64; Alpha HS2 HS29 HS30 HS31 HS20 DP1 EG41 EG21 EG17 OPT24 OPT27
GA8 EG25 EG27 EG29 MP24 ON30; Release candidate the 15 GS rows GS2 GS4 GS5 GS6 GS10
GS12 GS13 GS14 GS17 GS18 GS19 GS24 GS25 GS30 GS32; 0 rows in Preproduction and Beta
- so the 0 + 11 + 16 + 17 + 0 + 15 = 59 split, the buffers and the six
`passed: no` fields below stand. No stage receipt moved on disk since r401: the
newest look report is still `look-44fd1088-20261017T0312Z.txt` (written
2026-10-17T03:12:40Z), the product report is still
`target/reports/product/latest.json` (2026-10-13T09:21:48Z) and the G9 receipt
`potato-2026-10-17.md` is still stamped 2026-10-17T02:44:48Z (`floor: WITHHELD`
on a loaded box) - so no stage section is re-pasted, no `passed:` field moves and
no bar is lowered. `node tools/workspace-check.mjs` read 0 FAIL lines / exit 0
after these edits.

Item 2 trigger check r403 (2026-10-17T04:16Z, run `planner-producer-r403`): the
stage table above is NOT recomputed this run. No line seven days back exists yet
(the file's oldest line is 2026-10-12T06:33Z, 4.9 days back; the first true 7-day
comparison is possible from 2026-10-19), so the comparison falls back to the
nearest earlier line (r402, 2026-10-17T03:54Z, claim `planner-producer-r402`):
open 60 -> 60 (0%), ready 19 -> 16 (-15.8%), landed 4 -> 4 (0%), overruns 0 -> 0,
returned 0 -> 0, failed 83 -> 83 (0%), parked 147 -> 153 (+4.1%). Nothing moves
20%, so the stage table, the buffers and the six `passed:` fields stand unchanged
from r396. The live FOCUS set was re-read this run
(`node tools/board-slice.mjs --focus --open` = 60 lines = 59 rows + 1 summary line
`59 row(s)` at 04:17Z) and its 59 ids match the r402 list id by id: the same
0 + 11 + 16 + 17 + 0 + 15 = 59 split (First Playable GP27 GP31 GP37 VS1 EG18 EG45
LF10 LF11 LF14 LF17 EG19b; Vertical Slice FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4
EG13 EG14 RL21 FC3 LF16 SP61 SP64; Alpha HS2 HS29 HS30 HS31 HS20 DP1 EG41 EG21
EG17 OPT24 OPT27 GA8 EG25 EG27 EG29 MP24 ON30; Release candidate the 15 GS rows
GS2 GS4 GS5 GS6 GS10 GS12 GS13 GS14 GS17 GS18 GS19 GS24 GS25 GS30 GS32; 0 rows in
Preproduction and Beta). Two stage receipts moved on disk and are re-pasted below
to stay true - the newest look report is now
`look-631e7192-20261017T0412Z.txt` (written 2026-10-17T04:13:25Z, present 73.778
ms, cpu p99 500.527) and the G9 receipt `potato-2026-10-17.md` is re-stamped
2026-10-17T03:53:09Z (`floor: judged`, three legs PASS, box LOADED 36 cargo + 1
rustc) - but no exit test passes, so no `passed:` field moves and no bar is
lowered. `node tools/workspace-check.mjs` read 0 FAIL lines / exit 0 after these
edits.

Item 2 trigger check r404 (2026-10-17T04:27Z, run `planner-producer-r404`): the
stage table above is NOT recomputed this run. No line seven days back exists yet
(the file's oldest line is 2026-10-12T06:33Z, 4.9 days back; the first true 7-day
comparison is possible from 2026-10-19), so the comparison falls back to the
nearest earlier line (r403, 2026-10-17T04:16Z, claim `planner-producer-r403`):
open 60 -> 60 (0%), ready 16 -> 15 (-6.3%), landed 4 -> 4 (0%), overruns 0 -> 0,
returned 0 -> 0, failed 83 -> 83 (0%), parked 153 -> 152 (-0.7%). Nothing moves
20%, so the stage table, the buffers and the six `passed:` fields stand unchanged
from r396. The live FOCUS set was re-read this run
(`node tools/board-slice.mjs --focus --open` = 60 lines = 59 rows + 1 summary line
`59 row(s)` at 04:28Z) and carries the same 59 row ids as r403: First Playable
GP27 GP31 GP37 VS1 EG18 EG45 LF10 LF11 LF14 LF17 EG19b; Vertical Slice FC1 EG8
SP63 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 RL21 FC3 LF16 SP61 SP64; Alpha HS2 HS29
HS30 HS31 HS20 DP1 EG41 EG21 EG17 OPT24 OPT27 GA8 EG25 EG27 EG29 MP24 ON30;
Release candidate the 15 GS rows (GS2 GS4 GS5 GS6 GS10 GS12 GS13 GS14 GS17 GS18
GS19 GS24 GS25 GS30 GS32); 0 rows in Preproduction and Beta - so the
0 + 11 + 16 + 17 + 0 + 15 = 59 split, the buffers and the six `passed: no` fields
below stand. One stage receipt moved on disk: the newest look report is now
`look-631e7192-20261017T0427Z.txt` (written 2026-10-17T04:27:54Z, present
48.305 ms, cpu p99 233.724, framing frames=300 empty=0 fewest_in_view=2) - while
the product report is still `target/reports/product/latest.json`
(2026-10-13T09:21:48Z) and the G9 receipt `potato-2026-10-17.md` is still stamped
2026-10-17T03:53:09Z - so no exit test passes, no `passed:` field moves and no bar
is lowered. The stage sections below are left as r403/r404 read them (they are
re-pasted only when the stage table is recomputed); this run's fresh look read is
quoted here instead.

## 1. Preproduction - passed: no

Scope and plans exist (FOCUS "## Now", finish-line G1-G9, FINISH-RICH A/B/C/P).
Exit test: `node tools/finish.mjs`, which must print every gate with no fallback
warning.
Open FOCUS rows that belong here: 0. Buffer: none.
Still closed: r396 re-ran the exit test (`node tools/finish.mjs`, 02:16Z):
`A1: met`, `A2: met`, `A3: met`, `A4: partial 10/12`, `A5: partial
8/10`, `A6: open 8/8`, `A7: open 4/4`, `A8: open 1/1`, `A9: partial 7/8`, `A10:
partial 4/8`, the B/C/P tables and `Summary: 4 met, 10 partial, 12 open`, plus
the same two warnings: `finish: warning: fallback rows used for A2, A3, A4, A5,
A6, A7, A9, A10, C1, C2, C3, C4, C5` and `finish: warning: dropped rows filtered
(A4:GX5, C1:GX5, P1:GX5)`, so the open set is still stale and the exit test is
not clean. The tool names its own fix: `update FALLBACK_ROWS in tools/finish.mjs`
and `update DROPPED_ROWS in tools/finish.mjs if the board changed`.

## 2. First Playable (the duel plays start to end, product check green: G1) - passed: no

Exit test: `node tools/product-check.mjs` GREEN (finish-line.md:12, G1).
Open FOCUS rows that belong here: 11 (Game: GP27 GP31 GP37; fight feel: VS1
EG18 EG45 LF10 LF11 LF14 LF17; combat cue: EG19b).
Buffer: 3 (25% of 11; design-iteration work).
Still closed: the newest product report on disk is
`target/reports/product/latest.json` (mtime 2026-10-13T09:21:48Z, unchanged
across r385-r396, 89 h old, verdict GREEN), so no report inside 24 h proves the
current tree - all 59 open FOCUS rows sit on a tree with no fresh G1 print.
r380 showed `node tools/product-check.mjs` did not finish inside 180 s (timed
out); a second identical failure ends a step, so it was not retried by r381-r396
either, and G1 stays unproven at HEAD, not just ungated.

## 3. Vertical Slice (one bout at near-final quality) - passed: no

Look, feel and frame time are the baselines every later bout is held to.
Exit: product check green (`node tools/product-check.mjs`), plus the art review
score and the ARENA frame-split line pasted beside it.
Open FOCUS rows that belong here: 16 (Look/render: FC1 EG8 SP63 LF1 LF2 EG11
EG12 LF3 LF4 EG13 EG14 RL21 FC3 LF16; view truth: SP61 SP64).
Buffer: 4 (25% of 16; research-heavy render work).
Still closed: the art review score is still absent, and the product check leg
above has no report inside 24 h. The ARENA frame-split line is re-pasted here
fresh this run from the newest look report on disk
(`target/reports/look-631e7192-20261017T0412Z.txt`, written 2026-10-17T04:13:25Z;
RTX 3050 6GB Laptop GPU driver 32.0.16.1062, 1280x720, present_mode Immediate):
`arena frame-split: sim=0.141ms present=73.778ms fighters=7.878ms
fighters_pose=2.542ms fighters_upload=5.217ms per frame (mean over 300 frames)
cpu_ms p50=11.040 p99=500.527 interval_ms p50=11.278 p99=432.355 gpu_ms=none
acquire=0.457ms present_mode=Immediate`,
`arena framing: frames=300 empty=0 first_empty=- fewest_in_view=2
steepest_down_deg=18.4 tinted_share=0.04`, per-pass `sky=71.823ms layers=1.011ms
particles=0.765ms decals=4.549ms ibl=25.411ms fog=13.705ms gtao=11.518ms
shadow=0.040ms forward=0.000ms validation_errors=0`. That is a real print, not a
baseline: present 73.778 ms is about 14 fps at 1280x720 (worse than the
66.057 ms r402 pasted, still far from 30 fps), cpu p99 500.527 ms is still
a spike, and the two near-zero passes (shadow 0.040 ms, forward 0.000 ms) are
exactly the EG11/EG12 rows still open, so the slice baseline is far from
near-final. The window does draw frames, so the `ARENA forward pipeline crash`
row (BLOCKERS 2026-10-15T23:02Z, arena.exe exit 102 x3) does not cover the
current live frame; the newest BLOCKERS row is 2026-10-17T01:20Z (EG29
link-env x2, see Risks).

## 4. Alpha (feature complete for the slice scope) - passed: no

Scope lock: a new feature names the row it cuts.
Exit test: `node tools/proof.mjs` (golden x2; the slice replays exactly, G6).
Open FOCUS rows that belong here: 17 (Bodies/workbench: HS2 HS29 HS30 HS31
HS20; AI door: DP1 EG41 EG21 EG17 OPT24 OPT27 GA8; flow/content: EG25 EG27
EG29; net: MP24 ON30).
Buffer: 5 (25% of 17; determinism and benchmark research).
Still closed: G6 and G8 stay open, and the shipped app keeps the sim off by the
owner ruling 2026-10-12 (inbox line 14; EB-2026-10-13-S1 and EB-2026-10-16-S3
both DECLINED on it), so sim-body rows cannot land in the shipped app. HS29 and
HS31 also sit behind the playtest link-env wall (newest such row
2026-10-16T12:15Z, flat again this run). EG41 gained a second wall this run: the
new BLOCKERS row 2026-10-17T00:55Z holds its P2P leg (`cargo test -p engine-perf
--lib` red, 7 of 121) until an arena owner says which side of the arena hash
move is right.

## 5. Beta (content and UI done; only fixes, stability and speed land) - passed: no

Exit test: `node tools/product-check.mjs` with no fix, stability or speed row
left on the slice scope.
Open FOCUS rows that belong here: 0 (content and UI are not done, so no row is
in fix-only state yet). Buffer: none.
Still closed: the HUD and flow rows (EG14, EG25) still sit in Vertical Slice
and Alpha work, so the stage has no rows to burn and cannot be entered.

## 6. Release candidate (the release checklist) - passed: no

Checklist: potato gate G9, settings, crash report, store captures.
Exit test: `node tools/potato-check.mjs` (finish-line.md:20, G9, GS16).
Open FOCUS rows that belong here: 15 (Potato tier and steals: GS2 GS4 GS5 GS6
GS10 GS12 GS13 GS14 GS17 GS18 GS19 GS24 GS25 GS30 GS32).
Buffer: 3 (15% of 15; checklist work).
Still closed: `tools/potato-check.mjs` exists on disk (with `tools/finish.mjs`,
`tools/product-check.mjs` and `tools/proof.mjs`). The newest G9 receipt on disk is
`target/reports/potato-2026-10-17.md` (re-stamped 2026-10-17T03:53:09.830Z, read
fresh this run, not produced by this seat): adapter `Intel(R)_UHD_Graphics`
(E2040_ADAPTER=integrated E2040_LIMITS=downlevel), `p50_ms 7.855 <= 50 PASS`,
`p99_ms 22.296 <= 80 PASS`, `peak_ws_mb 1517 <= 3072 PASS` - the three legs have
been PASS since 00:12:30Z (the 23:27:53Z receipt failed p99 at 136.719), and both
latency numbers improved on the 02:44:48Z receipt (10.745 / 75.588). The receipt
itself now prints `floor: judged` and
`load: LOADED 36 cargo + 1 rustc running (filed numbers, not a speed claim)`; the
GS16 slice landed at 02:45Z (claim `speed-ds-v41-potato-floor-20261017`, its F2P
`node --test tools/potato-check.test.mjs` 26 pass 0 fail) gives the `--floor` leg
the withholder behaviour (exit 3 on a loaded box), so it is the `judged` stamp on a
still-loaded box - not a quiet-tree run - that keeps this stage honest. The field
stays `passed: no`: the stage needs one rerun on a quiet tree plus the three
other checklist items (settings, crash report, store captures), which are still
absent. GS14 also sits behind the BLOCKERS row 2026-10-17T00:55Z on the perf lib
hash. The 15 own rows stay open either way.

## Risks (top 3, ranked by blocked rows) - producer run 2026-10-17T04:27Z planner-producer-r404

Sources read this run: `docs/BLOCKERS.md` (109 table rows by the same
`^\| 20` count as r403; the 4 newest timestamps read), the first lines of the 3
newest of 152 parked files (`0-farm-quick-render-ratchet-20261016.md`,
`0-farm-test-20261016.md` and `0-farm-quick-ratchet-20261016.md`, all 03:57:44Z)
and the 3 oldest parked records (`harvest-render-v1-tonemap.md`
2026-10-10T15:10:35Z, `cut-ai-ladder-writers.md` 2026-10-10T18:22:17Z,
`reds-202610110135-d87c8fbb.md` 2026-10-11T07:37:19Z). The 3 oldest ready files
this run: `0-farm-export-sprite-pack-20261017.md` (319 min,
`row: bug:farm-export-sprite-pack-20261017`),
`0-lf4-bloom-grade-close-20261017.md` (296 min, `row: LF4`) and
`perf-fc3-live-present-wall-20261016T2320Z.md` (295 min, `row: FC3`). Ranked by
blocked FOCUS rows, one line each in the shape
`risk | owner seat | done-when command`, with the receipt under it.

1. link-env link wall | builder | `(Select-String docs/BLOCKERS.md -Pattern 'link-env|will not link|E0689|E0106|undefined symbol|rust-lld|link fail|link red' | Measure-Object).Count`
   prints under 30 (this run 30 of 109, flat since r393), after one named link
   packet's F2P command exits 0. It is the top risk again by blocked rows.
   Evidence: 30 of the 109 table rows match that pattern and 28 of them carry no
   `Resolved <date>:` marker (the 2026-10-14T10:05Z and 2026-10-14T20:45Z rows do,
   both resolved 2026-10-14T22:45Z after a shared-target clean). The newest
   BLOCKERS row is still not a link row - it is 2026-10-17T03:23Z (endpoint lib
   suite red x2 on the foreign door-budget test: the view-pack verb `look.judge`
   pushes a loaded session to 44241 > 43514 chars, first error
   `crates/endpoint/src/packs/mod.rs:1202`; blocks the crate-wide P2P of AX27 and
   HS21, neither an open FOCUS row). The newest link record is still
   2026-10-17T01:20Z (EG29 window-reload close, `rust-lld: error: undefined symbol:
   anon.f0549b8fca8791f...` from libengine_render-*.rlib, cargo check green). The
   28 live rows name 14 open FOCUS rows by the seat's rule unchanged since r401:
   EG29 SP63 EG12 VS1 LF16 EG21 LF1 LF14 GA8 LF10 LF11 HS29 HS31 HS30 (HS30 counts
   on its own live 2026-10-15T14:09Z row; HS20 does not, because the only row that
   names it, 2026-10-14T20:30Z, is the tally row whose own remedy says "same as
   10:05Z row" and the 2026-10-14T10:05Z fingerprint carries a Resolved marker).
   Two of the 28 rows are victim rollups, not one-row reports - 2026-10-16T01:35Z
   lists victims gs10/gs18/gs5/eg12/gs12/eg8/rl21 and 2026-10-16T10:35Z lists
   eg18/eg45/lf16/sp64 - so a raw id match over the row text reads 25; the 14 above
   is the conservative read and no bar is lowered. A green check proves the
   toolchain env, never a code bug, so the remedy is unchanged: one bug packet with
   the fingerprint line, then `cargo clean -p engine-render` and an off-peak
   re-verify on a quiet tree, oldest record first (SP63 10-14T21:44Z, VS1
   10-15T12:34Z, LF16 12:39Z, EG12 02:47Z/13:25Z, EG21 13:59Z/22:04Z, LF1 14:13Z,
   HS30 14:09Z, GA8/LF10/LF11 10-15T21:44Z, HS29/HS31 10-16T12:15Z, EG29
   10-17T01:20Z).

2. Rule 4 holds | keeper | a `paths:` prefix compare of `target/queue/ready`
   against `target/queue/running` prints `rule4_fires=0` (this run `ready=15
   running=11 rule4_fires=15 rule4_fires_over30min=9`; a ready path fires when it
   equals, or is a path prefix of, a running packet's path).
   Evidence: declared-path prefix compare of `target/queue/ready` (15 files at
   04:28Z, all carrying `paths:`) against `target/queue/running` (11 files in the
   same read, all carrying `paths:`): every ready packet is held and none is free,
   fourth run in a row. Both dirs drained since r403 (ready 16 -> 15, running
   18 -> 11); the holding is by the packets that are still running:
   `0-lf1-sky-live-close.md`, `0-rl52-live-particles-draw-20261017.md`,
   `0-rl43-hdr-lightmap-20261017.md`,
   `bug-art-palette-washout-44fd1088.md` (render side, holding the `crates/render`
   farm carrier), `0-gp42-debug-camera-20261017.md`,
   `bug-art-crowd-empty-b46768e0.md`, `perf-fc1-pose-half-20261017T0345Z.md`,
   `0-hs30-mirror-craft-close-20261017.md`, `bug-arena-fightevent-kind-20261017.md`,
   `0-fv3-eye-camera-feed-20261017.md` and `0-eg17-pinned-tls-close-20261017.md`.
   Blocked FOCUS rows: 2 over the 30 min bar -
   `0-lf4-bloom-grade-close-20261017.md` (`row: LF4`, 296 min) and
   `perf-fc3-live-present-wall-20261016T2320Z.md` (`row: FC3`, 295 min); six more
   held packets name a FOCUS row but sit under the bar
   (`0-eg11-sun-depth-live-close.md` EG11 21 min,
   `perf-fighter-upload-20261017T0215Z.md` 21 min,
   `0-vs1-arena-kit-close.md` VS1 20 min,
   `0-eg45-balance-look-close-20261017.md` EG45 17 min,
   `0-hs31-mirror-logic-close-20261017.md` HS31 17 min, and the FC2b carrier at
   111 min, which is not in the 59-row set). The other `row:` values are `bug:*`
   (`bug:farm-export-sprite-pack-20261017`, `bug:arena-live-gpu-map-wait`), `new`
   (`perf-draw-bind-scan-20261016T2159Z.md`, `level-zones-reader-game-20261017.md`),
   `feel-hit-tint` and `perf:arena-frame-tail-20261017`. Remedy: sequence each held
   packet after its running holder; this is a queue-order and holder-duration
   problem, not capacity - `running` (11) is now below `ready` (15), so the queue
   drained by dispatch, not by landing.
   Snapshot note: the queue moved while this run was writing (16 -> 15 ready
   between 04:16Z and 04:28Z), so the counted basis is the 04:28Z read and the
   Unblock list below carries it.

3. land refused + ratchet wall | keeper (land side) + rot-hunter (ratchet debt) |
   `(Get-ChildItem target/queue/parked -File | Measure-Object).Count` prints
   under 140 (this run 152) and `node tools/rust-quality.mjs`
   prints PASS with the crates named below back at their accepted baseline.
   Evidence: 23 of the 109 rows name ratchet or refused landings
   (`(Select-String docs/BLOCKERS.md -Pattern 'ratchet|refused|REFUSED|big-file|big_files' | Measure-Object).Count`
   = 23, flat since r399); the newest such row is still 2026-10-16T22:15Z (render
   leftovers fog.rs + shadow_cluster.rs + v1_skin.rs hook-refused alone and as a
   trio x2). Parked fell 153 -> 152 (-1) and 35 parked files carry a REFUSED or
   `land.mjs exit` marker anywhere in the file (the same read r403 used; a
   first-line-only shortcut reads 27). The ratchet is still RED and GREW one
   regression, 3 -> 4: `node tools/rust-quality.mjs` read this run prints
   `WORKSPACE 266.8 93 A` and `RESULT FAIL: 4 ratchet regression(s)` (exit 1) -
   `engine-game: casts 129 -> 132` (numeric `as` casts),
   `engine-game: print_in_lib 0 -> 1` (new since r403: a print!/dbg! in library
   code), `engine-game: dup_fns 1 -> 2` and `engine-render: big_files 7 -> 8`
   (files over 1500 production lines), against
   `[INFO] 17 metric(s) improved below baseline`. The `engine-game: print_in_lib`
   and `dup_fns` hits sit in the in-flight `crates/game/src/fighter_look.rs` work
   (`perf-fc1-pose-half`, `perf-fighter-upload`, `0-feel-hit-tint`); the `casts`
   and `big_files` hits are the same debt r403 named; the repair path is unchanged:
   fold the duplicated body, name the cast conversions, drop the print out of
   library code and fold the big file, or move the baseline by rule - never
   `--accept`. The 3 oldest ready files are the same debt in farm form:
   `0-farm-export-sprite-pack-20261017.md` 319 min `paths: crates/render`,
   `0-lf4-bloom-grade-close-20261017.md` 296 min and
   `perf-fc3-live-present-wall-20261016T2320Z.md` 295 min. Blocked FOCUS rows: 2 -
   the hash-law standoff row (2026-10-17T00:55Z, `cargo test -p engine-perf --lib`
   114 passed 7 failed, `fingerprint: perf-lib-arena-hash-x2`) holds GS14 and EG41
   P2P until an arena owner says which side of the arena hash move is right; the
   03:23Z endpoint door-budget row blocks AX27 and HS21, neither an open FOCUS row.
   Remedy: fold the duplicated body, name the cast conversions, drop the print from
   library code, fold the big file back under 1500 lines and fold the duplicate
   farm carriers before the next land; never `--accept`.


## Unblock (Rule 4) - r404

Rule 4 FIRES on all 15 of the 15 ready packets, 9 of them older than 30 min
(ready 15 files at 04:28Z, running 11 files in the same read, all 15 and 11
carrying a `paths:` line). Wrote nothing; the 9 older than 30 min are listed here
and in Risks item 2. Count rule, stated so it can be re-run: a ready packet fires
when one of its `paths:` entries equals, or is a path prefix of, one of a running
packet's `paths:` entries; a `paths:` line is split on commas and whitespace. Read
over both dirs at 2026-10-17T04:28Z (prefix compare, file mtime for age):
`ready=15 running=11 rule4_fires=15 rule4_fires_over30min=9`. Six further
packets fire but sit under the 30 min bar; they are named at the end of this
section, not listed.

1. `0-farm-export-sprite-pack-20261017.md` age 319 min, `paths: crates/render`;
   held by the 4 running render packets: `0-lf1-sky-live-close.md`
   (`sky.rs`/`present/gpu_present/live.rs`/`shaders/forward.wgsl`),
   `0-rl43-hdr-lightmap-20261017.md` (`gi.rs`/`baked_gi.rs`/`baked_bake.rs`),
   `0-rl52-live-particles-draw-20261017.md` (`particles_gpu.rs`/`submit.rs`) and
   `bug-art-palette-washout-44fd1088.md` (`shaders/tonemap.wgsl`).
   `row: bug:farm-export-sprite-pack-20261017`.
2. `0-lf4-bloom-grade-close-20261017.md` age 296 min, `paths:
   crates/render/src/present/gpu_present/live.rs,
   crates/render/src/shaders/blit.wgsl, crates/render/src/post.rs`; 1 holder,
   `0-lf1-sky-live-close.md` (`present/gpu_present/live.rs`); `blit.wgsl` and
   `post.rs` are free. `row: LF4` - the oldest ready packet that holds an open
   FOCUS row, because the farm carriers carry `row: bug:*` or `row: new`.
3. `perf-fc3-live-present-wall-20261016T2320Z.md` age 295 min, `paths:
   crates/render/src/present/gpu_present/live.rs,
   crates/render/src/present/gpu_present/frame.rs`; 1 holder,
   `0-lf1-sky-live-close.md` (`present/gpu_present/live.rs`);
   `frame.rs` is free. `row: FC3`, the second locked FOCUS row.
4. `bug-arena-live-gpu-map-wait-20261016T2324Z.md` age 257 min, `paths:
   crates/render/src/present/gpu_present/live.rs,
   crates/render/src/present/gpu_present/submit.rs,
   crates/render/src/present/gpu_present/fog_tranche.rs`; 2 holders,
   `0-rl52-live-particles-draw-20261017.md` (`submit.rs`) and
   `0-lf1-sky-live-close.md` (`live.rs`); `fog_tranche.rs` is free.
   `row: bug:arena-live-gpu-map-wait`.
5. `perf-draw-bind-scan-20261016T2159Z.md` age 159 min, `paths:
   crates/render/src/present/gpu_present/submit.rs,
   crates/render/src/present/gpu_present/live.rs`; the same 2 holders as item 4
   (`0-rl52-live-particles-draw` on `submit.rs`, `0-lf1-sky-live-close` on
   `live.rs`). `row: new`.
6. `level-zones-reader-game-20261017.md` age 129 min, `paths:
   crates/game/src/arena_kit.rs, crates/game/src/bin/arena.rs,
   crates/game/src/fight_ring.rs`; 2 holders,
   `bug-art-crowd-empty-b46768e0.md` (`arena_kit.rs`) and
   `0-gp42-debug-camera-20261017.md` (`bin/arena.rs`); `fight_ring.rs` is free.
   `row: new`.
7. `perf-fighter-skin-window-gpu-20261017T0233Z.md` age 111 min, `paths:
   crates/render/src/present/gpu_present/presenter.rs,
   crates/render/src/present/gpu_present/submit.rs, crates/game/src/fighter_look.rs`;
   2 holders, `0-rl52-live-particles-draw-20261017.md` (`submit.rs`) and
   `perf-fc1-pose-half-20261017T0345Z.md` (`fighter_look.rs`);
   `presenter.rs` is free. `row: FC2b`, not in the 59-row `--focus --open` set.
8. `0-feel-hit-tint-20261017.md` age 103 min, `paths:
   crates/game/src/fighter_look.rs, crates/render/src/present/gpu_present/`; 5
   holders, `perf-fc1-pose-half-20261017T0345Z.md` (`fighter_look.rs`),
   `0-lf1-sky-live-close.md`, `0-rl43-hdr-lightmap-20261017.md`,
   `0-rl52-live-particles-draw-20261017.md`,
   `bug-art-palette-washout-44fd1088.md`; no path is free (the directory entry
   covers the whole tree). `row: feel-hit-tint`.
9. `perf-fc1-frame-tail-20261017T0252Z.md` age 101 min, `paths:
   crates/game/src/bin/arena.rs, crates/game/src/bin/arena_frame/mod.rs`; 1 holder,
   `0-gp42-debug-camera-20261017.md` (`bin/arena.rs`); `bin/arena_frame/mod.rs` is
   free. `row: perf:arena-frame-tail-20261017`, not in the 59-row set.

Six ready packets fire but sit under the 30 min bar, so they are not listed:
`0-eg11-sun-depth-live-close.md` (21 min, `sun_shadow.rs` + `submit.rs` held by
`0-rl52-live-particles-draw-20261017.md`, `row: EG11`),
`perf-fighter-upload-20261017T0215Z.md` (21 min, `crates/game/src/fighter_look.rs`
held by `perf-fc1-pose-half-20261017T0345Z.md`), `0-vs1-arena-kit-close.md`
(20 min, `arena_kit.rs` + `bin/arena.rs` held by `bug-art-crowd-empty-b46768e0.md`
and `0-gp42-debug-camera-20261017.md`, `row: VS1`),
`0-eg45-balance-look-close-20261017.md` (17 min, `crates/game/src/fighter_look.rs`
held by `perf-fc1-pose-half-20261017T0345Z.md`, `row: EG45`),
`0-hs31-mirror-logic-close-20261017.md` (17 min, `crates/playtest/src/lib.rs` held
by `0-hs30-mirror-craft-close-20261017.md`, `row: HS31`) and
`bug-perf-ibl-column-span-20261017.md` (5 min, `present/gpu_present/live.rs` held
by `0-lf1-sky-live-close.md`). No ready packet is free this run (15 of 15 fire);
every one declares at least one path a running packet also declares.


No owner decision was filed this run, so `## Open` in `docs/EMPIRE-INBOX.md` is
unchanged (read first: `node tools/inbox-open.mjs` prints `open 12 item(s)` at
r404, and the file's own cap at line 12 is 10). Reasons: the inbox is 2 over its
cap, so a new PROD line would break it further; the owner choices that do block
stage rows are already open there - EB-2026-10-16-S16 (EG11 sun depth 1024 vs 2048
and the live-receipt gate; EG11 is a Vertical Slice FOCUS row, and its close packet
sits held in `ready` this run) and EB-2026-10-16-S17 (PROD-netcode-path, decision
D2; blocks MP24 and ON30 in Alpha), with EB-2026-10-17-01 (DIR-concept-wording)
newest. D4 (physics sim off in the shipped app) is ticked closed twice
(EB-2026-10-13-S1, EB-2026-10-16-S3); D5 (audio) stays with the lead by the
parked-audio ruling; D1 (two finish files), D3 (handles/residency) and D6
(single-threaded sim) name no stage exit test in the table above (re-read this run
at `docs/plan/BOOK-GAPS-2026-10-12.md:235-236`). The newest BLOCKERS row
(2026-10-17T03:23Z, endpoint door budget) blocks two packets but no FOCUS row, and
its remedy is an owned shrink or a dated budget comment, not a D1-D6 choice. The
two things that do move a stage - the G9 receipt stamped 2026-10-17T03:53:09Z with
three PASS legs and `floor: judged` on a loaded box (36 cargo + 1 rustc), and the
ratchet red up to 4 regressions in Risks item 3 - need a builder seat (a
quiet-tree rerun; the duplicated body folded, the cast conversions named, the print
dropped out of library code, the render big file folded back), not an owner
decision. Filed lines this run: 0, the price of the cap.


## Run log (one line per run; r404 is first and newest, then r390 to r403 oldest first)

- r404 2026-10-17T04:27Z: burndown open 60, ready 15, landed 4, overruns 0, returned 0, failed 83, parked 152; item 2 NOT fired - the nearest earlier line is r403 (04:16Z), no field moves 20% (ready 16 -> 15 = -6.3%, parked 153 -> 152 = -0.7%, the rest flat) - so the stage table, its buffers and the six `passed:` fields stand from r396 and were not re-pasted; the live FOCUS set was re-read at 04:28Z (`node tools/board-slice.mjs --focus --open` = 60 lines = 59 rows + 1 summary `59 row(s)`, the same 59 ids as r403 and the same 0+11+16+17+0+15 split); one stage receipt moved on disk and its fresh read is quoted in the r404 trigger paragraph (newest look report `look-631e7192-20261017T0427Z.txt` at 04:27:54Z, present 48.305 ms, cpu p99 233.724, framing frames=300 empty=0 fewest_in_view=2) while the product report is still `target/reports/product/latest.json` (2026-10-13T09:21:48Z) and the G9 receipt `potato-2026-10-17.md` is still 03:53:09Z, so no `passed:` field moves and no bar is lowered; risks rewritten one line each (link wall 30 of 109, flat since r393, 28 of those 30 carry no `Resolved <date>:` marker and name 14 open FOCUS rows by the r401 rule, two rows being victim rollups so a raw id match reads 25; Rule 4 holds all 15 of 15 ready, 9 over 30 min, running 11 now below ready 15, 2 blocked FOCUS rows over the bar LF4 + FC3; land-refused/ratchet wall 23 of 109 + 152 parked, 35 parked files carry a REFUSED or `land.mjs exit` marker anywhere in the file, and the ratchet red GREW 3 -> 4 - `engine-game: casts 129 -> 132`, `engine-game: print_in_lib 0 -> 1` new, `engine-game: dup_fns 1 -> 2`, `engine-render: big_files 7 -> 8`, `WORKSPACE 266.8 93 A`, `RESULT FAIL: 4 ratchet regression(s)`); unblock rewritten in place (no ready packet free, 15 of 15 fire; the 3 oldest ready are `0-farm-export-sprite-pack` 319 min, `0-lf4-bloom-grade-close` 296 min, `perf-fc3-live-present-wall` 295 min); `node tools/workspace-check.mjs` read 0 FAIL / exit 0 at 04:35Z (re-run after these edits); no inbox item filed (inbox 2 over its 10-item cap at 12 open; S16 and S17 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:235-236`, none newly blocking).
- r403 2026-10-17T04:16Z: burndown open 60, ready 16, landed 4, overruns 0, returned 0, failed 83, parked 153; item 2 NOT fired - the nearest earlier line is r402 (03:54Z), no field moves 20% (ready 19 -> 16 = -15.8%, parked 147 -> 153 = +4.1%, the rest flat) - so the stage table, its buffers and the six `passed:` fields stand from r396 and were not re-pasted; the live FOCUS set was re-read at 04:17Z (`node tools/board-slice.mjs --focus --open` = 60 lines = 59 rows + 1 summary `59 row(s)`, the same 59 ids as r402 and the same 0+11+16+17+0+15 split); two stage receipts moved on disk and their pasted lines were refreshed (newest look report `look-631e7192-20261017T0412Z.txt` at 04:13:25Z, present 73.778 ms, cpu p99 500.527; G9 `potato-2026-10-17.md` re-stamped 2026-10-17T03:53:09Z, three legs PASS, `floor: judged`, box LOADED 36 cargo + 1 rustc) but no `passed:` field moves and no bar is lowered; risks rewritten one line each (link wall 30 of 109, flat since r393, recounted 28 live link rows of 30 = 14 blocked FOCUS rows EG29 SP63 EG12 VS1 LF16 EG21 LF1 LF14 GA8 LF10 LF11 HS29 HS31 HS30 - HS30 counts on its own live 2026-10-15T14:09Z row, HS20 does not because its only link row is the tally of the resolved 10:05Z fingerprint, so this run reads 14 where r402 read 13; Rule 4 holds all 16 of 16 ready, 9 over 30 min, running 18 above ready, 2 blocked FOCUS rows over the bar (LF4, FC3); land-refused/ratchet wall 23 of 109 + 153 parked, the ratchet red SHRANK 7 -> 3 (`engine-game: casts 129 -> 132` new, `engine-game: dup_fns 1 -> 2`, `engine-render: big_files 7 -> 8`), `WORKSPACE 266.6 93 A`, `RESULT FAIL: 3 ratchet regression(s)`, and 35 parked files now carry a REFUSED or `land.mjs exit` marker, up from 27); unblock rewritten in place (no ready packet free, 16 of 16 fire; the 3 oldest ready are now `0-farm-export-sprite-pack` 304 min, `0-lf4-bloom-grade-close` 281 min, `perf-fc3-live-present-wall` 279 min, after the farm carriers were parked at 03:58Z); `node tools/workspace-check.mjs` read 0 FAIL lines / exit 0 after these edits; no inbox item filed (inbox 1 over its 10-item cap at 11 open; S16 and S17 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:233-235`, none newly blocking).
- r402 2026-10-17T03:54Z: burndown open 60, ready 19, landed 4, overruns 0, returned 0, failed 83, parked 147; item 2 NOT fired - the nearest earlier line is r401 (03:29Z), no field moves 20% (ready 17 -> 19 = +11.8%, parked 146 -> 147 = +0.7%, the rest flat) - so the stage table, its buffers and the six `passed:` fields stand from r396; the live FOCUS set was re-read at 03:54Z (`node tools/board-slice.mjs --focus --open` = 60 lines = 59 rows + 1 summary `59 row(s)`, the same 59 ids as r401 and the same 0+11+16+17+0+15 split); no stage receipt moved on disk (newest look report still `look-44fd1088-20261017T0312Z.txt` at 03:12:40Z, product report still `target/reports/product/latest.json` 2026-10-13T09:21:48Z, G9 `potato-2026-10-17.md` still 02:44:48Z), so no stage section was re-pasted and no `passed:` field moves; risks rewritten one line each (link wall 30 of 109, flat since r393, recounted over the 27 live link records of 30 = 13 blocked FOCUS rows EG29 SP63 EG12 VS1 LF16 EG21 LF1 LF14 GA8 LF10 LF11 HS29 HS31, HS20 and HS30 excluded because their only link row is the 2026-10-14T20:30Z tally of the resolved 10:05Z fingerprint; Rule 4 holds all 19 of 19 ready, 16 over 30 min at 03:54Z, and all 12 of 12 ready, 10 over 30 min at 03:58Z, running 22 -> 23 above ready, blocked FOCUS rows 4 then 2 (LF4 FC3 EG12 HS30, then LF4 FC3 once EG12 and HS30 were dispatched); land-refused/ratchet wall 23 of 109 + 147 parked, the 1 new park `0-hs29-gdb-mirror-close-20261017.md` at 03:32:35Z is a duplicate-row park not a refused land, and the ratchet red GREW 6 -> 7 - engine-game panic_sites 6 -> 7 and dup_fns 1 -> 2 plus five engine-render metrics (panic_sites 21 -> 23, casts 990 -> 1015, dead_code_allows 0 -> 1, long_fns 30 -> 31, big_files 7 -> 8), `WORKSPACE 266.4 93 A`, `RESULT FAIL: 7 ratchet regression(s)`, while engine-ai's two r401 regressions are gone from the FAIL list); unblock rewritten in place (ready 19 -> 12 within four minutes - the keeper dispatched seven held packets while this run wrote; the render holders now include `0-eg13-msaa-close`, `0-opt34-pipeline-counter-close` and `0-rl52-live-particles-draw`, which were not holders at r401, while `perf-per-draw-tobytes`, `0-rl21-twin-arena-close` and `0-eg11-sun-depth-live-close` left the running set); `node tools/workspace-check.mjs` read 0 FAIL / exit 0 after these edits; no inbox item filed (inbox 1 over its 10-item cap at 11 open; S16, S17 and S18 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:233-235`, none newly blocking).
- r390 2026-10-17T00:56Z: burndown open 60, ready 19, landed 2, overruns 0, returned 0, failed 83, parked 139; item 2 FIRED - ready moved +26.7% vs r389 (15 -> 19), past the 20% bar - so the stage table, the buffers and the stage receipts were recomputed and repasted this run onto the same live 0+11+16+17+0+15 = 59 split (`node tools/finish.mjs` re-run at 00:57Z: A1/A2/A3 met, its two fallback warnings and `Summary: 4 met, 10 partial, 12 open`; product report still 2026-10-13T09:21:48Z; ARENA frame-split present 82.579 ms at 00:54Z from `look-c9eed786-20261017T0054Z.txt`, cpu p99 644.154; G9 `potato-2026-10-17.md` 00:33:50Z p50 7.336 / p99 17.364 / peak 1508 all PASS but still stamped LOADED); no `passed:` field moved; risks rewritten one line each (link wall 29 of 126, land/ratchet wall 23 of 126 + 139 parked, Rule 4 holds 16 of 19 ready, 15 over 30 min); unblock rewritten in place; `node tools/workspace-check.mjs` read 0 FAIL / exit 0 at 00:58Z; no inbox item filed (inbox 1 over its 10-item cap at 11 open; S16 and S17 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:229-230`, none newly blocking).
- r391 2026-10-17T01:10Z: burndown open 60, ready 17, landed 3, overruns 0, returned 0, failed 83, parked 139; item 2 FIRED - landed moved +50.0% vs r390 (2 -> 3), past the 20% bar - so the stage table, the buffers and the stage receipts were recomputed and repasted this run onto the same live 0+11+16+17+0+15 = 59 split (`node tools/finish.mjs` re-run at 01:09Z: A1/A2/A3 met, its two fallback warnings and `Summary: 4 met, 10 partial, 12 open`; product report still 2026-10-13T09:21:48Z; ARENA frame-split present 67.350 ms at 01:03Z from `look-b46768e0-20261017T0102Z.txt`, cpu p99 465.855, framing frames=300 empty=0, validation_errors=0; G9 `potato-2026-10-17.md` 00:33:50Z p50 7.336 / p99 17.364 / peak 1508 all PASS but still stamped LOADED); no `passed:` field moved; the third landing is `builder-tester-r317` -> cb20c28c at 01:03Z (ratchet-foreign WARN engine-endpoint big_files 3 -> 4, landed anyway) and BLOCKERS grew a row (2026-10-17T00:55Z, engine-perf lib red on the arena hash move, blocks GS14 + EG41); risks rewritten one line each (link wall 29 of 127, land/ratchet wall 23 of 127 + 139 parked, Rule 4 holds 17 of 19 ready, 15 over 30 min); unblock rewritten in place (the r390 render holders `0-sp63-draw-id-close` and `0-lf3-gtao-close` left the running set, so `0-lf1-sky-close` and `perf-fc3-live-present-wall` each dropped to 1 holder); `node tools/workspace-check.mjs` read 0 FAIL / exit 0 at 01:13Z; no inbox item filed (inbox 1 over its 10-item cap at 11 open; S16 and S17 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:229-230`, none newly blocking).
- r393 2026-10-17T01:37Z: burndown open 60, ready 16, landed 3, overruns 0, returned 0, failed 83, parked 144; item 2 NOT fired - the nearest earlier line is r392 (01:21Z), no field moves 20% (ready 17 -> 16 = -5.9%, parked 140 -> 144 = +2.9%, the rest flat) - so the stage table, its buffers and the six `passed:` fields stand from r391 and were not re-pasted; the live FOCUS set was re-read row by row at 01:36Z (`node tools/board-slice.mjs --focus --open` = 59 rows + 1 summary line, same 59 row ids, board re-grouped); risks rewritten one line each (link wall 30 of 108 - one row grew this run, EG29 BLOCKED x2 on the render rlib undefined symbol, so the wall now holds 3 FOCUS rows; land-refused/ratchet wall 23 of 108 + 144 parked, +4 parked this run all land REFUSED; Rule 4 holds all 16 of 16 ready, 14 over 30 min); unblock rewritten in place (no ready packet is free this run; `0-ga8-eqs-bind-close` and `bug-assets-clippy-import` have left ready); `node tools/workspace-check.mjs` read 0 FAIL / exit 0 at 01:36Z; no inbox item filed (inbox 1 over its 10-item cap at 11 open; S16 and S17 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:229-231`, none newly blocking); BLOCKERS.md itself shrank 127 -> 108 table rows, rows older than 3 days archived at 01:23Z.
- r394 2026-10-17T01:49Z: burndown open 60, ready 17, landed 3, overruns 0, returned 0, failed 83, parked 144; item 2 NOT fired - the nearest earlier line is r393 (01:37Z), no field moves 20% (ready 16 -> 17 = +6.3%, the rest flat) - so the stage table, its buffers and the six `passed:` fields stand from r391; the live FOCUS set was re-read row by row at 01:48Z (`node tools/board-slice.mjs --focus --open` = 59 rows + 1 summary line, same 59 row ids, same 0+11+16+17+0+15 split); two stage receipts moved on disk and were re-pasted to stay true - the newest look report is now `look-316ee0b5-20261017T0149Z.txt` (present 48.928 ms, cpu p99 268.131, still not near-final) and the G9 receipt `potato-2026-10-17.md` re-stamped 01:23:54Z (p50 7.552 / p99 25.696 / peak 1517 all PASS but still stamped LOADED 30 cargo), so no `passed:` field moves; risks rewritten one line each (link wall 30 of 108, flat since r393; land-refused/ratchet wall 23 of 108 + 144 parked, flat; Rule 4 holds all 17 of 17 ready, 14 over 30 min, running 15 now below ready 17); unblock rewritten in place (no ready packet is free this run); `node tools/workspace-check.mjs` read 0 FAIL / exit 0 at 01:48Z; no inbox item filed (inbox over its 10-item cap at 11 open; S16 and S17 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:229-231`, none newly blocking).
- r395 2026-10-17T02:01Z: burndown open 60, ready 17, landed 3, overruns 0, returned 0, failed 83, parked 144; item 2 NOT fired - the nearest earlier line is r394 (01:49Z), no field moves 20% (all seven fields flat) - so the stage table, its buffers and the six `passed:` fields stand from r391; the live FOCUS set was re-read row by row at 02:01Z (`node tools/board-slice.mjs --focus --open` = 59 rows + 1 summary line, same 59 row ids, same 0+11+16+17+0+15 split); one stage receipt moved on disk and was re-pasted to stay true - the newest look report is now `look-316ee0b5-20261017T0157Z.txt` (present 53.738 ms, cpu p99 418.142, still not near-final) - so no `passed:` field moves; risks rewritten one line each (link wall 30 of 108, flat; land-refused/ratchet wall 23 of 108 + 144 parked, flat; Rule 4 holds 16 of 17 ready, 12 over 30 min, running 13 now well below ready 17, one free ready packet `0-gs30-isovist-close-20261017.md`); unblock rewritten in place (`0-lf1-sky-close-20261016.md` left the running set, so `0-lf3-gtao-close` and `0-cq24-post-jitter-motion-split` are the render holders); `node tools/workspace-check.mjs` read 0 FAIL / exit 0 at 02:02Z; no inbox item filed (inbox over its 10-item cap at 11 open; S16 and S17 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:230-231`, none newly blocking).
- r396 2026-10-17T02:15Z: burndown open 60, ready 16, landed 4, overruns 0, returned 0, failed 83, parked 144; item 2 FIRED - landed moved +33.3% vs r395 (3 -> 4), past the 20% bar (the new landing is `builder-design-systems-r28` -> d0e5dc17 `docs/design/balance-log.md` at 02:10Z) - so the stage table, the buffers and the stage receipts were recomputed and repasted this run onto the same live 0+11+16+17+0+15 = 59 split (`node tools/finish.mjs` re-run at 02:16Z: A1/A2/A3 met, its two fallback warnings and `Summary: 4 met, 10 partial, 12 open`; product report still 2026-10-13T09:21:48Z; ARENA frame-split present 74.905 ms at 02:05Z from `look-5a7631c1-20261017T0205Z.txt`, cpu p99 623.748, framing frames=300 empty=0, validation_errors=0; G9 `potato-2026-10-17.md` 01:23:54Z p50 7.552 / p99 25.696 / peak 1517 all PASS but still stamped LOADED); no `passed:` field moved; risks rewritten one line each (link wall 30 of 108, flat since r393; land-refused/ratchet wall 23 of 108 + 144 parked, flat; Rule 4 holds all 16 of 16 ready, 12 over 30 min, running 8 now far below ready 16, no free ready packet); unblock rewritten in place (`0-gs5-pipeline-gate-close.md` and `0-ad3-spatial-door-close.md` left the running set, so the render holders are `0-cq24-post-jitter-motion-split`, `0-eg11-sun-depth-live-close`, `0-lf3-gtao-close` and `0-sp63-draw-id-close`); `node tools/workspace-check.mjs` read 0 FAIL / exit 0 at 02:21Z; no inbox item filed (inbox over its 10-item cap at 11 open; S16 and S17 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:231-233`, none newly blocking).
- r397 2026-10-17T02:31Z: burndown open 60, ready 14, landed 4, overruns 0, returned 0, failed 83, parked 144; item 2 NOT fired - the nearest earlier line is r396 (02:15Z), no field moves 20% (ready 16 -> 14 = -12.5%, landed 4 -> 4 = 0%, the rest flat) - so the stage table, its buffers and the six `passed:` fields stand from r391; the live FOCUS set was re-read at 02:31Z (`node tools/board-slice.mjs --focus --open` = 59 rows + 1 summary line `59 row(s)`, same 59 row ids, same 0+11+16+17+0+15 split); no stage receipt moved on disk (newest look report still `look-5a7631c1-20261017T0205Z.txt` at 02:05:48Z, product report still 2026-10-13T09:21:48Z, G9 `potato-2026-10-17.md` still 01:23:54Z), so no section was re-pasted and no `passed:` field moves; risks rewritten one line each (link wall 30 of 108, flat since r393, holds EG29 + HS29 + HS31; land-refused/ratchet wall 23 of 108 + 144 parked, flat; Rule 4 holds all 14 of 14 ready, 10 over 30 min, running 15 now level with ready 14, no free ready packet); unblock rewritten in place (the r396 render holder `0-lf3-gtao-close` left the running set, so `perf-per-draw-tobytes-20261016T2159Z` joins `0-cq24-post-jitter-motion-split`, `0-eg11-sun-depth-live-close` and `0-sp63-draw-id-close` as render holders; `level-gp35-kit-walls` and `perf-fc2-fighter-skin-wall` left, replaced by `level-physical-real-match`, `perf-fighter-pose-tick` and `0-gp27-tables-text-close`); `node tools/workspace-check.mjs` read 0 FAIL / exit 0 at 02:33Z; no inbox item filed (inbox 1 over its 10-item cap at 11 open; S16 and S17 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:232-233`, none newly blocking).
- r398 2026-10-17T02:47Z: burndown open 60, ready 16, landed 4, overruns 0, returned 0, failed 83, parked 145; item 2 NOT fired - the nearest earlier line is r397 (02:31Z), no field moves 20% (ready 14 -> 16 = +14.3%, parked 144 -> 145 = +0.7%, the rest flat) - so the stage table, its buffers and the six `passed:` fields stand from r391; the live FOCUS set was re-read at 02:44-02:47Z (`node tools/board-slice.mjs --focus --open` = 59 rows + 1 summary line `59 row(s)`, the 59 ids re-extracted and matching the r397 list as a set, same 0+11+16+17+0+15 split); two stage receipts moved on disk and sections 3 and 6 were re-pasted to stay true - the newest look report is now `look-c0623232-20261017T0241Z.txt` (02:41:41Z, present 59.736 ms, cpu p99 375.219, framing frames=300 empty=0, validation_errors=0, still about 17 fps so not a baseline) and the G9 receipt `potato-2026-10-17.md` is re-stamped 02:44:48.983Z (p50 10.745 / p99 75.588 / peak 1508 all PASS but `floor: WITHHELD` on `load: LOADED 22 cargo + 2 rustc`, the GS16 floor-withholder slice landing at 02:45Z) - so no `passed:` field moves and no bar is lowered; risks rewritten one line each (link wall 30 of 108, flat since r393, holds EG29 + HS29 + HS31 = 3 rows; land-refused/ratchet wall 23 of 108 + 145 parked, and the 1 new park is the integrator's duplicate-copy park at 02:38:58Z, not a refused land; Rule 4 holds all 16 of 16 ready, 10 over 30 min, running 16 level with ready 16, no free ready packet, blocked FOCUS rows 2 = LF4 + FC3 because GS17 + GP31 left ready for running); unblock rewritten in place (`0-cq24-post-jitter-motion-split` and `0-sp63-draw-id-close` left the running set between r397 and this run, so the render holders at 02:47Z are `0-eg11-sun-depth-live-close`, `0-lf3-gtao-live-close` and `perf-per-draw-tobytes`; the ready set grew to 16 and every one fires); `node tools/workspace-check.mjs` read 0 FAIL / exit 0 at 02:48Z (pre-edit run) and again at 02:52Z after these edits (0 FAIL lines); no inbox item filed (inbox 1 over its 10-item cap at 11 open; S16 line 40 and S17 line 42 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:232-234`, none newly blocking).
- r399 2026-10-17T03:01Z: burndown open 60, ready 15, landed 4, overruns 0, returned 0, failed 83, parked 145; item 2 NOT fired - the nearest earlier line is r398 (02:47Z), no field moves 20% (ready 16 -> 15 = -6.3%, the rest flat) - so the stage table, its buffers and the six `passed:` fields stand from r391; the live FOCUS set was re-read at 03:01Z (`node tools/board-slice.mjs --focus --open` = 59 rows + 1 summary line `59 row(s)`, all 59 ids checked id by id against the r398 list and matching, same 0+11+16+17+0+15 split); no stage receipt moved on disk (newest look report still `look-c0623232-20261017T0241Z.txt` at 02:41:41Z, product report still `target/reports/product/latest.json` 2026-10-13T09:21:48Z, G9 `potato-2026-10-17.md` still 02:44:48Z), so no section was re-pasted and no `passed:` field moves; risks rewritten one line each (link wall 30 of 108, flat since r393, holds EG29 + HS29 + HS31 = 3 rows; land-refused/ratchet wall 23 of 108 + 145 parked, flat, and NEW this run the ratchet is RED on the working tree - `node tools/rust-quality.mjs` at 03:03Z prints `WORKSPACE 265.6 93 A` and `RESULT FAIL: 2 ratchet regression(s)`, engine-ai print_in_lib 0 -> 1 from crates/ai/src/depth.rs (running des-enemy-tier-skill-rung) plus engine-game print_in_lib 0 -> 1 from crates/game/src/fighter_look.rs (running perf-fighter-upload), both holding their crate's landings until the print leaves library code; Rule 4 holds all 15 of 15 ready, 11 over 30 min, running 16 now above ready 15, no free ready packet, blocked FOCUS rows 2 = LF4 + FC3); unblock rewritten in place (ready 16 -> 15: `level-raised-point-20261017.md` and `level-physical-real-match-20261017.md` left ready, `bug-art-crowd-empty-b46768e0.md` and `0-lf2-ibl-close-20261017.md` joined the running set, so the render holders are eg11 + lf2 + lf3 + perf-per-draw-tobytes and the game holders bug-art-crowd-empty + perf-fighter-upload + bug-arena-staged-fight-receipt-red; `level-zones-reader-game` crossed the 30 min bar); `node tools/workspace-check.mjs` read 0 FAIL lines / exit 0 at 03:05Z after these edits; no inbox item filed (inbox 1 over its 10-item cap at 11 open; S16 line 40 and S17 line 42 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:232-234`, none newly blocking).
- r400 2026-10-17T03:14Z: burndown open 60, ready 17, landed 4, overruns 0, returned 0, failed 83, parked 145; item 2 NOT fired - the nearest earlier line is r399 (03:00Z), no field moves 20% (ready 15 -> 17 = +13.3%, the rest flat) - so the stage table, its buffers and the six `passed:` fields stand from r391; the live FOCUS set was re-read at 03:15Z (`node tools/board-slice.mjs --focus --open` = 60 lines = 59 rows + 1 summary `59 row(s)`, all 59 ids matching the r399 list, same 0+11+16+17+0+15 split); one stage receipt moved on disk and section 3 was re-pasted to stay true - the newest look report is now `look-44fd1088-20261017T0312Z.txt` (03:12:40Z, present 66.057 ms, cpu p99 595.382, framing frames=300 empty=0, per-pass shadow 0.049 / forward 0.000, validation_errors=0, about 15 fps so not a baseline) - while the product report is still `target/reports/product/latest.json` (2026-10-13T09:21:48Z) and the G9 receipt `potato-2026-10-17.md` is still 02:44:48Z, so no `passed:` field moves and no bar is lowered; risks rewritten one line each (link wall 30 of 108, flat since r393, now counted over every link record = 7 blocked FOCUS rows EG29 HS29 HS31 LF14 GA8 LF10 LF11; Rule 4 holds all 17 of 17 ready, 12 over 30 min, running 21 above ready 17, blocked FOCUS rows 4 = LF4 FC3 EG12 GP42; land-refused/ratchet wall 23 of 108 + 145 parked, and the ratchet red GREW 2 -> 4 regressions - engine-ai print_in_lib 0 -> 1 plus big_files 1 -> 2 from crates/ai/src/depth.rs and engine-game print_in_lib 0 -> 1 plus dup_fns 1 -> 2 from crates/game/src/fighter_look.rs, both owned by running packets); unblock rewritten in place (ready 15 -> 17: `0-eg12-ambient-torch-normal-close-20261017.md` and `0-gp42-debug-camera-20261017.md` joined, `perf-fighter-skin-window-gpu-20261017T0233Z.md` crossed the 30 min bar at 37 min; the render holders are now eg11 + eg14 + lf2 + lf3 + rl21 + rl43 + rl53 + perf-per-draw-tobytes); `node tools/workspace-check.mjs` read 0 FAIL / exit 0 at 03:20Z after these edits; no inbox item filed (inbox 1 over its 10-item cap at 11 open; S16 line 40 and S17 line 42 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:234-235`, none newly blocking).
- r401 2026-10-17T03:29Z: burndown open 60, ready 17, landed 4, overruns 0, returned 0, failed 83, parked 146; item 2 NOT fired - the nearest earlier line is r400 (03:14Z), no field moves 20% (ready 17 -> 17 = 0%, parked 145 -> 146 = +0.7%, the rest flat) - so the stage table, its buffers and the six `passed:` fields stand from r391; the live FOCUS set was re-read at 03:28Z (`node tools/board-slice.mjs --focus --open` = 60 lines = 59 rows + 1 summary `59 row(s)`, all 59 ids matching the r400 list, same 0+11+16+17+0+15 split); no stage receipt moved on disk (newest look report still `look-44fd1088-20261017T0312Z.txt` at 03:12:40Z, product report still `target/reports/product/latest.json` 2026-10-13T09:21:48Z, G9 `potato-2026-10-17.md` still 02:44:48Z), so no section was re-pasted and no `passed:` field moves; risks rewritten one line each (link wall 30 of 109, flat since r393, recounted this run over every live link record = 13 blocked FOCUS rows EG29 SP63 EG12 VS1 LF16 EG21 LF1 LF14 GA8 LF10 LF11 HS29 HS31, not the 7 r400 printed from the two newest records plus the GA8 tally, with HS20/HS30 excluded because the 10-14T22:45Z Resolved note closes the tally row that names them; Rule 4 holds all 17 of 17 ready, 14 over 30 min, running 18 now above ready 17, blocked FOCUS rows 3 = LF4 FC3 EG12, since r400's 4 counted GP42 which sits in FOCUS "Next" and not in the 59-row open set; land-refused/ratchet wall 23 of 109 + 146 parked, the 1 new park `builder-fixer-r145.md` at 03:19:47Z is an unlandable PASS not a refused land, the ratchet red GREW 4 -> 6 - engine-ai print_in_lib 0 -> 1 plus big_files 1 -> 2 from crates/ai/src/depth.rs, engine-game panic_sites 6 -> 7 plus print_in_lib 0 -> 1 plus dup_fns 1 -> 2 from crates/game/src/fighter_look.rs, engine-render casts 990 -> 1012 from the in-flight render refactor, `WORKSPACE 266.2 93 A`; BLOCKERS grew a row, newest 2026-10-17T03:23Z endpoint lib red x2 on the door budget `look.judge` 44241 > 43514, blocks AX27 + HS21, no open FOCUS row); unblock rewritten in place (running 21 -> 18: `0-eg11-sun-depth-live-close-20261017.md` left the running set, so `submit.rs` and `sun_shadow.rs` are free paths again and three ready packets that listed them now hold on their other paths; the render holders are eg8 + eg14 + lf2 + lf3 + rl21 + rl43 + rl53 + perf-per-draw-tobytes, the game holders bug-art-crowd-empty + perf-fighter-upload + bug-arena-staged-fight-receipt-red; 14 ready packets over 30 min, 3 under, no ready packet free); `node tools/workspace-check.mjs` read 0 FAIL / exit 0 at 03:34Z after these edits; no inbox item filed (inbox 1 over its 10-item cap at 11 open; S16 and S17 already open; D1-D6 re-read at `BOOK-GAPS-2026-10-12.md:234-235`, none newly blocking).
