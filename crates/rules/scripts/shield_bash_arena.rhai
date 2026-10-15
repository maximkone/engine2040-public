// Shield bash for the real arena: a scripted fighter that has a foe in reach and stamina to
// spare raises its block and strikes in the same tick, then waits before it can do it again.
//
// The arena asks this text once per tick for each scripted slot. The actor's numbers are plain
// read-only variables (id, x, z, stamina, react, knockdown, buttons, ...); the movement and
// facing the brain proposed stay unless the script changes them (`walk`, `face`). `press`
// writes this slot's button bits for the tick (1 attack, 2 block, 4 dodge). Memory slot 0 keeps
// (tick + 1) of the last bash, so the move cannot be spammed.

// Out of reach: run in behind the shield. In reach: block and strike together.
// A staggered or downed fighter cannot act.
if react > 0 || knockdown > 0 {
    return;
}
// The arena keeps stamina in points, 0 to 100.
if stamina < 30 {
    return;
}

// Cooldown: 90 ticks after the last bash (slot 0 is 0 until the first one).
let last = mem_get(0);
if last > 0 && tick() < last + 90 {
    return;
}

// The nearest living foe, close enough to hit.
let foe = nearest_foe(id);
if foe < 0 {
    return;
}
let gap = dist(id, foe);
// Charge: run in with the shield up, veering 15 degrees to its right, until the foe is in reach.
if gap > 2500 {
    walk(100, 0);
    face(clamp(facing + 1500, -18000, 18000));
    press(2);
    return;
}

press(3);
mem_set(0, tick() + 1);
