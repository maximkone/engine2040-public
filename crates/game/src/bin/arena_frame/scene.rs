//! The window's scene desk (rows SP62, EG35): answers the agent seat's `scene`
//! and `steer` lines and the `--scene-at` schedule on the frame it is about to
//! present, so an agent reads the live scene and steers the renderer.
//!
//! Consumer: `ArenaApp::present_frame`, which calls [`ArenaApp::serve_desk`]
//! once the frame's camera is final and [`ArenaApp::apply_pin`] when it
//! aims the camera. The grammar, the steer state and the reply shapes are
//! `engine_game::scene_desk`'s; the report is `engine_game::scene_report`'s;
//! this file owns only what needs the window: the presenter, the look, the
//! sim handle and the pause tick. Units: meters, ticks. Presentation only:
//! nothing here reaches the simulation or its hash; `hold` and `step` pace
//! wall time and never change what a tick computes. Errors: a refused line is
//! answered `{"ok":false,..}` and changes nothing.

use std::fs;

use engine_game::capture::fnv_hex;
use engine_game::fighter_look::{
    mesh_owner, owner_id, shot_name, shot_named, FighterLook, SceneRunSpec, Shot, SlotExtras,
};
use engine_game::scene_desk::{
    camera_field, capture_path, parse_request, reply_err, reply_ok, DeskError, Request, ShotPick,
    Steer, SteerState, Undo, CAPTURE_DIR,
};
use engine_game::scene_report::emit::{obj, q};
use engine_game::scene_report::geom::{unit, P3};
use engine_game::scene_report::things::{prop_things, spear_things, tally_pixels};
use engine_game::scene_report::{self, Laws, Light, Pixels, Report, SceneInput, ShadowMap, Thing};
use engine_render::pass_truth::LookToggles;
use engine_render::present::{sun_shadow_view_proj, DrawList};

use super::arena_args::{split_view_line, VIEW_LINE_FLAG};
use super::ArenaApp;

/// `--scene-report <SEED> <TICKS> <FIGHTERS>`: play a bout with no window and
/// print the scene report of its last frame (rows SP62, EG35).
pub(super) const SCENE_REPORT_FLAG: &str = "--scene-report";
/// `--scene-at <T[,T..]>`: in a window run, print the scene report when the
/// bout reaches each tick.
const SCENE_AT_FLAG: &str = "--scene-at";
/// `--shot <hero|wide|action>`: the director's shot for `--scene-report`.
const SHOT_FLAG: &str = "--shot";

/// The scene flags of one command line, split off before the strict parser.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SceneFlags {
    /// `--scene-report`: the windowless report request (seed, ticks, fighters).
    pub report: Option<(u64, u32, u8)>,
    /// `--shot`: the director's shot name for the report.
    pub shot: Option<String>,
    /// `--scene-at`: ticks at which a window run prints its report.
    pub at: Vec<u32>,
}

/// `--scene-report` takes the same three numbers under the same ranges as
/// `--view-line`, so it is read by that parser: the flag is renamed, split, and
/// a refusal names the flag the user typed.
fn split_report(raw: &[String]) -> Result<(Option<(u64, u32, u8)>, Vec<String>), String> {
    // A real `--view-line` belongs to that parser, which `run` calls first; leave it be.
    if raw.iter().any(|arg| arg == VIEW_LINE_FLAG) {
        return Ok((None, raw.to_vec()));
    }
    let renamed: Vec<String> = raw
        .iter()
        .map(|arg| {
            if arg == SCENE_REPORT_FLAG {
                VIEW_LINE_FLAG.to_owned()
            } else {
                arg.clone()
            }
        })
        .collect();
    split_view_line(&renamed).map_err(|err| err.replace(VIEW_LINE_FLAG, SCENE_REPORT_FLAG))
}

