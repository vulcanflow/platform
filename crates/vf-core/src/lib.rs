#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-core` — the I/O-free domain core (architecture §A1.3).
//!
//! Everything here is pure: identity newtypes (§6.2), `CanonicalHost` and
//! scope matching (§5.3), the state enums and transition functions (§6.3,
//! §8.2, §15.1, §17.3), the role x action policy (§4.2), the Problem type
//! catalogue (§13.3), the event shapes (§A3.3) and the port traits (§A6.1).
//!
//! The crate depends on no I/O crate — no tokio, sqlx, reqwest or kube. That
//! rule is one of the two in §A1.4 that `deny.toml` cannot express as a ban, so
//! it is enforced by `just graph-rules` (`cargo tree`, per target) and by the
//! workspace test pack T9. `deny.toml` enforces the rest of §A1.4, including
//! which crates may depend on this one.

pub mod ports;
