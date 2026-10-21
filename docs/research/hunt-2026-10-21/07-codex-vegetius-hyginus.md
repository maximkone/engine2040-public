# The codex numbers, with the passage behind each one (2026-10-21)

Hunter 0. Not a steal from GitHub: a steal from **public-domain primary sources**,
verified live on 2026-10-21, so `content/rome/*.toml` can carry a `ref` that a
human can check. Our tables refuse an entry with a number and no `ref` (row ROM2),
so this file is the raw material for those `ref` fields.

Two sources, both public domain, both read on the Latin Wikisource and the Latin
Library on 2026-10-21:

| id | work | where | licence |
|---|---|---|---|
| `VEG` | Vegetius, *De re militari* (Epitoma rei militaris), Liber I | <https://www.thelatinlibrary.com/vegetius1.html> (book index: `vegetius.html`) | public domain |
| `PHG` | Pseudo-Hyginus, *De munitionibus castrorum* | <https://la.wikisource.org/wiki/De_munitionibus_castrorum> (the Latin Library copy: `thelatinlibrary.com/hyginus/hyginus6.shtml`) | public domain |

**The machine-readable carrier** (found by hunter 5, verified today): the corpus
`cltk/lat_text_latin_library` (28 stars, Public Domain Mark 1.0) holds the same
texts as plain UTF-8 with the numbering already in the file:
`hyginus/hyginus6.txt` (22.6K, paragraphs `1.` to `58.`) and
`vegetius1.txt` (32.8K, chapters in Roman numerals `XXI.`, `XXIIII.`). The
paragraph numbers were checked against the passages below and they match, so the
codex splitter is a line regex and not an XML walk. Fetch:
`https://raw.githubusercontent.com/cltk/lat_text_latin_library/master/<path>`.

Unit: **1 Roman foot = 296 mm** (the value row ROM2 already uses; 5 feet = 1
passus, 1000 passus = 1 Roman mile = 480 ft).

## 1. The camp (ROM2 `camp.toml`, ROM5 `rome.camp`)

