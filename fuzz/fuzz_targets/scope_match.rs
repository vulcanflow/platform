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
//!    the approval width on its own terms.
//!
//! Reworked per [VFL-199 F1/F2](/VFL/issues/VFL-199#document-review-c39241c):
//!
//! - **F1 (oracle integrity).** The prior `within_approval_root` matched on
//!   `host == root` / `host.is_descendant_of(root)` — the same match arms,
//!   the same `PartialEq`, the same `is_descendant_of` as the module's own
//!   private `approves` (`scope.rs:637-642`). It did not *call* `approves`,
//!   but it **was** `approves`, re-typed, so both assertions below held
//!   regardless of any bug in `approves`, `is_descendant_of` or
//!   `CanonicalHost`'s derived `PartialEq` — unfalsifiable by construction.
//!   `within_approval_root` now derives the relation from the host's
//!   *string* form instead: `as_str().split('.')` label vectors, compared by
//!   slice equality for `ExactHost` and by trailing-labels equality for
//!   `DomainTree`. That shares no code with `is_descendant_of` (no byte
//!   arithmetic over `checked_sub`/`ends_with`) and no code with
//!   `CanonicalHost`'s derived `PartialEq` (plain `&str` / `Vec<&str>`
//!   equality instead), so a dot-boundary bug in either — e.g.
//!   `is_descendant_of` loosened to `child.ends_with(parent)` without the
//!   boundary-byte check, which would wrongly admit `notexample.com` under
//!   an `example.com` `DomainTree` approval — changes `approves`'s answer
//!   without changing this oracle's, and the assertion can fail again.
//! - **F2 (reachability).** The prior `ScopeInputs` decoded `root`, `target`
//!   and `candidate` as three independent `&str`s straight from the fuzz
//!   buffer. `CanonicalHost::parse` requires, among other gates, a known
//!   Public Suffix List suffix that is not the whole name; three
//!   independently-truncated byte strings satisfying that gate *and*
//!   standing in a non-trivial approval/descendant relationship to each
//!   other was reachable only by mutation luck, so the security assertions
//!   below were rarely executed at all, on top of being unfalsifiable (F1).
//!   `root` is now built from a small label alphabet over a known-good
//!   suffix-plus-one base (see `canonical_host.rs`'s `HostSeed::Labels`
//!   shape), and `target` / `candidate` are each *derived* from the
//!   previous host — same string, a labelled descendant of it, or
//!   (retaining the original unstructured coverage) independent of it —
//!   which reaches the `ExactHost`-match, `DomainTree`-descendant,
//!   candidate-equals-target and candidate-descends-target-under-
//!   `include_subdomains` branches of `check_run_scope` / `check_candidate`
//!   with meaningful probability instead of almost never.
//!
//! Corpus seeding and crash triage are Test Runner's step (`cargo fuzz run
//! scope_match`, documented in `fuzz/README.md`), per VFL-145 and VFL-201
//! (nightly + cargo-fuzz toolchain provisioning).

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use vf_core::scope::{
    ApprovedScope, CanonicalHost, RunScope, ScopeType, ScopeVerdict, check_candidate,
    check_run_scope,
};

/// Known-good registrable-domain bases, suffix-plus-one under the Public
/// Suffix List — see `canonical_host.rs` for why this reliably clears
/// `parse`'s gates instead of failing at the first one.
const BASES: [&str; 3] = ["example.com", "example.org", "example.co.uk"];

/// A small label alphabet, all lowercase ASCII.
const LABELS: [&str; 6] = ["a", "b", "api", "sub", "stage", "www"];

/// Escape hatch alongside the structured shape below, so the harness still
/// exercises inputs a structured arm would never construct.
#[derive(Debug, Arbitrary)]
enum HostSeed<'a> {
    Labels(Vec<u8>, u8),
    Raw(&'a str),
}

fn label_stack(indices: &[u8], max_depth: usize) -> Vec<&'static str> {
    indices
        .iter()
        .take(max_depth)
        .map(|i| LABELS[usize::from(*i) % LABELS.len()])
        .collect()
}

fn materialize(seed: &HostSeed) -> String {
    match seed {
        HostSeed::Labels(indices, base) => {
            let base = BASES[usize::from(*base) % BASES.len()];
            let mut parts = label_stack(indices, 4);
            parts.push(base);
            parts.join(".")
        }
        HostSeed::Raw(s) => (*s).to_owned(),
    }
}

/// How one host in the triple relates to the previous one.
#[derive(Debug, Arbitrary)]
enum Related<'a> {
    /// Unrelated: an independent [`HostSeed`] (structured or raw).
    Independent(HostSeed<'a>),
    /// Identical string to the host it is relative to.
    Same,
    /// The prior host with 1..=3 extra labels prepended — a genuine
    /// descendant on a dot boundary.
    Descendant(Vec<u8>),
}

fn derive(base_str: &str, related: &Related) -> String {
    match related {
        Related::Independent(seed) => materialize(seed),
        Related::Same => base_str.to_owned(),
        Related::Descendant(indices) => {
            let mut parts = label_stack(indices, 3);
            if parts.is_empty() {
                // An empty `indices` would otherwise materialize to
                // `base_str` unchanged — indistinguishable from `Same` and
                // not a descendant at all. Force at least one extra label
                // so this arm always holds the "1..=3 extra labels" shape
                // documented below.
                parts.push(LABELS[0]);
            }
            parts.push(base_str);
            parts.join(".")
        }
    }
}

#[derive(Debug, Arbitrary)]
struct ScopeInputs<'a> {
    root: HostSeed<'a>,
    target: Related<'a>,
    candidate: Related<'a>,
    domain_tree: bool,
    include_subdomains: bool,
}

/// Independently-derived oracle for "inside the approval root", per §A3.1 /
/// §5.3 — over the host's *string* form, sharing no code with
/// `is_descendant_of`'s byte arithmetic or `CanonicalHost`'s derived
/// `PartialEq` (VFL-199 F1; see module doc comment).
fn within_approval_root(scope_type: ScopeType, root: &CanonicalHost, host: &CanonicalHost) -> bool {
    let root_labels: Vec<&str> = root.as_str().split('.').collect();
    let host_labels: Vec<&str> = host.as_str().split('.').collect();

    match scope_type {
        ScopeType::ExactHost => host_labels == root_labels,
        ScopeType::DomainTree => {
            host_labels.len() >= root_labels.len()
                && host_labels[host_labels.len() - root_labels.len()..] == root_labels[..]
        }
    }
}

fuzz_target!(|inputs: ScopeInputs| {
    let root_str = materialize(&inputs.root);
    let target_str = derive(&root_str, &inputs.target);
    let candidate_str = derive(&target_str, &inputs.candidate);

    let Ok(root) = CanonicalHost::parse(&root_str) else {
        return;
    };
    let Ok(target) = CanonicalHost::parse(&target_str) else {
        return;
    };
    let Ok(candidate) = CanonicalHost::parse(&candidate_str) else {
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
