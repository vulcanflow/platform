#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-remediation` — guidance and verification (architecture §A1.3, §15).
//!
//! Three-tier guidance resolution with version pinning, verification plan
//! derivation, and verification outcome interpretation. All of it is pure logic
//! over data the API and ingest already hold, which is mapping point 1 of
//! §A1.3: this is a library in the workspace, not a separate service. The M3
//! content publishing and feed mirror jobs become `[[bin]]` targets here.
