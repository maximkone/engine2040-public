//! `audio` pack: the authored cue sheet and one musical anchor.
//!
//! Not loaded by default: a session pays for these verbs only when it asks.
//! The data owners are `engine-audio` (`cue_sheet`, `cue_anchor`): this pack
//! validates and queries what those crates own, and copies no rule.
//!
//! - `audio.cues` (observe): rows in, then the validated sheet's digest, the
//!   rows sounding at one tick, the four lane trims there (duck applied) and
//!   the gap report (ticks with neither a music nor an ambience bed).
//! - `audio.cue.resolve` (observe): anchor plus offset plus timebase in, the
//!   absolute tick out, or the typed `before_zero` refusal. This is how an
//!   author checks a downbeat cue without loading a sheet.
//!
//! Units: ticks are fixed steps, gains whole dB and offsets whole beats or
//! whole milliseconds. Limits: [`MAX_CUES`] rows and a gap span of
//! [`MAX_GAP_SPAN_TICKS`](engine_audio::cue_sheet::MAX_GAP_SPAN_TICKS); the
//! answer lists at most [`MAX_GAP_TICKS_OUT`] gap ticks and says when it cut
//! the list. Determinism: the same rows and tick give the same digest, picks,
//! trims and gaps; no clock, RNG or file IO.

use engine_audio::cue::CueId;
use engine_audio::cue_anchor::{resolve, Anchor, AnchorError, CueOffset, CueTimebase};
use engine_audio::cue_sheet::{BusGains, Cue, CueBus, CueRow, CueSheet, MAX_CUES};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::Value;

use super::ui::{field_refusal, read_i32, read_u32};
use super::{bad_args, PackDef, PackError, VerbSpec};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "audio";

/// Verb: validate cue rows and answer one tick.
pub const TOOL_AUDIO_CUES: &str = "audio.cues";

/// Verb: resolve one anchor plus offset to a tick.
pub const TOOL_AUDIO_CUE_RESOLVE: &str = "audio.cue.resolve";

/// Most gap ticks one answer lists; `gap_count` still carries the true count.
pub const MAX_GAP_TICKS_OUT: usize = 256;

/// The audio pack: two verbs, not loaded by default.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "audio: validate an authored cue sheet (rows, lanes, duck, loops) and \
         answer one tick, or resolve a downbeat anchor to a tick, in memory",
        false,
    );
    pack.add(VerbSpec::of::<AudioCuesTool>(
        TOOL_AUDIO_CUES,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<CueResolveTool>(
        TOOL_AUDIO_CUE_RESOLVE,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

/// One authored cue row as the artist writes it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde", deny_unknown_fields)]
#[schemars(crate = "rmcp::schemars")]
pub struct CueRowArgs {
    /// Fixed tick the row starts on.
    pub tick: u64,
    /// Sound to play: an index into the caller's cue bank.
    pub sound: u16,
    /// Lane: `music`, `sfx`, `voice` or `ambience`.
    pub bus: String,
    /// Row gain in dB, `-80..=6`.
    pub gain_db: i16,
    /// Lower the music lane by 12 dB while this row sounds; music only.
    #[serde(default)]
    pub duck: bool,
    /// Loop until this tick (exclusive); omitted plays once at `tick`.
    #[serde(default)]
    pub loop_until: Option<u64>,
    /// Playback priority, higher wins.
    #[serde(default)]
    pub priority: u8,
}

/// Args for `audio.cues`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde", deny_unknown_fields)]
#[schemars(crate = "rmcp::schemars")]
pub struct AudioCuesArgs {
    /// Authored rows, in authored order.
    pub rows: Vec<CueRowArgs>,
    /// Tick to answer for.
    pub at: u64,
}

/// One parsed row as the sheet holds it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CueOut {
    /// Start tick.
    pub tick: u64,
    /// Sound index.
    pub sound: u16,
    /// Parsed lane name.
    pub bus: String,
    /// Row gain in dB.
    pub gain_db: i16,
    /// True when the row ducks the music lane.
    pub duck: bool,
    /// Loop end tick (exclusive), when the row loops.
    pub loop_until: Option<u64>,
    /// Playback priority.
    pub priority: u8,
}

/// The four lane trims at one tick, duck already applied.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GainsOut {
    /// Music lane trim, dB.
    pub music_db: i16,
    /// Effects lane trim, dB.
    pub sfx_db: i16,
    /// Voice lane trim, dB.
    pub voice_db: i16,
    /// Ambience lane trim, dB.
    pub ambience_db: i16,
}

/// Ticks in the scanned span with no music or ambience bed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GapOut {
    /// First tick of the scanned span.
    pub first_tick: u64,
    /// Exclusive end of the scanned span.
    pub last_tick: u64,
    /// Number of bedless ticks in the whole span.
    pub gap_count: usize,
    /// The first [`MAX_GAP_TICKS_OUT`] bedless ticks, ascending.
    pub gap_ticks: Vec<u64>,
    /// True when `gap_ticks` stopped short of `gap_count`.
    pub truncated: bool,
}

