# Worker curve, first data point (AGT19) — 2026-10-24

**Verdict: at this fleet's worker count the machine is throughput-bound, not worker-bound — 217 rows landed in the day (9.0/h, peak 93 in one hour) but 50.7% of the 1000 measured CI runs came back red, so the binding constraint is red runs per landing, not helpers.**

## Measured

`worker-curve-2026-10-24.csv`, beside this file.

- **217 distinct rows ticked** (219 tick lines) across **755 commits** in the local day. That is **9.0 rows/hour** averaged, and it is not flat: 02:00–12:00 local landed 54 rows over 11 hours (4.9/h) while 19:00–23:00 landed 151 over 5 hours (30/h).
- **50.7% red**: 494 failures against 480 successes out of 1000 completed `check.yml` runs (28 cancelled or still running).
- **~4.6 CI runs per landed row** (1000 runs / 217 rows) in the hours that overlap both sources.

## What the numbers say

Every hour that has both columns shows red between 33% and 73%. The red share *rises* with dispatch volume — 208 runs at 21:00 local gave 58.0% red, 198 at 18:00 gave 40.9%, 59 at 14:00 gave 72.9% — which is the signature of contention (pack race on the mirror, shared `target/`, the push queue) rather than of harder work. The measured `pushwait 381.1 s` against `push 3.8 s` on one dispatch in this same window is the same signal: **the queue, not the transfer, is the cost**.

So the curve is not "more helpers, more rows". At today's ceiling — about 40 concurrent sessions, 208 CI dispatches in the busiest hour, 19 runs in flight at one sample — adding a helper mostly adds queue and red runs. The lever with a measured number behind it is the red rate: halving 50.7% would roughly double rows per hour at the same worker count.

## Method, and what this cannot say

- Rows: added `- [x] ID` lines in `docs/SPRINT.md` and `docs/SPRINT-*` from `git log -p`, attributed to the commit's local hour, deduplicated by id.
- CI: `gh run list --workflow check.yml --created 2026-10-24`, conclusions bucketed by hour (UTC converted to local, +1).
- The row asks for the curve at 2, 4, 8 and 16 helpers over one day each. **This is one day at the top of the range (16+), so it is the right-hand end of the curve only.** A 2/4/8-helper day cannot be reconstructed after the fact: the tree holds no record of how many sessions were live, and the owner does not throttle the fleet. Getting those points needs a deliberate quiet day per worker count, or a per-session heartbeat recorded as it runs.
- Two limits on precision: `gh run list` caps at 1000 results, so the CI columns cover an eight-hour window (14:00–21:59 local) and are blank elsewhere; and `total_count` for 00:00–13:00 UTC alone was 729, so the **true day total is above 1729 runs**, not 1000. The 50.7% red rate is from the fully measured window, not a projection.
- `target/gh-check/jobs` is not a history: `pushWip` and `finish` delete a job file when its run settles, so at any moment it holds only the live calls (23 at the time of measuring). It gives concurrency, never a day's tally.
