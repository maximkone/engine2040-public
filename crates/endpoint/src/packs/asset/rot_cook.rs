//! Rotation sidecar for `asset.cook`: a `RotBlob` beside the position clips.
//!
//! Reads the GLB JSON and BIN chunks directly (no extra dependency), takes the
//! first animation's rotation and translation channels for skin joints, and
//! writes `<name>.rot` (magic `R204`) through `engine_anim::clip_rot`. A GLB
//! without a skinned rotation channel writes nothing, so its bundle bytes do
//! not move. Deterministic: joints and channels are walked in sorted order.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use engine_anim::clip_rot::{bake_rot_track, rot_blob_hash, write_rot_blob, RotBlob, RotTrack};
use rmcp::serde_json::Value;

use super::cook_verbs::{io_failed, wide};
use crate::mcp::McpToolError;

/// GLB JSON and BIN chunks, when `bytes` is a GLB.
fn glb_chunks(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    if bytes.get(0..4)? != b"glTF" {
        return None;
    }
    let mut at = 12usize;
    let mut json = None;
    let mut bin: &[u8] = &[];
    while at.checked_add(8)? <= bytes.len() {
        let len = usize::try_from(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
            .unwrap_or(usize::MAX);
        let kind = bytes.get(at + 4..at + 8)?;
        let body = bytes.get(at + 8..at.checked_add(8)?.checked_add(len)?)?;
        match kind {
            b"JSON" => json = Some(body),
            b"BIN\0" => bin = body,
            _ => {}
        }
        at = at.checked_add(8)?.checked_add(len)?;
    }
    Some((json?, bin))
}

/// Read one float accessor (SCALAR, VEC3 or VEC4) as a flat list.
fn accessor_f32(doc: &Value, bin: &[u8], index: u64) -> Option<Vec<f32>> {
    let accessors = doc.get("accessors")?.as_array()?;
    let accessor = accessors.get(usize::try_from(index).ok()?)?;
    if accessor.get("componentType")?.as_u64()? != 5126 {
        return None;
    }
    let components = match accessor.get("type")?.as_str()? {
        "SCALAR" => 1,
        "VEC3" => 3,
        "VEC4" => 4,
        _ => return None,
    };
    let count = usize::try_from(accessor.get("count")?.as_u64()?).ok()?;
    let view_index = usize::try_from(accessor.get("bufferView")?.as_u64()?).ok()?;
    let view = doc.get("bufferViews")?.as_array()?.get(view_index)?;
    let base = usize::try_from(view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0)).ok()?
        + usize::try_from(
            accessor
                .get("byteOffset")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        )
        .ok()?;
    let stride = view
        .get("byteStride")
        .and_then(Value::as_u64)
        .and_then(|s| usize::try_from(s).ok())
        .filter(|s| *s >= components * 4)
        .unwrap_or(components * 4);
    let mut out = Vec::with_capacity(count.checked_mul(components)?);
    for i in 0..count {
        let start = base.checked_add(i.checked_mul(stride)?)?;
        for c in 0..components {
            let at = start.checked_add(c * 4)?;
            out.push(f32::from_le_bytes(bin.get(at..at + 4)?.try_into().ok()?));
        }
    }
    Some(out)
}

