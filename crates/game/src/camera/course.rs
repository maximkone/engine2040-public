use super::*;

// ---------------------------------------------------------------------------
// Traversal-course rule sets: the GE1 `course` schema (version 3) loads here
// so the mode framework boots it with no engine edits. Validation mirrors
// the GE1 authority (`tests/ge1_genres.rs`): ticks, 1..=2 runners, cruise
// speed, ordered ledges, chase-camera table, ordered run tweaks.
// ---------------------------------------------------------------------------

/// Chase-camera parameters as data (presentation only, never hashed): how
/// far behind the runner the eye sits and how high over it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CourseCamera {
    /// Chase distance behind the runner in meters, `1.0..=20.0`.
    pub distance_m: f32,
    /// Eye height over the runner in meters, `1.0..=20.0`.
    pub height_m: f32,
}

impl CourseCamera {
    /// Range-check the camera data; a bad table fails before use. Pure.
    pub fn validate(self) -> Result<(), FixtureError> {
        if !self.distance_m.is_finite()
            || !(COURSE_CAM_MIN_M..=COURSE_CAM_MAX_M).contains(&self.distance_m)
        {
            return Err(FixtureError::BadCamera("distance_m"));
        }
        if !self.height_m.is_finite()
            || !(COURSE_CAM_MIN_M..=COURSE_CAM_MAX_M).contains(&self.height_m)
        {
            return Err(FixtureError::BadCamera("height_m"));
        }
        Ok(())
    }

    /// Derive the director table for this chase camera as data: the boom
    /// shape reproduces the fixture geometry exactly (eye sits `height_m`
    /// over and `distance_m` behind the look point), the margin is zero so
    /// the follow distance never chases, and the rails bracket the eye
    /// length with headroom so the agent is always framable. Total over
    /// validated tables; the single `sqrt` runs at table-build time
    /// (presentation mapping, never sim state). Pure.
    #[must_use]
    pub fn director_table(&self) -> DirectorParams {
        let eye_len = (self.distance_m * self.distance_m + self.height_m * self.height_m).sqrt();
        DirectorParams {
            fov_y_rad: 70.0f32.to_radians(),
            fit_margin_m: 0.0,
            base_dist_m: self.distance_m,
            min_dist_m: self.distance_m * 0.5,
            max_dist_m: eye_len * 1.1,
            boom_up: self.height_m / self.distance_m,
            boom_back: 1.0,
            max_shake_m: 0.25,
            decay_permille_per_tick: 45,
            ..DirectorParams::default()
        }
    }
}

/// One course ledge as data: position along the course and top height.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CourseLedge {
    /// Ledge position along the course in meters.
    pub x_m: f32,
    /// Ledge top height in meters (authored in millimetres).
    pub top_m: f32,
}

/// Validated traversal-course rules as data (GE1 `course` schema, version
/// 3). Built only by [`CourseRules::parse`], so every instance is already
/// range-checked.
#[derive(Clone, PartialEq, Debug)]
pub struct CourseRules {
    pub(super) version: u16,
    pub(super) seed: u64,
    pub(super) ticks: u32,
    pub(super) runners: u8,
    pub(super) run_speed_mps: f32,
    pub(super) camera: CourseCamera,
    pub(super) ledges: Vec<CourseLedge>,
    pub(super) moves: Vec<FixtureInput>,
}

impl CourseRules {
    /// Parse plus validate one traversal-course rule-set text (the
    /// `fixtures/games/traversal-course/rules.json` schema). Ranges mirror
    /// the GE1 authority; unknown keys are ignored; the run-tweak script
    /// must be strictly ordered (it may be empty: a coasting course still
    /// simulates). Budget: O(ledges + moves) time, two small `Vec`s at the
    /// load boundary (never per tick).
    pub fn parse(text: &str) -> Result<Self, FixtureError> {
        let version = fixture_num_after(text, "version")? as u16;
        if version != COURSE_RULES_VERSION {
            return Err(FixtureError::BadVersion(version));
        }
        if !fixture_str_is(text, "fixture", "\"traversal-course\"")? {
            return Err(FixtureError::BadValue("fixture"));
        }
        if !fixture_str_is(text, "mode", "\"course\"")? {
            return Err(FixtureError::UnknownMode);
        }
        let seed = fixture_num_after(text, "seed")? as u64;
        let ticks = fixture_num_after(text, "ticks")? as u32;
        let runners = fixture_num_after(text, "runners")? as u8;
        let run_speed_mps = fixture_num_after(text, "run_speed_mps")? as f32;
        if ticks == 0 || ticks > FIXTURE_MAX_TICKS {
            return Err(FixtureError::BadTicks(ticks));
        }
        if !(1..=COURSE_MAX_RUNNERS).contains(&runners) {
            return Err(FixtureError::BadRunners(runners));
        }
        if !run_speed_mps.is_finite()
            || !(COURSE_SPEED_MIN_MPS..=COURSE_SPEED_MAX_MPS).contains(&run_speed_mps)
        {
            return Err(FixtureError::BadSpeed);
        }
        let ledges = parse_course_ledges(text)?;
        let camera_block = text
            .find("\"camera\"")
            .and_then(|start| {
                text[start..]
                    .find('{')
                    .map(|open| &text[start + open + 1..])
            })
            .ok_or(FixtureError::MissingKey("camera"))?;
        let camera_end = camera_block
            .find('}')
            .ok_or(FixtureError::BadRules("camera"))?;
        let camera_text = &camera_block[..camera_end];
        let camera = CourseCamera {
            distance_m: fixture_num_after(camera_text, "distance_m")? as f32,
            height_m: fixture_num_after(camera_text, "height_m")? as f32,
        };
        camera.validate()?;
        let moves = parse_course_moves(text)?;
        check_fixture_inputs(ticks, runners, &moves)?;
        Ok(Self {
            version,
            seed,
            ticks,
            runners,
            run_speed_mps,
            camera,
            ledges,
            moves,
        })
    }
}