| # | number | value | source | Latin |
|---|---|---|---|---|
| 1 | ditch width, minimum | 5 ft = **1480 mm** | `PHG` 49 | "Quibus latitudo dari debeat ad minimum pedum quinque; altum pedes tres." |
| 2 | ditch depth | 3 ft = **888 mm** | `PHG` 49 | same line |
| 3 | ditch shape | two species: *fastigata* (sloped sides) or *punica* (vertical outer face) | `PHG` 49 | "cuius species est fasigata vel punica" |
| 4 | rampart (*vallum*) width | 8 ft = **2368 mm** | `PHG` 50 | "Sufficit latum pedes VIII, altum pedes VI" |
| 5 | rampart height | 6 ft = **1776 mm** | `PHG` 50 | same line |
| 6 | clear space outside the ditch at the gates | 60 ft = **17760 mm** | `PHG` 49 | "Regressis pedibus exterius sexaginta per latitudinem portarum" |
| 7 | via sagularis (the broad central street) width | 30 ft = **8880 mm**; 40 ft = 11840 mm with five legions | `PHG` 20 | "Viae sagulariae XXX pedes sufficient; [si] quinque legiones fuerint, XL pedes latitudinis accipere debebit." |
| 8 | via vicenaria width | 20 ft = **5920 mm** | `PHG` 13 | "Viae vicenariae ideo dantur percurrentes proxime sagularem" |
| 9 | via principalis / via quintana width | 40 ft = 11840 mm; 50 ft when the fifth gate stands there | `PHG` 17 | "accipere debet latitudinem pedum XL; si portae ibi datae fuerint, L pedes accipiet" |
| 10 | camp shape (tertiata) | 800 x 600 ft = **236.8 m x 177.6 m** | `PHG` 21 | "Castra ... tertiata esse debebunt, ut puta longum pedes DCCC, latum pedes CDC." (the transcription carries OCR noise, `CI)CI)CCCC` = DCCC and `CI)DC` = CDC) |
| 11 | other camp shapes | *classica* (longer) and *dictatio* (the one next to the square) | `PHG` 21 | "Si longiora fuerint, classica dicintur ... si latiora fuerint, proxime quadraturam sequens est dictatio." |
| 12 | the camp is one third of the ground it needs | the rule that sets the shape | `PHG` 21 | "in quantum fieri potuerit, tertiata esse debebunt, ut flatus aurae aestus exercitus leniat" |
| 13 | the square camp | 1000 passus a side = **480 ft = 142.08 m** | `PHG` 21 (*quadratura*) + the standard metation rule | "proxime quadraturam" |
| 14 | gate rule | the praetorian gate always faces the enemy; the tenth gate stands on the highest point | `PHG` 56 | "porta praetoria semper hostem spectare debet ... porta decimana eminentissimo loco constituitur" |
| 15 | ground rules for siting | rising ground, then plain, then hill, then mountain, then forced ground | `PHG` 56 | "primum locum habent quae ex campo in eminentiam leniter attolluntur ... tertium in colle, quartum in monte, quintum in loco necessario" |
| 16 | the groma (surveyor's cross) at the camp entrance | the layout is drawn with a groma and a circinus, not by eye | `PHG` 12 and 55 | "gromae locus appellatur ... groma superponetur" / "adaperto circino ad cardinem portae" |
| 17 | the augmented wall | when the rampart cannot be built, four lines of armed men hold the wall instead | `PHG` 52 | "armorum ordinibus IIII castra muniunt, ut per singulos ordines vigiliae crebrius ponantur" |
| 18 | the fortified camp ditch (hasty and regular) | hasty ditch 9 x 7 ft = 2664 x 2072 mm; regular ditch 12 ft wide, 9 ft deep (to the line) = 3552 x 2664 mm; turf wall 3 ft high with 4 ft of earth on top, so 13 ft overall = 3848 mm | `VEG` 1.24 | "tumultuaria fossa fit lata pedes nouem et alta pedes VII ... legitima fossa ... XII pedes lata sit et alta sub linea ... pedes nouem ... terra congeritur et crescit in altum IIII pedes. Sic fit, ut sit XIII alta" |

Note the two ditch figures do not agree, and that is the point: `PHG` 49 gives
the **marching** camp (5 x 3 ft, one night's work) and `VEG` 1.24 gives the
**fortified** camp (9 x 7 ft hasty, 12 x 9 ft regular). Row ROM2 asks for a
"one-night ditch reads 1480 mm wide and 888 mm deep", which is `PHG` 49 exactly.
The table should therefore carry a `threat` key: `march` -> 5 x 3, `hasty` ->
9 x 7, `regular` -> 12 x 9.

**The corpus read on 2026-10-21 (HX6) puts the fortified works in book I
chapter `XXIIII`, not `XXI`.** `vegetius1.txt` carries 27 chapters; chapter
`XXIIII` ("Castrorum autem diuersa triplexque munitio est") holds the turf wall
of three feet, the earth raised four feet so the wall stands thirteen, the
*tumultuaria* ditch nine by seven and the *legitima* ditch twelve by nine.
Chapter `XXI` only says a *tiro* must learn the camp works. So the refs of this
file are corrected to `VEG 1.24` where they said `VEG 1.21`.

Three numbers of this file are **not in the corpus** and stay out of the check:

- the *via vicenaria* width of 20 ft: paragraph 13 ("Viae vicenariae ideo
  dantur percurrentes proxime sagularem") carries no number at all;
- the camp shape of 800 x 600 ft: paragraph 21 reads `longum pedes CI)CI)CCCC,
  latum pedes CI)DC`, and no rule turns that noise into DCCC and CDC;
- the square camp of 1000 passus a side: paragraph 21 names the square
  ("proxime quadraturam") but gives no size for it.

## 2. The line and the drill (ROM2 `line.toml`, LEG3, LEG4, ROM4)

| # | number | value | source | Latin |
|---|---|---|---|---|
| 19 | one cohort's frontage | 30 ft = **8880 mm** of frontage per 720 ft (213.12 m) of depth | `PHG` 2 | "XXX enim pedes per DCCXX cohors una occupat" |
| 20 | the first cohort takes double frontage | 120 ft at the standards (35.52 m), 360 ft at the rear (106.68 m); or 180 ft / 240 ft | `PHG` 3 | "duplam pedaturam accipiet, ut puta signis pedes CXX, tabulino pedes CCCLX, vel signis CLXXX, tabulino pedes CCXL" |
| 21 | the hemistrigium (half a century front) | the unit the frontage is built from: 30 ft of frontage = 10 files of 3 ft = 1 hemistrigium | `PHG` 1, 36 | "quod ad latitudinem hemistrigii pedum XXX attinet ... quo VI hemistrigia nascantur" |
| 22 | the drill: the three forms of the line | (1) a single extended line, straight, no bends or curves, every man an equal lawful distance from his neighbour; (2) the line doubled on the instant; (3) a square line, then changed into the *trigonum*, the wedge they call the *cuneus* | `VEG` 1.26 | "ut aequali legitimoque spatio miles distet a milite. Tunc praecipiendum, ut subito duplicent aciem ... Tertio praecipiendum, ut quadratam aciem repente constituant, quo facto in trigonum, quem cuneum uocant, acies ipsa mutanda est." |
| 23 | the drill: circles | *orbes*, the circle formation, for when the enemy breaks the line | `VEG` 1.26 | "Iubetur etiam, it instruant orbes, quo genere, cum uis hostium interruperit aciem, resisti ab exercitatis militibus consueuit" |
| 24 | the drill: how often, how far | **three times a month** the cavalry and the infantry march out under arms (*ambulatum*); the infantry go **10 000 paces** (10 Roman miles = 4800 ft = 1420.8 m) armed and fully equipped at a military pace and back, part of it at a faster pace; the drill is done on slopes and steep ground, the line going up and down | `VEG` 1.27 | "ut ter in mense tam equites quam pedites educantur ambulatum ... Decem milia passuum armati instructique omnibus telis pedites militari gradu ire ac redire iubebantur in castra, ita ut aliquam itineris partem cursu alacriore conficerent ... Non solunt autem in campis, sed etiam in cliuosis et arduis locis et descendere et ascendere utraque acies cogebatur" |
| 25 | the camp, its parts | praetorium, praetentura, auguratorium, tribunal, the scammum of the envoys, the porta praetoria | `PHG` 10-14 | "In aris institutis in praetorii parte ima, auguratorium parte dextra ... parte laeva tribunal statuitur" |
| 26 | the cavalry frontage | each horseman takes three feet; the standard-bearer of a squadron takes more room | `PHG` 16 | "In eo pede praefectus alae pedaturam accipiat ... alioquin singuli binos pedes et semisses acciperent" |

## 3. How to use this file

1. Every number above becomes one row in `content/rome/camp.toml` or
   `content/rome/line.toml`, with `ref = "PHG 49"` or `ref = "VEG 1.24"`.
2. The text is fetched from the machine-readable corpus
   (`cltk/lat_text_latin_library`, Public Domain Mark 1.0) and split on the
   numbering the file already carries: `hyginus/hyginus6.txt` paragraphs
   `1.` to `58.`, `vegetius1.txt` chapters in Roman numerals.
3. **Store the passage's text hash beside the ref.** Then a re-numbered edition,
   a corrected transcription or a substituted text **fails the lint** instead
   of silently changing a ditch from 5 ft to 4 ft. This is the rule that makes
   "realism is checkable, not a vibe" true.
4. A cook script prints the numbers beside the tables, so a change in the text
   shows up as a diff. That is the proof the codex rows (CDX1, CDX2) want.
5. Where a figure disagrees (row 1 vs row 18), the table keeps **both** with a
   `threat` or `kind` key. A rule that hides the disagreement is a bug.
6. The passages are short and quotable, so `docs/design/legionary/` can carry
   the Latin next to the picture of the camp.
7. The splitter can be made scheme-driven instead of per-work: the CTS/CapiTainS
   reader in `tianlynn/tei-cts` (MIT, read 2026-10-21) walks a TEI edition with
   the edition's own `cRefPattern` XPath, and its **trust manifest** (blocking
   checks and advisory checks with the thresholds in one commit) is the shape a
   codex source registry wants. Its fixtures are CC BY-SA Perseus and stay out
   of our tree; the machinery is the steal.

## 4. The check (HX6)

`node tools/codex-check.mjs` reads the corpus and checks every number of
`content/rome/camp.toml` and `content/rome/line.toml` against the passage it
names. The corpus is fetched once into `content/local/codex/` (gitignored, never
committed), so the second run is offline and byte-identical:

```
node tools/codex-check.mjs --fetch   # fetch the two files into the cache
node tools/codex-check.mjs           # check the tables, offline
```

Each table carries one `codex.*` section per number: the entry and the key that
hold the number, the passage `ref`, the spelling the source writes and the hash
of that passage's text. The hash sits beside the ref in its own section and not
in the entry itself: the Rome loader (`crates/rome/src/lib.rs`) refuses any key
but `ref` and the ones its typed form reads, so a `hash` key inside an entry
would break the loader. The loader therefore reads the same numbers as before,
and the proof never touches a number the game reads.

The 25 numbers the check reads, with the passage and the spelling:

| # | number | ft | ref | the source writes |
|---|---|---|---|---|
| 1 | ditch width | 5 | `PHG 49` | quinque |
| 2 | ditch depth | 3 | `PHG 49` | tres |
| 3 | hasty ditch width | 9 | `VEG 1.24` | nouem |
| 4 | hasty ditch depth | 7 | `VEG 1.24` | VII |
| 5 | regular ditch width | 12 | `VEG 1.24` | XII |
| 6 | regular ditch depth | 9 | `VEG 1.24` | nouem |
| 7 | turf wall height | 3 | `VEG 1.24` | tribus |
| 8 | earth raised on the wall | 4 | `VEG 1.24` | IIII |
| 9 | the wall stands | 13 | `VEG 1.24` | XIII |
| 10 | rampart width | 8 | `PHG 50` | VIII |
| 11 | rampart height | 6 | `PHG 50` | VI |
| 12 | via principalis width | 60 | `PHG 14` | LX |
| 13 | via quintana width | 40 | `PHG 17` | XL |
| 14 | via quintana with the fifth gate | 50 | `PHG 17` | L |
| 15 | via sagularis width | 30 | `PHG 20` | XXX |
| 16 | via sagularis width with five legions | 40 | `PHG 20` | XL |
| 17 | clear space outside the ditch at the gates | 60 | `PHG 49` | sexaginta |
| 18 | one cohort's frontage | 30 | `PHG 2` | XXX |
| 19 | one cohort's depth | 720 | `PHG 2` | DCCXX |
| 20 | the first cohort at the standards | 120 | `PHG 3` | CXX |
| 21 | the first cohort at the rear | 360 | `PHG 3` | CCCLX |
| 22 | the first cohort at the standards, other reading | 180 | `PHG 3` | CLXXX |
| 23 | the first cohort at the rear, other reading | 240 | `PHG 3` | CCXL |
| 24 | the hemistrigium's width | 30 | `PHG 1` | XXX |
| 25 | the cavalry frontage | 3 | `PHG 16` | terni |

Two of the 25 need a word:

- the via principalis of 60 ft is in none of the rows above; it is in `PHG` 14
  ("latitudine eadem, qua opus pedum LX"), and the camp table already named
  "Hyginus 14" for it;
- the hemistrigium's 30 ft is in `PHG` 1 ("quod ad latitudinem hemistrigii
  pedum XXX attinet"), not in `PHG` 2 and 36.

The two numbers of `line.toml` that the loader reads (the frontage and the rank
gap, Vegetius 3.14) are **not** in the 25: the corpus file of this row is book
I, and book III is not fetched.

## 5. What is still missing in the sources

- The *via sagularis* width for 5 legions (40 ft) is `PHG` 20; Vegetius 1.24
  gives the street plan from the other side. Read `VEG` 1.24 before the camp
  verb is written, so the two agree or the difference is named.
- `PHG` 1-20 give the layout by *metation* (survey), which needs the groma
  procedure of chapters 12 and 55 turned into a rule. That is the smallest
  piece of real algorithm in the whole hunt: lay the camp with a groma, from
  the praetorian gate outward, with the decima gate on the high point.
- The Roman foot itself is contested (295.7 mm vs 297.4 mm). Our 296 mm is a
  choice; the table should name it in one place, not in every row.
- The drill's 10 000 passus of row 24 is in `VEG` 1.27 ("Decem milia
  passuum") but stays out of the check: this file gives two passus rules that
  disagree (5 ft to a passus, and 1000 passus to a mile of 480 ft), and the
  4800 ft of row 24 follows the second. The drill row (LEG4) must pick one and
  say which.
