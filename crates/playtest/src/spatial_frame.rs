//! SPF2 closed-form camera frame fit, SPF11 subject coverage and camera
//! sanity, and the SPR10 shot ranking built on both: pure functions, no door.
//!
//! Consumer: lane A's `scene.frame` and `coverage` verbs call these; until
//! then the unit tests are the callers. Data owner: the caller passes boxes in
//! integer cm (`min`, `max`); this module owns the math. Convention: world
//! x east, y up, z north. A view yaw is the azimuth of the EYE seen from the
//! target (0 = on the +z side, 90 = on the +x side); the returned camera yaw
//! is the look direction (eye yaw + 180) and the camera pitch is negative
//! when it looks down. Trig comes from a frozen half-degree sine table and
//! `sqrt` (IEEE exact), so no libm call can differ between machines.
//! Determinism: same inputs give the same bytes; box order never matters.
//! Failure mode: bad boxes or ranges return a typed [`FrameError`].

use std::fmt;

/// Smallest eye distance in cm (a tiny box never puts the eye inside it).
pub const MIN_DISTANCE_CM: i32 = 50;
/// Coverage grid side: 32 x 32 = 1,024 rays.
pub const GRID: i32 = 32;
/// Rays in the grid.
const RAYS: u32 = 1024;
/// Coverage under this many permille earns a next-call line.
pub const HIDDEN_PERMILLE: u32 = 5;

/// sin(k / 2 degrees) for k = 0..=180 (0 to 90 degrees).
const SIN_HALF_DEG: [f64; 181] = [
    0.000000000000,
    0.008726535498,
    0.017452406437,
    0.026176948308,
    0.034899496703,
    0.043619387365,
    0.052335956243,
    0.061048539535,
    0.069756473744,
    0.078459095728,
    0.087155742748,
    0.095845752520,
    0.104528463268,
    0.113203213768,
    0.121869343405,
    0.130526192220,
    0.139173100960,
    0.147809411130,
    0.156434465040,
    0.165047605861,
    0.173648177667,
    0.182235525492,
    0.190808995377,
    0.199367934417,
    0.207911690818,
    0.216439613938,
    0.224951054344,
    0.233445363856,
    0.241921895600,
    0.250380004054,
    0.258819045103,
    0.267238376078,
    0.275637355817,
    0.284015344704,
    0.292371704723,
    0.300705799504,
    0.309016994375,
    0.317304656405,
    0.325568154457,
    0.333806859234,
    0.342020143326,
    0.350207381259,
    0.358367949545,
    0.366501226724,
    0.374606593416,
    0.382683432365,
    0.390731128489,
    0.398749068925,
    0.406736643076,
    0.414693242656,
    0.422618261741,
    0.430511096808,
    0.438371146789,
    0.446197813110,
    0.453990499740,
    0.461748613235,
    0.469471562786,
    0.477158760260,
    0.484809620246,
    0.492423560103,
    0.500000000000,
    0.507538362961,
    0.515038074910,
    0.522498564716,
    0.529919264233,
    0.537299608347,
    0.544639035015,
    0.551936985312,
    0.559192903471,
    0.566406236925,
    0.573576436351,
    0.580702955711,
    0.587785252292,
    0.594822786751,
    0.601815023152,
    0.608761429009,
    0.615661475326,
    0.622514636638,
    0.629320391050,
    0.636078220278,
    0.642787609687,
    0.649448048330,
    0.656059028991,
    0.662620048216,
    0.669130606359,
    0.675590207616,
    0.681998360062,
    0.688354575694,
    0.694658370459,
    0.700909264300,
    std::f64::consts::FRAC_1_SQRT_2,
    0.713250449154,
    0.719339800339,
    0.725374371012,
    0.731353701619,
    0.737277336810,
    0.743144825477,
    0.748955720789,
    0.754709580223,
    0.760405965600,
    0.766044443119,
    0.771624583388,
    0.777145961457,
    0.782608156852,
    0.788010753607,
    0.793353340291,
    0.798635510047,
    0.803856860617,
    0.809016994375,
    0.814115518356,
    0.819152044289,
    0.824126188622,
    0.829037572555,
    0.833885822067,
    0.838670567945,
    0.843391445813,
    0.848048096156,
    0.852640164354,
    0.857167300702,
    0.861629160442,
    0.866025403784,
    0.870355695940,
    0.874619707139,
    0.878817112662,
    0.882947592859,
    0.887010833178,
    0.891006524188,
    0.894934361602,
    0.898794046299,
    0.902585284350,
    0.906307787037,
    0.909961270877,
    0.913545457643,
    0.917060074385,
    0.920504853452,
    0.923879532511,
    0.927183854567,
    0.930417567982,
    0.933580426497,
    0.936672189248,
    0.939692620786,
    0.942641491092,
    0.945518575599,
    0.948323655206,
    0.951056516295,
    0.953716950748,
    0.956304755963,
    0.958819734868,
    0.961261695938,
    0.963630453209,
    0.965925826289,
    0.968147640378,
    0.970295726276,
    0.972369920398,
    0.974370064785,
    0.976296007120,
    0.978147600734,
    0.979924704621,
    0.981627183448,
    0.983254907564,
    0.984807753012,
    0.986285601537,
    0.987688340595,
    0.989015863362,
    0.990268068742,
    0.991444861374,
    0.992546151641,
    0.993571855677,
    0.994521895368,
    0.995396198367,
    0.996194698092,
    0.996917333733,
    0.997564050260,
    0.998134798422,
    0.998629534755,
    0.999048221582,
    0.999390827019,
    0.999657324976,
    0.999847695156,
    0.999961923064,
    1.000000000000,
];