/// Bake the rotation sidecar beside the bundle: one lane per skin joint, from the
/// first animation's rotation channels. `None` when the GLB has no skinned
/// rotation channel, so a static mesh writes no `.rot` file and the bundle bytes
/// stay exactly what they were before this verb knew about rotations.
pub(super) fn rot_sidecar(
    bytes: &[u8],
    name: &str,
    folder: &Path,
    warnings: &mut Vec<String>,
) -> Result<Option<(PathBuf, u64, String)>, McpToolError> {
    let Some((json, bin)) = glb_chunks(bytes) else {
        return Ok(None);
    };
    let Ok(doc) = rmcp::serde_json::from_slice::<Value>(json) else {
        return Ok(None);
    };
    let mut joints: Vec<u64> = Vec::new();
    for skin in doc
        .get("skins")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for joint in skin
            .get("joints")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(node) = joint.as_u64() {
                joints.push(node);
            }
        }
    }
    joints.sort_unstable();
    joints.dedup();
    if joints.is_empty() {
        return Ok(None);
    }
    let Some((rotations, translated)) = rot_channels(&doc, bin, &joints) else {
        return Ok(None);
    };
    if rotations.is_empty() {
        return Ok(None);
    }
    let (lanes, end) = bake_lanes(&joints, &rotations, &translated, warnings);
    let blob = RotBlob {
        joints: u16::try_from(lanes.len()).unwrap_or(u16::MAX),
        lanes,
        sockets: Vec::new(),
        ticks_len: end,
    };
    let blob_bytes = write_rot_blob(&blob);
    let path = folder.join(format!("{name}.rot"));
    std::fs::write(&path, &blob_bytes).map_err(|e| io_failed("write", &path, &e))?;
    let digest = format!("{:016x}", rot_blob_hash(&blob));
    Ok(Some((path, wide(blob_bytes.len()), digest)))
}

/// Rotation tracks and translation nodes of the first animation, kept only for
/// nodes that are skin joints.
fn rot_channels(
    doc: &Value,
    bin: &[u8],
    joints: &[u64],
) -> Option<(BTreeMap<u64, (Vec<f32>, Vec<[f32; 4]>)>, BTreeSet<u64>)> {
    let animation = doc
        .get("animations")
        .and_then(Value::as_array)
        .and_then(|list| list.first())?;
    let samplers = animation.get("samplers").and_then(Value::as_array)?;
    let channels = animation.get("channels").and_then(Value::as_array)?;
    let mut rotations: BTreeMap<u64, (Vec<f32>, Vec<[f32; 4]>)> = BTreeMap::new();
    let mut translated: BTreeSet<u64> = BTreeSet::new();
    for channel in channels {
        let Some(target) = channel.get("target") else {
            continue;
        };
        let (Some(node), Some(path)) = (
            target.get("node").and_then(Value::as_u64),
            target.get("path").and_then(Value::as_str),
        ) else {
            continue;
        };
        if !joints.contains(&node) {
            continue;
        }
        if path == "translation" {
            translated.insert(node);
            continue;
        }
        if path != "rotation" {
            continue;
        }
        let Some(sampler) = channel
            .get("sampler")
            .and_then(Value::as_u64)
            .and_then(|i| samplers.get(usize::try_from(i).ok()?))
        else {
            continue;
        };
        let times = sampler
            .get("input")
            .and_then(Value::as_u64)
            .and_then(|i| accessor_f32(doc, bin, i));
        let quats = sampler
            .get("output")
            .and_then(Value::as_u64)
            .and_then(|i| accessor_f32(doc, bin, i));
        let (Some(times), Some(quats)) = (times, quats) else {
            continue;
        };
        if quats.len() % 4 != 0 {
            continue;
        }
        let values: Vec<[f32; 4]> = quats
            .chunks_exact(4)
            .map(|q| [q[0], q[1], q[2], q[3]])
            .collect();
        if values.len() != times.len() {
            continue;
        }
        rotations.insert(node, (times, values));
    }
    Some((rotations, translated))
}

/// One lane per joint in joint order; a joint with rotation but no translation
/// track still bakes, with one warning row.
fn bake_lanes(
    joints: &[u64],
    rotations: &BTreeMap<u64, (Vec<f32>, Vec<[f32; 4]>)>,
    translated: &BTreeSet<u64>,
    warnings: &mut Vec<String>,
) -> (Vec<RotTrack>, f32) {
    let mut lanes = Vec::with_capacity(joints.len());
    let mut end = 0.0f32;
    for node in joints {
        if let Some((times, values)) = rotations.get(node) {
            for t in times {
                end = end.max(*t);
            }
            lanes.push(bake_rot_track(times.clone(), values.clone()));
            if !translated.contains(node) {
                warnings.push(format!(
                    "joint node {node} has rotation but no translation track in the first animation; its lane bakes alone (a warning row, not a fail)"
                ));
            }
        } else {
            lanes.push(RotTrack::default());
        }
    }
    (lanes, end)
}
