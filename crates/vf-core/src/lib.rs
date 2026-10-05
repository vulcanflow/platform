#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-core` — the I/O-free domain core (architecture §A1.3).
//!
//! Everything here is pure: identity newtypes (§6.2), `CanonicalHost` and
//! scope matching (§5.3), the state enums and transition functions (§6.3,
//! §8.2, §15.1, §17.3), the role x action policy (§4.2), the Problem type
//! catalogue (§13.3), the event shapes (§A3.3) and the port traits (§A6.1).
//!
//! The crate depends on no I/O crate — no tokio, sqlx, reqwest or kube — and
//! that is enforced mechanically by `deny.toml`, by `just graph-rules` and by
//! the workspace test pack T9.

pub mod ports;
