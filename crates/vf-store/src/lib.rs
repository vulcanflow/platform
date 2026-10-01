#![forbid(unsafe_code)]
//! Object storage for findings artifacts, report snapshots and scan evidence
//! (TDD §6.5).
//!
//! # Why this crate exists separately
//!
//! VUL-14 allows the single S3 client constructor to live either in `vf-core` or
//! in a dedicated library crate. It lives here, because `vf-core` holds the
//! invariant that it performs no I/O at all — putting an S3 client in it would
//! retire the property that makes scope and allowance logic fuzzable. `vf-store`
//! is a library crate with the storage wrapper, not a service, which is what
//! VUL-14 requires.
//!
//! # Invariants
//!
//! **Exactly one constructor.** The raw `aws_sdk_s3::Client` is never re-exported.
//! A second `Client::new` elsewhere in the tree is how the mandatory
//! configuration below gets bypassed, so operations are exposed through a wrapper
//! type and the constructor is the only way in.
//!
//! **Four mandatory settings, because the SDK defaults are wrong for Aether**
//! (ADR-0002 §4.4):
//!
//! - `endpoint_url` — explicit. Never AWS-resolved.
//! - `force_path_style(true)` — virtual-host addressing needs wildcard DNS per
//!   bucket, and Aether is self-operated and IPv4-only. It does not provide it.
//! - `request_checksum_calculation(WhenRequired)` — the SDK default is
//!   `WhenSupported`, which attaches a CRC checksum to every upload.
//! - `response_checksum_validation(WhenRequired)` — same default, same problem on
//!   download.
//!
//! The checksum defaults are the sharp edge: an out-of-the-box `aws-sdk-s3` client
//! is *expected* to fail against Ceph RGW and RustFS. `aws-sdk-s3` was chosen over
//! `object_store` and `rust-s3` precisely because it exposes these opt-outs, so
//! omitting them means we picked the heaviest client and kept the defect.
//!
//! **Static credentials only.** No IMDS, no profile files, no environment
//! credential chain. There is no instance metadata service on Aether and a client
//! that tries to reach one hangs until its timeout.
//!
//! # Status
//!
//! Scaffold. The constructor and wrapper are VUL-14; `aws-sdk-s3 1.151.0` and
//! `aws-config 1.12.0` are pinned in the workspace manifest and are added to this
//! crate's dependencies there.
