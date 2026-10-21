//! ossim tests: timeline, refusal, swap, scoring, determinism and stage paths.

use super::*;

const LAB_MIN: &[(u32, u8)] = &[(0, 1)];

fn lab() -> Stage {
    Stage {
        id: 0,
        name: "LAB",
        start_processes: 1,
        spawn_first: 0,
        spawn_every: 0,
        spawn_count: 0,
        min_curve: LAB_MIN,
        ram_rows: 2,
        swap_slots: 8,
        swap_ticks: (6, 6),
        win_tick: 100_000,
        max_kills: 10,
        priority_every: 0,
    }
}

fn lab_start(count: u8) -> Stage {
    Stage {
        start_processes: count,
        ..lab()
    }
}

fn silent() -> Config {
    Config {
        ambient_period: 0,
        ..Config::default()
    }
}

fn process(sim: &Sim, id: u32) -> Process {
    sim.state()
        .processes
        .iter()
        .flatten()
        .find(|p| p.id == id)
        .copied()
        .expect("process id exists")
}

fn assign(sim: &mut Sim, cpu: u8, id: u32) -> Result<(), Refusal> {
    sim.tick(Some(Action::Assign { cpu, process: id }))
}

fn autopilot(sim: &Sim) -> Option<Action> {
    let state = sim.state();
    let free = state.cpus.iter().position(Option::is_none);
    let ready = state
        .processes
        .iter()
        .flatten()
        .filter(|p| p.state == ProcState::Idle && p.resident && p.pending == Pending::None)
        .max_by_key(|p| p.wait_ticks);
    if let (Some(cpu), Some(p)) = (free, ready) {
        return Some(Action::Assign {
            cpu: u8::try_from(cpu).expect("cpu slot fits in u8"),
            process: p.id,
        });
    }
    state
        .processes
        .iter()
        .flatten()
        .filter(|p| !p.resident && p.pending == Pending::None && p.state == ProcState::BlockedPage)
        .min_by_key(|p| p.id)
        .map(|p| Action::SwapIn { process: p.id })
}

fn tape_action(tick: u64) -> Option<Action> {
    if tick.is_multiple_of(97) {
        Some(Action::Assign {
            cpu: u8::try_from(tick % 5).ok()?,
            process: u32::try_from(tick / 97 % 12).ok()?,
        })
    } else if tick.is_multiple_of(131) {
        Some(Action::SwapIn {
            process: u32::try_from(tick / 131 % 12).ok()?,
        })
    } else if tick.is_multiple_of(173) {
        Some(Action::DoIo {
            process: u32::try_from(tick / 173 % 12).ok()?,
        })
    } else if tick.is_multiple_of(211) {
        Some(Action::Unassign {
            cpu: u8::try_from(tick % 4).ok()?,
        })
    } else {
        None
    }
}

fn run_tape(seed: u64) -> (u64, i32, Option<Outcome>) {
    let mut sim = Sim::new(seed, STAGES[3]);
    for tick in 0..3_000 {
        let _ = sim.tick(tape_action(tick));
    }
    (sim.hash(), sim.score(), sim.outcome())
}

#[test]
fn same_tape_twice_gives_the_same_hash() {
    let first = run_tape(2040);
    let second = run_tape(2040);
    assert_eq!(first, second);
    assert_ne!(first.0, 0, "hash is a real fold");
}

#[test]
fn hash_depends_on_the_seed() {
    let (a, _, _) = run_tape(2040);
    let (b, _, _) = run_tape(2041);
    assert_ne!(a, b, "CounterRng expands adjacent seeds apart");
}

#[test]
fn starvation_climbs_every_600_ticks() {
    let mut sim = Sim::with_config(7, lab(), silent());
    for _ in 0..599 {
        sim.tick(None).expect("time advances");
    }
    assert_eq!(process(&sim, 0).starvation, 0);
    sim.tick(None).expect("time advances");
    assert_eq!(process(&sim, 0).starvation, 1);
    for _ in 0..600 {
        sim.tick(None).expect("time advances");
    }
    assert_eq!(process(&sim, 0).starvation, 2);
}

#[test]
fn priority_starvation_climbs_every_360_ticks() {
    let stage = Stage {
        priority_every: 1,
        ..lab()
    };
    let mut sim = Sim::with_config(7, stage, silent());
    assert!(process(&sim, 0).priority);
    for _ in 0..359 {
        sim.tick(None).expect("time advances");
    }
    assert_eq!(process(&sim, 0).starvation, 0);
    sim.tick(None).expect("time advances");
    assert_eq!(process(&sim, 0).starvation, 1);
}

