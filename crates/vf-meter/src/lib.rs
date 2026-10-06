#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-meter` — metering unit accounting (architecture §A1.3, §17).
//!
//! Atomic reserve, settle and release over a `TenantTx`, billing-period usage,
//! the append-only ledger, and target admission. The Lago and Stripe sync is a
//! Phase 4 `[[bin]]` target of this crate and does not exist yet.
