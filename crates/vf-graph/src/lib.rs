#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-graph` — the pipeline graph DSL (architecture §A1.3, §7.1-7.3,
//! Appendix A).
//!
//! Graph and node config types, the type lattice, the validation rules, JSON
//! schema generation and the `wasm-bindgen` export consumed by the SPA.
//!
//! Two graph rules from §A1.4 apply to this crate, and they are enforced in
//! different places:
//!
//! - it depends on no I/O crate, so the wasm32 build stays clean. Not
//!   expressible as a cargo-deny ban; checked by `just graph-rules`
//!   (`cargo tree --target wasm32-unknown-unknown`) and test pack T9.
//! - it does not depend on `vf-core`. It exports its own small types instead,
//!   so the browser bundle carries nothing it does not need. This one *is* a
//!   ban: `vf-core`'s `wrappers` list in `deny.toml` omits `vf-graph`.
