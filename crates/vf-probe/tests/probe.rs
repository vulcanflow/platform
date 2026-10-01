//! Probe fixture for VUL-2 — the test half of the deliberate lane crossing.
//!
//! A test author (lane 2) would legitimately write this. What no one may do is ship
//! it in the same pull request as `crates/vf-probe/src/lib.rs`, because then a coding
//! agent one assertion away from green could simply move the assertion.

#[test]
fn classifies_a_negative_number() {
    assert_eq!(vf_probe::classify(-1), "negative");
}
