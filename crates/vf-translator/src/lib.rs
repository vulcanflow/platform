#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-translator` — validated graph to executable plan (architecture §A1.3,
//! §7.4, §22.3).
//!
//! Turns a validated graph plus a run scope plus reservations into a
//! `WorkUnitPlan`, builds the typed secureCodeBox `Scan` and `CascadingRule`
//! objects, and owns the per-node argv builders. Argv construction is the
//! control that makes the authorization gate real (§22.3): it must never be
//! reachable with an unvalidated string.
