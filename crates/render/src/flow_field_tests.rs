//! flow_field tests (SW41): the CPU oracle's four bars.
//!
//! Each test asserts an exact value or a byte-for-byte comparison, never "it
//! did not panic": a splat's mask, a zero-strength splat, a seeded rerun and the
//! quarter-res grid size.

use super::{FlowField, Mask};

/// A splat writes only inside its mask, and nothing at all outside it.
#[test]
fn a_splat_stays_inside_its_mask() {
    let mut field = FlowField::with_grid(7, 64, 32);
    let mask = Mask {
        x0: 16,
        y0: 8,
        x1: 48,
        y1: 24,
    };
    // The radius reaches well past the mask on every side.
    field.splat([32.0, 16.0], 40.0, 3.0, [1.0, 0.0], mask);
    let mut written = 0;
    for y in 0..field.height() {
        for x in 0..field.width() {
            let dye = field.dye_at(x, y);
            let velocity = field.velocity(x, y);
            if mask.contains(x, y) {
                if dye != 0.0 || velocity != [0.0, 0.0] {
                    written += 1;
                }
            } else {
                assert_eq!(dye, 0.0, "dye outside the mask at ({x}, {y})");
                assert_eq!(
                    velocity,
                    [0.0, 0.0],
                    "velocity outside the mask at ({x}, {y})"
                );
            }
        }
    }
    assert_eq!(
        written,
        usize::try_from(32 * 16).expect("the mask holds 512 cells"),
        "every cell of the mask was reached"
    );
    assert!(field.dye_total() > 0.0, "the splat dyed the mask");
    assert_eq!(
        field.dye_at(15, 16),
        0.0,
        "one cell left of the mask is dry"
    );
}

/// A zero-strength splat leaves every byte of the field as it was.
#[test]
fn a_splat_of_strength_zero_is_byte_equal() {
    let mut field = FlowField::with_grid(11, 32, 32);
    field.splat([16.0, 16.0], 6.0, 2.5, [0.0, 1.0], Mask::whole(32, 32));
    field.step(1.0 / 60.0);
    let before = field.snapshot();
    field.splat([8.0, 24.0], 9.0, 0.0, [1.0, 1.0], Mask::whole(32, 32));
    assert_eq!(field.snapshot(), before, "strength 0 writes nothing");
    field.splat([8.0, 24.0], 9.0, f32::NAN, [1.0, 1.0], Mask::whole(32, 32));
    assert_eq!(
        field.snapshot(),
        before,
        "a non-finite strength writes nothing"
    );
}

/// The same seed and the same calls give the same bytes, and the seed matters.
#[test]
fn the_same_seed_runs_twice_identical() {
    let run = |seed: u64| {
        let mut field = FlowField::seeded(seed, 192, 108);
        let mask = Mask::whole(field.width(), field.height());
        for step in 0..40u64 {
            if step % 5 == 0 {
                field.ambient(step, 1.5, mask);
            }
            field.step(1.0 / 60.0);
        }
        field
    };
    let first = run(2040);
    let again = run(2040);
    assert!(first.dye_total() > 0.0, "the run carried dye");
    assert_eq!(
        first.snapshot(),
        again.snapshot(),
        "the same seed x2 gives the same bytes"
    );
    assert_eq!(first.dye_total(), again.dye_total(), "the same dye total");
    assert_ne!(
        first.snapshot(),
        run(2041).snapshot(),
        "a different seed draws different bytes"
    );
}

/// A 1920x1080 frame runs the field at a quarter of it: 480x270 cells.
#[test]
fn quarter_res_of_1080p_is_480x270() {
    let field = FlowField::new(1920, 1080);
    assert_eq!((field.width(), field.height()), (480, 270));
    assert_eq!(field.cells(), 480 * 270);
    let seeded = FlowField::seeded(2040, 1920, 1080);
    assert_eq!((seeded.width(), seeded.height()), (480, 270));
}
