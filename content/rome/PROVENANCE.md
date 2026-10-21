# Rome content sources (HX28)

The two donor files behind the shipped Rome map tables. Both were fetched
through the GitHub API on 2026-10-21 and only the slice the game needs was
kept, so no donor bytes ship: `content/rome/places.toml` holds the gazetteer
rows of the chapter boxes and `content/rome/fortifications.toml` holds the
named frontier lines, each one as a table row with its own `ref`.

| id | source URL | licence | downloaded | file (bytes) | sha256 | what we keep |
|---|---|---|---|---|---|---|
| urbesetorbis | https://github.com/Diasito/urbesetorbis/blob/main/public/downloads/301023_AD130.csv | BSD-2-Clause | 2026-10-21 | 1704784 | b9c98bf71389b3fee4e17280cb70cb319a009342e7c6915afb2dc9c0f6b41315 | the 25 settlement rows inside the boxes of the six chapter areas, with the Pleiades, Barrington, DARE and Wikidata ids and the founding year |
| dare | https://github.com/johaahlf/dare/blob/master/fortifications.geojson | Apache-2.0 | 2026-10-21 | 166739 | f20449fc3a72594d10a88294d0d2c8d3c6b27312a8d6691ab53701674a0bc98d | the 51 named frontier lines: the Upper German and Raetian Limes, the Main, the Neckar, the frontier road and the mile markers of Hadrian's Wall |

The licence of a donor applies to the rows cut from it. The gazetteer is a
compilation of other people's data (Pleiades, the Barrington Atlas, DARE,
DARMC, Hanson, ORBIS), and each of those keeps its own terms for its own
columns; the table ships the ids as pointers, so a reader can check a place
against the database it names.

The `sha256` column is the digest of the donor file as it was fetched. The
same digest is written in the `provenance` section of each table, so the
loader can tell which donor a table was cut from.
