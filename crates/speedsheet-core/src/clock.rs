//! The only place that reads the system clock.
//!
//! What the user sees is local time (NOW(), TODAY()), and CI runs the tests in UTC+14 and UTC-11
//! to prove it. `clippy.toml` forbids calling `now()` anywhere else, so new code cannot quietly
//! assume UTC or make a test depend on the time of day.
#![allow(clippy::disallowed_methods)]

/// The current local date and time.
pub(crate) fn local_now() -> chrono::DateTime<chrono::Local> {
    chrono::Local::now()
}
