# Skill-on vs skill-off pairs: anim-combat-rules

Three situations. "Off" is what an agent says from memory; "On" is what the skill makes it say. Pass = the On answer applies the rule, searches or cites the source for each claim, and names the test or capture that proves it.

## P1: blend from live pose into ragdoll (bodies)

Situation: A builder makes a fighter go limp when hit. They blend the live pose into a ragdoll pose over 0.3 s.

Off: "Blend the live pose into the ragdoll pose over 0.3 s. It is smooth and the player will not notice."

On: "Rule 7: never LERP a live pose into a ragdoll pose. Drive the bones with powered (PD) constraints, then release them. Go ragdoll as soon as any bone touches solid. Search `node tools/books.mjs search "ragdoll powered constraints"` and cite [geaen 12.4.8.8, 12.5.3.8]. Check a clip of the hit for a float or a jump at the handoff."

What skill-less misses: a blend between two shapes that do not match. The limbs float or jump at the handoff, and the builder reads it as a timing bug.

## P2: looping clip with a repeated last sample (bodies)

Situation: A builder exports a 24-frame idle loop with 25 samples, the last equal to the first. The loop pops at the seam and the builder asks why.

Off: "Keep all 25 samples. The loop is closed, so the extra one is fine."

On: "Rule 4: a looping clip of N frames stores N samples. The repeated last sample makes the loop pop. Search `node tools/books.mjs search "looping animation clip"` and cite [geaen 11.4.1.4]. Drop the last sample and check the seam frame again."

What skill-less misses: the cause of the pop. The extra sample looks harmless, and the builder tunes the blend time instead of fixing the data.

## P3: fighter moved by force (bodies)

Situation: A builder moves a fighter by applying a force each tick. The fighter clips through a wall at high speed.

Off: "Apply a force and let the physics solver stop it at the walls. That is the physics way."

On: "Rule 8: move characters with capsule casts and a manual slide, step and slope, not forces. Casts never change the world. Search `node tools/books.mjs search "character controller capsule"` and cite [geaen 12.5.3.6, 12.3.7]. Sweep the capsule along the move each tick and stop it at the first hit."

What skill-less misses: the capsule sweep that stops the clip-through, and the rule that a cast only asks a question and never moves anything.
