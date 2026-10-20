// Writes the six starter class files. The advanced classes are authored by
// hand under content/classes/; the starters are the shared roots every
// advanced class branches from, so they live in one place.
//
// Run: node tools/starter-classes.mjs
import { writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "content", "classes");

const ab = (id, name, tier, kind, shape, range, radius, cast, cooldown, mana, telegraph, base, scale, effects, notes) => ({
  id, name, tier, kind, shape, range, radius, cast, cooldown, mana, telegraph,
  damage: { base, scale }, effects, notes,
});

const starters = {
  warden_fighter: {
    id: "warden_fighter",
    faction: "wardens",
    tier: 0,
    branch: null,
    role: "line breaker",
    growth: { str: 1, dex: 0, con: 1, int: 0, wis: 0 },
    primary: ["str", "con"],
    playstyle: "Stand in front. Every point of your HP is a wall the other side has to break before it reaches anyone you protect.",
    abilities: [
      ab("iron_snare", "Iron Snare", 0, "control", "single", 9, 0, 0.4, 14, 12, "none", 0, "none", ["root", "damage"], "Paralyse one enemy for 1.6 s. Breaks on any knockback. The answer to a caster, and the thing a caster hates."),
      ab("heavy_arc", "Heavy Arc", 0, "melee", "cone", 2.6, 3, 0.5, 3, 0, "none", 28, "str", ["damage"], "A wide swing for everything in front of you. Low damage per hit, hits five people at once."),
      ab("fencing_step", "Fencing Step", 0, "utility", "self", 0, 0, 0, 12, 0, "none", 0, "none", ["move_speed", "evade"], "Half a second of faster footwork and a small dodge window. The skill you press when a spear comes down."),
      ab("breaker_blow", "Breaker Blow", 1, "melee", "single", 2.4, 0, 0.6, 5, 0, "none", 52, "str", ["damage", "stagger"], "A committed overhead that staggers. Slow enough that a good duelist will read it."),
      ab("shield_wall", "Shield Wall", 2, "buff", "circle", 6, 6, 0, 22, 18, "none", 0, "none", ["damage_reduction"], "A short-lived guard stance. Halves incoming damage for everyone inside it, including the enemies you are standing next to."),
      ab("last_stand", "Last Stand", 4, "utility", "self", 0, 0, 0, 90, 30, "none", 0, "none", ["damage_reduction", "heal"], "For ten seconds you cannot be knocked down and you regenerate. The panic button of the frontline."),
    ],
    loadout: { weapon: "sword", offhand: "buckler" },
    counters: ["warden_sorcerer", "warden_enchanter"],
    weakness: "Low damage. If the line holds you forever, you never win it.",
  },
  warden_rogue: {
    id: "warden_rogue",
    faction: "wardens",
    tier: 0,
    branch: null,
    role: "skirmisher",
    growth: { str: 1, dex: 1, con: 0, int: 0, wis: 0 },
    primary: ["dex", "str"],
    playstyle: "You are not the wall. You are the thing that appears behind the wall after it has already been broken.",
    abilities: [
      ab("scout_pulse", "Scout Pulse", 0, "utility", "circle", 20, 20, 0, 20, 8, "none", 0, "none", ["reveal"], "A short pulse that reveals hidden enemies inside it. A third of a team has one; nobody else does."),
      ab("quick_strike", "Quick Strike", 0, "melee", "single", 2.2, 0, 0.3, 2, 0, "none", 24, "dex", ["damage"], "Fast, cheap, and the second half of every kill you make."),
      ab("shade_step", "Shade Step", 0, "utility", "self", 0, 0, 0, 18, 0, "none", 0, "none", ["move_speed", "evade"], "A short burst of speed with a dodge window. Gets you off a plate and onto a healer."),
      ab("thrown_dust", "Thrown Dust", 1, "control", "circle", 14, 4, 0.3, 16, 14, "none", 0, "none", ["blind"], "A cone of grit that makes a target miss for three seconds."),
      ab("bleeding_edge", "Bleeding Edge", 2, "melee", "single", 2.4, 0, 0.4, 6, 0, "none", 40, "dex", ["damage", "dot"], "A wound that keeps working after you have moved on. The class does not have burst, it has patience."),
      ab("shadow_lunge", "Shadow Lunge", 4, "melee", "line", 8, 0, 0.5, 12, 20, "line", 66, "dex", ["damage", "dash"], "A long lunge that crosses a gap and hits everything on the way. Expensive, and you will be seen doing it."),
    ],
    loadout: { weapon: "short_sword", offhand: null },
    counters: ["warden_cleric", "warden_priest"],
    weakness: "Dies to anything that can reach you. No armour, no shield.",
  },
  warden_mage: {
    id: "warden_mage",
    faction: "wardens",
    tier: 0,
    branch: null,
    role: "caster",
    growth: { str: 0, dex: 1, con: 0, int: 1, wis: 0 },
    primary: ["int", "dex"],
    playstyle: "You win by making the other side stop moving. A mage who is casting is a mage who is being hit, so every second of charge is a decision.",
    abilities: [
      ab("mana_ward", "Mana Ward", 0, "buff", "self", 0, 0, 0, 20, 10, "none", 0, "none", ["spell_shield"], "A short shield that blocks one spell. The most important button on a low level mage."),
      ab("ember_bolt", "Ember Bolt", 0, "projectile", "single", 24, 0, 0.6, 2, 14, "none", 34, "int", ["damage"], "A single target bolt. The slowest thing you own and the one you cast most."),
      ab("static_burst", "Static Burst", 1, "aoe", "circle", 18, 5, 0.9, 8, 26, "circle", 30, "int", ["damage", "shock"], "A burst around the target that shocks everyone close to it. The ground decal is the whole point."),
      ab("frost_line", "Frost Line", 1, "aoe", "line", 20, 3, 1.0, 10, 24, "line", 26, "int", ["damage", "slow"], "A line of ice along the ground. Slows what it crosses so the line holds for two seconds."),
      ab("drain_life", "Drain Life", 2, "projectile", "single", 22, 0, 0.8, 6, 22, "none", 40, "int", ["damage", "heal"], "Steals a little of what it takes. The mage's answer to running dry."),
      ab("elemental_burst", "Elemental Burst", 4, "aoe", "circle", 22, 8, 1.6, 24, 60, "charge", 120, "int", ["damage"], "A large detonation that builds through a visible charge. Three seconds of standing still for a screen full of hurt."),
    ],
    loadout: { weapon: "staff", offhand: null },
    counters: ["ashkin_attacker", "ashkin_combatant"],
    weakness: "Paper. Every enemy in the game is looking for you.",
  },
  warden_acolyte: {
    id: "warden_acolyte",
    faction: "wardens",
    tier: 0,
    branch: null,
    role: "medic",
    growth: { str: 0, dex: 0, con: 1, int: 0, wis: 1 },
    primary: ["wis", "con"],
    playstyle: "You do not win fights. You keep the people who win them standing, and you are the reason the other side kills you first.",
    abilities: [
      ab("field_dressing", "Field Dressing", 0, "heal", "single", 8, 0, 1.0, 5, 8, "none", 70, "wis", ["heal"], "A single heal that stops at missing health. The first thing you learn and the last thing you stop doing."),
      ab("courage_verse", "Courage Verse", 0, "buff", "circle", 10, 10, 0, 24, 16, "none", 0, "none", ["attack_speed"], "A short rally that raises how fast your party swings."),
      ab("cleanse", "Cleanse", 1, "utility", "single", 8, 0, 0.5, 16, 18, "none", 0, "none", ["cleanse"], "Removes one debuff from one ally. The most valuable button in a war window."),
      ab("group_chant", "Group Chant", 2, "heal", "circle", 8, 8, 1.0, 14, 30, "none", 18, "wis", ["heal"], "A steady chant that ticks healing to everyone standing in it."),
      ab("sanctuary", "Sanctuary", 3, "buff", "circle", 6, 6, 0, 40, 40, "none", 0, "none", ["damage_reduction"], "A bubble of safety for eight seconds. It is a wall you can carry."),
      ab("last_chant", "Last Chant", 4, "heal", "circle", 10, 10, 1.5, 60, 70, "charge", 40, "wis", ["heal", "cleanse"], "A long, loud, charged chant that heals a group and strips what is on them."),
    ],
    loadout: { weapon: "mace", offhand: "tome" },
    counters: ["warden_sorcerer", "ashkin_rune"],
    weakness: "You cannot fight back. If the fight reaches you, the fight wins.",
  },
  ashkin_combatant: {
    id: "ashkin_combatant",
    faction: "ashkin",
    tier: 0,
    branch: null,
    role: "bruiser",
    growth: { str: 1, dex: 0, con: 0, int: 0, wis: 0 },
    primary: ["str"],
    playstyle: "You walk into the room and the room gets smaller. Everything else is detail.",
    abilities: [
      ab("safeguard", "Safeguard", 0, "buff", "self", 0, 0, 0, 20, 0, "none", 0, "none", ["damage_reduction"], "A bracing stance that blunts what lands on you."),
      ab("heavy_blow", "Heavy Blow", 0, "melee", "single", 2.8, 0, 0.6, 3, 0, "none", 32, "str", ["damage"], "A slow swing with a lot behind it."),
      ab("brawling_roar", "Brawling Roar", 1, "buff", "self", 0, 0, 0, 30, 0, "none", 0, "none", ["damage"], "A roar that makes your next three hits land harder."),
      ab("iron_tether", "Iron Tether", 2, "control", "single", 9, 0, 0.4, 16, 14, "none", 0, "none", ["root", "drag"], "A hook that drags an enemy to you and holds them there."),
      ab("spike_burst", "Spike Burst", 3, "aoe", "circle", 4, 4, 0.5, 12, 0, "circle", 58, "str", ["damage", "knockback"], "An eruption around your feet that throws people off you."),
      ab("fury_cycle", "Fury Cycle", 4, "buff", "self", 0, 0, 0, 90, 0, "none", 0, "none", ["damage", "move_speed"], "A cycle of rage that trades your safety for a great deal of damage."),
    ],
    loadout: { weapon: "great_axe", offhand: null },
    counters: ["warden_rogue", "warden_archer"],
    weakness: "You are big and slow and everything clever walks around you.",
  },
  ashkin_officiator: {
    id: "ashkin_officiator",
    faction: "ashkin",
    tier: 0,
    branch: null,
    role: "caster",
    growth: { str: 0, dex: 1, con: 0, int: 0, wis: 0 },
    primary: ["dex"],
    playstyle: "You are a knife in the dark wearing a robe. The Ashkin casters are not scholars; they are killers with a different delivery.",
    abilities: [
      ab("holdfast", "Holdfast", 0, "control", "single", 20, 0, 0.3, 18, 16, "none", 0, "none", ["root", "damage"], "A short binding bolt that stops a caster mid-cast. The most valuable thing an Officiator can do."),
      ab("hex_bolt", "Hex Bolt", 0, "projectile", "single", 24, 0, 0.5, 2, 12, "none", 30, "dex", ["damage"], "A fast, cheap bolt. You will cast it more than anything else."),
      ab("shadow_veil", "Shadow Veil", 1, "stealth", "self", 0, 0, 0, 24, 18, "none", 0, "none", ["stealth"], "A short veil that dims you. It breaks when you act."),
      ab("split_arrow", "Split Arrow", 2, "projectile", "single", 22, 0, 0.7, 5, 20, "none", 44, "dex", ["damage"], "One arrow that arrives as two. The pair comes apart around the target."),
      ab("night_tide", "Night Tide", 3, "aoe", "circle", 20, 7, 1.1, 16, 40, "circle", 62, "dex", ["damage", "slow"], "A wave of darkness that spreads out and slows what it touches."),
      ab("last_rite", "Last Rite", 4, "utility", "self", 0, 0, 0, 120, 50, "none", 0, "none", ["execute", "stealth"], "A rite that lets you finish a wounded enemy and leaves you untouchable for a moment afterwards."),
    ],
    loadout: { weapon: "rite_rod", offhand: "charm" },
    counters: ["warden_acolyte", "warden_priest"],
    weakness: "No armour, and the Ashkin do not heal each other the way the Wardens do.",
  },
};

let written = 0;
for (const [name, data] of Object.entries(starters)) {
  const path = join(OUT, `${name}.json`);
  writeFileSync(path, JSON.stringify(data, null, 2) + "\n", "utf8");
  written += 1;
  console.log(`wrote ${name}.json`);
}
console.log(`RESULT starter-classes ok files=${written}`);
