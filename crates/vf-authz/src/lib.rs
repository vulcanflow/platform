#![forbid(unsafe_code)]
//! Authorization evidence: Track A challenge issue and verify, Track B permission
//! attestation, corroborating scope signals, and the immutable basis record
//! (TDD §5.2, §5.4–§5.6).
//!
//! # Invariants
//!
//! **Decide here, probe elsewhere.** This crate constructs challenges and decides
//! whether a presented response satisfies one. It does not perform the DNS lookup
//! or the HTTP request that fetches the response — that I/O belongs to the
//! service that calls in, so the decision stays property-testable and the
//! authorization boundary is not entangled with a network stack.
//!
//! **Deny by default.** An unparseable, expired, ambiguous or merely unrecognised
//! response is not a verification. A corroborating signal (§5.5) corroborates; it
//! never authorises on its own.
//!
//! **Comparisons are constant-time where a secret is involved.** A challenge token
//! comparison that short-circuits on the first differing byte leaks the token.
//!
//! **History is append-only.** Approval and revocation are recorded as events, not
//! as a mutable flag, because §5.6 and §25 `audit/authorization-history` require
//! the history to be reconstructable.
//!
//! # Status
//!
//! Scaffold. Behaviour lands in the issues that follow VUL-6.