#[test]
fn happiness_resets_after_300_running_ticks() {
    let mut sim = Sim::with_config(7, lab(), silent());
    for _ in 0..600 {
        sim.tick(None).expect("time advances");
    }
    assert_eq!(process(&sim, 0).starvation, 1);
    assign(&mut sim, 0, 0).expect("assign");
    for _ in 0..299 {
        sim.tick(None).expect("time advances");
    }
    let p = process(&sim, 0);
    assert_eq!(p.state, ProcState::Running);
    assert_eq!(p.starvation, 0);
    assert_eq!(p.run_streak, 0, "the reset restarts the streak");
}

#[test]
fn ten_kills_reboot_the_machine() {
    let mut sim = Sim::new(2040, STAGES[0]);
    let mut guard = 0;
    while sim.outcome().is_none() && guard < 12_000 {
        sim.tick(None).expect("time advances");
        guard += 1;
    }
    assert_eq!(sim.outcome(), Some(Outcome::Lost(LossReason::Reboot)));
    assert_eq!(sim.state().kills, 10);
    assert_eq!(sim.parts().kills, 10);
}

#[test]
fn page_fault_blocks_until_swap_in() {
    let mut sim = Sim::with_config(7, lab(), silent());
    assign(&mut sim, 0, 0).expect("assign");
    assert_eq!(sim.fault(0), Ok(()));
    let p = process(&sim, 0);
    assert_eq!(p.state, ProcState::BlockedPage);
    assert_eq!(p.pending, Pending::SwapOut);
    assert!(p.resident, "pages leave RAM only when the swap out ends");
    assert_eq!(sim.state().cpus[0], None);
    assert_eq!(sim.parts().faults, 1);
    assert_eq!(sim.score(), SCORE_PAGE_FAULT);
    assert!(sim
        .events()
        .iter()
        .any(|e| e.kind == EventKind::PageFault && e.process == 0));

    for _ in 0..7 {
        sim.tick(None).expect("time advances");
    }
    let p = process(&sim, 0);
    assert_eq!(p.pending, Pending::None);
    assert!(!p.resident);
    assert_eq!(p.state, ProcState::BlockedPage);
    assert!(sim.events().iter().any(|e| e.kind == EventKind::SwapDone));

    assert_eq!(
        assign(&mut sim, 0, 0),
        Err(Refusal::WrongState),
        "a swapped process cannot run"
    );
    sim.tick(Some(Action::SwapIn { process: 0 }))
        .expect("swap in");
    for _ in 0..7 {
        sim.tick(None).expect("time advances");
    }
    let p = process(&sim, 0);
    assert!(p.resident);
    assert_eq!(p.state, ProcState::Idle);
    assign(&mut sim, 0, 0).expect("assign after swap in");
}

#[test]
fn io_ready_delivers_after_the_timer() {
    let cfg = Config {
        io_ticks: (60, 60),
        ..silent()
    };
    let mut sim = Sim::with_config(7, lab(), cfg);
    assign(&mut sim, 0, 0).expect("assign");
    sim.tick(Some(Action::DoIo { process: 0 })).expect("io");
    let p = process(&sim, 0);
    assert_eq!(p.state, ProcState::BlockedIo);
    assert_eq!(p.io_ready, 61);
    assert_eq!(sim.state().io_len, 1);

    let mut ready = false;
    for _ in 0..61 {
        sim.tick(None).expect("time advances");
        ready |= sim
            .events()
            .iter()
            .any(|e| e.kind == EventKind::IoReady && e.process == 0);
    }
    assert!(ready, "IoReady event was delivered");
    let p = process(&sim, 0);
    assert_eq!(p.state, ProcState::Idle);
    assert_eq!(sim.state().io_len, 0);
    assert_eq!(sim.parts().io_ready, 1);
    assert_eq!(sim.score(), SCORE_IO_READY);
}

#[test]
fn assign_and_unassign_move_the_cpu() {
    let mut sim = Sim::with_config(7, lab(), silent());
    assign(&mut sim, 2, 0).expect("assign");
    assert_eq!(sim.state().cpus[2], Some(0));
    assert_eq!(process(&sim, 0).state, ProcState::Running);
    sim.tick(Some(Action::Unassign { cpu: 2 }))
        .expect("unassign");
    assert_eq!(sim.state().cpus[2], None);
    assert_eq!(process(&sim, 0).state, ProcState::Idle);
}

