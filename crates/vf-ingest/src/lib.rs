#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-ingest` — the artifact intake path (architecture §A1.3, §8.4, §10).
//!
//! Receives the signed completion notification, fetches and parses artifacts as
//! a bounded stream, derives fresh observations, applies the false-positive
//! matchers, enriches, settles the metering reservation, and runs the
//! 60-second reconciliation sweep that closes runs whose notification never
//! arrived.