/// Pull `--scene-at <T[,T..]>` out of the argument list: the ticks ascending
/// and without repeats. A missing or unparsable list fails loudly.
fn split_at(raw: &[String]) -> Result<(Vec<u32>, Vec<String>), String> {
    let mut at: Vec<u32> = Vec::new();
    let mut rest = Vec::with_capacity(raw.len());
    let mut index = 0usize;
    while index < raw.len() {
        let arg = &raw[index];
        if arg == SCENE_AT_FLAG {
            let list = raw
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("{SCENE_AT_FLAG} needs a tick or a comma list of ticks"))?;
            for part in list.split(',') {
                let tick: u32 = part
                    .parse()
                    .map_err(|_| format!("{SCENE_AT_FLAG} tick {part}"))?;
                at.push(tick);
            }
            index += 2;
            continue;
        }
        rest.push(arg.clone());
        index += 1;
    }
    at.sort_unstable();
    at.dedup();
    Ok((at, rest))
}

/// Pull `--shot <NAME>` out of the argument list: `hero`, `wide` or `action`,
/// checked here so a typo fails at boot.
fn split_shot_flag(raw: &[String]) -> Result<(Option<String>, Vec<String>), String> {
    let Some(at) = raw.iter().position(|arg| arg == SHOT_FLAG) else {
        return Ok((None, raw.to_vec()));
    };
    let name = raw
        .get(at + 1)
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| format!("{SHOT_FLAG} needs hero, wide or action"))?;
    if shot_named(name).is_none() {
        return Err(format!("{SHOT_FLAG} {name} is not hero, wide or action"));
    }
    let mut rest = raw.to_vec();
    rest.drain(at..=at + 1);
    Ok((Some(name.clone()), rest))
}

/// Split the three scene flags off the argument list.
pub(super) fn split_flags(raw: &[String]) -> Result<(SceneFlags, Vec<String>), String> {
    let (report, rest) = split_report(raw)?;
    let (shot, rest) = split_shot_flag(&rest)?;
    let (at, rest) = split_at(&rest)?;
    Ok((SceneFlags { report, shot, at }, rest))
}

/// Prefix of the one JSON line `--scene-report` prints, mirrored by the door's
/// scene host (`engine_endpoint`'s `SCENE_REPORT_PREFIX`).
const SCENE_REPORT_PREFIX: &str = "arena scene-report: ";

/// `--scene-report <seed> <ticks> <fighters>`: play a bout with no window and
/// print its last frame's report, the JSON line and then the text lines. The
/// exit code is 0 for a report and 2 for a refusal.
pub(super) fn run_report(
    (seed, ticks, fighters): (u64, u32, u8),
    showcase: bool,
    seat: bool,
    shot: Option<String>,
) -> i32 {
    if seat {
        tell!("arena: --scene-report folds a bout; --agent-seat plays a window; they do not mix");
        return 2;
    }
    let spec = SceneRunSpec {
        seed,
        ticks,
        fighters,
        showcase,
        shot: shot.as_deref().and_then(shot_named).unwrap_or(Shot::Action),
        viewport: (1280, 720),
    };
    match FighterLook::scene_run(spec) {
        Ok(report) => {
            say!("{SCENE_REPORT_PREFIX}{}", report.json);
            for line in report.text.lines() {
                say!("arena scene-text| {line}");
            }
            0
        }
        Err(err) => {
            tell!("arena: --scene-report: {err}");
            2
        }
    }
}

/// A reply held until the camera it describes has been drawn once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Deferred {
    /// The operation that asked (`shot` or `pose`).
    op: &'static str,
}

/// What the window keeps for the desk between frames.
#[derive(Debug, Default)]
pub(super) struct Desk {
    /// The camera pin, the undo stack and the capture count.
    pub steer: SteerState,
    /// Ticks (ascending) at which `--scene-at` prints a report.
    pub scene_at: Vec<u32>,
    /// Replies waiting for the next frame's camera.
    deferred: Vec<Deferred>,
}

impl ArenaApp {
    /// Where each fighter stands, for the named shots.
    fn desk_roots(&self) -> Vec<(u8, P3)> {
        self.app
            .fighter_views()
            .iter()
            .map(|view| (view.slot, view.pos_m))
            .collect()
    }

