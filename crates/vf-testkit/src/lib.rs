#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-testkit` — the local test harness (architecture §A1.3, §A6.2).
//!
//! A dev-dependency everywhere and shipped nowhere: the testcontainers
//! Postgres bootstrap with a migrated template schema, the fixture loaders, the
//! deterministic `Clock` and `IdGen`, dev token minting, and the fixture corpus
//! under `fixtures/`.
//!
//! `fixtures/` is a neutral-lane directory, so a coder may add a scanner output
//! sample without crossing into the test lane. Task F2 fills this crate in.
