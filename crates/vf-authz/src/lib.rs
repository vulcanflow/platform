#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-authz` — authorization and the start barrier (architecture §A1.3, §5.2,
//! §5.6, §5.7).
//!
//! Track A challenge issue and verify (DNS-TXT, HTTP file, manual), the
//! authorization basis and its lifecycle, the live basis resolver with the
//! 90-day and 14-day rules, and the start-barrier decision function. The
//! barrier decision is the single function the dispatcher, the operator and the
//! admission handler all call.
