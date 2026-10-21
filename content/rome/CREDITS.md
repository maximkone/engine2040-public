# Rome credits

Every source the shipped Rome content uses, with the url it came from and the
licence it states. `node tools/codex/credits.mjs` writes this file; do not edit it
by hand.

The raw fetches stay local: `content/local/` is gitignored. A file that says
`source table` is a source the local cache does not hold: its url and its licence
come from `tools/codex/sources.json`.

Our own plan (`docs/plan/rome.md`) is not a source and is not credited here.

A donor file a Rome table carries in its `provenance` section is credited from
that section: its url, its licence and its sha256.

## Art Institute of Chicago open access search (`artic`)

- licence: CC0
- used by: `boards/dolabra.json`, `boards/gladius.json`, `boards/helmet.json`, `boards/lorica.json`, `boards/scutum.json`

- `artic` - url <https://api.artic.edu/api/v1/artworks/search?q={id}> - licence CC0 - from the source table

## Wikimedia Commons Trajan's Column files (`commons`)

- licence: per file, as the file page states
- used by: `column.json`, `kit.toml`

- `commons/category-trajans-column.json` - url <https://commons.wikimedia.org/w/api.php?action=query&titles=Category%3ATrajan's%20Column&prop=imageinfo&iiprop=url%7Cextmetadata%7Cmime&iilimit=500&format=json> - licence per file, as the file page states - from the source table

## `dare`

- licence: Apache-2.0
- used by: `fortifications.toml`

- `dare/fortifications.geojson` - url <https://github.com/johaahlf/dare/blob/master/fortifications.geojson> - licence Apache-2.0 - sha256 `f20449fc3a72594d10a88294d0d2c8d3c6b27312a8d6691ab53701674a0bc98d` - from the Rome file

## Project Gutenberg plain text (`gutenberg`)

- licence: US public domain
- used by: `camp.toml`, `kit.toml`, `line.toml`

- `gutenberg/polybius-histories-vol1.txt` - url <https://www.gutenberg.org/ebooks/44125.txt.utf-8> - licence US public domain - from the source table
- `gutenberg/polybius-histories-vol2.txt` - url <https://www.gutenberg.org/ebooks/44126.txt.utf-8> - licence US public domain - from the source table
- `gutenberg/vegetius-de-re-militari.txt` - url <https://www.gutenberg.org/ebooks/{id}.txt.utf-8> - licence US public domain - from the source table

## Itiner-e Roman road network (`itinere`)

- licence: CC BY 4.0
- used by: `lexicon.json`, `places.json`, `roads.json`

- `itinere/itinere_roads.geojson` - url <https://zenodo.org/records/17122148/files/itinere_roads.geojson> - licence CC BY 4.0 - sha256 `4f8f5cf99f387bab2c509b1bf50143e9a3117691e316f2f46a9bb1d3d4b1cc34` - from the Rome file

## Met Museum open access object list (`met`)

- licence: CC0
- used by: `boards/dolabra.json`, `boards/gladius.json`, `boards/helmet.json`, `boards/lorica.json`, `boards/scutum.json`

- `met` - url <https://media.githubusercontent.com/media/metmuseum/openaccess/master/MetObjects.csv> - licence CC0 - from the source table

## Perseus canonical Latin literature (`perseus`)

- licence: CC BY-SA 4.0
- used by: `lexicon.json`

- `perseus/phi0448.phi001.perseus-eng2.xml` - url <https://raw.githubusercontent.com/PerseusDL/canonical-latinLit/master/data/phi0448/phi001/phi0448.phi001.perseus-eng2.xml> - licence CC BY-SA 4.0 - from the source table
- `perseus/phi0448.phi001.perseus-lat2.xml` - url <https://raw.githubusercontent.com/PerseusDL/canonical-latinLit/master/data/phi0448/phi001/phi0448.phi001.perseus-lat2.xml> - licence CC BY-SA 4.0 - from the source table

