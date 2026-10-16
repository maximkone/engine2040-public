# CT56 splitter receipt — urban-war pilot corner (Size M, one piece only)

Row: CT56 content urban-war pilot corner. Tier B. Size M.
Owns `content/maps/urban-war-pilot/`. This run lands the splitter only:
1 variant + 1 corner selection + probe + provenance + x2. Remainder open.

## Files (lane `builder-content-r4-ct56`)

- `variant.json` 207 B SHA `A0DBE4C860FE49D44239D5F726E3EF0D86A2D59A884025FAC2A88B6B00FE328A`
- `corner.json` 734 B SHA `CB3B30F19B2A2EA2024374FB71D1EAB6C9CDE26848D93527D2E068F9CA1889C7`
- `PROVENANCE.md` per-file rows (CC0-1.0, splitter lane)
- This receipt

## Accept checklist (splitter scope, real numbers only)

- Variant + corner cook via CT1/CT2: SPLITTER — no new glTF. Reuses frozen
  CT9 assets (insula, street_wall, market_stall). CT1/CT2 needs ticked
  (`board-slice CT56`: CT1 [x], CT2 [x]). CT9 proven cook cited as reference
  only (`content/maps/report.json`: total_lod0 25968, bundle 33244267 B /
  568 entries, 3 maps 0 findings). Full pilot mesh cook rides remainder.
- SP5 placement 0 findings: SPLITTER PROBE ONLY — `node ct56-probe.mjs`:
  `actors=3 findings=0`; `insula_a x=[-47.2,-40.8] z=[-3.3,3.3]`;
  `stall_fish x=[-27.5,-24.5] z=[-3.1,-0.9]`; `city_wall x=[-7.0,3.0]
  z=[-0.3,0.3]`; all inside 99 m, XZ disjoint. Full SP5 `run()` deferred.
- No engine hunk fork: `node tools/lanes.mjs diff builder-content-r4-ct56`
  shows `2 files changed, 46 insertions(+)`, both under
  `content/maps/urban-war-pilot/`; zero crate hunks.
- Provenance per file: see `PROVENANCE.md` (2 rows, SHAs above).
- Cook x2 identical (new JSON): `Get-FileHash` twice —
  variant `A0DBE4C860FE49D44239D5F726E3EF0D86A2D59A884025FAC2A88B6B00FE328A`
  x2 identical; corner
  `CB3B30F19B2A2EA2024374FB71D1EAB6C9CDE26848D93527D2E068F9CA1889C7`
  x2 identical.
- Bundle loads via AS1: DEFERRED — AS1 ticked ([x] per `board-slice AS1`),
  but splitter claims no bundle. Full pilot bundle rides UW39 remainder.

## Consumer

UW39 receipt, Finish B second-game bar (UW39 open, needs UW34-UW38 + GM3).
Splitter feeds CT78 scout / CT56 remainder, claims no UW39 line.

## Remainder (not in this run)

Full corner mesh cook via CT1/CT2, SP5 solver `run()`, AS1 bundle write +
`BundleLoader` load + corrupt-refused, VRAM/bytes guess, UW39 evidence line.
