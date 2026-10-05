#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-api` — the control-plane HTTP surface (architecture §A1.3, §A3.8, §4,
//! §8.1, §9, §13).
//!
//! REST and SSE, code-first OpenAPI 3.1, the auth middleware and role layer,
//! the dispatcher module (validate, estimate, reserve, write outbox intent in
//! one transaction, per plan item 3 and §8.1 steps 3-4), the outbox worker and
//! the tenant-level event stream.
//!
//! The crate is a binary with a library target so the test lane can drive the
//! router through `tower::Service` without binding a port.

pub mod openapi;
