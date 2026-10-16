# MP WAN-proof checklist

Turns MP6 into steps you can check.
Scope: 2 humans plus 6 AI, two machines, one bout.

## Setup

1. Tick MP1, MP2, MP3 first.
2. Use two machines on real LAN.
3. Start one bout with 2 humans plus 6 AI.

## Play with drop and rejoin

4. Play mid-fight. Keep both humans moving.
5. Drop player 2 for 10 seconds.
6. Rejoin player 2. Play to match end.
7. Do steps 4 to 6 one more time.

## Checkpoint equality slots

- Run 1 hash A: ____ hash B: ____ equal: [ ]
- Run 2 hash A: ____ hash B: ____ equal: [ ]
- Checkpoint file A: ____
- Checkpoint file B: ____
- Full checkpoint equality: [ ] yes [ ] no

## Distributions and soak

- Loss and latency table posted at: ____
- 8-tick resim window set from data: [ ]
- 16-bot x600 soak hashes x2: [ ]

## Evidence command

Run this read-only command and paste output:

node tools/mp-evidence.mjs

Pass needs both runs equal plus soak green.
If a step cannot finish write skipped plus why.
