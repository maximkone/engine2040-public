//! digest: the deterministic artifact digest of a staged batch, plus the
//! little-endian push helpers it and the display bound share.
//!
//! Owner: `engine-editor`; consumer `crate::commands` (one digest per staged
//! batch) and every module that bounds display text through
//! `crate::commands::bound_to`.
//! Determinism: fixed op order, little-endian integers, four FNV-1a passes;
//! no maps, pointers or wall clock, so the same batch gives the same 32 bytes.

use engine_core::hash::{FNV64_OFFSET, GOLDEN_GAMMA};
use engine_core::Fnv64;
use engine_scene::ids::ObjectId;

use crate::save::push_transform;

use super::Op;

/// Deterministic artifact digest over the batch content: key, revisions, op
/// count, then per op a tag plus node/parent IDs plus raw transform bits.
/// Four FNV-1a passes with distinct seeds fill 32 bytes. Fixed op order,
/// little-endian integers, no maps or pointers: same batch, same digest.
pub(super) fn artifact_digest(
    ops: &[Op],
    key: u64,
    base_revision: u64,
    result_revision: u64,
) -> [u8; 32] {
    let mut input = Vec::new();
    input.extend_from_slice(&key.to_le_bytes());
    input.extend_from_slice(&base_revision.to_le_bytes());
    input.extend_from_slice(&result_revision.to_le_bytes());
    input.extend_from_slice(&(ops.len() as u64).to_le_bytes());
    for op in ops {
        match op {
            Op::Insert { id, parent, local } => {
                input.push(0);
                push_id(&mut input, *id);
                push_id(&mut input, *parent);
                push_transform(&mut input, *local);
            }
            Op::Remove { id } => {
                input.push(1);
                push_id(&mut input, *id);
            }
            Op::SetLocal { id, local } => {
                input.push(2);
                push_id(&mut input, *id);
                push_transform(&mut input, *local);
            }
            Op::Reparent { id, new_parent } => {
                input.push(3);
                push_id(&mut input, *id);
                push_id(&mut input, *new_parent);
            }
            Op::SetMaterial { id, material } => {
                input.push(4);
                push_id(&mut input, *id);
                match material {
                    Some(name) => {
                        let text = name.as_str();
                        let len = name.len_bytes();
                        input.extend_from_slice(&len.to_le_bytes());
                        input.extend_from_slice(text.as_bytes());
                    }
                    None => input.extend_from_slice(&0u64.to_le_bytes()),
                }
            }
            Op::SetComponent { id, component } => {
                input.push(5);
                push_id(&mut input, *id);
                let bytes = component.to_bytes();
                let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                input.extend_from_slice(&len.to_le_bytes());
                input.extend_from_slice(&bytes);
            }
            Op::RemoveComponent { id, kind } => {
                input.push(6);
                push_id(&mut input, *id);
                input.push(kind.tag());
            }
            Op::PutMaterial { text } => {
                input.push(7);
                push_text(&mut input, text);
            }
            Op::SetMaterialParam {
                material,
                path,
                value,
            } => {
                input.push(8);
                push_text(&mut input, material);
                push_text(&mut input, path);
                push_text(&mut input, value);
            }
            Op::RemoveLastMaterial => input.push(9),
            Op::SetBond { id, a, b, strength } => {
                input.push(10);
                push_id(&mut input, *id);
                input.extend_from_slice(&a.to_le_bytes());
                input.extend_from_slice(&b.to_le_bytes());
                match strength {
                    Some(value) => {
                        input.push(1);
                        input.extend_from_slice(&value.permille().to_le_bytes());
                    }
                    None => input.push(0),
                }
            }
        }
    }
    digest32(&[&input])
}

/// One length-prefixed text into the digest input (`u64` length first).
pub(crate) fn push_text(input: &mut Vec<u8>, text: &str) {
    let len = u64::try_from(text.len()).unwrap_or(u64::MAX);
    input.extend_from_slice(&len.to_le_bytes());
    input.extend_from_slice(text.as_bytes());
}

/// 32-byte digest over concatenated byte parts: four FNV-1a passes whose
/// start states differ by pass number, one little-endian word per pass. Same
/// parts, same digest on every machine. Not collision-resistant against an
/// adversary; no new crypto.
fn digest32(parts: &[&[u8]]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (pass, slot) in (1u64..).zip(out.chunks_mut(8)) {
        let mut hash = Fnv64::with_state(FNV64_OFFSET ^ GOLDEN_GAMMA.wrapping_mul(pass));
        for part in parts {
            hash.write(part);
        }
        slot.copy_from_slice(&hash.finish().to_le_bytes());
    }
    out
}

/// Raw UUID bytes of an object ID into the digest input.
fn push_id(input: &mut Vec<u8>, id: ObjectId) {
    input.extend_from_slice(id.as_uuid().as_bytes());
}
