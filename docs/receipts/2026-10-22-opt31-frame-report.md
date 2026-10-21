# OPT31 receipt: dev window arena 59.3 fps p50, x4 runs inside the pair spread

2026-10-22T02:02Z keeper beat, on this PC. Raw receipts: `target/evidence/build/opt31-frame-report-20261022T0202Z.md`
plus the four `opt31-{a,b,c,d}-20261022T01*.out` stdout dumps in that folder.

Row: OPT31 pilot, dev window arena >= 30 fps on the reference GPU over the fixed arena camera
path after OPT3, x2 within 5%.

Adapter: `NVIDIA GeForce RTX 3050 6GB Laptop GPU`, driver `32.0.16.1062`, read from
`arena per-pass:` and `input_latency_gpu` in every run. 1280x720, `present_mode=Immediate`,
`backend=wgpu` (compiled-in dx12 default).

Method: `target\release\arena.exe --scene arena --fighters 8 --frames 300 --exit` started with
`Start-Process`, stdout and stderr redirected, a poll for the receipt line `interval_ms p50=`,
then the process stopped. `Get-Counter '\Processor(_Total)\% Processor Time'` once a second
during each run; `cargo`/`rustc`/`arena` counted every 900 ms.

Box load at beat start (3 samples): 52.7 / 47.3 / 44.7 %, 0 `cargo`, 0 `rustc`, 0 `arena`.

| run | frames | wall s | cpu avg / max % | cargo max | rustc max | interval_ms p50 | interval_ms p99 | slow_frames | fps from p50 |
|---|---|---|---|---|---|---|---|---|---|
| opt31-a | 300 | 47.9 | 61.8 / 66.7 | 2 | 0 | 16.874 | 457.945 | 35 | 59.26 |
| opt31-b | 300 | 49.1 | 66.0 / 67.6 | 3 | 0 | 17.421 | 399.658 | 72 | 57.40 |
| opt31-c | 300 | 38.8 | 62.0 / 64.1 | 2 | 3 | 16.829 | 123.496 | 44 | 59.42 |
| opt31-d | 300 | 45.7 | 62.7 / 66.3 | 4 | 4 | 16.863 | 225.799 | 62 | 59.30 |

Every run printed the full receipt and then exited on its own; no teardown hang this beat.

Label for every run: **contended** - CPU 61.8-66.3 % average from other AI sessions, far above
the 15 % quiet gate, with 0-4 short-lived `cargo` and 0-4 `rustc` and no build storm (the x8
BLOCKERS class was 34-51 `cargo` / 38-43 `rustc` at 99-100 % CPU).

## Accept

- Floor 30 fps: median interval 16.8685 ms = 59.3 fps, 1.98x the floor; slowest of the four is
  57.40 fps. Pass.
- x2 within 5 %: pair a-b 3.19 % (17.421 vs 16.874, against the pair mean); all four
  max-to-min 3.52 % (17.421 vs 16.829). Pass.
- Numbers name adapter and driver: yes, 4 of 4 receipts.

Honest caveat: contention shows in the tail, not the median - `interval_ms p99` 123-458 ms and
35-72 slow frames of 300 are CPU starvation bursts on this shared box, all under the product
bar's `stall_p99_ms_max` 1500 ms. A quiet box can only improve both numbers, so this is a lower
bound on the floor and the spread, not a flattering sample.

## Same-receipt side readings

`arena framing: frames=300 fighters=8 seen=8 empty=0 fewest_in_view=6`;
`arena view: nonfinite=0 error_parts=0 eye_in_body=0 pops=0 sunk_cm_max=0.0`;
`arena break: breaks=12 debris=1`; `physfx: bodies=1 contacts=1`; GPU p50 0.566-2.173 ms;
`validation_errors=0`; `arena waits: live_waits=0 steady_bind_groups=0`.

## Blocker closed

`docs/BLOCKERS.md` row 2026-10-20T21:47Z (`opt31-window-stall-buildstorm-x8`) closes: the window
path presents frames on this box whenever it is not in a build storm (4 of 4 runs reached the
frame loop and exited alone). The earlier stalls were a 100 % CPU build storm, not a broken
window path. OPT33's dx12-vs-vulkan A/B is still unmeasured and keeps its own row.
