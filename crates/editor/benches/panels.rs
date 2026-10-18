//! panels bench (ED17): CPU frame cost of every dock panel on a siege-sized
//! scene.
//!
//! One dock frame is an egui pass plus tessellation, the same CPU work the
//! window host does before it uploads. Each panel is focused in turn and
//! timed over quiet frames (nothing edited, so cached rows are reused). The
//! bench prints a per-panel ms table and exits non-zero when a panel's median
//! frame is over the 1 ms budget. No GPU is involved, so there is no adapter
//! or driver line: the machine line names the CPU thread count instead.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "bench harness times wall clock outside the sim"
)]

use std::process::ExitCode;
use std::time::{Duration, Instant};

use engine_editor::commands::{Batch, Op};
use engine_editor::shell::{Panel, Shell};
use engine_editor::shell_ui::ShellUi;
use engine_editor::tuning::CombatField;
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

/// Per-panel median budget in milliseconds.
const BUDGET_MS: f64 = 1.0;
/// Scene size: the ED19 siege shape (64 players, 500 AI, props).
const NODES: usize = 2_000;
/// Untimed frames that load fonts and fill the row caches.
const WARM: usize = 8;
/// Timed frames per panel.
const FRAMES: usize = 201;

fn pose(x: f32) -> Transform {
    Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
}

/// Siege-sized scene with a selection, a staged proposal and tuning drafts,
/// so no panel draws its empty state.
fn scene() -> Shell {
    let mut shell = Shell::new();
    let root = ObjectId::new();
    shell
        .insert_node(root, ObjectId::NIL, pose(0.0))
        .expect("root inserts");
    let mut last = root;
    for index in 1..NODES {
        last = ObjectId::new();
        shell
            .insert_node(last, root, pose(index as f32))
            .expect("child inserts");
    }
    assert!(shell.select(last));
    let batch = Batch {
        ops: vec![Op::SetLocal {
            id: last,
            local: pose(0.0),
        }],
        base_revision: shell.revision(),
        idempotency_key: 17,
        provenance: None,
    };
    shell.stage_proposal(&batch).expect("proposal stages");
    shell
        .tune_set_combat(CombatField::AttackDamage, 240)
        .expect("draft sets");
    shell
}

/// Sorted per-frame times of `panel` drawn in a 1280x720 dock.
fn time_panel(shell: &mut Shell, ui_state: &mut ShellUi, panel: Panel) -> Vec<Duration> {
    assert!(ui_state.focus_panel(panel), "{panel} is docked");
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0));
    let mut frame = || {
        let input = egui::RawInput {
            screen_rect: Some(screen),
            ..egui::RawInput::default()
        };
        let start = Instant::now();
        let mut output = ctx.run_ui(input, |ui| ui_state.show(shell, ui));
        output.textures_delta.clear();
        let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
        let took = start.elapsed();
        assert!(!meshes.is_empty(), "{panel} painted nothing");
        took
    };
    for _ in 0..WARM {
        frame();
    }
    let mut times: Vec<Duration> = (0..FRAMES).map(|_| frame()).collect();
    times.sort();
    times
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn main() -> ExitCode {
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    println!(
        "ED17 panel frames: {NODES} nodes, 1280x720, {FRAMES} quiet frames, {} {} cpu x{threads}, no GPU (egui pass + tessellation)",
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    println!("panel        p50 ms   p99 ms   max ms   verdict");
    let mut shell = scene();
    let mut ui_state = ShellUi::new(&shell);
    let mut over = Vec::new();
    for panel in Panel::all() {
        let times = time_panel(&mut shell, &mut ui_state, panel);
        let p50 = ms(times[times.len() / 2]);
        let p99 = ms(times[times.len() * 99 / 100]);
        let max = ms(times[times.len() - 1]);
        let verdict = if p50 <= BUDGET_MS { "ok" } else { "OVER" };
        println!(
            "{:<12} {p50:>6.3}   {p99:>6.3}   {max:>6.3}   {verdict}",
            panel.title()
        );
        if p50 > BUDGET_MS {
            over.push(panel.title());
        }
    }
    if over.is_empty() {
        println!("every panel at or under {BUDGET_MS} ms");
        ExitCode::SUCCESS
    } else {
        println!("over the {BUDGET_MS} ms budget: {}", over.join(", "));
        ExitCode::FAILURE
    }
}
