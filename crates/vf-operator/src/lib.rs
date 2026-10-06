#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-operator` — reconciliation and provisioning (architecture §A1.3, §3.3,
//! §7.4, §8).
//!
//! The `ScanFlow` reconcile loop: live approval, per-candidate reservation
//! through the start barrier, scan creation via the `ScanRuntime` port,
//! fingerprint binding, cascade candidate registration, the completion
//! barrier, cancellation and deterministic adoption. It also holds `Tenant`
//! provisioning and both `ScanRuntime` implementations.
//!
//! Kubernetes sits behind the `kube` feature, which is off by default.