/// True when `"key": "want"` (whitespace-tolerant, exact quoted match).
pub(super) fn fixture_str_is(
    text: &str,
    key: &'static str,
    want: &str,
) -> Result<bool, FixtureError> {
    // One small parse-time allocation at the load boundary (never per tick).
    let pattern = ["\"", key, "\""].concat();
    let start = text.find(&pattern).ok_or(FixtureError::MissingKey(key))?;
    let colon = text[start..].find(':').ok_or(FixtureError::BadValue(key))?;
    Ok(text[start + colon + 1..].trim_start().starts_with(want))
}

/// Parse the course `ledges` array: `{x, top_mm}` objects in strictly
/// increasing x, tops in millimetres. Non-empty (GE1 mirror).
pub(super) fn parse_course_ledges(text: &str) -> Result<Vec<CourseLedge>, FixtureError> {
    let start = text
        .find("\"ledges\"")
        .ok_or(FixtureError::MissingKey("ledges"))?;
    let open = text[start..]
        .find('[')
        .ok_or(FixtureError::BadRules("ledges"))?;
    let body = &text[start + open + 1..];
    let mut ledges = Vec::new();
    let mut cursor = 0usize;
    loop {
        let rest = &body[cursor..];
        let next_obj = rest.find('{');
        let next_close = rest.find(']').ok_or(FixtureError::BadRules("ledges"))?;
        match next_obj {
            Some(at) if at < next_close => {
                let obj_start = cursor + at;
                let obj_end = body[obj_start..]
                    .find('}')
                    .ok_or(FixtureError::BadRules("ledges"))?;
                let obj = &body[obj_start..obj_start + obj_end + 1];
                let x = fixture_num_after(obj, "x")? as f32;
                let top_mm = fixture_num_after(obj, "top_mm")? as f32;
                if !x.is_finite()
                    || !(COURSE_X_MIN_M..=COURSE_X_MAX_M).contains(&x)
                    || !top_mm.is_finite()
                    || !(COURSE_TOP_MIN_MM..=COURSE_TOP_MAX_MM).contains(&top_mm)
                    || ledges
                        .last()
                        .is_some_and(|prev: &CourseLedge| x <= prev.x_m)
                {
                    return Err(FixtureError::BadLedge(ledges.len()));
                }
                ledges.push(CourseLedge {
                    x_m: x,
                    top_m: top_mm / 1000.0,
                });
                cursor = obj_start + obj_end + 1;
                if ledges.len() > COURSE_MAX_LEDGES {
                    return Err(FixtureError::TooManyLedges);
                }
            }
            _ => break,
        }
    }
    if ledges.is_empty() {
        return Err(FixtureError::BadLedges(0));
    }
    Ok(ledges)
}