/// Sine of `half_deg` half-degree steps (720 = full circle).
fn sin_half(half_deg: i32) -> f64 {
    let k = usize::try_from(half_deg.rem_euclid(720)).unwrap_or(0);
    match k {
        0..=180 => SIN_HALF_DEG[k],
        181..=360 => SIN_HALF_DEG[360 - k],
        361..=540 => -SIN_HALF_DEG[k - 360],
        _ => -SIN_HALF_DEG[720 - k],
    }
}

fn sin_deg(deg: i32) -> f64 {
    sin_half(deg.saturating_mul(2))
}

fn cos_deg(deg: i32) -> f64 {
    sin_half(deg.saturating_mul(2).saturating_add(180))
}

/// tan(fov / 2) for an integer fov in degrees.
fn tan_half_fov(fov_deg: i32) -> f64 {
    sin_half(fov_deg) / sin_half(fov_deg + 180)
}

type V3 = [f64; 3];

/// Nearest whole cm (saturating at the i32 range).
fn cm(v: f64) -> i32 {
    v.round() as i32
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: V3) -> V3 {
    let n = dot(a, a).sqrt();
    if n < 1e-12 {
        [0.0, 0.0, 1.0]
    } else {
        [a[0] / n, a[1] / n, a[2] / n]
    }
}

/// Camera basis for a look yaw and pitch (degrees; pitch up positive):
/// forward, right, up. `up.y` is `cos(pitch)`, negative when flipped.
fn basis(yaw_deg: i32, pitch_deg: i32) -> (V3, V3, V3) {
    let (sy, cy) = (sin_deg(yaw_deg), cos_deg(yaw_deg));
    let (sp, cp) = (sin_deg(pitch_deg), cos_deg(pitch_deg));
    let forward = [sy * cp, sp, cy * cp];
    let up = [-sy * sp, cp, -cy * sp];
    (forward, unit(cross(up, forward)), up)
}

