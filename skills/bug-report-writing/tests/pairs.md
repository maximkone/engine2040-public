# Pairs: bug-report-writing

## Case 1: vague title, no build
Input: "Level 3 doesn't work. Must fix. Tested on iPhone, latest build."
With skill: title "iOS_playtest_Level3: crash at level end on tap retry"; source lists build hash, iPhone 13, iOS 16.4; row, paths, test command filled; comment holds no "must fix".
Without skill misses: standardized title, device plus OS, version, and severity versus priority split; packet bounces as more-info.

## Case 2: merged steps, no expected
Input: "Opened game, played a while, shop names look wrong. Repro 1/1."
With skill: preconditions plus 4 short steps one action each; expected new UI names, actual old logical strings; capture attached; repro rate 10/10 over 10 tries; Goal and Scope map to queue Goal and Scope.
Without skill misses: numbered redoable steps, expected versus actual pair, real repro rate, and capture; builder cannot redo it.

## Case 3: crash with no proof or stop
Input: "Game crashes sometimes on retry. High priority, fix now."
With skill: Proof holds crash log plus video and rate 9/10; severity set by player block at level 3, priority left to producer; Stop names cant-repro exit after two identical fails.
Without skill misses: log, rate over 5 tries, severity by player impact, and a stop rule; the report reads as opinion and gets ignored.
