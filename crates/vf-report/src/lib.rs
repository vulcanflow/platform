#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-report` — report assembly and delivery (architecture §A1.3, §16).
//!
//! Immutable input snapshots, compile-time templates with context-restricted
//! escaping so the disclaimer block cannot be omitted, the renderer handoff and
//! delivery.
//!
//! No task in the current architecture document touches this crate; its
//! contracts freeze after M1. It exists now so the §A1.3 inventory is complete.