/// Typed refusal of a frame or coverage request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// `max` is below `min` on some axis.
    EmptyBox,
    /// A number is outside its legal range; names the field and the range.
    OutOfRange(&'static str),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyBox => write!(f, "frame: box max is below min; pass min and max in cm"),
            Self::OutOfRange(what) => write!(f, "frame: {what}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Axis-aligned box in integer cm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Box3 {
    /// Caller id (the coverage subject is matched by it).
    pub id: u32,
    /// Lower corner.
    pub min: [i32; 3],
    /// Upper corner.
    pub max: [i32; 3],
}

impl Box3 {
    fn center(&self) -> V3 {
        let mid = |i: usize| (f64::from(self.min[i]) + f64::from(self.max[i])) / 2.0;
        [mid(0), mid(1), mid(2)]
    }

    fn valid(&self) -> bool {
        (0..3).all(|i| self.min[i] <= self.max[i])
    }

    fn contains(&self, p: V3) -> bool {
        (0..3).all(|i| p[i] >= f64::from(self.min[i]) && p[i] <= f64::from(self.max[i]))
    }
}

/// Named view; the angle pairs are Forge's `CameraShot` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Eye yaw 35, pitch 30.
    ThreeQuarter,
    /// Eye yaw 0, pitch 10.
    Front,
    /// Eye yaw 180, pitch 10.
    Back,
    /// Eye yaw 90, pitch 10.
    Side,
    /// Eye yaw 0, pitch 89.
    TopDown,
    /// Behind the target's facing yaw (`FrameSpec::yaw_deg`), pitch 20.
    ThirdPerson,
    /// Eye yaw and pitch from the spec.
    Orbit,
}

impl View {
    /// Every view, in report order.
    pub const ALL: [Self; 7] = [
        Self::ThreeQuarter,
        Self::Front,
        Self::Back,
        Self::Side,
        Self::TopDown,
        Self::ThirdPerson,
        Self::Orbit,
    ];

    /// Door word.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ThreeQuarter => "three_quarter",
            Self::Front => "front",
            Self::Back => "back",
            Self::Side => "side",
            Self::TopDown => "top_down",
            Self::ThirdPerson => "third_person",
            Self::Orbit => "orbit",
        }
    }

    /// Default fill in percent (third person keeps the subject small).
    pub const fn default_fill_pct(self) -> i32 {
        match self {
            Self::ThirdPerson => 40,
            _ => 85,
        }
    }

    fn eye_angles(self, spec: &FrameSpec) -> (i32, i32) {
        match self {
            Self::ThreeQuarter => (35, 30),
            Self::Front => (0, 10),
            Self::Back => (180, 10),
            Self::Side => (90, 10),
            Self::TopDown => (0, 89),
            Self::ThirdPerson => (spec.yaw_deg + 180, 20),
            Self::Orbit => (spec.yaw_deg, spec.pitch_deg),
        }
    }
}

/// Lens and view choice for [`frame_fit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSpec {
    /// Which view.
    pub view: View,
    /// Vertical field of view in degrees, 20..=120.
    pub fov_deg: i32,
    /// Width over height in percent (16:9 = 178, 4:3 = 133), 50..=300.
    pub aspect_pct: i32,
    /// Share of the frame the box may fill, percent, 10..=100.
    pub fill_pct: i32,
    /// Orbit eye yaw, or the target facing yaw for third person.
    pub yaw_deg: i32,
    /// Orbit eye pitch in degrees, -89..=89.
    pub pitch_deg: i32,
}

/// Camera pose that frames a box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameFit {
    /// Eye position in cm.
    pub eye_cm: [i32; 3],
    /// Box centre in cm (the point looked at).
    pub look_at_cm: [i32; 3],
    /// Eye to centre distance in cm.
    pub distance_cm: i32,
    /// Look yaw in degrees (eye yaw + 180).
    pub yaw_deg: i32,
    /// Look pitch in degrees, negative when looking down.
    pub pitch_deg: i32,
}

