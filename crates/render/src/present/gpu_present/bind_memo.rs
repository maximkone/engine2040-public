//! bind_memo: the per-frame draw-to-bind lookup.
//!
//! Draws repeat a handful of textures, so the frame asks the resident list
//! once per distinct texture and answers every repeat from a small
//! direct-mapped table on the stack. Pure: no allocation, no device, no
//! clock. Presentation only, never hashed.
use super::super::TextureKey;

/// Slots in the memo; a power of two so the slot is the top hash bits.
const MEMO_SLOTS: usize = 64;
/// Right shift that leaves `log2(MEMO_SLOTS)` hash bits.
const MEMO_SHIFT: u32 = 64 - MEMO_SLOTS.trailing_zeros();

/// One frame's texture-key to bind-index memo. Only found keys are stored,
/// so an unknown key is asked again and refused every time.
pub(super) struct BindMemo {
    slots: [Option<(TextureKey, usize)>; MEMO_SLOTS],
    scans: u32,
}

impl BindMemo {
    pub(super) fn new() -> Self {
        Self {
            slots: [None; MEMO_SLOTS],
            scans: 0,
        }
    }

    /// Memo slot for `key`: Fibonacci hash of the id mixed with the first
    /// eight digest bytes. Same key, same slot, on every run.
    fn slot(key: &TextureKey) -> usize {
        let mut head = [0u8; 8];
        head.copy_from_slice(&key.digest[..8]);
        let mixed = (key.id ^ u64::from_le_bytes(head)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        usize::try_from(mixed >> MEMO_SHIFT).unwrap_or(0) % MEMO_SLOTS
    }

    /// Index for `key`: a memo hit costs one key compare; a miss asks `scan`
    /// once and remembers a found index. `None` means the key is unknown.
    pub(super) fn find(
        &mut self,
        key: &TextureKey,
        scan: impl FnOnce(&TextureKey) -> Option<usize>,
    ) -> Option<usize> {
        let at = Self::slot(key);
        if let Some(Some((known, index))) = self.slots.get(at) {
            if known == key {
                return Some(*index);
            }
        }
        self.scans = self.scans.saturating_add(1);
        let found = scan(key)?;
        if let Some(slot) = self.slots.get_mut(at) {
            *slot = Some((*key, found));
        }
        Some(found)
    }

    /// Resident-list scans paid so far.
    pub(super) fn scans(&self) -> u32 {
        self.scans
    }
}

/// Fill `out` with one entry per draw: `None` for a draw with no base-color
/// key (the white fallback bind), else the index `scan` finds for its key.
/// `out` is cleared first and keeps its allocation. `Err(draw)` names the
/// first draw whose key is not resident; entries before it stay in `out`.
/// Returns the number of resident-list scans the whole frame paid.
pub(super) fn resolve_bind_choices<'a>(
    keys: impl Iterator<Item = Option<&'a TextureKey>>,
    out: &mut Vec<Option<usize>>,
    mut scan: impl FnMut(&TextureKey) -> Option<usize>,
) -> Result<u32, usize> {
    out.clear();
    let mut memo = BindMemo::new();
    for (draw, key) in keys.enumerate() {
        let choice = match key {
            None => None,
            Some(key) => Some(memo.find(key, &mut scan).ok_or(draw)?),
        };
        out.push(choice);
    }
    Ok(memo.scans())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: u64) -> TextureKey {
        TextureKey::new(id, [id as u8; 32])
    }

    /// Resident list the closure scans, counting every comparison.
    fn resident(keys: &[TextureKey]) -> (Vec<TextureKey>, std::cell::Cell<u64>) {
        (keys.to_vec(), std::cell::Cell::new(0))
    }

    fn scan_of<'a>(
        list: &'a [TextureKey],
        compares: &'a std::cell::Cell<u64>,
    ) -> impl FnMut(&TextureKey) -> Option<usize> + 'a {
        move |wanted| {
            list.iter().position(|known| {
                compares.set(compares.get() + 1);
                known == wanted
            })
        }
    }

    #[test]
    fn repeat_draws_scan_once_per_texture() {
        // Caller: `Presenter::resolve_draw_binds`. Behavior: 10k draws over
        // six textures pay about six scans, not 10k, and every draw still
        // lands on its own texture's index.
        let (list, compares) = resident(&(0..64).map(key).collect::<Vec<_>>());
        // Six resident textures that own six different memo slots.
        let mut wanted: Vec<TextureKey> = Vec::new();
        for candidate in list.iter() {
            if wanted
                .iter()
                .all(|k| BindMemo::slot(k) != BindMemo::slot(candidate))
            {
                wanted.push(*candidate);
            }
            if wanted.len() == 6 {
                break;
            }
        }
        assert_eq!(wanted.len(), 6, "six distinct slots exist");
        let draws: Vec<TextureKey> = (0..10_000).map(|i| wanted[i % wanted.len()]).collect();
        let mut out = Vec::new();
        let scans =
            resolve_bind_choices(draws.iter().map(Some), &mut out, scan_of(&list, &compares))
                .expect("every key is resident");
        assert_eq!(out.len(), 10_000);
        for (i, choice) in out.iter().enumerate() {
            let id = wanted[i % wanted.len()].id;
            let index = usize::try_from(id).expect("test id fits");
            assert_eq!(*choice, Some(index), "draw {i}");
        }
        assert_eq!(
            scans, 6,
            "one scan per distinct texture, repeats ride the memo"
        );
        assert!(
            compares.get() <= 6 * 64,
            "the resident list is walked six times at most, got {} compares",
            compares.get()
        );
    }

    #[test]
    fn colliding_keys_stay_correct() {
        // Behavior: two keys that share a slot evict each other but every
        // draw still resolves to its own index.
        let a = key(1);
        let b = (2..100_000)
            .map(key)
            .find(|k| BindMemo::slot(k) == BindMemo::slot(&a))
            .expect("some key shares the slot");
        let (list, compares) = resident(&[a, b]);
        let draws = [a, b, a, b, a, b];
        let mut out = Vec::new();
        resolve_bind_choices(draws.iter().map(Some), &mut out, scan_of(&list, &compares))
            .expect("both resident");
        assert_eq!(
            out,
            vec![Some(0), Some(1), Some(0), Some(1), Some(0), Some(1)]
        );
    }

    #[test]
    fn fallback_draws_cost_no_scan() {
        let (list, compares) = resident(&[key(1)]);
        let mut out = vec![Some(9)];
        let scans = resolve_bind_choices(
            [None, None, Some(&list[0]), None].into_iter(),
            &mut out,
            scan_of(&list, &compares),
        )
        .expect("resident");
        assert_eq!(
            out,
            vec![None, None, Some(0), None],
            "stale entries cleared"
        );
        assert_eq!(scans, 1);
    }

    #[test]
    fn unknown_key_names_its_draw_and_is_never_remembered() {
        // Behavior: the first non-resident key fails with its own draw
        // index, entries before it stay, and asking again refuses again.
        let known = key(1);
        let unknown = key(2);
        let (list, compares) = resident(&[known]);
        let mut out = Vec::new();
        let draws = [Some(&known), None, Some(&unknown), Some(&known)];
        let err = resolve_bind_choices(draws.into_iter(), &mut out, scan_of(&list, &compares))
            .expect_err("unknown key refuses");
        assert_eq!(err, 2, "the draw index, not the key");
        assert_eq!(out, vec![Some(0), None]);
        let mut memo = BindMemo::new();
        for _ in 0..3 {
            assert_eq!(memo.find(&unknown, |_| None), None);
        }
        assert_eq!(memo.scans(), 3, "an unknown key is never memoised");
    }

    #[test]
    fn resolve_draw_binds_uses_the_memo_not_a_per_draw_scan() {
        // Source pin: the per-draw body holds no list scan; the one scan
        // sits in `bind_index_of`, which only the memo's misses reach.
        let src = include_str!("submit.rs");
        let start = src
            .find("fn resolve_draw_binds")
            .expect("resolve_draw_binds exists");
        let body = &src[start..];
        // The method closes at the first four-space brace (line endings may
        // be LF or CRLF).
        let end = body.find("\n    }").expect("function closes");
        let body = &body[..end];
        assert!(
            body.contains("resolve_bind_choices"),
            "the per-frame lookup goes through the memo"
        );
        assert!(
            !body.contains(".position("),
            "no per-draw scan over the resident binds"
        );
        assert!(
            body.contains("unknown texture key"),
            "the refusal text is unchanged"
        );
    }
}
