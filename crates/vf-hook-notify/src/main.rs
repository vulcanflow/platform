#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![deny(clippy::unwrap_used)]

//! `vf-hook-notify` — the secureCodeBox completion hook image (architecture
//! §A1.3, §8.4).
//!
//! Builds the completion notification, signs it, and posts it to `vf-ingest`.
//! It is a binary only: nothing in the workspace links against it. That §A1.4
//! rule is about edges into a root crate, which a cargo-deny ban cannot
//! express, so `just graph-rules` enforces it from cargo metadata rather than
//! `deny.toml`. Task G5 implements it.
//!
//! This image ships on its own (§A1.3 mapping point 2), so the crate's own
//! dependency graph has to be complete: it names `rustls` directly to supply
//! the crypto provider that `reqwest`'s `rustls-no-provider` feature leaves to
//! the binary. See the comment on that dependency in `Cargo.toml`.

fn main() {}