fn check_spec(spec: &FrameSpec) -> Result<(), FrameError> {
    let ok = |lo, hi, v: i32| (lo..=hi).contains(&v);
    if !ok(20, 120, spec.fov_deg) {
        return Err(FrameError::OutOfRange("fov_deg must be 20..=120"));
    }
    if !ok(50, 300, spec.aspect_pct) {
        return Err(FrameError::OutOfRange("aspect_pct must be 50..=300"));
    }
    if !ok(10, 100, spec.fill_pct) {
        return Err(FrameError::OutOfRange("fill_pct must be 10..=100"));
    }
    if !ok(-89, 89, spec.pitch_deg) {
        return Err(FrameError::OutOfRange("pitch_deg must be -89..=89"));
    }
    Ok(())
}

fn corners(b: &Box3) -> [V3; 8] {
    let h = |i: usize| (f64::from(b.max[i]) - f64::from(b.min[i])) / 2.0;
    std::array::from_fn(|n| std::array::from_fn(|i| if n >> i & 1 == 1 { h(i) } else { -h(i) }))
}

/// Nearest eye on the view ray from the box centre that keeps all eight
/// corners inside `fill_pct` of the frame on both axes (closed form: the
/// largest of sixteen per-corner distances, never under 50 cm).
pub fn frame_fit(subject: &Box3, spec: &FrameSpec) -> Result<FrameFit, FrameError> {
    if !subject.valid() {
        return Err(FrameError::EmptyBox);
    }
    check_spec(spec)?;
    let (eye_yaw, eye_pitch) = spec.view.eye_angles(spec);
    let (forward, right, up) = basis(eye_yaw + 180, -eye_pitch);
    let ty = tan_half_fov(spec.fov_deg) * f64::from(spec.fill_pct) / 100.0;
    let tx = ty * f64::from(spec.aspect_pct) / 100.0;
    let mut dist = f64::from(MIN_DISTANCE_CM);
    for p in corners(subject) {
        let along = dot(p, forward);
        dist = dist
            .max(dot(p, right).abs() / tx - along)
            .max(dot(p, up).abs() / ty - along);
    }
    let centre = subject.center();
    let eye = |i: usize| cm(centre[i] - forward[i] * dist);
    let at = |i: usize| cm(centre[i]);
    Ok(FrameFit {
        eye_cm: [eye(0), eye(1), eye(2)],
        look_at_cm: [at(0), at(1), at(2)],
        distance_cm: cm(dist.ceil()),
        yaw_deg: (eye_yaw + 180).rem_euclid(360),
        pitch_deg: -eye_pitch,
    })
}

/// What a camera pose sees of one subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// Share of the 32 x 32 ray grid whose first hit is the subject, permille.
    pub permille: u32,
    /// The camera looks away from the subject (forward . to-target <= 0).
    pub looks_away: bool,
    /// The camera is upside down (`up.y < 0`, pitch past +-90).
    pub flipped: bool,
    /// Refusal-style next step when under [`HIDDEN_PERMILLE`], else empty.
    pub next: String,
}

/// Camera for [`coverage`]: eye in cm, look yaw and pitch in degrees (pitch
/// past +-90 flips the camera), lens as in [`FrameSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Camera {
    /// Eye position in cm.
    pub eye_cm: [i32; 3],
    /// Look yaw in degrees.
    pub yaw_deg: i32,
    /// Look pitch in degrees, -180..=180.
    pub pitch_deg: i32,
    /// Vertical field of view in degrees.
    pub fov_deg: i32,
    /// Width over height in percent.
    pub aspect_pct: i32,
}

/// Entry distance of a ray into a box (slab test; 0 when the origin is
/// inside), or `None` on a miss.
fn hit_t(origin: V3, dir: V3, b: &Box3) -> Option<f64> {
    let (mut near, mut far) = (0.0_f64, f64::INFINITY);
    for i in 0..3 {
        let (lo, hi) = (f64::from(b.min[i]), f64::from(b.max[i]));
        if dir[i].abs() < 1e-12 {
            if origin[i] < lo || origin[i] > hi {
                return None;
            }
        } else {
            let (a, c) = ((lo - origin[i]) / dir[i], (hi - origin[i]) / dir[i]);
            near = near.max(a.min(c));
            far = far.min(a.max(c));
        }
    }
    (near <= far).then_some(near)
}