#[test]
fn invalid_actions_are_refused_without_panic() {
    let mut sim = Sim::with_config(7, lab_start(2), silent());
    assert_eq!(
        assign(&mut sim, 7, 0),
        Err(Refusal::BadCpu),
        "cpu slot out of range"
    );
    assert_eq!(
        assign(&mut sim, 0, 999),
        Err(Refusal::NoSuchProcess),
        "process id unknown"
    );
    assert_eq!(
        sim.tick(Some(Action::Unassign { cpu: 1 })),
        Err(Refusal::CpuEmpty)
    );
    assign(&mut sim, 0, 0).expect("first process");
    assert_eq!(assign(&mut sim, 0, 1), Err(Refusal::CpuBusy));
    assert_eq!(
        sim.tick(Some(Action::DoIo { process: 1 })),
        Err(Refusal::WrongState),
        "an idle process is not on the I/O path"
    );
    assert_eq!(sim.fault(1), Err(Refusal::WrongState));
    assert_eq!(
        sim.tick(Some(Action::SwapIn { process: 1 })),
        Err(Refusal::WrongState)
    );
}

#[test]
fn actions_after_the_outcome_are_refused() {
    let mut sim = Sim::new(2040, STAGES[0]);
    while sim.outcome().is_none() {
        sim.tick(None).expect("time advances");
    }
    let ended = sim.state().tick;
    assert_eq!(assign(&mut sim, 0, 0), Err(Refusal::Finished));
    sim.tick(None).expect("a quiet tick is allowed");
    assert_eq!(sim.state().tick, ended, "time stops after the stage ends");
}

#[test]
fn swap_out_needs_swap_room() {
    let stage = Stage {
        swap_slots: 0,
        ..lab()
    };
    let mut sim = Sim::with_config(7, stage, silent());
    assign(&mut sim, 0, 0).expect("assign");
    assert_eq!(
        sim.tick(Some(Action::SwapOut { process: 0 })),
        Err(Refusal::NoSwap)
    );
    assert!(process(&sim, 0).resident);
}

#[test]
fn a_full_swap_file_refuses_the_page_fault() {
    // The page fault has to land in the swap file. A full swap refuses it:
    // the process keeps its pages and no fault is counted.
    let stage = Stage {
        swap_slots: 0,
        ..lab()
    };
    let mut sim = Sim::with_config(7, stage, silent());
    assign(&mut sim, 0, 0).expect("assign");
    assert_eq!(sim.fault(0), Err(Refusal::NoSwap));
    assert!(process(&sim, 0).resident, "the process keeps its pages");
    assert_eq!(sim.parts().faults, 0, "a refused fault is not a fault");
}

#[test]
fn the_swap_file_never_holds_more_pages_than_it_owns() {
    // A tiny swap file with ambient faults and later spawns: the swap usage
    // must never pass the slots the stage owns. A swap on its way claims its
    // pages, so two swaps in flight may not both pass a check that only
    // looks at the pages already in swap.
    let stage = Stage {
        ram_rows: 2,
        swap_slots: 2,
        start_processes: 2,
        spawn_first: 30,
        spawn_every: 30,
        spawn_count: 6,
        ..lab()
    };
    let cfg = Config {
        ambient_period: 1,
        ..Config::default()
    };
    let mut sim = Sim::with_config(7, stage, cfg);
    for _ in 0..300 {
        // Keep the fighters on the CPUs so the ambient roll can fault them.
        let _ = sim.tick(Some(Action::Assign { cpu: 0, process: 0 }));
        let _ = sim.tick(Some(Action::Assign { cpu: 1, process: 1 }));
        let state = sim.state();
        assert!(
            state.swap_slots_used <= state.swap_slots_cap,
            "swap holds {} of {} at tick {}",
            state.swap_slots_used,
            state.swap_slots_cap,
            state.tick
        );
        assert!(
            state.ram_pages_used <= state.ram_pages_cap,
            "ram holds {} of {} at tick {}",
            state.ram_pages_used,
            state.ram_pages_cap,
            state.tick
        );
        let pages: u16 = state
            .processes
            .iter()
            .flatten()
            .filter(|p| p.state != ProcState::Done)
            .map(|p| u16::from(p.pages))
            .sum();
        assert_eq!(
            state.ram_pages_used + state.swap_slots_used,
            pages,
            "pages are counted once at tick {}",
            state.tick
        );
    }
}

#[test]
fn swap_in_needs_ram_room() {
    let stage = Stage {
        ram_rows: 0,
        ..lab()
    };
    let mut sim = Sim::with_config(7, stage, silent());
    let p = process(&sim, 0);
    assert!(!p.resident, "a process with no RAM starts in swap");
    assert_eq!(p.state, ProcState::BlockedPage);
    assert_eq!(
        sim.tick(Some(Action::SwapIn { process: 0 })),
        Err(Refusal::NoRam)
    );
}

