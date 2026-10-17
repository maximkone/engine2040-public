//! The shell's source replaces the platform clock for `now` and `since`, and only once.

use std::time::Duration;

fn shell_time() -> Duration {
    Duration::from_millis(40)
}

fn other_time() -> Duration {
    Duration::from_millis(1)
}

#[test]
fn the_installed_source_drives_now_and_since() {
    assert!(engine_clock::install(shell_time));
    assert_eq!(engine_clock::now(), Duration::from_millis(40));
    assert_eq!(
        engine_clock::since(Duration::from_millis(15)),
        Duration::from_millis(25)
    );
    assert!(
        !engine_clock::install(other_time),
        "the first source stays for the life of the process"
    );
    assert_eq!(engine_clock::now(), Duration::from_millis(40));
}
