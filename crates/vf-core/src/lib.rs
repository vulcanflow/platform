#![forbid(unsafe_code)]
//! Safety-critical domain logic for VulcanFlow: authorization scope matching,
//! allowance reservation and settlement, observation state, and role policy.
//!
//! # Invariants this crate exists to hold
//!
//! **No I/O.** Every function here takes values and returns values. No database,
//! no clock, no network, no filesystem, no environment. Time and identifiers are
//! parameters, never ambient reads. This is what makes the crate
//! property-testable and fuzzable in isolation (TDD §2.5.2, §23.1).
//!
//! **Deny by default.** When scope, policy or authorization input is ambiguous,
//! unparseable, or merely unrecognised, the answer is "no". There is no
//! fall-through to permit (TDD §5.3, §5.7).
//!
//! **Canonical input only.** Hostnames, IP addresses, CIDRs and URLs are
//! canonicalized once, on the way in, and everything downstream compares
//! canonical forms. Comparing two spellings of the same host is how a scope check
//! gets bypassed (TDD §5.3). Where a type cannot encode that it holds a
//! canonical value, the function's documentation states the requirement.
//!
//! **Exact integer accounting.** Allowances reserve and settle in `i64` with
//! checked arithmetic. No floating point and no decimal crate anywhere in the
//! allowance path (ADR-0002 §3.5) — a saturating subtraction in a billing path
//! silently gives work away. `clippy::arithmetic_side_effects` is denied in this
//! crate so unchecked `+`/`-`/`*` cannot be written by accident.
//!
//! **Closed state machines.** Allowance and observation state are enums with
//! explicit transitions, and an illegal transition is unrepresentable rather than
//! merely rejected. If a reviewer has to trust a comment, the type is wrong.
//!
//! **Determinism.** Same input, same output, every run. No `HashMap` iteration
//! order in anything observable (`clippy::disallowed_types` is denied here), no
//! ambient randomness, no system clock.
//!
//! # Status
//!
//! Scaffold. The behaviour named above is implemented by the issues that follow
//! VUL-6; this crate currently establishes the boundary, the lints and the
//! dependency set that enforce the invariants.
