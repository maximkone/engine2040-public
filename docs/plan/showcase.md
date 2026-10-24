# Showcase - every system we built, in one tour

Owner 2026-10-24: "how can we show all of our shits we made on a demo ... i afraid we can
forget these cool stuff". The answer is one scripted tour, `play-showcase.bat`
(`arena.exe --showcase`), and this table. A system is not finished until it has a station in
the tour; a system that is built but not hooked into the game sits in the table with the
row that wires it, so nothing landed is forgotten.

## How the tour works

- `crates/game/src/showcase.rs` (`Showcase`) drives slot 0 through the stations in order.
  The rival is a training dummy (`Showcase::rival_input`, staged as the remote slot through
  `App::offer_remote`): it stands and faces you, and swings once in the rope-cut station, so the
  bot never ends the bout before the tour does.
- Each station has a caption (the window title) and a grade (`ok`, `missed`, `pending`)
  read from what the match reports: spear phases and events, fight cues. A station that
  cannot pass in 20 s is marked missed and the tour moves on.
- At exit the window prints `showcase: yank=ok carry=ok cut=ok swing=ok melee=ok finisher=ok`.
  The headless test `showcase::tests::the_tour_passes_every_station_against_the_dummy`
  plays the same tour on the window's session and asserts every station passes.
- A new station is one `Station` variant, its caption, its script in `Showcase::input` and
  its grade in `Showcase::observe`, plus a row below.

## Every system and where it stands (2026-10-24)

| System | Landed | State | Station row |
|---|---|---|---|
| Spear yank, hold back as it sticks | SG9 | in tour (station 1) | SHOW1 |
| Spear carry | SG1 | in tour (station 2) | SHOW1 |
| Wall rope swing, corner wrap | SG4, SG5, SG7, `93dc9a671` | in tour (station 4) | SHOW1 |
| Toned hit reactions, sword hits | FEEL9, `cb278da5b` | in tour (station 5) | SHOW1 |
| Ragdoll with muscle tone, braced tow | `cb278da5b`, `298e8cb22` | in tour (station 6, and the yank tow) | SHOW1 |
| A blade cuts the rope | SG13 | in tour (station 3) | SHOW2 |
| Yank look (taut rope, victim towed) | SG10 | in tour (station 1), hand capture open | SHOW1 |
| Laquearius noose (loop, snare, drag, shield push) | SG11 | wired (Ctrl), station not in tour: it was written and taken out, see SHOW10 | SHOW10 |
| Destruction sag before collapse | DX18 | built, not hooked in (nothing reads `sag_q16`) | SHOW3 |
| Seeded machine props (ballista, crane, cart) | `cc3120dde` | built, not in props or the cook | SHOW4 |
| MarkovJunior rule maps (streets, insula, ruins) | MJ1 | built, no door or map yet (MJ2) | SHOW5 |
| Octahedral impostors | RL63, RL64 | bake and draw landed, the stands crowd is 96 baked bodies in the window code; frame proof pending | SHOW6 |
| AI decision trace | AIN8 | built, not on in a live match | SHOW7 |
| Animation budget allocator | AM33 | built, wiring AM34 | SHOW8 |
| Motion warping for lunges | AM40 | built, wiring AM41 | SHOW8 |
| Caesar III service walkers | CITY1 | built in `crates/rome`, no city view | SHOW9 |
| Seat-driven play (agents play the window) | AN rows | wired | n/a (it drives the tour's checks) |

Research backlog for the next steals: `docs/research/plugins-2026-10-24/01-07` and
`docs/research/grapple-2026-10-24/`.

## Rows

On the board shard `docs/SPRINT-38-showcase.md`: SHOW1 to SHOW10.
