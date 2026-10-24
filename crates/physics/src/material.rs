//! material: one table of what things are made of: weight, grip, bounce and drag.
//!
//! Data owner: `engine-physics`. Three readers, one table:
//! - the float physical layer (`engine-physfx`): density, bounce and damping for the colliders of
//!   ragdoll bones, limbs and debris, and grip for their contacts;
//! - the authority walker (`controller2::Config::on`): grip (ground acceleration) and the step
//!   height of the ground it stands on, all integers;
//! - the audio cue picker (`Material::cue`): which `SoundClass` an impact makes and how
//!   `Loudness` it is. It names a cue; it plays nothing.
//!
//! Every number is an integer (thousandths for the coefficients) and the `f32` accessors only
//! divide, so integer authority code and float presentation read the very same values.
//! Std-only, no state, nothing to step.
//!
//! The rows `Flesh` and `Stone` are the constants the Rapier code already used: bones and limbs
//! are `Flesh`, ground and thrown rubble are `Stone`. A test reads those source files and fails
//! when a constant there drifts from its row, so the table cannot silently fork.

/// What a body is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    /// Dense hardwood and iron-banded crates: heavier than water, grippy, bounces a little.
    Wood,
    /// Stone and brick.
    Stone,
    /// Living tissue: water density, grippy, never bounces.
    Flesh,
    /// Plate, blades and chains: heaviest, slippery, rings and bounces.
    Metal,
    /// Sand and loose earth: sticks where it lands.
    Sand,
    /// Cloth and furs: light, soaks up motion and air-drags hard.
    Cloth,
}

/// The family of sound an impact on a material makes. A cue picker maps each class to its sample
/// bank; this crate only names the class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundClass {
    /// A hollow knock.
    Knock,
    /// A hard clack.
    Clack,
    /// A dull thud.
    Thud,
    /// A ringing clang.
    Ring,
    /// A hiss of grains.
    Hiss,
    /// A soft rustle.
    Rustle,
}

/// How hard an impact sounds, from the speed it lands at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Loudness {
    /// Too gentle to make a sound.
    Silent,
    /// A tap.
    Soft,
    /// A knock.
    Medium,
    /// A crash.
    Hard,
}

/// What the cue picker is told about one impact: which class to play and how loud.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cue {
    /// The sound family of the material that was hit.
    pub class: SoundClass,
    /// How hard it was hit.
    pub loudness: Loudness,
}

/// Impact speeds, millimeters per second, where `Loudness` steps up: below the first nothing is
/// heard, then soft, medium and hard.
pub const LOUDNESS_STEPS_MM_S: [u32; 3] = [400, 1_500, 4_000];

/// The ground the authority walker is tuned for: today's constants are this row's numbers.
pub const GROUND: Material = Material::Stone;

/// The numbers of one material. Coefficients are thousandths (`900` is `0.9`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Props {
    /// Kilograms per cubic meter.
    pub density_kg_m3: u16,
    /// Coulomb friction coefficient, thousandths.
    pub friction_pm: u16,
    /// Bounce: the share of impact speed kept, thousandths.
    pub restitution_pm: u16,
    /// Linear damping per second, thousandths.
    pub linear_damping_pm: u16,
    /// Angular damping per second, thousandths.
    pub angular_damping_pm: u16,
    /// Tallest rise a walker steps onto from ground of this material, millimeters: soft ground
    /// gives way under the foot.
    pub step_mm: u16,
    /// The sound family an impact on this material makes.
    pub sound: SoundClass,
}

impl Material {
    /// Every material, in table order.
    pub const ALL: [Material; 6] = [
        Material::Wood,
        Material::Stone,
        Material::Flesh,
        Material::Metal,
        Material::Sand,
        Material::Cloth,
    ];

    /// The row of this material. The match has no wildcard, so a new variant cannot compile
    /// without a row.
    pub const fn props(self) -> Props {
        match self {
            Material::Wood => Props {
                density_kg_m3: 1_200,
                friction_pm: 600,
                restitution_pm: 450,
                linear_damping_pm: 30,
                angular_damping_pm: 50,
                step_mm: 500,
                sound: SoundClass::Knock,
            },
            Material::Stone => Props {
                density_kg_m3: 2_000,
                friction_pm: 800,
                restitution_pm: 200,
                linear_damping_pm: 20,
                angular_damping_pm: 50,
                step_mm: 500,
                sound: SoundClass::Clack,
            },
            Material::Flesh => Props {
                density_kg_m3: 1_000,
                friction_pm: 900,
                restitution_pm: 0,
                linear_damping_pm: 50,
                angular_damping_pm: 50,
                step_mm: 500,
                sound: SoundClass::Thud,
            },
            Material::Metal => Props {
                density_kg_m3: 7_800,
                friction_pm: 400,
                restitution_pm: 300,
                linear_damping_pm: 10,
                angular_damping_pm: 20,
                step_mm: 500,
                sound: SoundClass::Ring,
            },
            Material::Sand => Props {
                density_kg_m3: 1_600,
                friction_pm: 1_000,
                restitution_pm: 0,
                linear_damping_pm: 200,
                angular_damping_pm: 300,
                step_mm: 400,
                sound: SoundClass::Hiss,
            },
            Material::Cloth => Props {
                density_kg_m3: 250,
                friction_pm: 700,
                restitution_pm: 0,
                linear_damping_pm: 400,
                angular_damping_pm: 500,
                step_mm: 450,
                sound: SoundClass::Rustle,
            },
        }
    }
}

