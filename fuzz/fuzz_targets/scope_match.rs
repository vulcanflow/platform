//! VFL-145 (T2 addendum to C2 / VFL-15): fuzz target for `check_run_scope` /
//! `check_candidate`.
//!
//! Decodes an arbitrary host/approval/run triple (`root`, `target`,
//! `candidate` strings, plus the `ScopeType` and `include_subdomains` bits
//! that complete an `ApprovedScope` and `RunScope`) and checks two things,
//! both from VFL-145's scope:
//!
//! 1. No panic — same totality invariant as `canonical_host`.
//! 2. No admission of a host outside the approval root: if either function
//!    returns `ScopeVerdict::InScope`, the admitted host (the run target for
//!    `check_run_scope`, the candidate for `check_candidate`) must satisfy
//!    the approval width on its own terms. The oracle below
//!    (`within_approval_root`) is independently derived from architecture
//!    §A3.1 / TDD §5.3 — "`ExactHost` permits only the identical host";
//!    "`DomainTree` permits the root and any dot-boundary descendant" — the
//!    same two sentences the existing `crates/vf-core/tests/scope_match.rs`
//!    property tests assert; it does not call the module's own private
//!    `approves` helper, so a bug in that helper cannot hide behind an oracle
//!    that mirrors it.
//!
//! Corpus seeding and crash triage are Test Runner's step (`cargo fuzz run
//! scope_match`), per VFL-145 and VFL-142 (nightly toolchain availability).

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use vf_core::scope::{
    check_candidate, check_run_scope, ApprovedScope, CanonicalHost, RunScope, ScopeType,
    ScopeVerdict,
};

#[derive(Debug, Arbitrary)]
struct ScopeInputs<'a> {
    root: &'a str,
    target: &'a str,
    candidate: &'a str,
    domain_tree: bool,
    include_subdomains: bool,
}

/// Independently-derived oracle for "inside the approval root", per §A3.1 /
/// §5.3 — not a call into the module's own private `approves` helper.
fn within_approval_root(scope_type: ScopeType, root: &CanonicalHost, host: &CanonicalHost) -> bool {
    match scope_type {
        ScopeType::ExactHost => host == root,
        ScopeType::DomainTree => host == root || host.is_descendant_of(root),
    }
}

fuzz_target!(|inputs: ScopeInputs| {
    let Ok(root) = CanonicalHost::parse(inputs.root) else {
        return;
    };
    let Ok(target) = CanonicalHost::parse(inputs.target) else {
        return;
    };
    let Ok(candidate) = CanonicalHost::parse(inputs.candidate) else {
        return;
    };

    let scope_type = if inputs.domain_tree {
        ScopeType::DomainTree
    } else {
        ScopeType::ExactHost
    };
    let approval = ApprovedScope {
        scope_type,
        root: root.clone(),
    };
    let run = RunScope {
        target: target.clone(),
        include_subdomains: inputs.include_subdomains,
    };

    let run_verdict = check_run_scope(&approval, &run);
    if run_verdict == ScopeVerdict::InScope {
        assert!(
            within_approval_root(scope_type, &approval.root, &target),
            "check_run_scope admitted a run target outside the approval root: \
             approval={approval:?} run={run:?}"
        );
    }

    let candidate_verdict = check_candidate(&approval, &run, &candidate);
    if candidate_verdict == ScopeVerdict::InScope {
        assert!(
            within_approval_root(scope_type, &approval.root, &candidate),
            "check_candidate admitted a candidate outside the approval root: \
             approval={approval:?} run={run:?} candidate={candidate:?}"
        );
    }
});
