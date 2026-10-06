#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![deny(clippy::unwrap_used)]

//! `vf-scanner-adapter` — runs inside the scanner images (architecture §A1.3,
//! mapping point 2, §7.4, §8.2, §22.3).
//!
//! Materializes validated inputs, builds argv from the typed node config using
//! the `vf-translator` builders, executes the upstream tool through
//! `Command::args` (never a shell), classifies the outcome and writes
//! `manifest.json`. It lives in `platform` so it shares `CanonicalHost`, the
//! node config structs and the argv builders with `vf-graph` and
//! `vf-translator` under one lockfile; the `scanners` repository copies the
//! released binary into each image. Task G5 implements it.

fn main() {}
