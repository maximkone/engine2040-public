//! stage (SD7 + HC9): what the speed demo stands on and what its blade cuts. Two worlds: the
//! dunes (SD1 terrain, SD3 towers) and the procedural New York (HC1 to HC4 through
//! `nyc::world`). Either way the buildings are cuttable shells; they draw as merged meshes,
//! one group per chunk and look, rebuilt only when a cut changes a member, so a city of
//! thousands of buildings stays inside the frame's draw budget. New York's facades draw with
//! their tiling textures (HC3) through a VR2 material set; if the textures cannot upload, they
//! draw in flat colours instead. Presentation only. Units: meters.

use engine_destruction::csg::PropMesh;
use engine_render::present::{DrawItem, DrawList, MeshKey, Presenter, TextureKey};
use engine_render::v1::PbrMaterial;

use super::city::{place_towers, tower_shell};
use super::draw::{item, shell_bounds, Soup};
use super::nyc::world::{facade_uploads, NycWorld};
use super::nyc::Style;
use super::terrain::DemoTerrain;

/// Dune towers stand inside this half-extent, meters.
const CITY_HALF_M: f32 = 700.0;
/// Mesh key ranges: ground meshes, building groups (16 meshes per group at most).
const GROUND_KEY: u64 = 0x5D00_0000;
const GROUP_KEY: u64 = 0x5D10_0000;
const MESHES_PER_GROUP: u64 = 16;
/// Facade texture keys: base colours and material sets, one per style.
const FACADE_BASE_ID: u64 = 0x5D50_0000;
const FACADE_SET_ID: u64 = 0x5D58_0000;
const FACADE_DIGEST: [u8; 32] = [0x5E; 32];

const DUNE: PbrMaterial = PbrMaterial {
    base_color: [0.42, 0.40, 0.30],
    metallic: 0.0,
    roughness: 0.95,
    exposure: 1.0,
};

/// A ground band's material: the dune surface with the band's colour (SD8). Sand keeps
/// [`DUNE`]'s colour; grass and rock are the same sand with their own colour.
fn ground_material(base_color: [f32; 3]) -> PbrMaterial {
    PbrMaterial { base_color, ..DUNE }
}

/// How a building looks: a flat colour (dune towers) or a New York facade style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Look {
    Plain([f32; 3]),
    Facade(Style),
}

/// One building as the blade sees it.
pub(super) struct Standing {
    /// What is left standing (cuts take the top away).
    pub shell: PropMesh,
    /// Bounds of `shell`, meters.
    pub bounds: ([f32; 3], [f32; 3]),
    pub look: Look,
    /// Its draw group.
    group: usize,
    /// Cuts it took.
    pub cuts: u32,
}

/// Buildings drawn together: one chunk, one look.
struct Group {
    look: Look,
    members: Vec<usize>,
    /// Meshes uploaded for it last time.
    meshes: usize,
    dirty: bool,
}

/// What the demo stands on.
pub(super) enum Ground {
    Dunes(DemoTerrain),
    Nyc(Box<NycWorld>),
}

/// The world: ground and buildings.
pub(super) struct Stage {
    pub ground: Ground,
    pub standing: Vec<Standing>,
    groups: Vec<Group>,
    /// Ground meshes uploaded, with their materials (empty until the first frame).
    ground_draws: Vec<PbrMaterial>,
    /// Facade textures: `None` until tried, then whether they are resident.
    textured: Option<bool>,
}

