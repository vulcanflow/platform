#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![deny(clippy::unwrap_used)]

//! `vf-hook-notify` — the secureCodeBox completion hook image (architecture
//! §A1.3, §8.4).
//!
//! Builds the completion notification, signs it, and posts it to `vf-ingest`.
//! It is a binary only: nothing in the workspace links against it, which is one
//! of the §A1.4 rules `deny.toml` enforces. Task G5 implements it.

fn main() {}
