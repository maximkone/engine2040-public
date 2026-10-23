# Task 6: measure a planted foot in the right frame

The pack stores **local** joint offsets: the foot's offset inside the leg. The
receipt reports `avg_slip` over that local offset, so a foot that is planted on
the sand reads 0.109 m/s of "slip" while the sand never sees it move.

Measure the foot in world space instead: compose the chain (hips rotation, leg
offset, foot offset) and average the foot's world speed between ticks. The bar
stays 0.08 m/s; only the meter changes.

The referee recomputes both meters from `pack.json`. A receipt that carries the
local number, or a made-up number under the bar, stays red.

## Start

```sh
node tools/tasks/06-slip-meter/referee.mjs --apply start --dest target/task-06
```

## Accept

```sh
node tools/tasks/06-slip-meter/referee.mjs --tree target/task-06
```

Exit 0 needs `receipt.avg_slip` equal to the world meter (1e-5) and under 0.08.
