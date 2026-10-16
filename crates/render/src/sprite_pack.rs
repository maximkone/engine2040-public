/// Atlas cap in pixels: rects wider or taller never pack.
pub const ATLAS_MAX_PX: u32 = 2048;
/// One sprite rect to pack, in pixels.
pub struct SpriteRect {
    pub id: u32,
    pub w: u32,
    pub h: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Placed rect: sprite id plus atlas origin and size, in pixels.
pub struct PackedRect {
    pub id: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}
fn pk(id: u32, x: u32, y: u32, w: u32, h: u32) -> PackedRect {
    PackedRect { id, x, y, w, h }
}
#[derive(Clone, Debug, Default)]
/// Packed atlas: placed rects plus atlas size, in pixels.
pub struct SpritePack {
    pub packed: Vec<PackedRect>,
    width: u32,
    height: u32,
}
struct FreeRect(u32, u32, u32, u32);
impl SpritePack {
    /// Packs `rects` into a square atlas; true when every rect fits.
    pub fn pack(&mut self, rects: &[SpriteRect]) -> bool {
        self.packed.clear();
        self.width = 0;
        self.height = 0;
        let area: u64 = rects.iter().map(|r| u64::from(r.w) * u64::from(r.h)).sum();
        for r in rects {
            if r.w == 0 || r.h == 0 || r.w > ATLAS_MAX_PX || r.h > ATLAS_MAX_PX {
                return false;
            }
        }
        let mut bw = u32::try_from(area.isqrt() + 1).unwrap_or(ATLAS_MAX_PX);
        bw = bw.min(ATLAS_MAX_PX).max(1);
        loop {
            if try_place(rects, bw, bw, &mut self.packed) {
                self.width = bw;
                self.height = bw;
                return true;
            }
            self.packed.clear();
            if bw >= ATLAS_MAX_PX {
                return false;
            }
            bw = bw.saturating_mul(2).min(ATLAS_MAX_PX);
        }
    }
    /// Packed rect count.
    pub fn packed_count(&self) -> usize {
        self.packed.len()
    }
    /// Atlas `(width, height)` in pixels; `(0, 0)` before `pack` runs.
    pub fn atlas_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
fn contains(a: &FreeRect, b: &FreeRect) -> bool {
    a.0 <= b.0 && a.1 <= b.1 && a.0 + a.2 >= b.0 + b.2 && a.1 + a.3 >= b.1 + b.3
}
fn split_free(free: &mut Vec<FreeRect>, x: u32, y: u32, w: u32, h: u32) {
    let mut next = Vec::new();
    for f in free.drain(..) {
        if !(f.0 < x + w && x < f.0 + f.2 && f.1 < y + h && y < f.1 + f.3) {
            next.push(f);
            continue;
        }
        if x + w < f.0 + f.2 {
            next.push(FreeRect(x + w, f.1, f.0 + f.2 - (x + w), f.3));
        }
        if y + h < f.1 + f.3 {
            next.push(FreeRect(f.0, y + h, f.2, f.1 + f.3 - (y + h)));
        }
    }
    *free = next;
}
fn prune(free: &mut Vec<FreeRect>) {
    let mut keep = vec![true; free.len()];
    for i in 0..free.len() {
        for j in 0..free.len() {
            if i != j && keep[i] && contains(&free[j], &free[i]) {
                keep[i] = false;
                break;
            }
        }
    }
    let mut next = Vec::new();
    for (i, f) in free.drain(..).enumerate() {
        if keep[i] && f.2 > 0 && f.3 > 0 {
            next.push(f);
        }
    }
    *free = next;
}
fn try_place(rects: &[SpriteRect], bw: u32, bh: u32, out: &mut Vec<PackedRect>) -> bool {
    let mut free = vec![FreeRect(0, 0, bw, bh)];
    for r in rects {
        let Some(i) = free
            .iter()
            .enumerate()
            .filter(|(_, f)| f.2 >= r.w && f.3 >= r.h)
            .min_by_key(|(_, f)| ((f.2 - r.w).min(f.3 - r.h), (f.2 - r.w).max(f.3 - r.h)))
            .map(|(i, _)| i)
        else {
            return false;
        };
        let (x, y) = (free[i].0, free[i].1);
        out.push(pk(r.id, x, y, r.w, r.h));
        split_free(&mut free, x, y, r.w, r.h);
        prune(&mut free);
    }
    true
}
#[cfg(test)]
fn sr(id: u32, w: u32, h: u32) -> SpriteRect {
    SpriteRect { id, w, h }
}
#[cfg(test)]
fn disj(a: &PackedRect, b: &PackedRect) -> bool {
    !(a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h)
}
#[test]
fn sprite_pack_three_no_overlap() {
    let v = vec![sr(0, 65, 65), sr(1, 33, 17), sr(2, 17, 33)];
    let mut p = SpritePack::default();
    assert!(p.pack(&v));
    assert_eq!(p.packed_count(), 3);
    assert!(p.atlas_size().0 <= ATLAS_MAX_PX && p.atlas_size().1 <= ATLAS_MAX_PX);
    for i in 0..p.packed.len() {
        for j in (i + 1)..p.packed.len() {
            assert!(disj(&p.packed[i], &p.packed[j]));
        }
    }
}
#[test]
fn sprite_pack_full_refuses() {
    let mut p = SpritePack::default();
    assert!(!p.pack(&[sr(0, 4096, 64)]));
    assert_eq!(p.packed_count(), 0);
}
#[test]
fn sprite_pack_stable_twice() {
    let v = vec![sr(0, 65, 65), sr(1, 33, 17), sr(2, 17, 33)];
    let (mut a, mut b) = (SpritePack::default(), SpritePack::default());
    assert!(a.pack(&v) && b.pack(&v));
    assert_eq!(a.packed, b.packed);
    assert_eq!(a.atlas_size(), b.atlas_size());
}
