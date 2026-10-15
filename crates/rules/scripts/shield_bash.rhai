// Shield bash: an actor who blocks and attacks in the same tick, with a foe in reach and
// stamina to spare, staggers that foe, shoves them back, and holds a short shield.
//
// This text runs once per living actor per tick. The actor's numbers are plain read-only
// variables (id, team, x, z, facing, hp, stamina, attack_age, attacking, blocking, dodging,
// react, knockdown, shielded, mx, mz, buttons, attack, block, dodge); everything else is a
// host call (tick, nearest_foe, dist, x_of, z_of, clamp, roll, mem_get, mem_set) or an action
// (damage, heal, stun, push, drain_stamina, shield). Memory slot 0 keeps (tick + 1) of the last
// bash, so the move cannot be spammed.

// Needs block and attack held together, and an actor who can act.
if !blocking || !attack {
    return;
}
// Stamina runs 0 to 100 points in the real arena (0 to 1000 in the sim worlds);
// below 30 the bash is spent.
if stamina < 30 || react > 0 || knockdown > 0 {
    return;
}

// Cooldown: 90 ticks after the last bash (slot 0 is 0 until the first one).
let last = mem_get(0);
if last > 0 && tick() < last + 90 {
    return;
}

// The nearest living foe, close enough to hit.
let foe = nearest_foe(id);
if foe < 0 || dist(id, foe) > 1200 {
    return;
}

// Shove along each axis away from the basher, 600 mm; integer signs only.
let sx = clamp(x_of(foe) - x, -1, 1);
let sz = clamp(z_of(foe) - z, -1, 1);
push(foe, sx * 600, sz * 600);
stun(foe, 18);
damage(foe, 40);
drain_stamina(id, 250);
shield(id, 30);
mem_set(0, tick() + 1);