    /// Replace the frame's camera when the desk pins one. Runs after the free
    /// camera, so a pin wins over every other aim.
    pub(super) fn apply_pin(&self, list: &mut DrawList) {
        let sun = unit(list.light.direction.to_array()).unwrap_or([0.0, 1.0, 0.0]);
        if let Some(camera) = self
            .desk
            .steer
            .camera(&list.camera, &self.desk_roots(), sun)
        {
            list.camera = camera;
        }
    }

    /// The renderer's draw-id buffer for `list` tallied per scene thing: which
    /// pixels of the picture each fighter, prop and spear really owns. A
    /// blocking readback of a second render of the frame, taken on request.
    fn desk_pixels(&mut self, list: &DrawList, things: &[Thing]) -> Result<Pixels, DeskError> {
        let presenter = self
            .presenter
            .as_mut()
            .ok_or_else(|| DeskError::new("no presenter yet"))?;
        let live = presenter
            .capture_live_pixels(list, &LookToggles::all_on())
            .map_err(|err| DeskError::new(format!("the id buffer: {err}")))?;
        let owners: Vec<String> = list
            .items
            .iter()
            .map(|item| owner_id(mesh_owner(item.mesh.id), things))
            .collect();
        Ok(tally_pixels(&live.ids, &owners))
    }

    /// Every scene thing the window holds now: the posed fighters, the staged
    /// props and the spears.
    fn desk_things(&self) -> Result<Vec<Thing>, DeskError> {
        let look = self
            .look
            .as_ref()
            .ok_or_else(|| DeskError::new("the window draws staged boxes, not the fighter look"))?;
        let extras: Vec<SlotExtras> = (0..self.config.fighters)
            .map(|slot| SlotExtras {
                hp: self.app.hp().get(usize::from(slot)).copied(),
                armed: self.app.weapon_name(slot).map(str::to_owned),
            })
            .collect();
        let mut things = look
            .scene_fighters(usize::from(self.config.fighters), self.last_alpha, &extras)
            .map_err(|err| DeskError::new(err.to_string()))?;
        // The skinned look clears the kit draws before it stages its ring, so a
        // window with the look bound collides with kit props it never draws.
        things.extend(prop_things(true, false));
        if let Some(scene) = self.spear.scene.as_ref() {
            things.extend(spear_things(
                scene,
                self.app.live_match().spear_view(),
                self.last_alpha,
            ));
        }
        Ok(things)
    }

    /// The report for the frame in `list`, from the live look, sim, spear scene
    /// and presenter; with `pixels`, plus the id-buffer tally.
    fn desk_report(&mut self, list: &DrawList, pixels: bool) -> Result<Report, DeskError> {
        let things = self.desk_things()?;
        let tallied = if pixels {
            Some(self.desk_pixels(list, &things)?)
        } else {
            None
        };
        let presenter = self
            .presenter
            .as_ref()
            .ok_or_else(|| DeskError::new("no presenter yet"))?;
        let facts = presenter.render_facts();
        let sun = unit(list.light.direction.to_array()).unwrap_or([0.0, 1.0, 0.0]);
        let shot = if self.desk.steer.pin_name() == "auto" {
            shot_name(engine_game::fighter_look::shot_for_frame(
                self.frames_presented,
                self.args.frames,
                self.args.camera_path.is_some(),
            ))
            .to_owned()
        } else {
            self.desk.steer.pin_name()
        };
        let mode = if self.desk.steer.pin_name() != "auto" {
            "pinned"
        } else if self.cam_mode == engine_game::camera::CameraMode::Free {
            "free"
        } else if self.legionary.is_some() {
            "eye"
        } else {
            "director"
        };
        let input = SceneInput {
            source: "window",
            tick: self.ticks_done,
            seed: self.config.seed,
            viewport: facts.size,
            camera: list.camera.clone(),
            shot,
            mode: mode.to_owned(),
            light: Light {
                sun_dir: sun,
                color: list.light.color.to_array(),
                intensity: list.light.intensity,
                sky_sun_dir: unit(engine_render::arena_day::ARENA_DAY_SUN_DIR),
            },
            shadow: ShadowMap {
                on: facts.sun_shadow.on,
                edge_px: facts.sun_shadow.edge_px,
                half_extent_m: facts.sun_shadow.half_extent_m,
                texel_m: facts.sun_shadow.texel_m,
                view_proj: sun_shadow_view_proj(sun),
            },
            contact: facts.contact_strength,
            passes: facts
                .passes
                .iter()
                .map(|p| (p.name.to_owned(), p.on))
                .collect(),
            things,
            laws: Laws {
                line: self.view.line().unwrap_or_default(),
                verdict: self.view.verdict(),
                warnings: self.view.warnings(),
            },
            pixels: tallied,
            renderer_live: true,
        };
        Ok(scene_report::build(&input))
    }