#[test]
fn ram_and_swap_accounting_balances() {
    let stage = Stage {
        ram_rows: 1,
        swap_slots: 16,
        start_processes: 4,
        ..lab()
    };
    let mut sim = Sim::with_config(7, stage, silent());
    let state = sim.state();
    let pages: u16 = state
        .processes
        .iter()
        .flatten()
        .filter(|p| p.state != ProcState::Done)
        .map(|p| u16::from(p.pages))
        .sum();
    assert!(state.ram_pages_used <= state.ram_pages_cap);
    assert!(state.swap_slots_used <= state.swap_slots_cap);
    assert_eq!(state.ram_pages_used + state.swap_slots_used, pages);

    let room = state.swap_slots_cap - state.swap_slots_used;
    let resident = state
        .processes
        .iter()
        .flatten()
        .find(|p| {
            p.resident
                && p.pending == Pending::None
                && u16::from(p.pages) <= room
                && p.state != ProcState::Done
        })
        .map(|p| p.id)
        .expect("a resident process fits in swap");
    sim.tick(Some(Action::SwapOut { process: resident }))
        .expect("swap out");
    for _ in 0..7 {
        sim.tick(None).expect("time advances");
    }
    let state = sim.state();
    let pages: u16 = state
        .processes
        .iter()
        .flatten()
        .filter(|p| p.state != ProcState::Done)
        .map(|p| u16::from(p.pages))
        .sum();
    assert_eq!(state.ram_pages_used + state.swap_slots_used, pages);
    assert!(!process(&sim, resident).resident);
}

#[test]
fn swap_timing_stays_inside_the_stage_range() {
    let mut sim = Sim::new(7, STAGES[0]);
    assign(&mut sim, 0, 0).expect("assign");
    let before = sim.state().tick;
    sim.tick(Some(Action::SwapOut { process: 0 }))
        .expect("swap out");
    let p = process(&sim, 0);
    assert_eq!(p.pending, Pending::SwapOut);
    let wait = p.swap_done - before;
    assert!(
        (u64::from(SWAP_TICKS_MIN)..=u64::from(SWAP_TICKS_MAX)).contains(&wait),
        "swap wait {wait} outside the stage range"
    );
    let mut elapsed = 0u64;
    while process(&sim, 0).pending != Pending::None {
        sim.tick(None).expect("time advances");
        elapsed += 1;
        assert!(elapsed <= u64::from(SWAP_TICKS_MAX));
    }
    assert_eq!(elapsed, wait);
}

#[test]
fn stage_table_has_twelve_rows_and_a_six_minute_stage() {
    assert_eq!(STAGES.len(), 12);
    for (index, stage) in STAGES.iter().enumerate() {
        assert_eq!(usize::from(stage.id), index);
        assert!(stage.win_tick > 0);
        assert!(stage.ram_pages() <= RAM_PAGES);
        assert!(stage.swap_slots <= SWAP_SLOTS);
        for n in 0..stage.spawn_count {
            let tick = stage.spawn_tick(n).expect("spawn on the timeline");
            assert!(tick < stage.win_tick, "{} spawns after the win", stage.name);
        }
    }
    assert!(STAGES.iter().any(|s| s.win_tick == SIX_MINUTES));
    assert_eq!(STAGES[0].spawn_tick(0), Some(600));
    assert_eq!(STAGES[0].spawn_tick(17), None);
}

#[test]
fn every_stage_has_a_win_path() {
    for stage in STAGES {
        let mut sim = Sim::new(2040, stage);
        let mut guard = 0;
        while sim.outcome().is_none() && guard <= u64::from(stage.win_tick) + 8 {
            let action = autopilot(&sim);
            let _ = sim.tick(action);
            guard += 1;
        }
        assert_eq!(
            sim.outcome(),
            Some(Outcome::Won),
            "{} did not win under the fair autopilot",
            stage.name
        );
        assert!(sim.score() >= SCORE_WIN);
    }
}

#[test]
fn every_stage_has_a_loss_path() {
    for stage in STAGES {
        let mut sim = Sim::new(2040, stage);
        let mut guard = 0;
        while sim.outcome().is_none() && guard <= u64::from(stage.win_tick) + 8 {
            sim.tick(None).expect("time advances");
            guard += 1;
        }
        assert!(
            matches!(sim.outcome(), Some(Outcome::Lost(_))),
            "{} survived without a player",
            stage.name
        );
    }
}

