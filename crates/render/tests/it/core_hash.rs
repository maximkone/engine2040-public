use engine_render::core::{ForwardPlus, Light, Sprite};
use engine_render::{MAX_LIGHTS_PER_TILE, TILE_COUNT_1080P};

struct R(u64);
impl R {
    fn u(&mut self) -> u64 {
        let mut x = self.0 | 1;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn f(&mut self, a: f32, b: f32) -> f32 {
        a + ((self.u() >> 11) as f32 / 9_007_199_000_000_000.0) * (b - a)
    }
}
fn scene(seed: u64) -> (Vec<Sprite>, Vec<Light>) {
    let mut r = R(seed);
    let mut s = Vec::with_capacity(10_000);
    for i in 0..10_000 {
        s.push(Sprite {
            id: i,
            x: r.f(0.0, 1920.0),
            y: r.f(0.0, 1080.0),
            hw: r.f(4.0, 16.0),
            hh: r.f(4.0, 16.0),
            depth: r.f(0.0, 1000.0),
        });
    }
    let mut l = Vec::with_capacity(24);
    for i in 0..24u64 {
        l.push(Light {
            x: ((i * 433) % 1920) as f32 + 8.0,
            y: ((i * 569) % 1080) as f32 + 8.0,
            r: 90.0,
        });
    }
    (s, l)
}
fn frame(seed: u64) -> (u64, usize, u32) {
    let (s, l) = scene(seed);
    let mut f = ForwardPlus::new(1920, 1080, 0.0, 0.0);
    f.render(&s, &l);
    (f.hash(), f.visible(), f.overflow())
}
#[test]
fn grid_1080p_dims() {
    let f = ForwardPlus::new(1920, 1080, 0.0, 0.0);
    assert_eq!((f.gw, f.gh, f.tile_count()), (120, 68, TILE_COUNT_1080P));
}
#[test]
fn cull_hits_near_tile_misses_far_and_clamps() {
    let mut f = ForwardPlus::new(64, 64, 0.0, 0.0);
    assert_eq!(f.tile_count(), 16);
    f.cull_lights(&[Light {
        x: 8.0,
        y: 8.0,
        r: 8.0,
    }]);
    assert!(f.grid.counts[0] > 0);
    assert_eq!(f.grid.counts[15], 0);
    assert_eq!(f.grid.offsets.len(), 16);
    let big = vec![
        Light {
            x: 8.0,
            y: 8.0,
            r: 100.0
        };
        32
    ];
    let mut g = ForwardPlus::new(16, 16, 0.0, 0.0);
    g.cull_lights(&big);
    assert_eq!(
        (g.grid.counts[0] as usize, g.grid.indices.len()),
        (MAX_LIGHTS_PER_TILE, MAX_LIGHTS_PER_TILE)
    );
    assert_eq!(g.overflow(), 8);
}
#[test]
fn draw_culls_offscreen_and_sorts_by_depth_then_id() {
    let v = vec![
        Sprite {
            id: 5,
            x: 10.0,
            y: 10.0,
            hw: 2.0,
            hh: 2.0,
            depth: 2.0,
        },
        Sprite {
            id: 3,
            x: 10.0,
            y: 10.0,
            hw: 2.0,
            hh: 2.0,
            depth: 1.0,
        },
        Sprite {
            id: 7,
            x: 500.0,
            y: 500.0,
            hw: 2.0,
            hh: 2.0,
            depth: 0.0,
        },
    ];
    let mut a = ForwardPlus::new(64, 64, 0.0, 0.0);
    let mut b = ForwardPlus::new(64, 64, 0.0, 0.0);
    a.build_draw(&v);
    b.build_draw(&v);
    assert_eq!(a.draw, vec![3, 5]);
    assert_eq!(a.hash(), b.hash());
}
#[test]
fn ten_k_headless_hash_window_parity_no_overflow() {
    let (h1, v1, o1) = frame(2040);
    let (s, l) = scene(2040);
    let mut a = ForwardPlus::new(1920, 1080, 0.0, 0.0);
    a.cull_lights(&l);
    a.build_draw(&s);
    let mut b = ForwardPlus::new(1920, 1080, 0.0, 0.0);
    b.render(&s, &l);
    assert_eq!(a.hash(), b.hash());
    assert_eq!((h1, v1, o1), (b.hash(), b.visible(), b.overflow()));
    assert_eq!((v1, o1), (10_000, 0));
    assert_ne!(h1, 0xcbf29ce484222325);
}
#[test]
fn hash_sensitive_to_visible_set() {
    let (mut s, l) = scene(2040);
    let mut f = ForwardPlus::new(1920, 1080, 0.0, 0.0);
    f.render(&s, &l);
    let h0 = f.hash();
    s[0].x += 64.0;
    s[0].depth += 1.0;
    let mut g = ForwardPlus::new(1920, 1080, 0.0, 0.0);
    g.render(&s, &l);
    assert_ne!(h0, g.hash());
}
