# Skill-on vs skill-off pairs: audio-cue-rules

Three situations. "Off" is what an agent says from memory; "On" is what the skill makes it say. Pass = the On answer applies the rule, searches or cites the source for each claim, and names the setting or test that proves it.

## P1: linear fade for distance gain (audio)

Situation: A builder sets the gain for a gunshot to fall in a straight line from the source to 200 m, then zero.

Off: "Fade linearly to zero at 200 m. It sounds smooth and the code is simple."

On: "Rule 1: gain falls as 1/r between the falloff min and falloff max distances, then clamps to zero. Search `node tools/books.mjs search "falloff distance gain"` and cite [geaen 13.4.3.1-13.4.3.3]. Set the falloff min and max as two named values. Dialogue may bend the curve, but gunshots follow 1/r."

What skill-less misses: the 1/r law the books use, the falloff min below which the gain does not grow, and the clamp at the max. A linear fade reads too quiet near and too loud far.

## P2: one WAV for a sound that repeats (audio)

Situation: A builder wires the door-slam sound by playing one WAV file from the call site each time the door opens.

Off: "Play the WAV at the door. One file is enough for a slam."

On: "Rule 2: play cues, not clips. A cue is clips plus 3D or 2D, falloff, group and loop. Give each cue 3-4 variants. Search `node tools/books.mjs search "sound cue variants"` and cite [geaen 13.5.7.2]. Pick the door-slam cue from the 67 clips in `content/audio`."

What skill-less misses: the variants. The same file played each time sounds mechanical, and the group limit that stops a crowd of slams from eating every voice.

## P3: voice stealing by age (audio)

Situation: A builder runs out of voices. A new footstep needs one, so the code steals the oldest voice, and that voice is a line of dialogue.

Off: "Steal the oldest voice. It is the usual approach and it is cheap."

On: "Rule 3: when voices run out, steal by priority, not by age. Limit instances per group. Rule 4: duck the music under dialogue. Search `node tools/books.mjs search "voice stealing priority"` and cite [geaen 13.5.8.2, 13.5.8.4]. A footstep has lower priority than dialogue, so it should not take the dialogue voice."

What skill-less misses: priority. The oldest voice is often the dialogue, so a footstep cuts off a line the player needed to hear.