/// `audio.cues` result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct AudioCuesOutput {
    /// FNV digest of the validated sheet, 16 lowercase hex chars.
    pub digest: String,
    /// Tick the query answered for.
    pub at: u64,
    /// Rows sounding at `at`, in the sheet's stable order.
    pub active: Vec<CueOut>,
    /// Lane trims at `at`.
    pub gains: GainsOut,
    /// The gap report over the sheet's span.
    pub gaps: GapOut,
}

/// The caller's grid for one anchor: all four rates are at least 1.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde", deny_unknown_fields)]
#[schemars(crate = "rmcp::schemars")]
pub struct TimebaseArgs {
    /// Ticks in one minute.
    pub ticks_per_minute: u32,
    /// Beats per minute.
    pub bpm: u32,
    /// Beats in one bar (used by `downbeat`).
    pub beats_per_bar: u32,
    /// Milliseconds in one tick (used by an `ms` offset).
    pub ms_per_tick: u32,
}

/// Args for `audio.cue.resolve`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde", deny_unknown_fields)]
#[schemars(crate = "rmcp::schemars")]
pub struct CueResolveArgs {
    /// Anchor: `"segment_start"`, or exactly one of `beat`, `downbeat`,
    /// `marker` as a whole-number index.
    pub anchor: Value,
    /// Offset: exactly one of `beats` or `ms` as a whole (signed) number;
    /// omitted means no shift.
    #[serde(default)]
    pub offset: Option<Value>,
    /// The caller's grid.
    pub timebase: TimebaseArgs,
}

/// `audio.cue.resolve` result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CueResolveOutput {
    /// Absolute tick the anchor plus offset lands on.
    pub tick: u64,
    /// Anchor as read back (`Beat(2)`, `Downbeat(1)`, ...).
    pub anchor: String,
    /// Offset as read back (`Beats(-1)`, `Ms(250)`, ...).
    pub offset: String,
    /// Ticks in one beat under this timebase.
    pub ticks_per_beat: u64,
    /// Ticks in one bar under this timebase.
    pub ticks_per_bar: u64,
}

/// `audio.cues` handler: validate the rows, answer one tick.
fn audio_cues(
    service: &crate::mcp::McpServer,
    args: &AudioCuesArgs,
) -> Result<AudioCuesOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_AUDIO_CUES, |_state| run_cues(args))
}

/// `audio.cue.resolve` handler: resolve one anchor plus offset.
fn cue_resolve(
    service: &crate::mcp::McpServer,
    args: &CueResolveArgs,
) -> Result<CueResolveOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_AUDIO_CUE_RESOLVE, |_state| run_resolve(args))
}