impl Material {
    /// The sound family an impact on this material makes.
    pub const fn sound(self) -> SoundClass {
        self.props().sound
    }

    /// The cue for an impact on this material at `impact_speed_mm_s` (the closing speed along
    /// the contact normal): the material's `SoundClass` at the `Loudness` the speed earns.
    pub const fn cue(self, impact_speed_mm_s: u32) -> Cue {
        Cue {
            class: self.sound(),
            loudness: loudness(impact_speed_mm_s),
        }
    }
}

/// The `Loudness` of an impact at `speed_mm_s`.
pub const fn loudness(speed_mm_s: u32) -> Loudness {
    if speed_mm_s < LOUDNESS_STEPS_MM_S[0] {
        Loudness::Silent
    } else if speed_mm_s < LOUDNESS_STEPS_MM_S[1] {
        Loudness::Soft
    } else if speed_mm_s < LOUDNESS_STEPS_MM_S[2] {
        Loudness::Medium
    } else {
        Loudness::Hard
    }
}

impl Props {
    /// Density in kilograms per cubic meter.
    pub fn density(&self) -> f32 {
        f32::from(self.density_kg_m3)
    }

    /// Friction coefficient.
    pub fn friction(&self) -> f32 {
        thousandths(self.friction_pm)
    }

    /// Restitution coefficient.
    pub fn restitution(&self) -> f32 {
        thousandths(self.restitution_pm)
    }

    /// Linear damping per second.
    pub fn linear_damping(&self) -> f32 {
        thousandths(self.linear_damping_pm)
    }

    /// Angular damping per second.
    pub fn angular_damping(&self) -> f32 {
        thousandths(self.angular_damping_pm)
    }
}