/// Share of the frame the subject covers from `cam`, with occlusion by
/// every box in `boxes` (the subject is the box with `subject_id`).
pub fn coverage(cam: &Camera, subject_id: u32, boxes: &[Box3]) -> Result<Coverage, FrameError> {
    let spec = FrameSpec {
        view: View::Orbit,
        fov_deg: cam.fov_deg,
        aspect_pct: cam.aspect_pct,
        fill_pct: 100,
        yaw_deg: 0,
        pitch_deg: 0,
    };
    check_spec(&spec)?;
    if !(-180..=180).contains(&cam.pitch_deg) {
        return Err(FrameError::OutOfRange("pitch_deg must be -180..=180"));
    }
    let subject = boxes
        .iter()
        .find(|b| b.id == subject_id)
        .ok_or(FrameError::OutOfRange("subject id is not in boxes"))?;
    let eye = [
        f64::from(cam.eye_cm[0]),
        f64::from(cam.eye_cm[1]),
        f64::from(cam.eye_cm[2]),
    ];
    let (forward, right, up) = basis(cam.yaw_deg, cam.pitch_deg);
    let to_target = {
        let c = subject.center();
        [c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]]
    };
    let looks_away = !subject.contains(eye) && dot(forward, to_target) <= 0.0;
    let flipped = up[1] < 0.0;
    let mut hits = 0u32;
    if !looks_away {
        let ty = tan_half_fov(cam.fov_deg);
        let tx = ty * f64::from(cam.aspect_pct) / 100.0;
        for j in 0..GRID {
            for i in 0..GRID {
                let nx = (f64::from(2 * i + 1) / f64::from(GRID)) - 1.0;
                let ny = 1.0 - (f64::from(2 * j + 1) / f64::from(GRID));
                let dir: V3 =
                    std::array::from_fn(|k| forward[k] + nx * tx * right[k] + ny * ty * up[k]);
                let first = boxes
                    .iter()
                    .filter_map(|b| hit_t(eye, dir, b).map(|t| (t, b.id)))
                    .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                hits += u32::from(first.is_some_and(|(_, id)| id == subject_id));
            }
        }
    }
    let permille = hits * 1000 / RAYS;
    let next = if permille < HIDDEN_PERMILLE {
        format!("Next call: scene.frame target={subject_id} shot=medium for a pose that shows it")
    } else {
        String::new()
    };
    Ok(Coverage {
        permille,
        looks_away,
        flipped,
        next,
    })
}

/// Framing size of a shot: how much of the frame the subject fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shot {
    /// 40 percent: subject in its surroundings.
    Wide,
    /// 65 percent.
    Medium,
    /// 90 percent.
    Close,
}

impl Shot {
    const fn fill_pct(self) -> i32 {
        match self {
            Self::Wide => 40,
            Self::Medium => 65,
            Self::Close => 90,
        }
    }
}

/// One ranked camera candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// View that produced it.
    pub view: View,
    /// The pose.
    pub fit: FrameFit,
    /// What it sees of the subject.
    pub coverage: Coverage,
}

