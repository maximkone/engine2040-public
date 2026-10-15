// CQB slice: close-quarters shooter rules for the FPS round.
//
// This text runs once per living actor per tick. The actor's numbers are plain read-only
// variables (id, team, x, z, facing, hp, stamina, react, knockdown, ...); everything else
// is a host call (tick, nearest_foe, dist, clamp, mem_get, mem_set) or an action
// (damage, walk, face, press). Damage falls off by band: point-blank hits full,
// close hits hard, near hits light. Memory slot 0 keeps (tick + 1) of the last
// shot, so fire runs at one round per 30 ticks.

// A staggered or downed fighter cannot act.
if react > 0 || knockdown > 0 {
    return;
}

// The nearest living foe.
let foe = nearest_foe(id);
if foe < 0 {
    return;
}
let gap = dist(id, foe);
// Out of reach: advance behind the shield.
if gap > 5000 {
    walk(100, 0);
    face(clamp(facing + 1500, -18000, 18000));
    press(2);
    return;
}

// Fire rate: one round per 30 ticks (slot 0 is 0 until the first shot).
let last = mem_get(0);
if last > 0 && tick() < last + 30 {
    return;
}

// CQB falloff, integers only: full at point-blank, hard when close, light when near.
if gap <= 1500 {
    damage(foe, 120);
} else if gap <= 3000 {
    damage(foe, 90);
} else {
    damage(foe, 60);
}
press(1);
mem_set(0, tick() + 1);