/// `value / 1000` as the nearest `f32`, so `900` reads back as exactly the literal `0.9`.
fn thousandths(value: u16) -> f32 {
    f32::from(value) / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limb::MAX_DENSITY_KG_M3;

    const RAGDOLL: &str = include_str!("ragdoll.rs");
    const POWERED: &str = include_str!("powered.rs");
    const RAPIER: &str = include_str!("rapier.rs");

    /// The `f32` that `const NAME: f32 = X;` declares in `source`.
    fn const_f32(source: &str, name: &str) -> Option<f32> {
        let key = format!("const {name}: f32 = ");
        let rest = &source[source.find(&key)? + key.len()..];
        rest[..rest.find(';')?].trim().parse().ok()
    }

    /// Every `f32` written as the argument of `.call(X)` in `source`.
    fn call_args(source: &str, call: &str) -> Vec<f32> {
        source
            .match_indices(call)
            .filter_map(|(at, _)| {
                let rest = &source[at + call.len()..];
                rest[..rest.find(')')?].trim().parse().ok()
            })
            .collect()
    }

    #[test]
    fn material_table_gives_every_material_a_usable_row() {
        for material in Material::ALL {
            let p = material.props();
            assert!(
                (100..=MAX_DENSITY_KG_M3).contains(&u32::from(p.density_kg_m3)),
                "{material:?} density {}",
                p.density_kg_m3
            );
            assert!(
                (1..=1_500).contains(&p.friction_pm),
                "{material:?} friction"
            );
            assert!(
                p.restitution_pm < 1_000,
                "{material:?} would gain energy on a bounce"
            );
            assert!(
                p.linear_damping_pm <= 1_000 && p.angular_damping_pm <= 1_000,
                "{material:?} damping"
            );
        }
        for (i, a) in Material::ALL.iter().enumerate() {
            for b in &Material::ALL[i + 1..] {
                assert_ne!(a.props(), b.props(), "{a:?} and {b:?} are one material");
            }
        }
    }

    #[test]
    fn material_table_orders_density_metal_stone_wood_flesh_cloth() {
        let order = [
            Material::Metal,
            Material::Stone,
            Material::Wood,
            Material::Flesh,
            Material::Cloth,
        ];
        for pair in order.windows(2) {
            assert!(
                pair[0].props().density_kg_m3 > pair[1].props().density_kg_m3,
                "{:?} must be denser than {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn material_table_orders_what_you_feel() {
        let p = |m: Material| m.props();
        assert!(
            p(Material::Wood).restitution_pm > p(Material::Stone).restitution_pm,
            "wood springs, stone clacks"
        );
        assert_eq!(p(Material::Sand).restitution_pm, 0, "sand does not bounce");
        assert!(
            p(Material::Metal).friction_pm < p(Material::Stone).friction_pm,
            "metal is slippery"
        );
        assert!(
            p(Material::Cloth).linear_damping_pm > p(Material::Metal).linear_damping_pm,
            "cloth drags in the air"
        );
    }

    #[test]
    fn material_table_flesh_and_stone_are_the_constants_in_use_today() {
        let flesh = Material::Flesh.props();
        for (file, source) in [("ragdoll.rs", RAGDOLL), ("powered.rs", POWERED)] {
            let read = |name| const_f32(source, name).unwrap_or_else(|| panic!("{file} {name}"));
            assert_eq!(read("FRICTION"), flesh.friction(), "{file}");
            assert_eq!(read("LINEAR_DAMPING"), flesh.linear_damping(), "{file}");
            assert_eq!(read("ANGULAR_DAMPING"), flesh.angular_damping(), "{file}");
            assert_eq!(
                call_args(source, ".restitution("),
                [flesh.restitution(); 2],
                "{file}: bone and floor colliders do not bounce"
            );
        }
        // The Rapier adapter's ground and capsule grip is the stone row.
        let grips = call_args(RAPIER, ".friction(");
        assert!(!grips.is_empty(), "rapier.rs no longer sets friction");
        assert!(
            grips
                .iter()
                .all(|f| *f == Material::Stone.props().friction()),
            "{grips:?}"
        );
    }

    #[test]
    fn material_table_every_material_has_its_own_sound_class() {
        for (i, a) in Material::ALL.iter().enumerate() {
            for b in &Material::ALL[i + 1..] {
                assert_ne!(a.sound(), b.sound(), "{a:?} and {b:?} sound alike");
            }
        }
        assert_eq!(Material::Metal.sound(), SoundClass::Ring);
        assert_eq!(Material::Wood.sound(), SoundClass::Knock);
    }

    #[test]
    fn material_table_cue_picker_steps_loudness_with_impact_speed() {
        let [soft, medium, hard] = LOUDNESS_STEPS_MM_S;
        assert!(soft < medium && medium < hard);
        let at = |speed| Material::Stone.cue(speed);
        assert_eq!(at(0).loudness, Loudness::Silent);
        assert_eq!(at(soft - 1).loudness, Loudness::Silent);
        assert_eq!(at(soft).loudness, Loudness::Soft);
        assert_eq!(at(medium - 1).loudness, Loudness::Soft);
        assert_eq!(at(medium).loudness, Loudness::Medium);
        assert_eq!(at(hard - 1).loudness, Loudness::Medium);
        assert_eq!(at(hard).loudness, Loudness::Hard);
        assert_eq!(at(u32::MAX).loudness, Loudness::Hard);
        assert!(at(hard).loudness > at(soft).loudness);
        for material in Material::ALL {
            assert_eq!(material.cue(5_000).class, material.sound());
        }
    }

    #[test]
    fn material_table_step_heights_are_walkable_and_the_ground_row_is_the_legacy_step() {
        for material in Material::ALL {
            let step = material.props().step_mm;
            assert!((200..=500).contains(&step), "{material:?} step {step}");
        }
        assert!(Material::Sand.props().step_mm < GROUND.props().step_mm);
        assert_eq!(
            i32::from(GROUND.props().step_mm),
            crate::controller2::STEP_HEIGHT_MM
        );
    }

    #[test]
    fn material_table_source_readers_refuse_what_they_cannot_read() {
        assert_eq!(const_f32("const A: f32 = 0.9;", "A"), Some(0.9));
        assert_eq!(const_f32("const A: f32 = 0.9;", "B"), None);
        assert_eq!(const_f32("const A: f32 = nine;", "A"), None);
        assert_eq!(const_f32("const A: f32 = 0.9", "A"), None);
        assert_eq!(call_args("x.f(1.5).f( 2 ).f(oops).f(", ".f("), [1.5, 2.0]);
    }

    #[test]
    fn material_table_thousandths_read_back_as_the_literal() {
        // 900 / 1000 is the nearest f32 to 0.9, the same value the source literal parses to.
        assert_eq!(thousandths(900), 0.9_f32);
        assert_eq!(thousandths(50), 0.05_f32);
        assert_eq!(thousandths(0), 0.0_f32);
        assert_eq!(Material::Stone.props().density(), 2_000.0);
    }
}
