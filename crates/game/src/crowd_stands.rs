//! crowd_stands: the stands crowd as octahedral impostors.
//!
//! Consumer: [`crate::fighter_look::FighterLook::load_and_stage`] (the window
//! and its captures). Data owner: the seat layout is `engine_render::draw::
//! crowd_seats`; the figure is one fighter body posed once
//! ([`FighterLook::crowd_figure`]); the bake and the draw live in the
//! presenter (`present/impostor_gpu.rs`). This file only turns the seats into
//! instances and stages them. Units: meters, radians, linear colour.
//! Determinism: presentation only; the same ring gives the same instances
//! twice and nothing here is hashed into the match. Errors: typed
//! [`LookError`]. Budget: one 2 MiB atlas baked at load, 96 instances staged
//! once, one draw per frame.

use engine_assets::bc::srgb_byte_to_linear;
use engine_core::Fnv64;
use engine_render::draw::crowd_seats::{
    crowd_palette_index, crowd_palette_rgba, crowd_seat_pos, CROWD_ROWS_PER_SECTOR,
    CROWD_SEATS_PER_ROW, CROWD_SECTORS, CROWD_WALL_M,
};
use engine_render::impostor_gpu::{potato_layout, BakeMesh, ImpostorInstance};
use engine_render::present::Presenter;

use crate::fighter_look::{FighterLook, LookError};

/// Mesh id of the crowd figure's atlas in the presenter's impostor cache.
pub const CROWD_FIGURE_KEY: u64 = 0xF16D_0301;
/// The body the crowd is baked from: a CT7 body standing in its idle guard.
const FIGURE_VARIANT: &str = "opp_male_light";
/// Clip time of the baked pose: the idle guard at rest.
const FIGURE_TIME_S: f32 = 0.0;
/// Standing height of a spectator in meters (the body is 1.8 m).
const SPECTATOR_HEIGHT_M: f32 = 1.62;
/// Spectators turn up to this far off the pit centre, radians each way.
const YAW_JITTER_RAD: f32 = 0.45;
/// Size spread of the crowd: a spectator is this share larger or smaller.
const SCALE_SPREAD: f32 = 0.05;
/// Palette tints are brightened so a tunic reads against the lit stone.
const TUNIC_GAIN: f32 = 1.5;
/// The body parts a seat's palette slot recolours: part 0 of the CT7 body is
/// the torso quadrant, the rest keeps its own skin and hair colours.
const TUNIC_PARTS: [usize; 1] = [0];

/// What staging the crowd did, for receipts.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CrowdStaged {
    /// Spectators staged.
    pub seats: usize,
    /// Atlas bytes the figure holds (albedo plus normal).
    pub atlas_bytes: u64,
    /// Wall milliseconds the bake took.
    pub bake_ms: f64,
}

/// Height and the lowest point of a figure in its own space, meters.
fn figure_extent(mesh: &BakeMesh) -> (f32, f32) {
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    let drawn = mesh.parts.iter().filter(|part| !part.indices.is_empty());
    for p in drawn.flat_map(|part| part.positions.iter()) {
        lo = lo.min(p[1]);
        hi = hi.max(p[1]);
    }
    if lo > hi {
        return (0.0, 0.0);
    }
    (hi - lo, lo)
}

/// Unit float in `0..1` from three hash bytes, so no integer-to-float cast.
fn unit(byte: u8) -> f32 {
    f32::from(byte) / 255.0
}

/// The 96 spectators as impostor instances for a ring whose wall face sits at
/// `wall_m`. `figure_centre` is the baked sphere centre and `feet_y` the
/// figure's lowest point, both in the figure's own space; `scale` sizes the
/// figure to a spectator. Every seat faces the pit centre, turned a seeded
/// amount, and wears its palette slot as the tunic tint. Pure and
/// deterministic.
#[must_use]
pub fn crowd_instances(
    wall_m: f32,
    figure_centre: [f32; 3],
    feet_y: f32,
    scale: f32,
) -> Vec<ImpostorInstance> {
    let push = wall_m - CROWD_WALL_M;
    let mut out = Vec::with_capacity(CROWD_SECTORS * CROWD_ROWS_PER_SECTOR * CROWD_SEATS_PER_ROW);
    for sector in 0..CROWD_SECTORS {
        for row in 0..CROWD_ROWS_PER_SECTOR {
            for seat in 0..CROWD_SEATS_PER_ROW {
                let at = crowd_seat_pos(sector, row, seat);
                let (x, z) = match sector {
                    0 => (at[0], at[2] - push),
                    1 => (at[0], at[2] + push),
                    2 => (at[0] - push, at[2]),
                    _ => (at[0] + push, at[2]),
                };
                let mut hash = Fnv64::new();
                hash.write_u32(u32::try_from(sector * 1000 + row * 100 + seat).unwrap_or(0));
                let bytes = hash.finish().to_le_bytes();
                let yaw = (-x).atan2(-z) + (unit(bytes[0]) * 2.0 - 1.0) * YAW_JITTER_RAD;
                let size = scale * (1.0 + (unit(bytes[1]) * 2.0 - 1.0) * SCALE_SPREAD);
                // The figure's centre, turned with the seat and sized with it,
                // above the point its feet stand on.
                let local = [
                    figure_centre[0] * size,
                    (figure_centre[1] - feet_y) * size,
                    figure_centre[2] * size,
                ];
                let (s, c) = yaw.sin_cos();
                let centre = [
                    x + local[0] * c + local[2] * s,
                    at[1] + local[1],
                    z - local[0] * s + local[2] * c,
                ];
                let [r, g, b, _] = crowd_palette_rgba(crowd_palette_index(sector, row, seat));
                out.push(ImpostorInstance {
                    centre_m: centre,
                    scale: size,
                    tint: [
                        srgb_byte_to_linear(r) * TUNIC_GAIN,
                        srgb_byte_to_linear(g) * TUNIC_GAIN,
                        srgb_byte_to_linear(b) * TUNIC_GAIN,
                    ],
                    yaw_rad: yaw,
                });
            }
        }
    }
    out
}