crate::e2_tool!(
    /// `audio.cues` tool: validate rows and answer one tick.
    AudioCuesTool,
    name = TOOL_AUDIO_CUES,
    description = "Validate authored cue rows (tick, sound, bus music/sfx/voice/ambience, gain_db -80..=6, duck on music only, loop_until past tick, priority) and answer one tick: sheet digest, rows sounding there, the four lane trims with duck applied and the bedless-tick gap report. Overlapping loops on one bus refuse.",
    readonly = true,
    args = AudioCuesArgs,
    output = AudioCuesOutput,
    invoke = audio_cues
);

crate::e2_tool!(
    /// `audio.cue.resolve` tool: anchor plus offset to one tick.
    CueResolveTool,
    name = TOOL_AUDIO_CUE_RESOLVE,
    description = "Resolve a cue anchor (beat, downbeat, marker or segment_start) plus an offset (beats or ms) on a timebase (ticks_per_minute, bpm, beats_per_bar, ms_per_tick) to an absolute tick. Downbeat n is bar n, meter aware. An offset landing before tick 0 refuses with code before_zero.",
    readonly = true,
    args = CueResolveArgs,
    output = CueResolveOutput,
    invoke = cue_resolve
);

/// The cue sheet itself: rows in, one tick answered.
fn run_cues(args: &AudioCuesArgs) -> Result<AudioCuesOutput, crate::mcp::McpToolError> {
    if args.rows.len() > MAX_CUES {
        return Err(field_refusal(
            "rows",
            &format!("0..={MAX_CUES} rows"),
            "split the sheet into two scenes",
        ));
    }
    let rows = read_rows(&args.rows)?;
    let sheet = CueSheet::new(&rows).map_err(cue_refusal)?;
    let active = sheet.cues_at(args.at).into_iter().map(cue_out).collect();
    let gaps = sheet.gap_report().map_err(cue_refusal)?;
    Ok(AudioCuesOutput {
        digest: format!("{:016x}", sheet.digest()),
        at: args.at,
        active,
        gains: gains_out(sheet.bus_gains_at(args.at)),
        gaps: GapOut {
            first_tick: gaps.first_tick,
            last_tick: gaps.last_tick,
            gap_count: gaps.ticks.len(),
            gap_ticks: gaps.ticks.iter().take(MAX_GAP_TICKS_OUT).copied().collect(),
            truncated: gaps.ticks.len() > MAX_GAP_TICKS_OUT,
        },
    })
}

/// Read the caller's rows into the crate's row type, refusing a bad lane with
/// the crate's own legal-value sentence.
fn read_rows(rows: &[CueRowArgs]) -> Result<Vec<CueRow>, crate::mcp::McpToolError> {
    let mut out = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let bus = CueBus::from_name(&row.bus).ok_or_else(|| {
            field_refusal(
                &format!("rows[{index}].bus"),
                "one of music, sfx, voice, ambience",
                "rename the row's bus",
            )
        })?;
        out.push(CueRow {
            tick: row.tick,
            sound: CueId(row.sound),
            bus: bus.name(),
            gain_db: row.gain_db,
            duck: row.duck,
            loop_until: row.loop_until,
            priority: row.priority,
        });
    }
    Ok(out)
}

/// One parsed cue as the wire row.
fn cue_out(cue: &Cue) -> CueOut {
    CueOut {
        tick: cue.tick,
        sound: cue.sound.0,
        bus: cue.bus.name().to_owned(),
        gain_db: cue.gain_db,
        duck: cue.duck,
        loop_until: cue.loop_until,
        priority: cue.priority,
    }
}

/// The four lane trims as the wire row.
fn gains_out(gains: BusGains) -> GainsOut {
    GainsOut {
        music_db: gains.music_db,
        sfx_db: gains.sfx_db,
        voice_db: gains.voice_db,
        ambience_db: gains.ambience_db,
    }
}

/// Map a crate cue-sheet error onto a door refusal; the text already names
/// the row, the band and the recovery.
fn cue_refusal(error: engine_audio::cue_sheet::CueSheetError) -> crate::mcp::McpToolError {
    bad_args(format!("{error}"))
}