/// Parse the course `moves` array: `{tick, body, vx, vy}` run tweaks in
/// strictly increasing `(tick, body)` order (checked by the caller through
/// [`check_fixture_inputs`]). Empty scripts are legal: a coasting course
/// still simulates (GE1 mirror).
pub(super) fn parse_course_moves(text: &str) -> Result<Vec<FixtureInput>, FixtureError> {
    let start = text
        .find("\"moves\"")
        .ok_or(FixtureError::MissingKey("moves"))?;
    let open = text[start..]
        .find('[')
        .ok_or(FixtureError::BadRules("moves"))?;
    let body = &text[start + open + 1..];
    let mut moves = Vec::new();
    let mut cursor = 0usize;
    loop {
        let rest = &body[cursor..];
        let next_obj = rest.find('{');
        let next_close = rest.find(']').ok_or(FixtureError::BadRules("moves"))?;
        match next_obj {
            Some(at) if at < next_close => {
                let obj_start = cursor + at;
                let obj_end = body[obj_start..]
                    .find('}')
                    .ok_or(FixtureError::BadRules("moves"))?;
                let obj = &body[obj_start..obj_start + obj_end + 1];
                moves.push(FixtureInput {
                    tick: fixture_num_after(obj, "tick")? as u32,
                    body: fixture_num_after(obj, "body")? as u8,
                    vx: fixture_num_after(obj, "vx")? as f32,
                    vy: fixture_num_after(obj, "vy")? as f32,
                });
                cursor = obj_start + obj_end + 1;
                if moves.len() > FIXTURE_MAX_INPUTS {
                    return Err(FixtureError::TooManyInputs);
                }
            }
            _ => break,
        }
    }
    Ok(moves)
}

/// One booted traversal course: the authoritative [`FixtureSession`] (open
/// run, no goal disc) plus the presentation [`Director`] framing it under
/// the rule set's own chase-camera table. The framework boots this from
/// rule-set text and drives it tick by tick; camera records never enter
/// the sim hash.
pub struct CourseMode {
    pub(super) session: FixtureSession,
    pub(super) director: Director,
    pub(super) frames: Vec<CameraFrame>,
    /// Focus-agent plane per tick (the tracked agent's receipts).
    pub(super) agents: Vec<[f32; 2]>,
}

impl CourseMode {
    /// Boot from validated course rules under the rule set's own
    /// chase-camera table ([`CourseCamera::director_table`]): a bad derived
    /// table fails before any tick. Budget: O(runners + moves) time, three
    /// small `Vec`s at the boot boundary (never per tick).
    pub fn boot(rules: &CourseRules) -> Result<Self, CameraError> {
        let params = rules.camera.director_table();
        params.validate()?;
        let capacity = rules.ticks as usize;
        Ok(Self {
            session: FixtureSession::boot_course(rules),
            director: Director::new(0, params)?,
            frames: Vec::with_capacity(capacity),
            agents: Vec::with_capacity(capacity),
        })
    }

    /// Parse course rule-set text then boot under its own chase table.
    pub fn boot_from_text(rules_text: &str) -> Result<Self, FixtureError> {
        let rules = CourseRules::parse(rules_text)?;
        Self::boot(&rules).map_err(|_| FixtureError::BadCameraTable("row"))
    }

    /// True once every configured tick has simulated.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.session.is_done()
    }

    /// Simulate one fixed tick plus the presentation camera: the director
    /// frames the runners (focus 0) and the focus plane is recorded.
    /// Returns false once the course is complete. Positions stay finite by
    /// construction (bounded well below overflow).
    pub fn step(&mut self) -> Result<bool, CameraError> {
        if self.session.is_done() {
            return Ok(false);
        }
        let tick = self.session.tick;
        self.session.step();
        let mut views = [FighterView {
            id: 0,
            pos_m: Vec3::ZERO,
            alive: true,
        }; 8];
        let runners = fixture_views_into(self.session.positions(), &mut views);
        let frame = self.director.update(&views[..runners], tick)?;
        self.frames.push(frame);
        self.agents.push(self.session.positions()[0]);
        Ok(!self.session.is_done())
    }

    /// Drive the course to completion: sealed tape plus the director frames
    /// and the focus-agent planes.
    pub fn run(mut self) -> Result<CoursePlayback, CameraError> {
        while self.step()? {}
        let session = self.session;
        Ok(CoursePlayback {
            tape: FixtureTape {
                checkpoints: session.checkpoints,
                win_tick: session.win_tick,
                winner: session.winner,
            },
            frames: self.frames,
            agents: self.agents,
        })
    }
}

/// Played traversal course: sealed sim tape plus the director frames and
/// the focus-agent planes recorded alongside it (presentation receipts).
#[derive(Clone, PartialEq, Debug)]
pub struct CoursePlayback {
    /// Authoritative sim record: replays byte-exact under the same rules.
    pub tape: FixtureTape,
    /// Director frame per tick (presentation only, never hashed).
    pub frames: Vec<CameraFrame>,
    /// Focus-agent plane per tick (presentation only, never hashed).
    pub agents: Vec<[f32; 2]>,
}

impl CoursePlayback {
    /// Replay the authoritative tape under its course rules with
    /// first-divergence naming; camera receipts replay by equality (run
    /// twice, compare).
    pub fn replay(&self, rules: &CourseRules) -> Result<(), FixtureError> {
        self.tape.replay_course(rules)
    }
}
