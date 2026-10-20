# Faction war — style lock v0

Read from six reference images on 2026-10-20 (mmos.com gallery, rylpartner,
official promo sheet). They are reference only: no file, texture or mesh from
them enters the repo.

## The one idea

Two factions, two palettes, one shared accent. Humans are light, upright and
cool. Ak'Kan are dark, top-heavy and warm. Both carry gold. That single shared
metal is what makes them look like one world instead of two games.

## Palettes

Humans (light, cool, gold):

| role | hex | use |
|---|---|---|
| bone white | `#E8E2D4` | cloth, tabard, plate highlights |
| silver | `#B9BEC4` | plate body, chain |
| gold trim | `#C9A227` | edges, buckles, the shared accent |
| steel blue | `#3E5A78` | cloth under-layer, small areas only |
| leather | `#6B4B33` | straps, boots, belts |

Ak'Kan (dark, warm, bronze):

| role | hex | use |
|---|---|---|
| black iron | `#2B2724` | main plate |
| blood red | `#7A1F1C` | cloth, paint, faction mark |
| bronze | `#9C6B3A` | trim, the shared accent |
| bone | `#D9CBB2` | tusks, trophies, studs |
| ember | `#C8562A` | small glow, eyes, runes |

World: ochre sand `#C9A46A`, sepia sky `#D8C9A8`, sea teal `#3E7F7A`,
fog `#B9B29C`. Warm ground, cool water, low sun, heavy distance fog.

## Proportions

| | human | Ak'Kan |
|---|---|---|
| height in heads | 7.5 | 6.0 |
| shoulder width in head widths | 2.2 | 3.0 |
| hand length in head lengths | 0.8 | 1.2 |
| stance | upright, narrow | hunched, wide |
| read at 30 m | slim vertical | dark triangle |

The Ak'Kan silhouette is a triangle standing on two legs. If the triangle is
not obvious at 30 m, the model is wrong, whatever the texture says.

## Class silhouettes (Ak'Kan)

Class art is where the design lives; the in-game NPCs are drab on purpose.
Read from the Warfare Online class cards (RYL-lineage art).

| class | silhouette | weapon | read |
|---|---|---|---|
| Templar | upright, wide, tower shield on one side | shield + sword | a wall |
| Gunner | wide stance, cannon diagonal across the shoulder | arm/shoulder cannon, one mechanical limb | artillery that walks |
| Shadow-Off | crouched, narrow, blades low | twin short blades | a knife |

The Gunner is the one to build first. Its whole identity is the diagonal:
a barrel longer than the torso, mounted high, muzzle glowing, one arm
replaced by machinery, the other hand a claw. Enclosed helm, no face.
Stat identity matches the picture: +1.5 DEX +1.5 CON, mobile and tough.

## Colour is per set, red is constant

The high-tier Ak'Kan sets are blue-silver plate with red accents, not the
black-and-bronze of the low-level NPCs. So the faction colour is not a fixed
palette — it is **red as the always-present accent** on a heavy plate base.
Do not lock the Ak'Kan to black. Lock them to: heavy plate, enclosed head,
oversized weapon, red accent.

## Materials

- Two roughness values per character, no more: matte cloth/leather and one
  worn metal.
- Ambient occlusion painted into the texture. No normal maps at this tier.
- One 512 atlas per body, one 256 per weapon.
- Metal reads by value contrast against cloth, not by reflections.

## Silhouette tests

1. 30 m, flat black on white: faction and class readable.
2. 3 m, unlit grey: armour layers readable without colour.
3. Both faction line-ups side by side: the two silhouettes must not overlap.

## Environment

Chunky, oversized architecture. Hard edges, few materials, fog for depth.
Beach + palms + tower for the Ak'Kan start; dust, cracked earth and stone for
the contested middle. No lush fantasy forest.

## UI language

Dark metal frame, gold text, small numbers, hotbar at the bottom. One accent
colour per faction in the HUD.

## Do not copy

Names, faction marks, exact armour sets, the promo layout, logos, the
"Kartefant"/"Merkhadia" labels, or any texture from the references.
