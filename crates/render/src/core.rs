//! core: headless Forward+ (tile assign, light cull, draw build, visible hash). Borrows sim, never owns.
use crate::{LightGrid, TILE};

#[derive(Clone, Copy, Debug)]
pub struct Sprite {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub hw: f32,
    pub hh: f32,
    pub depth: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub x: f32,
    pub y: f32,
    pub r: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub w: u32,
    pub h: u32,
    pub cam_x: f32,
    pub cam_y: f32,
}
pub struct ForwardPlus {
    pub view: View,
    pub gw: usize,
    pub gh: usize,
    pub grid: LightGrid,
    pub draw: Vec<u32>,
}

fn mix(mut h: u64, b: &[u8]) -> u64 {
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl ForwardPlus {
    pub fn new(w: u32, h: u32, cam_x: f32, cam_y: f32) -> Self {
        let gw = if w == 0 { 0 } else { w.div_ceil(TILE) as usize };
        let gh = if h == 0 { 0 } else { h.div_ceil(TILE) as usize };
        Self {
            view: View { w, h, cam_x, cam_y },
            gw,
            gh,
            grid: LightGrid::new(gw * gh),
            draw: Vec::new(),
        }
    }
    pub fn tile_count(&self) -> usize {
        self.gw * self.gh
    }
    pub fn visible(&self) -> usize {
        self.draw.len()
    }
    pub fn overflow(&self) -> u32 {
        self.grid.overflow_tiles
    }
    pub fn tile_rect(&self, t: usize) -> (f32, f32, f32, f32) {
        let s = TILE as f32;
        let tx = (t % self.gw) as f32 * s;
        let ty = (t / self.gw) as f32 * s;
        (tx, ty, tx + s, ty + s)
    }
    pub fn cull_lights(&mut self, lights: &[Light]) {
        self.grid = LightGrid::new(self.gw * self.gh);
        if self.gw == 0 || self.gh == 0 {
            return;
        }
        for t in 0..self.gw * self.gh {
            let (x0, y0, x1, y1) = self.tile_rect(t);
            for (i, l) in lights.iter().enumerate() {
                if !l.x.is_finite() || !l.y.is_finite() || !l.r.is_finite() || l.r <= 0.0 {
                    continue;
                }
                let sx = l.x - self.view.cam_x;
                let sy = l.y - self.view.cam_y;
                let cx = sx.clamp(x0, x1);
                let cy = sy.clamp(y0, y1);
                let dx = sx - cx;
                let dy = sy - cy;
                if dx * dx + dy * dy <= l.r * l.r {
                    self.grid.push(t, i as u32);
                }
            }
        }
        let mut o = 0u32;
        for t in 0..self.grid.offsets.len() {
            self.grid.offsets[t] = o;
            o += self.grid.counts[t] as u32;
        }
    }
    pub fn build_draw(&mut self, sprites: &[Sprite]) {
        self.draw.clear();
        let (vx0, vy0) = (self.view.cam_x, self.view.cam_y);
        let (vx1, vy1) = (vx0 + self.view.w as f32, vy0 + self.view.h as f32);
        let mut b: Vec<(u32, f32)> = Vec::with_capacity(sprites.len().min(16384));
        for s in sprites {
            if !s.x.is_finite()
                || !s.y.is_finite()
                || !s.hw.is_finite()
                || !s.hh.is_finite()
                || !s.depth.is_finite()
            {
                continue;
            }
            if s.hw < 0.0 || s.hh < 0.0 {
                continue;
            }
            if s.x - s.hw < vx1 && s.x + s.hw > vx0 && s.y - s.hh < vy1 && s.y + s.hh > vy0 {
                b.push((s.id, s.depth));
            }
        }
        b.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        self.draw.extend(b.into_iter().map(|x| x.0));
    }
    pub fn render(&mut self, sprites: &[Sprite], lights: &[Light]) {
        self.cull_lights(lights);
        self.build_draw(sprites);
    }
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        h = mix(h, &self.view.w.to_le_bytes());
        h = mix(h, &self.view.h.to_le_bytes());
        h = mix(h, &self.view.cam_x.to_bits().to_le_bytes());
        h = mix(h, &self.view.cam_y.to_bits().to_le_bytes());
        h = mix(h, &(self.draw.len() as u64).to_le_bytes());
        for id in &self.draw {
            h = mix(h, &id.to_le_bytes());
        }
        h = mix(h, &(self.grid.counts.len() as u64).to_le_bytes());
        for c in &self.grid.counts {
            h = mix(h, &[*c]);
        }
        for ix in &self.grid.indices {
            h = mix(h, &ix.to_le_bytes());
        }
        h = mix(h, &self.grid.overflow_tiles.to_le_bytes());
        h
    }
}
