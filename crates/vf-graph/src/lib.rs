#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-graph` — the pipeline graph DSL (architecture §A1.3, §7.1-7.3,
//! Appendix A).
//!
//! Graph and node config types, the type lattice, the validation rules, JSON
//! schema generation and the `wasm-bindgen` export consumed by the SPA.
//!
//! Two graph rules from §A1.4 apply to this crate and are enforced by
//! `deny.toml`, `just graph-rules` and test pack T9:
//!
//! - it depends on no I/O crate, so the wasm32 build stays clean;
//! - it does not depend on `vf-core`. It exports its own small types instead,
//!   so the browser bundle carries nothing it does not need.
