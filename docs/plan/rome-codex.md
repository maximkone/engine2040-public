# Rome codex: the era's texts, places, art and tech as game data

Owner 2026-10-21: "real rome docs we can get all wikipedia and more books to reuse art, stories
places tech and all what we find on the era". The codex is a local library of the Roman era that a
tool fetches, indexes and turns into game data. The game plan that consumes it is `rome.md`. Rows
CDX1-CDX8 in `docs/SPRINT-28-rome.md`.

## 1. The rule: every number and name in the Rome game points at a source

A build recipe (ditch 5 x 3 ft), a march pace, a place on a map or a mission beat carries a codex
reference (`vegetius.3.8`, `caesar.bg.4.17`, `pleiades:423025`, `column.scene.11`). A lint fails a
Rome data file with a number and no reference (CDX6). The AI door answers "where does this come
from" with the passage itself (CDX7). Realism is checkable, not a vibe.

## 2. Sources, fetch first to last

Surveyed 2026-10-21 by a research agent (pages read live; sizes as stated by each site).

| # | Source | How a script gets it | Format, size | Licence as stated | Game use |
|---|---|---|---|---|---|
| 1 | Perseus `canonical-latinLit` and `canonical-greekLit` | `git clone` from github.com/PerseusDL; Caesar is `data/phi0448/phi001/phi0448.phi001.perseus-lat2.xml` and `perseus-eng2.xml` | TEI XML | CC BY-SA 4.0 | Latin and Greek text with English: Caesar, Tacitus, Polybius, Josephus |
| 2 | Project Gutenberg | `https://www.gutenberg.org/ebooks/{id}.txt.utf-8`: Caesar 10657, Vitruvius 20239, Polybius 44125 and 44126, Tacitus Germania 2995 and Histories 16927, Suetonius 6400, Pliny's Natural History volumes | UTF-8 text, about 1 MB each | US public domain | the English working text for search |
| 3 | Wikipedia | REST `en.wikipedia.org/api/rest_v1/page/summary/{Title}` and the category API (`Category:Military_of_ancient_Rome` 88 pages, `Category:Roman_legions` 77) | JSON | CC BY-SA 4.0 | glossary, unit and tech blurbs, mission lists; never the 26 GB dump |
| 4 | Wikidata SPARQL | `query.wikidata.org/sparql`, 60 s per query, a User-Agent | JSON | CC0 | legions, forts, battles with coordinates and dates |
| 5 | Pleiades | `atlantides.org/downloads/pleiades/json/pleiades-places-latest.json.gz` | JSON | CC BY 3.0 | every ancient place with coordinates: the map cells |
| 6 | Itiner-e roads | Zenodo record 17122148, `itinere_roads.geojson` (78 MB) | GeoJSON | CC BY 4.0 | the road network for march routes |
| 7 | Met Open Access | `MetObjects.csv` (git LFS) and `collectionapi.metmuseum.org/public/collection/v1/objects/{id}`; we stay under 5 requests a second | CSV, JSON | CC0 | arms, armour, tools, pottery reference |
| 8 | Art Institute of Chicago | `api.artic.edu/api/v1/artworks/search?q=` | JSON, IIIF images | CC0 per work | reference boards |
| 9 | Wikimedia Commons | Action API `categorymembers` on `Category:Trajan's_Column` (about 280 files) and `imageinfo` for each file's licence | JPG | per file | the Column scenes: the storyboard |
| 10 | Poly Haven | `api.polyhaven.com/assets?type=textures`, `/files/{slug}` | PNG, EXR | CC0 | stone, brick, plaster, mud, timber materials |
| 11 | Smith's Dictionary of Antiquities (1875) | LacusCurtius pages `SMIGRA*/...html`, one entry at a time | HTML | 1875 text public domain; Thayer's own notes are his | glossary entries for tools, camp, roads |
| 12 | Commons Latin pronunciation | `Category:Latin_pronunciation` (about 720 OGG files) | OGG | per file | commands and voice reference |

Not used: the Wikipedia full dump (26 GB, the API is enough), ORBIS (no download, NC tiles),
British Museum images (NC), bulk scraping of LacusCurtius (fetch single pages we cite).

## 3. Layout

- Raw fetches go to `content/local/codex/<source>/` (gitignored, never committed). Each file gets a
  sidecar `<file>.src.json`: URL, fetch time, sha256, the licence string the site states.
- `tools/codex/sources.json` lists every source: id, URL pattern, rate limit, size cap. One fetch
  tool reads it (CDX1).
- `tools/codex/index.mjs` cuts texts into passages (book.chapter.section), tags them (topic, place,
  year, tool, number) and writes `content/local/codex/index.jsonl` (CDX2).
- Derived game data is small and committed under `content/rome/`: `places.json`, `roads.json`,
  `tech/*.toml`, `missions/*.md`, `lexicon.json`, `boards/*.json` (CDX3-CDX5). Each entry keeps its
  codex reference.

## 4. From library to game

| Codex | Extractor | Game data | Consumer |
|---|---|---|---|
| Pleiades, Wikidata forts and battles, Itiner-e | CDX3 | `content/rome/places.json`, `roads.json` | chapter maps and the march route (`rome.md` LEG8) |
| Vegetius, Hyginus, Caesar, Vitruvius, Frontinus, Josephus passages with numbers | CDX4 | `content/rome/tech/*.toml` (ditch, rampart, bridge, road, crane, engine) | build jobs and making loops |
| Trajan's Column scenes, Dio, Tacitus, Suetonius | CDX5 | `content/rome/missions/*.md` (a beat list per chapter with scene refs) | chapters and set pieces |
| Met, AIC, Commons, Poly Haven | CDX5 | `content/rome/boards/*.json` (image refs per object: helmet, gladius, scutum, dolabra) and CC0 materials | art direction, the look judge, texture intake |
| Wikipedia, Smith's Dictionary, Latin audio | CDX3 | `content/rome/lexicon.json` (term, meaning, command, audio ref) | HUD words, orders, voice |

## 5. For the AI door

- `codex.search { query, topic?, limit }` returns passages with their references (CDX7).
- `codex.cite { file, key, ref }` attaches a reference to a game data entry as one undoable change.
- An agent asked to "build a marching camp" reads `tech/camp.toml`, which already holds Hyginus and
  Vegetius numbers with their refs; it never invents them.

## 6. Rows

- CDX1 fetch tool over `sources.json` with caps, sidecars and a dry run.
- CDX2 passage index with tags.
- CDX3 places, roads and lexicon extractors.
- CDX4 tech extractor: numbers with units and refs into `tech/*.toml`, reviewed by a person or the
  lead before a recipe uses them.
- CDX5 Column storyboard and reference boards.
- CDX6 lint: a Rome data number without a ref fails `workspace-check`.
- CDX7 door verbs `codex.search` and `codex.cite`.
- CDX8 a credits page generated from the sidecars.

Fetching is the owner's ask (2026-10-21). The fetch tool still prints the total bytes and asks for
`--yes` before it downloads more than 500 MB in one run.