/// Candidate poses for `subject` among `boxes`, best first: eye inside `room`,
/// ranked by subject coverage (permille, high first), then view order. The
/// ranking uses coverage only; `compose::score_shot` is not applied here.
pub fn frame_shots(
    subject_id: u32,
    boxes: &[Box3],
    room: &Box3,
    shot: Shot,
    fov_deg: i32,
    aspect_pct: i32,
) -> Result<Vec<Candidate>, FrameError> {
    let subject = boxes
        .iter()
        .find(|b| b.id == subject_id)
        .ok_or(FrameError::OutOfRange("subject id is not in boxes"))?;
    let mut out = Vec::new();
    for view in [
        View::ThreeQuarter,
        View::Front,
        View::Back,
        View::Side,
        View::TopDown,
    ] {
        let spec = FrameSpec {
            view,
            fov_deg,
            aspect_pct,
            fill_pct: shot.fill_pct(),
            yaw_deg: 0,
            pitch_deg: 0,
        };
        let fit = frame_fit(subject, &spec)?;
        let eye = [
            f64::from(fit.eye_cm[0]),
            f64::from(fit.eye_cm[1]),
            f64::from(fit.eye_cm[2]),
        ];
        if !room.contains(eye) {
            continue;
        }
        let cam = Camera {
            eye_cm: fit.eye_cm,
            yaw_deg: fit.yaw_deg,
            pitch_deg: fit.pitch_deg,
            fov_deg,
            aspect_pct,
        };
        let coverage = coverage(&cam, subject_id, boxes)?;
        out.push(Candidate {
            view,
            fit,
            coverage,
        });
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.coverage.permille));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUBJECT: Box3 = Box3 {
        id: 1,
        min: [-100, 0, -50],
        max: [100, 180, 50],
    };

    /// Independent projection with std trig: largest frame use of any corner.
    fn use_of(fit: &FrameFit, spec: &FrameSpec) -> f64 {
        let (yaw, pitch) = (
            f64::from(fit.yaw_deg).to_radians(),
            f64::from(fit.pitch_deg).to_radians(),
        );
        let f = [
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        let up = [
            -yaw.sin() * pitch.sin(),
            pitch.cos(),
            -yaw.cos() * pitch.sin(),
        ];
        let r = unit(cross(up, f));
        let ty = (f64::from(spec.fov_deg).to_radians() / 2.0).tan();
        let tx = ty * f64::from(spec.aspect_pct) / 100.0;
        let eye = fit.eye_cm.map(f64::from);
        corners(&SUBJECT)
            .iter()
            .map(|p| {
                let c = SUBJECT.center();
                let rel = [
                    c[0] + p[0] - eye[0],
                    c[1] + p[1] - eye[1],
                    c[2] + p[2] - eye[2],
                ];
                let depth = dot(rel, f);
                (dot(rel, r).abs() / (depth * tx)).max(dot(rel, up).abs() / (depth * ty))
            })
            .fold(0.0, f64::max)
    }

    fn spec(view: View, fov: i32, aspect: i32) -> FrameSpec {
        FrameSpec {
            view,
            fov_deg: fov,
            aspect_pct: aspect,
            fill_pct: 85,
            yaw_deg: 20,
            pitch_deg: 25,
        }
    }

    #[test]
    fn frame_fit_corners_inside_and_use_matches_fill() {
        for view in View::ALL {
            for fov in [55, 60, 65] {
                for aspect in [178, 133] {
                    let s = spec(view, fov, aspect);
                    let fit = frame_fit(&SUBJECT, &s).expect("fits");
                    let used = use_of(&fit, &s);
                    assert!(
                        (used - 0.85).abs() <= 0.01,
                        "{} fov {fov} aspect {aspect}: use {used}",
                        view.as_str()
                    );
                    assert_eq!(fit.look_at_cm, [0, 90, 0]);
                    assert_eq!(
                        fit,
                        frame_fit(&SUBJECT, &s).expect("fits"),
                        "same bytes twice"
                    );
                }
            }
        }
    }

    #[test]
    fn frame_fit_top_down_and_third_person_geometry() {
        let top = frame_fit(&SUBJECT, &spec(View::TopDown, 60, 178)).expect("fits");
        assert!(
            top.eye_cm[1] > SUBJECT.max[1] && top.pitch_deg == -89,
            "{top:?}"
        );
        // Target faces yaw 20 (azimuth of its front); the eye must sit behind it.
        let s = spec(View::ThirdPerson, 60, 178);
        let fit = frame_fit(&SUBJECT, &s).expect("fits");
        let face = [
            f64::from(s.yaw_deg).to_radians().sin(),
            f64::from(s.yaw_deg).to_radians().cos(),
        ];
        let behind = f64::from(fit.eye_cm[0]) * face[0] + f64::from(fit.eye_cm[2]) * face[1];
        assert!(behind < 0.0, "eye is in front of the target: {fit:?}");
    }

    #[test]
    fn frame_fit_refuses_bad_input() {
        let flat = Box3 {
            id: 1,
            min: [5, 0, 0],
            max: [0, 1, 1],
        };
        assert_eq!(
            frame_fit(&flat, &spec(View::Front, 60, 178)),
            Err(FrameError::EmptyBox)
        );
        assert!(frame_fit(&SUBJECT, &spec(View::Front, 5, 178)).is_err());
        let tiny = Box3 {
            id: 1,
            min: [0; 3],
            max: [1; 3],
        };
        assert!(
            frame_fit(&tiny, &spec(View::Front, 60, 178))
                .expect("fits")
                .distance_cm
                >= MIN_DISTANCE_CM
        );
    }

    fn cam(eye: [i32; 3], yaw: i32, pitch: i32) -> Camera {
        Camera {
            eye_cm: eye,
            yaw_deg: yaw,
            pitch_deg: pitch,
            fov_deg: 60,
            aspect_pct: 178,
        }
    }

    const FAR: Box3 = Box3 {
        id: 1,
        min: [-50, 40, 250],
        max: [50, 140, 350],
    };

    #[test]
    fn coverage_inside_behind_flipped_and_stable() {
        let inside = coverage(&cam([0, 90, 300], 0, 0), 1, &[FAR]).expect("ok");
        assert_eq!(inside.permille, 1000);
        let seen = coverage(&cam([0, 90, 0], 0, 0), 1, &[FAR]).expect("ok");
        assert!((40..=100).contains(&seen.permille), "{seen:?}");
        assert!(seen.next.is_empty() && !seen.looks_away && !seen.flipped);
        let behind = coverage(&cam([0, 90, 600], 0, 0), 1, &[FAR]).expect("ok");
        assert_eq!(behind.permille, 0);
        assert!(
            behind.looks_away && behind.next.starts_with("Next call:"),
            "{behind:?}"
        );
        assert!(
            coverage(&cam([0, 90, 0], 0, 120), 1, &[FAR])
                .expect("ok")
                .flipped
        );
        assert_eq!(
            seen,
            coverage(&cam([0, 90, 0], 0, 0), 1, &[FAR]).expect("ok")
        );
    }

    #[test]
    fn coverage_pillar_over_half_the_view_halves_the_share() {
        let pillar = Box3 {
            id: 2,
            min: [0, 0, 140],
            max: [400, 400, 160],
        };
        let clear = coverage(&cam([0, 90, 0], 0, 0), 1, &[FAR])
            .expect("ok")
            .permille;
        let half = coverage(&cam([0, 90, 0], 0, 0), 1, &[FAR, pillar])
            .expect("ok")
            .permille;
        assert!(
            half > 0 && half.abs_diff(clear / 2) <= clear / 10,
            "clear {clear} half {half}"
        );
    }

    #[test]
    fn frame_shots_rank_the_unblocked_view_first() {
        let room = Box3 {
            id: 0,
            min: [-600, 0, -600],
            max: [600, 400, 600],
        };
        let blocker = Box3 {
            id: 2,
            min: [-300, 0, 150],
            max: [300, 300, 180],
        };
        let boxes = [SUBJECT, blocker];
        let shots = frame_shots(1, &boxes, &room, Shot::Medium, 60, 178).expect("shots");
        assert!(!shots.is_empty());
        assert!(shots
            .windows(2)
            .all(|w| w[0].coverage.permille >= w[1].coverage.permille));
        let front = shots
            .iter()
            .find(|c| c.view == View::Front)
            .expect("front fits the room");
        assert!(
            shots[0].coverage.permille > front.coverage.permille,
            "blocked front must not win"
        );
        assert_eq!(
            shots,
            frame_shots(1, &boxes, &room, Shot::Medium, 60, 178).expect("shots")
        );
    }
}
