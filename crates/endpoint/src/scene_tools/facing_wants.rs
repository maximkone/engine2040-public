//! SPF3 and SPX10 at the door: the `scene.place` wants `in_front_of`, `behind`,
//! `left_of`, `right_of` (against the anchor's own facing, edge gap in cm) and
//! `outside` (keep the footprint clear of an object or a region).
//!
//! Grammar, one want per line: `<word> object:<uuid> [<lo>,<hi>]?` for the four
//! facing words (gap default 0..100 cm) and `outside object:<uuid>` or
//! `outside region:<x0>,<z0>,<x1>,<z1>` (cm). The solving is `engine-spatial`'s
//! `place_dir` and `place_outside`; this module parses the lines and names the
//! unmet ones in a conflict. A facing want solves alone; `outside` may ride
//! beside the classic wants. Units: whole cm. Determinism: the solvers scan a
//! fixed grid, so the same scene and wants give the same pose.

use std::collections::BTreeMap;

use engine_spatial::place::{place_dir, place_outside, DirWant, OutsideTarget, DEFAULT_DIR_GAP_CM};
use engine_spatial::relation::{parse_rel, Frame, Rel};
use engine_spatial::{Actor, Intent, TransformOut};

use super::{
    bad_want, conflict_output, parse_int, parse_object_id, McpToolError, ScenePlaceOutput,
};
use crate::spatial_glue::ParsedWant;

/// One parsed facing or outside want.
pub(crate) enum Extra {
    Dir(DirWant),
    Outside(OutsideTarget),
}

/// What a want line turned out to be.
pub(crate) enum Parsed {
    Extra(Extra),
    /// The named object is not staged: a conflict, not an error.
    Missing {
        reason: String,
    },
}

/// The words this module owns, for the grammar refusal.
pub(crate) const WORDS: &str = "in_front_of, behind, left_of, right_of, outside";

fn dir_word(word: &str) -> Option<Rel> {
    parse_rel(word).filter(|rel| matches!(rel, Rel::Front | Rel::Behind | Rel::Left | Rel::Right))
}

fn gap_range(text: Option<&str>, source: &str) -> Result<(i32, i32), McpToolError> {
    let Some(range) = text else {
        return Ok(DEFAULT_DIR_GAP_CM);
    };
    let (lo, hi) = range
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .and_then(|inner| inner.split_once(','))
        .ok_or_else(|| bad_want(source, "gap needs `[<lo>,<hi>]`"))?;
    Ok((parse_int(lo, "lo", source)?, parse_int(hi, "hi", source)?))
}

/// Parse `source` when its first word is one of [`WORDS`]; `None` leaves the
/// line to the classic parser.
pub(crate) fn parse(
    source: &str,
    index_of: &BTreeMap<String, u32>,
) -> Result<Option<Parsed>, McpToolError> {
    let mut parts = source.split_whitespace();
    let (Some(word), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(None);
    };
    let dir = dir_word(word);
    if dir.is_none() && word != "outside" {
        return Ok(None);
    }
    let range = parts.next();
    if parts.next().is_some() || (dir.is_none() && range.is_some()) {
        return Err(bad_want(source, "too many parts"));
    }
    let Some((kind, rest)) = target.split_once(':') else {
        return Err(bad_want(source, "target needs `object:<uuid>`"));
    };
    let extra =
        match (dir, kind) {
            (_, "object") => {
                let id = parse_object_id("target", rest).map_err(|e| {
                    McpToolError::new("bad_id", format!("want `{source}`: {}", e.message))
                })?;
                let Some(anchor) = index_of.get(&id.to_string()).copied() else {
                    return Ok(Some(Parsed::Missing {
                        reason: format!(
                            "unknown-target {rest}: no staged node with that id; \
                         re-read scene.brief for the staged ids"
                        ),
                    }));
                };
                match dir {
                    Some(word) => Extra::Dir(DirWant {
                        word,
                        anchor,
                        gap_cm: gap_range(range, source)?,
                        frame: Frame::Subject,
                    }),
                    None => Extra::Outside(OutsideTarget::Object(anchor)),
                }
            }
            (None, "region") => {
                let n: Vec<&str> = rest.split(',').collect();
                let [x0, z0, x1, z1] = n[..] else {
                    return Err(bad_want(source, "region needs `<x0>,<z0>,<x1>,<z1>` in cm"));
                };
                let (x0, z0) = (parse_int(x0, "x0", source)?, parse_int(z0, "z0", source)?);
                let (x1, z1) = (parse_int(x1, "x1", source)?, parse_int(z1, "z1", source)?);
                Extra::Outside(OutsideTarget::Region {
                    min_x: x0.min(x1),
                    max_x: x0.max(x1),
                    min_z: z0.min(z1),
                    max_z: z0.max(z1),
                })
            }
            _ => return Err(bad_want(
                source,
                "target needs `object:<uuid>` (outside also takes `region:<x0>,<z0>,<x1>,<z1>`)",
            )),
        };
    Ok(Some(Parsed::Extra(extra)))
}

/// The solved pose, or the conflict answer naming the wants that cannot hold.
pub(crate) type Solved = Result<TransformOut, ScenePlaceOutput>;

/// Solve the facing and outside wants of one call. `parsed` are the classic
/// wants already in `intent`, `extras` the parsed lines with their sources.
pub(crate) fn solve(
    revision: u64,
    intent: &Intent,
    parsed: &[ParsedWant],
    extras: &[(Extra, String)],
    actors: &[Actor],
) -> Result<Solved, McpToolError> {
    let (mut dirs, mut outs) = (Vec::new(), Vec::new());
    for (extra, source) in extras {
        match extra {
            Extra::Dir(want) => dirs.push((*want, source)),
            Extra::Outside(target) => outs.push((*target, source)),
        }
    }
    if !dirs.is_empty() && (!outs.is_empty() || !parsed.is_empty()) {
        return Err(McpToolError::new(
            "bad_want",
            format!(
                "facing wants ({WORDS}) solve alone: drop the other wants, place, then adjust with a second call"
            ),
        ));
    }
    let conflict = |texts: Vec<String>, reason: String| {
        conflict_output(revision, texts, reason, Vec::new(), Vec::new())
    };
    if !dirs.is_empty() {
        let wants: Vec<DirWant> = dirs.iter().map(|(d, _)| *d).collect();
        return Ok(place_dir(&intent.object, &wants, actors).map_err(|c| {
            conflict(
                c.unmet.iter().map(|&i| dirs[i].1.clone()).collect(),
                c.reason,
            )
        }));
    }
    let targets: Vec<OutsideTarget> = outs.iter().map(|(t, _)| *t).collect();
    Ok(place_outside(intent, &targets, actors).map_err(|c| {
        let wants = c.unmet_wants.iter().map(|&i| parsed[i].source.clone());
        let outside = c.unmet_outside.iter().map(|&i| outs[i].1.clone());
        conflict(wants.chain(outside).collect(), c.reason.to_owned())
    }))
}
