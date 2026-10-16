# PLAYTEST — newcomer sessions

Only people run this. Agents never run it.
Kind: 5 to 7 first-time players per round, in Maxim's hands.
One driving question per session. One session per player.

## Session goals (3)

1. Find where a new player gets lost in the first fight.
2. Learn what the player expected and what they got.
3. Collect one clean bug per confusion point.

## Script (read word for word)

> Hello, thanks for helping us test.
> You can stop at any time, and this records only game notes, no personal data. Is that okay?
> Play for 10 to 20 minutes. I will watch and take notes.
> I cannot give hints. Please think aloud as you play.

Do not hint. Do not teach. Watch and write.

## Optional questions (ask at most a few, never lead)

1. Did anything technical get in the way (install, start, sound, window, pad)?
2. What did you try first?
3. What did you expect to happen there?
4. What told you where to go?
5. What was unclear on the screen?
6. When did you feel stuck?
7. What did the enemy do that surprised you?
8. What did you want to do that you could not?
9. What helped you most?
10. What would you change about the first two minutes?
11. What do you remember about the controls?
12. Would you keep playing? Why?

## Survey (after play, 1 to 5: 1 no, 5 yes)

1. (+) I knew what to do next. (positive)
2. (-) I felt lost in the arena. (negative)
3. (+) The fight felt fair. (positive)
4. (-) The controls felt confusing. (negative)
5. (+) I want to play again. (positive)
6. (-) The screen felt too busy. (negative)

Three positive, three negative. Flip the negative items before you average.

## Comprehension checks

1. How do you start a fight?
2. How do you block or dodge?
3. How do you know you won?

## Funnel (fill per round)

| step | players reaching it | percent |
|---|---|---|
| started the fight | 7 | 100 |
| blocked once | 5 | 71 |
| won one bout | 3 | 43 |

Percent is players reaching it divided by players who started.

## Bug severity

- A: blocks play. No workaround. Fix first.
- B: wrong but playable. Has a workaround.
- C: small or visual. Fix when cheap.

## Log

Write one row per confusion point or bug.
Path: `docs/design/playtests/<date>.csv` (date is `YYYY-MM-DD`).
Columns:

`tester,in_audience,build_sha,minute,confusion_point,question,yes_no,what,where,why,conclusion,bug_id,severity`

- `tester`: short id like P1. `in_audience`: yes or no.
- `build_sha`: the build you played. `minute`: minute in session.
- `question`: the one driving question for that session.
- `yes_no`: yes, no, or empty. `bug_id`: empty or an id. `severity`: empty, A, B, or C.
- A row with empty `where` or empty `conclusion` fails.

Check a log (use the file you wrote above):

`node tools/playtest-log.mjs --check docs/design/playtests/<date>.csv`

It prints `PASS <row> ok` or `FAIL <row> <reason>` per row, then a count line.
Its test:

`node --test tools/playtest-log.test.mjs`
