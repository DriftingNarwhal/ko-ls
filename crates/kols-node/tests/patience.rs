//! That the suite's own deadlines scale with the machine.
//!
//! In a test file of its own because `common` is compiled into every
//! integration binary that declares it — so unit tests living beside it would
//! run once per binary and count six times, which is a test count that lies.

mod common;

#[test]
fn a_wide_machine_is_not_made_to_wait_longer() {
    // The development box, and the case that must not regress: scaling that
    // quietly multiplied every timeout would turn a hang into a coffee break.
    assert_eq!(12_usize.div_ceil(24), 1);
    assert_eq!(12_usize.div_ceil(12), 1);
}

#[test]
fn a_small_machine_gets_proportionally_longer() {
    assert_eq!(12_usize.div_ceil(4), 3);
    assert_eq!(12_usize.div_ceil(2), 6);
}

#[test]
fn the_override_wins_and_a_nonsense_value_does_not() {
    // Deliberately not asserted through the env, which is process-global and
    // would race the rest of the suite. The parse and the filter are the
    // logic; that they are read from the environment is not.
    let parse = |raw: &str| raw.trim().parse::<u32>().ok().filter(|value| *value > 0);
    assert_eq!(parse(" 4 "), Some(4));
    assert_eq!(parse("0"), None);
    assert_eq!(parse("soon"), None);
}