## Pleiades ancient world places (`pleiades`)

- licence: CC BY 3.0
- used by: `lexicon.json`, `places.json`, `roads.json`

- `pleiades/pleiades-places-latest.json.gz` - url <https://atlantides.org/downloads/pleiades/json/pleiades-places-latest.json.gz> - licence CC BY 3.0 - sha256 `d9c544b2fa8a1aa79f6b3195685f27aa65625a981b9c95b1022c89d7bd61f707` - from the Rome file

## `urbesetorbis`

- licence: BSD-2-Clause
- used by: `places.toml`

- `urbesetorbis/public/downloads/301023_AD130.csv` - url <https://github.com/Diasito/urbesetorbis/blob/main/public/downloads/301023_AD130.csv> - licence BSD-2-Clause - sha256 `b9c98bf71389b3fee4e17280cb70cb319a009342e7c6915afb2dc9c0f6b41315` - from the Rome file

## Wikidata query of the Roman legions and their places (`wikidata`)

- licence: CC0
- used by: `lexicon.json`, `places.json`, `roads.json`

- `wikidata/battles.json` - url <https://query.wikidata.org/sparql?query=SELECT%20%3Fitem%20%3FitemLabel%20%3Fkind%20%3Fcoord%20%3Fdate%20WHERE%20%7B%20%3Fitem%20wdt%3AP31%20wd%3AQ178561%20.%20BIND(wd%3AQ178561%20AS%20%3Fkind)%20%3Fitem%20wdt%3AP625%20%3Fcoord%20.%20OPTIONAL%20%7B%20%3Fitem%20wdt%3AP585%20%3Fdate%20.%20%7D%20SERVICE%20wikibase%3Alabel%20%7B%20bd%3AserviceParam%20wikibase%3Alanguage%20%22en%22.%20%7D%20%7D&format=json> - licence CC0 - sha256 `eec1913cb489b22822aca68f2d5fdbbffffe94dc3e102a9de8cf388998ff33df` - from the Rome file
- `wikidata/legions-and-places.json` - url <https://query.wikidata.org/sparql?query=SELECT%20%3Fitem%20%3FitemLabel%20%3Fkind%20%3Fplace%20%3FplaceLabel%20%3FplaceCoord%20WHERE%20%7B%20%3Fitem%20wdt%3AP31%20wd%3AQ163323%20.%20BIND(wd%3AQ163323%20AS%20%3Fkind)%20%3Fitem%20wdt%3AP276%20%3Fplace%20.%20%3Fplace%20wdt%3AP625%20%3FplaceCoord%20.%20SERVICE%20wikibase%3Alabel%20%7B%20bd%3AserviceParam%20wikibase%3Alanguage%20%22en%22.%20%7D%20%7D&format=json> - licence CC0 - sha256 `e4dba03f8c0f7e24da755881c21e52093b9644efcb2f0ccf9ccc26bfebeb76a2` - from the Rome file

## cltk/lat_text_latin_library, the Latin texts of the codex (`lat_text_latin_library`)

- licence: Public Domain Mark 1.0
- used by: `camp.toml`, `line.toml`

- `lat_text_latin_library/hyginus6.txt` - url <https://raw.githubusercontent.com/cltk/lat_text_latin_library/master/hyginus/hyginus6.txt> - licence Public Domain Mark 1.0 - from the corpus
- `lat_text_latin_library/vegetius1.txt` - url <https://raw.githubusercontent.com/cltk/lat_text_latin_library/master/vegetius1.txt> - licence Public Domain Mark 1.0 - from the corpus

## Refs that name no source

A ref the codex does not hold. It is reported, never guessed and never credited.
A ref that cites our own plan is not listed: the plan is not a source.

- `fortifications.toml`: WGS84: the length of one degree of latitude at 45 degrees north is 111132 m
- `places.toml`: WGS84: the length of one degree of latitude at 45 degrees north is 111132 m
