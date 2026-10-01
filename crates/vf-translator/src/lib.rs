#![forbid(unsafe_code)]
//! Translation from a validated VulcanFlow flow graph to secureCodeBox `Scan` and
//! `CascadingRule` resources (TDD §7.4).
//!
//! # Invariants
//!
//! **Translation is pure.** It produces resource *values*; it never applies them.
//! Applying is the operator's job (`vf-operator`), and keeping the split means a
//! translation can be snapshotted and diffed without a cluster
//! (§25 `graph/validator-translator-conformance`).
//!
//! **Only validated graphs are translatable.** A graph reaches this crate after
//! `vf-graph` has accepted it. Translation is not a second, weaker validation
//! pass, and it must not silently repair an input.
//!
//! **Deterministic output.** The same graph produces byte-identical resources,
//! because §7.5 requires a reproducible plan and the plan is historical evidence
//! for what a scan was authorised to do.
//!
//! # Status
//!
//! Scaffold. The secureCodeBox CRD Rust types this crate consumes are generated
//! with `kopium 0.24.1` from secureCodeBox `v5.9.0` and committed in their own
//! crate (ADR-0002 §6.3, owned by Kiln); `vf-translator` depends on that crate
//! once it lands.
