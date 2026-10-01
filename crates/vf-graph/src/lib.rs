#![forbid(unsafe_code)]
//! The VulcanFlow flow-graph validator (TDD §7.1–§7.3).
//!
//! One implementation, two targets. The browser builder and the server validate
//! the same graph with the same code compiled to `wasm32-unknown-unknown` and to
//! the native target, so the builder cannot accept a graph the server rejects or
//! the reverse.
//!
//! # Parity is a property, not a hope
//!
//! §25 `graph/native-wasm-parity` requires identical results from both builds.
//! Divergence is a defect in this crate, and these are the ways it happens:
//!
//! - **Floating point.** `f32`/`f64` arithmetic, formatting and `NaN` ordering
//!   can differ. The validator does not use floating point.
//! - **Integer width.** `usize`/`isize` are 64-bit natively and 32-bit on
//!   `wasm32`. Anything whose value can reach a diagnostic, a limit comparison or
//!   a serialized field uses an explicitly sized integer.
//! - **Iteration order.** `HashMap`/`HashSet` ordering is unspecified and the
//!   default hasher is seeded differently per target and per run. Ordered
//!   containers only; `clippy::disallowed_types` is denied in this crate.
//! - **Time and randomness.** Neither is available on `wasm32-unknown-unknown`
//!   without a host import, and both would make validation non-deterministic.
//!   The validator reads neither.
//! - **Panics.** A panic is a trap in WebAssembly and an unwind natively. The
//!   validator returns typed errors instead; `clippy::unwrap_used`,
//!   `expect_used` and `panic` are denied workspace-wide.
//!
//! # No I/O
//!
//! Same rule as `vf-core`: values in, values out. The crate cannot read a file or
//! open a socket, which is also what makes the WebAssembly build possible without
//! a host interface.
//!
//! # Status
//!
//! Scaffold. Graph parsing, the §7.2 type system and §7.3 validation land in the
//! issues that follow VUL-6. What exists here is the crate shape that makes the
//! native and WebAssembly builds come from one source, plus the lints that keep
//! them in step.