impl Stage {
    /// The dunes and their towers.
    pub(super) fn dunes(seed: u64) -> Result<Self, String> {
        let terrain = DemoTerrain::generate(seed)?;
        let ground = |x: f32, z: f32| terrain.height_at(x, z);
        let towers = place_towers(seed, CITY_HALF_M, &ground);
        let parts = towers
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let look = Look::Plain([t.colour[0], t.colour[1], t.colour[2]]);
                (tower_shell(t), look, i)
            })
            .collect();
        Ok(Self::with(Ground::Dunes(terrain), parts))
    }

    /// The procedural New York.
    pub(super) fn nyc(seed: u64) -> Self {
        let mut city = NycWorld::build(seed);
        let parts = city
            .buildings
            .drain(..)
            .map(|b| (b.shell, Look::Facade(b.style), b.chunk))
            .collect();
        Self::with(Ground::Nyc(Box::new(city)), parts)
    }

    fn with(ground: Ground, parts: Vec<(PropMesh, Look, usize)>) -> Self {
        let mut groups: Vec<Group> = Vec::new();
        let mut standing = Vec::with_capacity(parts.len());
        for (i, (shell, look, chunk)) in parts.into_iter().enumerate() {
            let group = match groups.iter().position(|g| {
                g.look == look && g.members.first().map(|m| chunk_key(m, &standing)) == Some(chunk)
            }) {
                Some(g) => g,
                None => {
                    groups.push(Group {
                        look,
                        members: Vec::new(),
                        meshes: 0,
                        dirty: true,
                    });
                    groups.len() - 1
                }
            };
            groups[group].members.push(i);
            standing.push((
                Standing {
                    bounds: shell_bounds(&shell),
                    shell,
                    look,
                    group,
                    cuts: 0,
                },
                chunk,
            ));
        }
        Self {
            ground,
            standing: standing.into_iter().map(|(s, _)| s).collect(),
            groups,
            ground_draws: Vec::new(),
            textured: None,
        }
    }

    /// True for the New York world.
    pub(super) fn is_nyc(&self) -> bool {
        matches!(self.ground, Ground::Nyc(_))
    }

    /// The city, when this is New York.
    pub(super) fn city(&self) -> Option<&NycWorld> {
        match &self.ground {
            Ground::Nyc(city) => Some(city),
            Ground::Dunes(_) => None,
        }
    }

    /// Ground height at (x, z), meters.
    pub(super) fn height_at(&self, x: f32, z: f32) -> f32 {
        match &self.ground {
            Ground::Dunes(t) => t.height_at(x, z),
            Ground::Nyc(city) => city.height_at(x, z),
        }
    }

    /// Building `i` lost its top: `stays` is what is left.
    pub(super) fn cut(&mut self, i: usize, stays: PropMesh) {
        let Some(s) = self.standing.get_mut(i) else {
            return;
        };
        s.bounds = shell_bounds(&stays);
        s.shell = stays;
        s.cuts += 1;
        if let Some(g) = self.groups.get_mut(s.group) {
            g.dirty = true;
        }
    }

    /// Building boxes within `radius_m` (horizontally) of `at`: what a web can hang from and a
    /// thrown body can hit. Standing bounds, so a cut building offers only what is left.
    pub(super) fn boxes_near(&self, at: [f32; 3], radius_m: f32) -> Vec<([f32; 3], [f32; 3])> {
        self.standing
            .iter()
            .filter(|s| {
                let (min, max) = s.bounds;
                let dx = (min[0] - at[0]).max(at[0] - max[0]).max(0.0);
                let dz = (min[2] - at[2]).max(at[2] - max[2]).max(0.0);
                dx.hypot(dz) <= radius_m
            })
            .map(|s| s.bounds)
            .collect()
    }

    /// The draw for mesh `mesh` in look `look`.
    pub(super) fn draw(&self, mesh: MeshKey, look: Look) -> DrawItem {
        match look {
            Look::Plain(c) => item(mesh, plain(c)),
            Look::Facade(style) if self.textured == Some(true) => DrawItem {
                // The ORM tile carries roughness and metalness; the factors let it through.
                material: PbrMaterial {
                    base_color: [1.0, 1.0, 1.0],
                    metallic: 1.0,
                    roughness: 1.0,
                    exposure: 1.0,
                },
                base_color: Some(facade_key(FACADE_BASE_ID, style)),
                normal_map: Some(facade_key(FACADE_SET_ID, style)),
                ..item(mesh, DUNE)
            },
            Look::Facade(style) => item(mesh, plain(style_colour(style))),
        }
    }

    /// Upload what changed and push the ground and the buildings.
    pub(super) fn push(
        &mut self,
        presenter: &mut Presenter,
        list: &mut DrawList,
    ) -> Result<(), String> {
        if self.ground_draws.is_empty() {
            self.upload_ground(presenter)?;
        }
        if self.textured.is_none() {
            self.textured = Some(self.upload_facades(presenter));
        }
        for (i, material) in self.ground_draws.iter().enumerate() {
            list.items.push(item(ground_key(i), material.clone()));
        }
        for g in 0..self.groups.len() {
            if self.groups[g].dirty {
                self.upload_group(presenter, g)?;
            }
            let group = &self.groups[g];
            for m in 0..group.meshes {
                list.items.push(self.draw(group_key(g, m), group.look));
            }
        }
        Ok(())
    }

    fn upload_ground(&mut self, presenter: &mut Presenter) -> Result<(), String> {
        let draws = match &self.ground {
            // SD8: one mesh per slope band, drawn with that band's colour.
            Ground::Dunes(t) => t
                .meshes()?
                .into_iter()
                .map(|(mesh, colour)| (mesh, ground_material(colour)))
                .collect(),
            Ground::Nyc(city) => city.ground_draws()?,
        };
        for (i, (mesh, material)) in draws.into_iter().enumerate() {
            presenter
                .upload_mesh(ground_key(i), &mesh)
                .map_err(|e| format!("ground: {e}"))?;
            self.ground_draws.push(material);
        }
        Ok(())
    }

    /// True when every facade texture and material set is resident.
    fn upload_facades(&self, presenter: &mut Presenter) -> bool {
        let Ground::Nyc(_) = &self.ground else {
            return false;
        };
        facade_uploads(0xFACADE).iter().all(|f| {
            let base = facade_key(FACADE_BASE_ID, f.style);
            presenter.upload_texture(base, &f.base).is_ok()
                && presenter
                    .upload_material_set(
                        facade_key(FACADE_SET_ID, f.style),
                        Some(base),
                        &f.normal,
                        Some(&f.orm),
                    )
                    .is_ok()
        })
    }

    fn upload_group(&mut self, presenter: &mut Presenter, g: usize) -> Result<(), String> {
        let mut soup = Soup::default();
        for &i in &self.groups[g].members {
            soup.add(&self.standing[i].shell, |p| p);
        }
        let meshes = if soup.is_empty() {
            Vec::new()
        } else {
            soup.meshes()?
        };
        let count = meshes
            .len()
            .min(usize::try_from(MESHES_PER_GROUP).unwrap_or(1));
        for (m, mesh) in meshes.iter().take(count).enumerate() {
            presenter
                .upload_mesh(group_key(g, m), mesh)
                .map_err(|e| format!("buildings: {e}"))?;
        }
        let group = &mut self.groups[g];
        group.meshes = count;
        group.dirty = false;
        Ok(())
    }

    /// Mesh groups drawn, and buildings standing (the exit line reads them).
    pub(super) fn counts(&self) -> (usize, usize) {
        (self.groups.len(), self.standing.len())
    }
}