    /// The pass switches as a reply object.
    fn desk_passes(&self) -> String {
        let pairs: Vec<(&str, String)> = self
            .presenter
            .as_ref()
            .map(|p| p.pass_switches())
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.name, p.on.to_string()))
            .collect();
        obj(&pairs)
    }

    /// Answer the seat's lines, flush replies the pin was waiting on, and print
    /// the `--scene-at` reports that came due. Call once per presented frame
    /// with the final draw list, before it is submitted.
    pub(super) fn serve_desk(&mut self, list: &DrawList) {
        let tick = self.ticks_done;
        for held in std::mem::take(&mut self.desk.deferred) {
            let fields = [
                ("pin", q(&self.desk.steer.pin_name())),
                ("camera", camera_field(&list.camera)),
            ];
            self.desk_reply("steer", &reply_ok(held.op, tick, &fields));
        }
        let lines = self
            .seat
            .as_ref()
            .map(engine_game::agent_seat::Seat::take_requests)
            .unwrap_or_default();
        for line in lines {
            match parse_request(&line) {
                Ok(Request::Scene { pixels }) => {
                    let reply = match self.desk_report(list, pixels) {
                        Ok(report) => report.json,
                        Err(why) => reply_err("scene", "no_scene", &why.to_string()),
                    };
                    self.desk_reply("scene", &reply);
                }
                Ok(Request::Steer(op)) => {
                    if let Some(reply) = self.desk_steer(op, list) {
                        self.desk_reply("steer", &reply);
                    }
                }
                Err(why) => {
                    let head = if line.starts_with("scene") {
                        "scene"
                    } else {
                        "steer"
                    };
                    self.desk_reply(head, &reply_err(head, "bad_request", &why.to_string()));
                }
            }
        }
        while self.desk.scene_at.first().is_some_and(|at| *at <= tick) {
            self.desk.scene_at.remove(0);
            match self.desk_report(list, false) {
                Ok(report) => {
                    say!("arena scene: {}", report.json);
                    for text in report.text.lines() {
                        say!("arena scene| {text}");
                    }
                }
                Err(why) => tell!("arena scene: no report at tick {tick}: {why}"),
            }
        }
    }

    /// Send one reply line on the seat. No seat, no reply (a `--scene-at`
    /// run has none).
    fn desk_reply(&self, head: &str, body: &str) {
        if let Some(seat) = self.seat.as_ref() {
            seat.publish_reply(&format!("{head} {body}\n"));
        }
    }

    /// Apply one steer operation; `None` when the reply waits for the next
    /// frame's camera.
    fn desk_steer(&mut self, op: Steer, list: &DrawList) -> Option<String> {
        let tick = self.ticks_done;
        match op {
            Steer::Shot(pick) => self.desk_shot(&pick, list),
            Steer::Pose {
                eye,
                target,
                fov_deg,
            } => {
                self.desk.steer.pose(eye, target, fov_deg);
                if self
                    .desk
                    .steer
                    .camera(&list.camera, &self.desk_roots(), [0.0, 1.0, 0.0])
                    .is_none()
                {
                    self.desk.steer.undo();
                    return Some(reply_err(
                        "pose",
                        "bad_pose",
                        "the eye sits on the look point or the pose is not a camera",
                    ));
                }
                self.desk.deferred.push(Deferred { op: "pose" });
                None
            }
            Steer::Pass { name, on } => Some(self.desk_pass(&name, on)),
            Steer::Capture { tag } => Some(self.desk_capture(&tag, list)),
            Steer::Hold => {
                self.desk.steer.undo.push(Undo::Pause(self.pause_at_tick));
                self.pause_at_tick = Some(u64::from(tick));
                self.pause_announced = false;
                Some(reply_ok("hold", tick, &[("held", "true".to_owned())]))
            }
            Steer::Resume => {
                self.desk.steer.undo.push(Undo::Pause(self.pause_at_tick));
                self.pause_at_tick = None;
                self.pause_announced = false;
                Some(reply_ok("resume", tick, &[("held", "false".to_owned())]))
            }
            Steer::Step => {
                if self.pause_at_tick.is_none() {
                    self.desk.steer.undo.push(Undo::Pause(None));
                    self.pause_at_tick = Some(u64::from(tick));
                }
                self.step_requested = true;
                self.pause_announced = false;
                Some(reply_ok("step", tick, &[("runs_tick", tick.to_string())]))
            }
            Steer::State => Some(reply_ok(
                "state",
                tick,
                &[
                    ("pin", q(&self.desk.steer.pin_name())),
                    ("held", self.pause_at_tick.is_some().to_string()),
                    ("passes", self.desk_passes()),
                    ("captures", self.desk.steer.captures.to_string()),
                    ("undo_depth", self.desk.steer.undo.len().to_string()),
                ],
            )),
            Steer::Undo => Some(self.desk_undo(tick)),
        }
    }

    /// Pin a shot; a named shot the fighters cannot resolve is refused.
    fn desk_shot(&mut self, pick: &ShotPick, list: &DrawList) -> Option<String> {
        self.desk.steer.pick(pick);
        let sun = unit(list.light.direction.to_array()).unwrap_or([0.0, 1.0, 0.0]);
        let resolves = matches!(pick, ShotPick::Auto | ShotPick::Director(_))
            || self
                .desk
                .steer
                .camera(&list.camera, &self.desk_roots(), sun)
                .is_some();
        if !resolves {
            self.desk.steer.undo();
            return Some(reply_err(
                "shot",
                "bad_shot",
                "that shot has no camera here: close:N needs a fighter in slot N",
            ));
        }
        self.desk.deferred.push(Deferred { op: "shot" });
        None
    }

    /// Switch one render pass and remember the state it had.
    fn desk_pass(&mut self, name: &str, on: bool) -> String {
        let tick = self.ticks_done;
        let Some(presenter) = self.presenter.as_mut() else {
            return reply_err("pass", "no_renderer", "no presenter yet");
        };
        match presenter.set_pass(name, on) {
            Ok(was) => {
                let now = presenter
                    .pass_switches()
                    .iter()
                    .find(|p| p.name == name)
                    .is_some_and(|p| p.on);
                self.desk.steer.undo.push(Undo::Pass {
                    name: name.to_owned(),
                    was,
                });
                reply_ok(
                    "pass",
                    tick,
                    &[
                        ("name", q(name)),
                        ("was", was.to_string()),
                        ("on", now.to_string()),
                        ("passes", self.desk_passes()),
                    ],
                )
            }
            Err(err) => reply_err("pass", "bad_pass", &err.to_string()),
        }
    }

    /// Render the frame offscreen, write the PNG and say what was captured.
    fn desk_capture(&mut self, tag: &str, list: &DrawList) -> String {
        let tick = self.ticks_done;
        let Some(presenter) = self.presenter.as_mut() else {
            return reply_err("capture", "no_renderer", "no presenter yet");
        };
        let out = match presenter.render_offscreen(list) {
            Ok(out) => out,
            Err(err) => return reply_err("capture", "capture_failed", &err.to_string()),
        };
        let Some(png) = out.png.as_ref() else {
            return reply_err(
                "capture",
                "capture_failed",
                "the window is suspended: nothing was drawn",
            );
        };
        self.desk.steer.captures = self.desk.steer.captures.saturating_add(1);
        let path = capture_path(tag, tick, self.desk.steer.captures);
        let written = fs::create_dir_all(CAPTURE_DIR).and_then(|()| fs::write(&path, png));
        if let Err(err) = written {
            return reply_err(
                "capture",
                "capture_failed",
                &format!("{}: {err}", path.display()),
            );
        }
        let fields = [
            ("path", q(&path.to_string_lossy().replace('\\', "/"))),
            ("bytes", png.len().to_string()),
            ("fnv", q(&fnv_hex(png))),
            (
                "size",
                format!("[{},{}]", out.receipt.width, out.receipt.height),
            ),
            ("pin", q(&self.desk.steer.pin_name())),
            ("passes", self.desk_passes()),
            ("per_pass", q(&out.receipt.per_pass_summary())),
        ];
        reply_ok("capture", tick, &fields)
    }

    /// Take back the newest camera, pass or pause change.
    fn desk_undo(&mut self, tick: u32) -> String {
        let Some(undone) = self.desk.steer.undo() else {
            return reply_err(
                "undo",
                "nothing_to_undo",
                "no steer change is left to take back",
            );
        };
        let what = match undone {
            Undo::Pass { name, was } => {
                if let Some(presenter) = self.presenter.as_mut() {
                    let _ = presenter.set_pass(&name, was);
                }
                format!("pass {name}")
            }
            Undo::Pause(prev) => {
                self.pause_at_tick = prev;
                self.pause_announced = false;
                "pause".to_owned()
            }
            Undo::Pin(_) => "camera".to_owned(),
        };
        reply_ok(
            "undo",
            tick,
            &[
                ("undone", q(&what)),
                ("pin", q(&self.desk.steer.pin_name())),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn scene_report_splits_off_with_the_view_line_ranges_and_refuses_the_rest() {
        let (flags, rest) = split_flags(&strings(&[
            "arena",
            "--scene-report",
            "2040",
            "120",
            "2",
            "--showcase",
        ]))
        .expect("valid request");
        assert_eq!(flags.report, Some((2040, 120, 2)));
        assert_eq!(rest, strings(&["arena", "--showcase"]));
        let (none, same) = split_flags(&strings(&["arena", "--frames", "3"])).expect("no flag");
        assert_eq!((none, same.len()), (SceneFlags::default(), 3));
        // The view-line flag is not the scene flag, and the scene flag is not the view line.
        let line = strings(&["arena", "--view-line", "1", "10", "2"]);
        assert_eq!(split_flags(&line).expect("ok").0.report, None);
        for bad in [
            vec!["arena", "--scene-report"],
            vec!["arena", "--scene-report", "2040", "120"],
            vec!["arena", "--scene-report", "x", "120", "2"],
            vec!["arena", "--scene-report", "2040", "120", "1"],
            vec!["arena", "--scene-report", "2040", "120", "33"],
            vec!["arena", "--scene-report", "2040", "100001", "2"],
        ] {
            let err = split_flags(&strings(&bad)).expect_err("refused");
            assert!(
                err.contains("--scene-report") && !err.contains("--view-line"),
                "{bad:?}: {err}"
            );
        }
    }

    #[test]
    fn scene_at_takes_a_sorted_deduplicated_tick_list() {
        let (flags, rest) = split_flags(&strings(&[
            "arena",
            "--scene-at",
            "400,60,60,200",
            "--exit",
        ]))
        .expect("valid list");
        assert_eq!(flags.at, vec![60, 200, 400]);
        assert_eq!(rest, strings(&["arena", "--exit"]));
        for bad in [
            vec!["arena", "--scene-at"],
            vec!["arena", "--scene-at", "--exit"],
            vec!["arena", "--scene-at", "60,x"],
            vec!["arena", "--scene-at", "-5"],
        ] {
            assert!(split_flags(&strings(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn scene_shot_names_one_of_the_directors_three_and_the_flags_split_together() {
        let (flags, rest) = split_flags(&strings(&[
            "arena",
            "--shot",
            "wide",
            "--scene-at",
            "90",
            "--frames",
            "5",
        ]))
        .expect("valid flags");
        assert_eq!(flags.shot.as_deref(), Some("wide"));
        assert_eq!((flags.report, flags.at), (None, vec![90]));
        assert_eq!(rest, strings(&["arena", "--frames", "5"]));
        assert!(split_flags(&strings(&["arena", "--shot", "top"])).is_err());
        assert!(split_flags(&strings(&["arena", "--shot"])).is_err());
    }
}
