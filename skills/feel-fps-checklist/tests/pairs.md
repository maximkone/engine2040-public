# Skill-on vs skill-off pairs: feel-fps-checklist

Three feel reviews. Off is general habit. On is the checklist verdict with item numbers plus three gates: frozen sim with live view, pure injected-time tests, release bench fit. Pass means On names the items, the failure stopped, and the evidence asked.

## P1: longer freeze, one all-on video (items 1, 2, 4)

Situation: a diff in combat lengthens freeze frames. Evidence is one all-on clip captioned punchier.

Off: Longer freeze means punchier. Ship it.

On: Not yet. Item 1: show the off/on pair of pause-off plus others-on against pause-only against all-on. Item 2: prove sim dt reads zero while the view still trembles from the live clock. Item 4: print live load plus remaining freeze on the HUD. Without the pair a soft or empty hit hides behind the longer freeze.

Skill-less misses: approves a longer freeze that still feels soft because shake and squash were never isolated.

## P2: shake sampled from wall clock (items 3, 5)

Situation: a diff in arena samples the shake offset from the wall clock and the raw frame dt. Tests use a canvas plus timer mocks and stay green.

Off: Tests are green and it looks smooth here. Approve.

On: Reject. Item 3: accrue real dt, run whole fixed steps, carry the leftover slice, never feed raw frame dt to the sim. Item 5: feel tests must inject dt with no window, canvas, wall clock, or timer mocks. Rerun with injected uneven dts and show the same seed gives the same offsets.

Skill-less misses: lets frame-rate shake through, so replay and rollback diverge.

## P3: slow-mo claim with no bench (items 6, 7, 8)

Situation: a diff in render adds a slow-motion stretch. The note says cinematic and cheap with one debug timing.

Off: Slow-mo looks nice and the debug number is low. Approve.

On: Need the gates. Item 6: release bench for update and impact with calls-per-16.67 ms fit count. Item 7: decay table proving halved load drops to about a quarter move. Item 8: prove the tremble stretches calm under slow factor with no decay while frozen. No PASS on eye-only plus a debug number.

Skill-less misses: ships eye-only slow-mo that fades layers during the freeze and carries no budget proof.
