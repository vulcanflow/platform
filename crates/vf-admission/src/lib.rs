#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-admission` — the validating admission handler (architecture §A1.3,
//! §5.7).
//!
//! Validates secureCodeBox `Scan`, `Job` and `Pod` objects through
//! `AdmissionReview` v1. It is fail-closed, and it reaches the same
//! `vf-authz` barrier decision function the dispatcher and the operator use, so
//! a scan cannot be admitted on a path that skips the barrier.