/// Map a crate anchor error onto its typed door refusal.
fn anchor_refusal(error: AnchorError) -> crate::mcp::McpToolError {
    match error {
        AnchorError::BeforeZero { .. } => {
            crate::mcp::McpToolError::new("before_zero", error.to_string())
        }
        AnchorError::BadTimebase { .. } => {
            crate::mcp::McpToolError::new("bad_timebase", error.to_string())
        }
    }
}

/// Read one anchor value.
fn read_anchor(value: &Value) -> Result<Anchor, crate::mcp::McpToolError> {
    if value.as_str() == Some("segment_start") {
        return Ok(Anchor::SegmentStart);
    }
    let map = value
        .as_object()
        .filter(|map| map.len() == 1)
        .ok_or_else(|| {
            field_refusal(
                "anchor",
                "segment_start, or exactly one of beat, downbeat, marker",
                "write \"segment_start\" or {\"downbeat\":2}",
            )
        })?;
    let (kind, body) = map.iter().next().ok_or_else(|| {
        field_refusal(
            "anchor",
            "segment_start, or exactly one of beat, downbeat, marker",
            "write {\"beat\":0}",
        )
    })?;
    let index = read_u32(body, &format!("anchor.{kind}"), 0, u32::MAX)?;
    match kind.as_str() {
        "beat" => Ok(Anchor::Beat(index)),
        "downbeat" => Ok(Anchor::Downbeat(index)),
        "marker" => Ok(Anchor::Marker(index)),
        other => Err(field_refusal(
            &format!("anchor.{other}"),
            "beat, downbeat or marker",
            "rename the anchor key",
        )),
    }
}

/// Read one offset value; `None` means no shift.
fn read_offset(value: Option<&Value>) -> Result<CueOffset, crate::mcp::McpToolError> {
    let Some(value) = value else {
        return Ok(CueOffset::Beats(0));
    };
    let map = value
        .as_object()
        .filter(|map| map.len() == 1)
        .ok_or_else(|| {
            field_refusal(
                "offset",
                "exactly one of beats, ms",
                "write {\"beats\":-1} or drop the offset",
            )
        })?;
    let (kind, body) = map
        .iter()
        .next()
        .ok_or_else(|| field_refusal("offset", "exactly one of beats, ms", "write {\"ms\":250}"))?;
    let field = format!("offset.{kind}");
    match kind.as_str() {
        "beats" => Ok(CueOffset::Beats(read_i32(
            body,
            &field,
            i64::from(i32::MIN),
            i64::from(i32::MAX),
        )?)),
        "ms" => Ok(CueOffset::Ms(read_i32(
            body,
            &field,
            i64::from(i32::MIN),
            i64::from(i32::MAX),
        )?)),
        other => Err(field_refusal(
            &format!("offset.{other}"),
            "beats or ms",
            "rename the offset key",
        )),
    }
}