/// The chunk of an already placed member (dune towers are their own chunk).
fn chunk_key(member: &usize, standing: &[(Standing, usize)]) -> usize {
    standing.get(*member).map_or(usize::MAX, |s| s.1)
}

fn plain(c: [f32; 3]) -> PbrMaterial {
    PbrMaterial {
        base_color: c,
        metallic: 0.05,
        roughness: 0.8,
        exposure: 1.0,
    }
}

/// A facade style's colour when its textures are not resident.
fn style_colour(style: Style) -> [f32; 3] {
    match style {
        Style::Brick => [0.45, 0.2, 0.14],
        Style::Stone => [0.72, 0.66, 0.56],
        Style::Deco => [0.8, 0.76, 0.66],
        Style::Glass => [0.16, 0.24, 0.28],
    }
}

fn facade_key(base: u64, style: Style) -> TextureKey {
    let n = Style::ALL.iter().position(|s| *s == style).unwrap_or(0);
    TextureKey::new(base + u64::try_from(n).unwrap_or(0), FACADE_DIGEST)
}

fn ground_key(i: usize) -> MeshKey {
    MeshKey::new(GROUND_KEY + u64::try_from(i).unwrap_or(0), [0x5D; 32])
}

fn group_key(g: usize, m: usize) -> MeshKey {
    let g = u64::try_from(g).unwrap_or(0);
    let m = u64::try_from(m).unwrap_or(0);
    MeshKey::new(GROUP_KEY + g * MESHES_PER_GROUP + m, [0x5D; 32])
}

#[cfg(test)]
mod tests {
    use super::super::nyc::world::{chunk_of, CHUNKS};
    use super::*;

    /// New York groups its buildings by chunk and style: far fewer groups than buildings,
    /// and never more than the chunks times the styles.
    #[test]
    fn new_york_draws_in_a_few_groups() {
        let stage = Stage::nyc(0x5EED_2040);
        let (groups, buildings) = stage.counts();
        assert!(buildings > 500);
        assert!(groups <= CHUNKS * Style::ALL.len(), "{groups} groups");
        for (i, s) in stage.standing.iter().enumerate() {
            let g = &stage.groups[s.group];
            assert!(g.members.contains(&i));
            assert_eq!(g.look, s.look);
        }
        let _ = chunk_of(0.0, 0.0);
    }

    /// A cut replaces the shell, shrinks the bounds and marks only its group for upload.
    #[test]
    fn a_cut_marks_its_group_dirty() {
        let mut stage = Stage::dunes(0x5EED_2040).expect("dunes");
        for g in &mut stage.groups {
            g.dirty = false;
        }
        let before = stage.standing[0].bounds;
        let half = PropMesh {
            positions: stage.standing[0]
                .shell
                .positions
                .iter()
                .map(|p| [p[0], p[1].min(before.0[1] + 5.0), p[2]])
                .collect(),
            indices: stage.standing[0].shell.indices.clone(),
        };
        stage.cut(0, half);
        assert!(stage.standing[0].bounds.1[1] < before.1[1]);
        assert_eq!(stage.standing[0].cuts, 1);
        let dirty = stage.groups.iter().filter(|g| g.dirty).count();
        assert_eq!(dirty, 1);
    }

    /// The boxes near a point are the buildings within reach.
    #[test]
    fn boxes_near_reach_only_close_buildings() {
        let stage = Stage::nyc(0x5EED_2040);
        let (min, max) = stage.standing[0].bounds;
        let at = [(min[0] + max[0]) * 0.5, 20.0, (min[2] + max[2]) * 0.5];
        assert!(!stage.boxes_near(at, 0.5).is_empty());
        assert!(stage.boxes_near([5.0e4, 0.0, 5.0e4], 100.0).is_empty());
        assert_eq!(stage.height_at(1.0e4, 1.0e4), stage.height_at(1.0e4, 1.0e4));
    }
}
