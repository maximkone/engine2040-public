//! material: one table of what things are made of: weight, grip, bounce and drag.
//!
//! Data owner: `engine-physics`. Readers: the float physical layer (`engine-physfx`) for the
//! colliders of ragdoll bones, limbs and debris; integer authority code may read the same rows,
//! which is why every number is an integer (thousandths for the coefficients) and the `f32`
//! accessors only divide. Std-only, no state, nothing to step.
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
            },
            Material::Stone => Props {
                density_kg_m3: 2_000,
                friction_pm: 800,
                restitution_pm: 200,
                linear_damping_pm: 20,
                angular_damping_pm: 50,
            },
            Material::Flesh => Props {
                density_kg_m3: 1_000,
                friction_pm: 900,
                restitution_pm: 0,
                linear_damping_pm: 50,
                angular_damping_pm: 50,
            },
            Material::Metal => Props {
                density_kg_m3: 7_800,
                friction_pm: 400,
                restitution_pm: 300,
                linear_damping_pm: 10,
                angular_damping_pm: 20,
            },
            Material::Sand => Props {
                density_kg_m3: 1_600,
                friction_pm: 1_000,
                restitution_pm: 0,
                linear_damping_pm: 200,
                angular_damping_pm: 300,
            },
            Material::Cloth => Props {
                density_kg_m3: 250,
                friction_pm: 700,
                restitution_pm: 0,
                linear_damping_pm: 400,
                angular_damping_pm: 500,
            },
        }
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