/// Resolve the anchor and offset on the caller's grid.
fn run_resolve(args: &CueResolveArgs) -> Result<CueResolveOutput, crate::mcp::McpToolError> {
    let anchor = read_anchor(&args.anchor)?;
    let offset = read_offset(args.offset.as_ref())?;
    let timebase = CueTimebase::new(
        args.timebase.ticks_per_minute,
        args.timebase.bpm,
        args.timebase.beats_per_bar,
        args.timebase.ms_per_tick,
    )
    .map_err(anchor_refusal)?;
    let tick = resolve(anchor, offset, &timebase).map_err(anchor_refusal)?;
    Ok(CueResolveOutput {
        tick,
        anchor: anchor.to_string(),
        offset: offset.to_string(),
        ticks_per_beat: timebase.ticks_per_beat(),
        ticks_per_bar: timebase.ticks_per_bar(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::{PackRegistry, PACK_NAMES};
    use rmcp::serde_json::json;

    fn row(tick: u64, sound: u16, bus: &str, gain_db: i16) -> CueRowArgs {
        CueRowArgs {
            tick,
            sound,
            bus: bus.to_owned(),
            gain_db,
            duck: false,
            loop_until: None,
            priority: 0,
        }
    }

    fn sheet() -> AudioCuesArgs {
        let mut music = row(0, 1, "music", -12);
        music.duck = true;
        music.loop_until = Some(64);
        let mut room = row(0, 2, "ambience", -9);
        room.loop_until = Some(16);
        AudioCuesArgs {
            rows: vec![music, room, row(32, 3, "sfx", -3), row(32, 4, "voice", -6)],
            at: 32,
        }
    }

    #[test]
    fn cue_sheet_answers_one_tick_with_stable_digest_gains_and_gaps() {
        let args = sheet();
        let first = run_cues(&args).expect("the sheet validates");
        assert_eq!(first.digest.len(), 16);
        assert_eq!(
            first.active.len(),
            3,
            "music, sfx and voice sound at tick 32"
        );
        let lanes: Vec<&str> = first.active.iter().map(|cue| cue.bus.as_str()).collect();
        assert!(lanes.contains(&"music"));
        assert!(lanes.contains(&"sfx"));
        assert!(lanes.contains(&"voice"));
        assert!(
            !lanes.contains(&"ambience"),
            "the ambience loop ended at tick 16"
        );
        assert_eq!(first.gains.music_db, -24, "duck is 12 dB under music");
        assert_eq!(first.gains.sfx_db, -3);
        assert_eq!(first.gains.voice_db, -6);
        assert_eq!(first.gains.ambience_db, -9);
        // The music bed covers 0..64, the whole scanned span: no gaps.
        assert_eq!((first.gaps.first_tick, first.gaps.last_tick), (0, 64));
        assert_eq!(first.gaps.gap_count, 0);
        assert!(first.gaps.gap_ticks.is_empty());
        assert!(!first.gaps.truncated);

        let again = run_cues(&args).expect("the sheet validates twice");
        assert_eq!(again, first, "the same rows and tick give the same answer");

        let quiet = run_cues(&AudioCuesArgs {
            rows: args.rows,
            at: 8,
        })
        .expect("tick 8 answers");
        assert_eq!(quiet.active.len(), 2, "music and ambience only at tick 8");
        assert_eq!(quiet.gains.music_db, -24);

        // A music bed that stops at 16 leaves 16..401 bedless: the answer
        // lists the first 256 and still counts all 385.
        let mut short_bed = row(0, 1, "music", -12);
        short_bed.loop_until = Some(16);
        let gapped = run_cues(&AudioCuesArgs {
            rows: vec![short_bed, row(400, 3, "sfx", -3)],
            at: 400,
        })
        .expect("the gapped sheet validates");
        assert_eq!((gapped.gaps.first_tick, gapped.gaps.last_tick), (0, 401));
        assert_eq!(gapped.gaps.gap_count, 385);
        assert_eq!(gapped.gaps.gap_ticks.len(), MAX_GAP_TICKS_OUT);
        assert!(gapped.gaps.truncated);
        assert_eq!(gapped.gaps.gap_ticks.first().copied(), Some(16));
    }

    #[test]
    fn cue_sheet_refusals_name_row_field_band_and_recovery() {
        let mut args = sheet();
        args.rows[0].bus = "musics".to_owned();
        let refusal = run_cues(&args).expect_err("a typo'd lane refuses");
        assert!(
            refusal.message.contains("rows[0].bus"),
            "{}",
            refusal.message
        );
        assert!(
            refusal.message.contains("music, sfx, voice, ambience"),
            "{}",
            refusal.message
        );
        assert!(refusal.message.contains("recovery"), "{}", refusal.message);

        let mut args = sheet();
        args.rows[1].gain_db = 12;
        let refusal = run_cues(&args).expect_err("12 dB refuses");
        assert!(refusal.message.contains("-80..=6"), "{}", refusal.message);

        let mut args = sheet();
        args.rows[1].duck = true;
        let refusal = run_cues(&args).expect_err("duck outside music refuses");
        assert!(refusal.message.contains("duck"), "{}", refusal.message);
    }

    fn timebase() -> TimebaseArgs {
        TimebaseArgs {
            ticks_per_minute: 960,
            bpm: 120,
            beats_per_bar: 4,
            ms_per_tick: 10,
        }
    }

    #[test]
    fn resolve_is_meter_aware_and_counts_offsets() {
        let downbeat = run_resolve(&CueResolveArgs {
            anchor: json!({"downbeat": 2}),
            offset: Some(json!({"beats": -1})),
            timebase: timebase(),
        })
        .expect("bar 2 minus one beat is a tick");
        assert_eq!(downbeat.tick, 56);
        assert_eq!(downbeat.ticks_per_beat, 8);
        assert_eq!(downbeat.ticks_per_bar, 32);
        assert_eq!(downbeat.anchor, "Downbeat(2)");
        assert_eq!(downbeat.offset, "Beats(-1)");

        let start = run_resolve(&CueResolveArgs {
            anchor: json!("segment_start"),
            offset: Some(json!({"ms": 250})),
            timebase: timebase(),
        })
        .expect("250 ms is 25 ticks at 10 ms per tick");
        assert_eq!(start.tick, 25);

        let marker = run_resolve(&CueResolveArgs {
            anchor: json!({"marker": 480}),
            offset: None,
            timebase: timebase(),
        })
        .expect("a marker is tempo independent");
        assert_eq!(marker.tick, 480);
        assert_eq!(marker.offset, "Beats(0)");
    }

    #[test]
    fn resolve_refuses_before_zero_and_a_zero_timebase_with_typed_codes() {
        let refusal = run_resolve(&CueResolveArgs {
            anchor: json!({"beat": 0}),
            offset: Some(json!({"beats": -1})),
            timebase: timebase(),
        })
        .expect_err("a beat before tick 0 refuses");
        assert_eq!(refusal.code, "before_zero");
        assert!(refusal.message.contains("Beat(0)"), "{}", refusal.message);
        assert!(refusal.message.contains("Beats(-1)"), "{}", refusal.message);
        assert!(refusal.message.contains("recovery"), "{}", refusal.message);

        let refusal = run_resolve(&CueResolveArgs {
            anchor: json!("segment_start"),
            offset: None,
            timebase: TimebaseArgs {
                bpm: 0,
                ..timebase()
            },
        })
        .expect_err("bpm 0 refuses");
        assert_eq!(refusal.code, "bad_timebase");
        assert!(refusal.message.contains("bpm"), "{}", refusal.message);

        let refusal = run_resolve(&CueResolveArgs {
            anchor: json!({"bar": 1}),
            offset: None,
            timebase: timebase(),
        })
        .expect_err("an unknown anchor refuses");
        assert!(
            refusal.message.contains("anchor.bar"),
            "{}",
            refusal.message
        );
    }

    #[test]
    fn the_registry_serves_both_audio_verbs_and_neither_is_default_loaded() {
        let registry = PackRegistry::shipped();
        assert!(registry.errors().is_empty(), "{:?}", registry.errors());
        assert!(PACK_NAMES.contains(&PACK));
        assert!(!registry.default_loaded().contains(PACK));
        for name in [TOOL_AUDIO_CUES, TOOL_AUDIO_CUE_RESOLVE] {
            let verb = registry.verb(name).expect("the verb registers");
            assert_eq!(verb.pack(), PACK);
            assert_eq!(verb.card(), CapabilityCard::Observe);
            assert!(verb.readonly());
        }
    }
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 25,
    name: PACK,
    default_loaded: false,
    tools: &["audio.cues", "audio.cue.resolve"],
    build: pack,
};
