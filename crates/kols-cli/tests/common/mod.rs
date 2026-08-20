//! Shared by the integration tests.
#![allow(dead_code)]

use std::time::Duration;

/// Scales a wall-clock timeout to how much machine there is.
///
/// # Why these are not constants
///
/// Every deadline in these tests was tuned on a 24-core development box, which
/// makes them an assumption about the machine rather than about the software.
/// The suite runs its tests in parallel and each one spawns two or three
/// daemons that sign, verify and encrypt, so a smaller machine does not run the
/// same work a little slower — it runs several tests' worth of it against a
/// fraction of the cores.
///
/// A four-core CI runner is where that first showed up, as three timeouts in
/// `two_nodes` on Windows and nowhere else, which reads exactly like a
/// platform bug. It is not one: pinning the same suite to two cores on Linux
/// reproduces it. The daemons were making steady progress and simply had less
/// of the machine than the numbers assumed.
///
/// So patience is a function of available parallelism. `KOLS_TEST_PATIENCE`
/// overrides it with a plain multiplier for anybody who wants to be explicit —
/// a loaded laptop looks nothing like an idle one, and `available_parallelism`
/// reports cores rather than idleness.
pub fn patience(base: Duration) -> Duration {
    base * factor()
}

fn factor() -> u32 {
    if let Some(explicit) = std::env::var("KOLS_TEST_PATIENCE")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
    {
        return explicit;
    }

    // Twelve is the width this suite was written on and comfortable at, so it
    // is the numerator rather than a tuning knob: at or above it nothing is
    // scaled, and below it the shortfall is the multiplier. Bounded at eight
    // because past that a hang should be reported as a hang rather than waited
    // out for a quarter of an hour.
    let cores = std::thread::available_parallelism().map_or(2, std::num::NonZeroUsize::get);
    u32::try_from(12_usize.div_ceil(cores))
        .unwrap_or(8)
        .clamp(1, 8)
}