#[test]
fn score_weights_are_additive() {
    let parts = ScoreParts {
        graceful: 2,
        io_ready: 3,
        kills: 1,
        faults: 4,
        won: false,
    };
    assert_eq!(parts.total(), 2 * 100 + 3 * 25 - 50 - 40);
    assert_eq!(
        ScoreParts {
            won: true,
            ..ScoreParts::default()
        }
        .total(),
        SCORE_WIN
    );
    assert_eq!(ScoreParts::default().total(), 0);
}

#[test]
fn score_always_matches_the_parts() {
    let mut sim = Sim::with_config(7, lab(), silent());
    assign(&mut sim, 0, 0).expect("assign");
    sim.tick(Some(Action::DoIo { process: 0 })).expect("io");
    for _ in 0..80 {
        sim.tick(None).expect("time advances");
    }
    assert_eq!(sim.score(), sim.parts().total());
    assert_eq!(sim.state().score, sim.score());
}

#[test]
fn grid_layout_wraps_after_42_cells() {
    assert_eq!(grid_cell(0), (0, 0));
    assert_eq!(grid_cell(6), (6, 0));
    assert_eq!(grid_cell(7), (0, 1));
    assert_eq!(grid_cell(41), (6, 5));
    assert_eq!(grid_cell(42), (0, 0));
    let mut sim = Sim::with_config(7, lab_start(2), silent());
    let state = sim.state();
    let p = state.processes[1].expect("second process");
    assert_eq!(p.cell(), (1, 0));
    sim.tick(None).expect("time advances");
}

#[test]
fn production_source_has_no_float_or_wall_clock() {
    let sources = [
        ("lib.rs", include_str!("lib.rs")),
        ("model.rs", include_str!("model.rs")),
        ("sim.rs", include_str!("sim.rs")),
        ("stages.rs", include_str!("stages.rs")),
    ];
    for (name, source) in sources {
        for token in ["f32", "f64", "std::time", "Instant", "SystemTime"] {
            assert!(!source.contains(token), "{name} must not contain {token}");
        }
    }
}

#[test]
fn events_are_cleared_at_each_tick() {
    let mut sim = Sim::new(7, STAGES[0]);
    for _ in 0..601 {
        sim.tick(None).expect("time advances");
    }
    assert!(sim.events().iter().any(|e| e.kind == EventKind::Spawn));
    sim.tick(None).expect("time advances");
    assert!(sim.events().is_empty(), "last tick produced no event");
}

#[test]
fn spawn_timeline_adds_processes() {
    let mut sim = Sim::new(7, STAGES[0]);
    assert_eq!(sim.live(), 1);
    for _ in 0..601 {
        sim.tick(None).expect("time advances");
    }
    assert_eq!(sim.live(), 2);
}

#[test]
fn graceful_finish_emits_an_event() {
    let mut sim = Sim::with_config(7, lab(), silent());
    assign(&mut sim, 0, 0).expect("assign");
    let mut finished = false;
    for _ in 0..1_600 {
        sim.tick(None).expect("time advances");
        finished |= sim
            .events()
            .iter()
            .any(|e| e.kind == EventKind::Graceful && e.process == 0);
        if finished {
            break;
        }
    }
    assert!(finished, "the process finished its work");
    assert_eq!(sim.parts().graceful, 1);
    assert_eq!(process(&sim, 0).state, ProcState::Done);
    assert_eq!(sim.state().cpus[0], None);
}

#[test]
fn min_curve_can_lose_the_stage() {
    const CURVE: &[(u32, u8)] = &[(0, 1), (300, 2)];
    let stage = Stage {
        min_curve: CURVE,
        ..lab()
    };
    let mut sim = Sim::with_config(7, stage, silent());
    let mut guard = 0;
    while sim.outcome().is_none() && guard < 400 {
        sim.tick(None).expect("time advances");
        guard += 1;
    }
    assert_eq!(
        sim.outcome(),
        Some(Outcome::Lost(LossReason::TooFewProcesses))
    );
    assert_eq!(sim.state().tick, 301);
}

#[test]
fn blocked_process_cannot_be_assigned() {
    let cfg = Config {
        io_ticks: (60, 60),
        ..silent()
    };
    let mut sim = Sim::with_config(7, lab(), cfg);
    assign(&mut sim, 0, 0).expect("assign");
    sim.tick(Some(Action::DoIo { process: 0 })).expect("io");
    assert_eq!(assign(&mut sim, 1, 0), Err(Refusal::WrongState));
    assert_eq!(sim.state().cpus[1], None);
}