/// The 96 spectators with the crowd's cheer applied (SW134): each seat rises
/// and turns by its [`instance_cheer`](crate::crowd_mood::instance_cheer), so
/// a roar moves the stands and a hush leaves them exactly as
/// [`crowd_instances`] places them.
#[must_use]
pub fn crowd_instances_mooded(
    wall_m: f32,
    figure_centre: [f32; 3],
    feet_y: f32,
    scale: f32,
    heat_permille: u32,
) -> Vec<ImpostorInstance> {
    let mut list = crowd_instances(wall_m, figure_centre, feet_y, scale);
    if heat_permille == 0 {
        return list;
    }
    for (seat, inst) in list.iter_mut().enumerate() {
        let cheer = crate::crowd_mood::instance_cheer(seat, heat_permille);
        let rise_mm = u16::try_from(cheer.rise_um / 1000).unwrap_or(0);
        let yaw_mrad = i16::try_from(cheer.yaw_millirad).unwrap_or(0);
        inst.centre_m[1] += f32::from(rise_mm) / 1000.0;
        inst.yaw_rad += f32::from(yaw_mrad) / 1000.0;
    }
    list
}

/// Bake the crowd figure from `look` and stage the stands for a ring whose
/// wall face is at `wall_m`. Cold path; loud on a refusal.
pub fn stage_crowd(
    presenter: &mut Presenter,
    look: &FighterLook,
    wall_m: f32,
) -> Result<CrowdStaged, LookError> {
    let refuse =
        |what: &str, err: &dyn std::fmt::Display| LookError::Ring(format!("crowd {what}: {err}"));
    let mesh = look.crowd_figure(FIGURE_VARIANT, 0, FIGURE_TIME_S, &TUNIC_PARTS, &[])?;
    let layout = potato_layout().map_err(|err| refuse("layout", &err))?;
    presenter
        .bake_impostor(CROWD_FIGURE_KEY, &mesh, layout, true)
        .map_err(|err| refuse("bake", &err))?;
    let (centre, _radius) = presenter
        .impostor_figure(CROWD_FIGURE_KEY)
        .ok_or_else(|| LookError::Ring("crowd figure missing after its bake".to_string()))?;
    let (height, feet_y) = figure_extent(&mesh);
    let scale = if height > 0.0 {
        SPECTATOR_HEIGHT_M / height
    } else {
        1.0
    };
    let list = crowd_instances(wall_m, centre, feet_y, scale);
    presenter
        .set_impostor_instances(CROWD_FIGURE_KEY, &list)
        .map_err(|err| refuse("stage", &err))?;
    let stats = presenter.impostor_stats();
    Ok(CrowdStaged {
        seats: list.len(),
        atlas_bytes: stats.atlas_bytes,
        bake_ms: presenter.impostor_bake_ms(CROWD_FIGURE_KEY).unwrap_or(0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_render::draw::crowd_seats::{crowd_total, CROWD_WALL_H_M};

    fn seats(wall: f32) -> Vec<ImpostorInstance> {
        crowd_instances(wall, [0.0, 0.9, 0.0], 0.0, 0.9)
    }

    #[test]
    fn every_seat_gets_one_spectator_standing_above_its_tread() {
        let list = seats(21.0);
        assert_eq!(list.len(), crowd_total(), "96 seats, 96 spectators");
        for inst in &list {
            assert!(inst.centre_m.iter().all(|c| c.is_finite()));
            assert!(
                inst.centre_m[1] > CROWD_WALL_H_M,
                "{:?} sits below the wall top",
                inst.centre_m
            );
            let depth = inst.centre_m[0].abs().max(inst.centre_m[2].abs());
            assert!(
                depth > CROWD_WALL_M,
                "{:?} hangs inside the pit",
                inst.centre_m
            );
        }
    }

    #[test]
    fn spectators_face_the_pit_within_the_jitter() {
        for inst in seats(21.0) {
            let (x, z) = (inst.centre_m[0], inst.centre_m[2]);
            let want = (-x).atan2(-z);
            let mut off = (inst.yaw_rad - want).rem_euclid(std::f32::consts::TAU);
            if off > std::f32::consts::PI {
                off -= std::f32::consts::TAU;
            }
            // The seat's own offset from the figure centre moves the aim a hair.
            assert!(
                off.abs() < YAW_JITTER_RAD + 0.06,
                "seat at {x:.1},{z:.1} turns {off:.2}"
            );
        }
    }

    /// SW134: a silent crowd is exactly the placed crowd, a roaring one moves.
    #[test]
    fn a_roaring_crowd_lifts_and_turns_every_seat() {
        let cold = crowd_instances_mooded(21.0, [0.0, 0.9, 0.0], 0.0, 0.9, 0);
        assert_eq!(
            cold,
            seats(21.0),
            "a hush leaves the stands exactly as placed"
        );
        let hot = crowd_instances_mooded(21.0, [0.0, 0.9, 0.0], 0.0, 0.9, 900);
        assert_eq!(hot.len(), crowd_total());
        let mut moved = 0;
        for (before, after) in cold.iter().zip(&hot) {
            assert!(after.centre_m.iter().all(|c| c.is_finite()));
            assert!(
                after.centre_m[1] >= before.centre_m[1],
                "a cheer never sinks a spectator"
            );
            if after.centre_m[1] > before.centre_m[1] || after.yaw_rad != before.yaw_rad {
                moved += 1;
            }
        }
        assert_eq!(moved, crowd_total(), "a roar moves every seat");
        // The same heat twice: the same stands.
        assert_eq!(
            hot,
            crowd_instances_mooded(21.0, [0.0, 0.9, 0.0], 0.0, 0.9, 900)
        );
    }

    #[test]
    fn tints_follow_the_palette_and_stay_in_range() {
        let list = seats(21.0);
        let mut distinct: Vec<[u32; 3]> = Vec::new();
        for inst in &list {
            assert!(
                inst.tint.iter().all(|c| (0.0..=1.6).contains(c)),
                "{:?}",
                inst.tint
            );
            let key = inst.tint.map(f32::to_bits);
            if !distinct.contains(&key) {
                distinct.push(key);
            }
            assert!(
                (0.9..=1.1).contains(&(inst.scale / 0.9)),
                "scale {}",
                inst.scale
            );
        }
        assert_eq!(distinct.len(), 4, "the four palette slots all appear");
    }

    #[test]
    fn the_stands_are_the_same_twice_and_a_wider_ring_pushes_them_out() {
        assert_eq!(seats(21.0), seats(21.0));
        let narrow = seats(21.0);
        let wide = seats(25.0);
        let reach = |list: &[ImpostorInstance]| {
            list.iter()
                .map(|i| i.centre_m[0].abs().max(i.centre_m[2].abs()))
                .fold(f32::MAX, f32::min)
        };
        assert!(
            reach(&wide) > reach(&narrow) + 3.0,
            "{} vs {}",
            reach(&wide),
            reach(&narrow)
        );
    }

    /// The real body: it loads, poses at rest and is a person-sized figure
    /// the baker accepts. Prints the part table the tunic choice reads.
    #[test]
    fn the_crowd_figure_is_a_posed_body_the_baker_accepts() {
        let look =
            FighterLook::load(&crate::fighter_look::fighters_content_dir()).expect("bodies load");
        let report = look
            .crowd_figure_report(FIGURE_VARIANT, 0, FIGURE_TIME_S)
            .expect("the report poses");
        for line in &report {
            eprintln!("{line}");
        }
        let mesh = look
            .crowd_figure(FIGURE_VARIANT, 0, FIGURE_TIME_S, &TUNIC_PARTS, &[])
            .expect("the figure poses");
        mesh.validate().expect("the baker accepts the figure");
        let (height, feet) = figure_extent(&mesh);
        assert!((1.5..2.1).contains(&height), "figure is {height} m tall");
        assert!(feet.abs() < 0.2, "feet at {feet}");
        let (centre, radius) = mesh.bounding_sphere();
        assert!(radius > 0.7 && radius < 1.6, "sphere {radius}");
        assert!(centre[1] > 0.5 && centre[1] < 1.3, "centre {centre:?}");
        assert!(
            mesh.parts.iter().any(|p| p.texture.is_some()),
            "the body is textured"
        );
    }
}
