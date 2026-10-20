# SG7 rope swing, hand-thrown anchor (2026-10-20)

The visual clause of SG7: a throw at a wall under `--height` leaves the floor
as a rope swing instead of zipping the body to the wall face. The throw is a
hand (agent-seat) throw, not the `--spear-demo` script, because that script
aims at the rival and always produces the straight zip.

## Run

```
target\release\arena.exe --scene arena --fighters 2 --frames 760 --exit --agent-seat 7123 --spear --height --capture-dir target/clips/sg7 --capture-every 30 --fps 30 --sha abcdef1 --view-brief target/clips/sg7/brief.jsonl
```

Seat tape (slot 0, world axes, facing 0 = +z):

- ticks 0..400: `mz=100`, run at the +z wall from the seed-2040 spawn.
- ticks 400..520: `mz=-100`, back off so the rope has room.
- tick 530..639: buttons `16` (`THROW_SPEAR`), throw at 530 and hold the
  tether.
- ticks 640..719: `mx=100`, run along the wall so the rope swings the body.
- ticks 720..: `mx=60`, release and carry momentum.

The seat client is `%TEMP%\opencode\sg7_client.mjs` (not repo code).

## Receipt lines

```
arena spear: thrown=1 hits=1 tethers=1 ends=0 spears_max=1 ropes_max=1
arena capture: frames=26 manifest="target/clips/sg7\manifest.json" sha=49f44599e9d8969e
arena view: frames=760 sim_ticks=1.0..577.1 bodies=1520 offscreen_share=0.00 nonfinite=0 error_parts=0 sunk_cm_max=0.0 idle_float_cm_max=2.0 pops=281 pop_acts=idle:8/run:273 pop_cm_max=83.8 eye_in_body=0 eye_below_ground=0 closest_cm=1007 cover_max=0.46 foot_slide_cm=0.0 planted_feet=10 ragdolls=1 spear=1 tether=1
arena sim: ticks=578 hash=c5c92c80786238ce
arena seat: port=7123 clients=1 inputs=586 late=0 superseded=0 held=0 invalid=0 dropped=0
```

The 760-frame bound landed at tick 578 because the frame stalls listed in the
run's frame-split line dropped fixed ticks; the seat tape itself is
deterministic. The capture frames are timestamped in `manifest.json`:

- `rope-taut-tick548.png` (frame 24): the rope is taut from the anchored spear
  to the fighter, who has just left the floor.
- `airborne-swing-tick570.png` (frame 25): the fighter is airborne over its
  shadow, the rope still taut, body well clear of the wall face.

`pops=281` is AM24's open look defect (the pop predictor treats the look's
0.25 rad/tick turn as a clip snap), not this row. It is recorded here so the
line is not read as clean.

## Verdict

The wall tether is a rope under `--height`: one spear thrown, one hit, one
tether, `ropes_max=1`, and the captured body leaves the floor on the arc
instead of reaching the wall face. The SG7 visual clause is proven.
