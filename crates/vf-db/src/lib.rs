#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-db` — Postgres and the infrastructure adapters (architecture §A1.3).
//!
//! Owns the migrations for the control schema `vf` and the tenant schema
//! template, the `TenantTx`/`ControlTx` transaction wrappers (§3.5), a
//! repository per table in §A4, the transactional outbox, and the
//! `ArtifactStore`/`WakeBus` adapters that implement the §A6.1 ports.
//!
//! Crate owner: Fred. Module `adapters` is owned by Jorge (task F3).
