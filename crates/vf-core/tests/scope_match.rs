//! T2 (VFL-41): scope matching — `ApprovedScope`/`RunScope`/`check_run_scope`/
//! `check_candidate`. Covers TDD §25 identifiers `authz/configured-scope` (the
//! `ExactHost`/`DomainTree` × `include_subdomains` matrix) and `authz/psl-exact-root`
//! (an `ExactHost` root that sits under a public/private suffix is never widened by
//! `registrable_domain`).
//!
//! Rules are from architecture §A3.1 / §5.3 (VFL-8 architecture document):
//!   - `ExactHost` approval permits only the identical host.
//!   - `DomainTree` approval permits the root and dot-boundary descendants, but a run
//!     includes descendants only when `RunScope::include_subdomains` is true.
//!   - `check_candidate` must pass both the approval scope and the run scope.
//!   - `registrable_domain` is informational and must never broaden an `ExactHost`
//!     approval.
//!
//! ASSUMPTION pending C2 skeleton finalization: this file needs `proptest` as a
//! dev-dependency of `vf-core` (the architecture doc's §A5 pins `proptest = "=1.11.0"`
//! at the workspace level already; `crates/vf-core/Cargo.toml` is production-lane and
//! not Halsey's to edit, so the `[dev-dependencies] proptest = { workspace = true }`
//! line is expected to land with Kelly's C2 skeleton commit). Two verdicts the §5.3
//! prose does not pin down precisely are deliberately tested loosely:
//!   - which exact `ScopeVerdict` (`OutsideApproval` vs `DiscoveryNotSelected`) a
//!     descendant candidate gets when `include_subdomains` is false — only that it is
//!     never `InScope`;
//!   - `ScopeVerdict::PublicSuffix`: since `CanonicalHost::parse` already rejects a
//!     bare public-suffix string, no valid `CanonicalHost` can reach `check_run_scope`
//!     / `check_candidate` already *being* a public suffix through the public API, so
//!     no test here asserts a specific trigger for that variant. Flagged for Cortana /
//!     Kelly at finalization rather than guessed.

use proptest::prelude::*;
use vf_core::scope::{
    ApprovedScope, CanonicalHost, RunScope, ScopeType, ScopeVerdict, check_candidate,
    check_run_scope,
};

fn host(s: &str) -> CanonicalHost {
    CanonicalHost::parse(s).unwrap_or_else(|e| panic!("fixture host {s:?} must parse: {e:?}"))
}

fn exact(root: &str) -> ApprovedScope {
    ApprovedScope {
        scope_type: ScopeType::ExactHost,
        root: host(root),
    }
}

fn domain_tree(root: &str) -> ApprovedScope {
    ApprovedScope {
        scope_type: ScopeType::DomainTree,
        root: host(root),
    }
}

fn run(target: &str, include_subdomains: bool) -> RunScope {
    RunScope {
        target: host(target),
        include_subdomains,
    }
}

// --- authz/configured-scope: ExactHost ---------------------------------------

#[test]
fn exact_host_admits_the_identical_run_target() {
    let approval = exact("example.com");
    assert_eq!(
        check_run_scope(&approval, &run("example.com", false)),
        ScopeVerdict::InScope
    );
}

#[test]
fn exact_host_refuses_a_subdomain_run_target_even_with_include_subdomains() {
    let approval = exact("example.com");
    assert_ne!(
        check_run_scope(&approval, &run("www.example.com", true)),
        ScopeVerdict::InScope
    );
}

#[test]
fn exact_host_refuses_an_unrelated_run_target() {
    let approval = exact("example.com");
    assert_ne!(
        check_run_scope(&approval, &run("other.com", false)),
        ScopeVerdict::InScope
    );
}

#[test]
fn exact_host_candidate_must_equal_the_root_exactly() {
    let approval = exact("example.com");
    let r = run("example.com", true);
    assert_eq!(
        check_candidate(&approval, &r, &host("example.com")),
        ScopeVerdict::InScope
    );
    assert_ne!(
        check_candidate(&approval, &r, &host("www.example.com")),
        ScopeVerdict::InScope
    );
}

// --- authz/configured-scope: DomainTree × include_subdomains ----------------

#[test]
fn domain_tree_admits_the_root_itself() {
    let approval = domain_tree("example.com");
    assert_eq!(
        check_run_scope(&approval, &run("example.com", false)),
        ScopeVerdict::InScope
    );
}

#[test]
fn domain_tree_admits_a_descendant_run_target_when_include_subdomains_is_true() {
    let approval = domain_tree("example.com");
    assert_eq!(
        check_run_scope(&approval, &run("www.example.com", true)),
        ScopeVerdict::InScope
    );
}

#[test]
fn domain_tree_admits_a_descendant_run_target_even_when_include_subdomains_is_false() {
    let approval = domain_tree("example.com");
    assert_eq!(
        check_run_scope(&approval, &run("www.example.com", false)),
        ScopeVerdict::InScope
    );
    assert_eq!(
        check_candidate(
            &domain_tree("example.com"),
            &run("www.example.com", false),
            &host("deep.www.example.com")
        ),
        ScopeVerdict::DiscoveryNotSelected
    );
}

#[test]
fn domain_tree_refuses_an_unrelated_run_target_regardless_of_include_subdomains() {
    let approval = domain_tree("example.com");
    assert_ne!(
        check_run_scope(&approval, &run("other.com", true)),
        ScopeVerdict::InScope
    );
    assert_ne!(
        check_run_scope(&approval, &run("other.com", false)),
        ScopeVerdict::InScope
    );
}

#[test]
fn domain_tree_refuses_a_label_prefix_lookalike_run_target() {
    let approval = domain_tree("example.com");
    assert_ne!(
        check_run_scope(&approval, &run("notexample.com", true)),
        ScopeVerdict::InScope
    );
}

#[test]
fn domain_tree_candidate_admits_root_and_descendants_when_run_selected_subdomains() {
    let approval = domain_tree("example.com");
    let r = run("example.com", true);
    assert_eq!(
        check_candidate(&approval, &r, &host("example.com")),
        ScopeVerdict::InScope
    );
    assert_eq!(
        check_candidate(&approval, &r, &host("api.example.com")),
        ScopeVerdict::InScope
    );
    assert_eq!(
        check_candidate(&approval, &r, &host("deep.sub.example.com")),
        ScopeVerdict::InScope
    );
}

#[test]
fn domain_tree_candidate_never_admits_a_descendant_when_run_did_not_select_subdomains() {
    let approval = domain_tree("example.com");
    let r = run("example.com", false);
    assert_ne!(
        check_candidate(&approval, &r, &host("api.example.com")),
        ScopeVerdict::InScope
    );
}

#[test]
fn domain_tree_candidate_never_admits_a_label_prefix_lookalike() {
    let approval = domain_tree("example.com");
    let r = run("example.com", true);
    assert_ne!(
        check_candidate(&approval, &r, &host("notexample.com")),
        ScopeVerdict::InScope
    );
    assert_ne!(
        check_candidate(&approval, &r, &host("evilexample.com")),
        ScopeVerdict::InScope
    );
}

#[test]
fn domain_tree_candidate_never_admits_a_sibling_domain() {
    let approval = domain_tree("example.com");
    let r = run("example.com", true);
    assert_ne!(
        check_candidate(&approval, &r, &host("other.com")),
        ScopeVerdict::InScope
    );
}

#[test]
fn domain_tree_candidate_inside_approval_but_outside_run_target_subtree_is_outside_approval() {
    // The approval covers all of example.com, but this run only targeted
    // www.example.com. A candidate inside the approval and outside the run's
    // own target subtree must not come back InScope merely because the
    // approval is wide — either as a sibling subtree (b.example.com) or as
    // the approval root itself, which is the run target's ancestor
    // (example.com). Cortana's recorded ruling pins the verdict at
    // OutsideApproval for both shapes (VFL-381 review, VFL-384 MEDIUM-1).
    let approval = domain_tree("example.com");
    let r = run("www.example.com", true);
    assert_eq!(
        check_candidate(&approval, &r, &host("b.example.com")),
        ScopeVerdict::OutsideApproval
    );
    assert_eq!(
        check_candidate(&approval, &r, &host("example.com")),
        ScopeVerdict::OutsideApproval
    );
}

// --- authz/psl-exact-root: registrable_domain must never widen ExactHost ----

#[test]
fn exact_host_under_a_private_suffix_rejects_a_sibling_with_the_same_registrable_suffix() {
    // "foo.github.io" and "bar.github.io" share the registrable suffix boundary
    // "github.io" (PSL private section) but are different exact hosts.
    let approval = exact("foo.github.io");
    let r = run("foo.github.io", true);
    assert_eq!(
        check_candidate(&approval, &r, &host("foo.github.io")),
        ScopeVerdict::InScope
    );
    assert_ne!(
        check_candidate(&approval, &r, &host("bar.github.io")),
        ScopeVerdict::InScope
    );
}

#[test]
fn exact_host_rejects_a_subdomain_sharing_its_own_registrable_domain() {
    // "deep.foo.github.io" has the same registrable_domain() as the ExactHost root
    // ("foo.github.io") but is not the identical host, so it must stay out of scope.
    let approval = exact("foo.github.io");
    let r = run("foo.github.io", true);
    assert_ne!(
        check_candidate(&approval, &r, &host("deep.foo.github.io")),
        ScopeVerdict::InScope
    );
}

#[test]
fn exact_host_registrable_domain_of_root_does_not_itself_gain_scope() {
    // registrable_domain("foo.github.io") is "foo.github.io" itself here, but for a
    // root one label further down (api.foo.github.io), the registrable ancestor
    // (foo.github.io) must not be treated as in scope just because it shares a suffix.
    let approval = exact("api.foo.github.io");
    let registrable_ancestor = host("api.foo.github.io")
        .registrable_domain()
        .expect("has a registrable domain");
    let r = run("api.foo.github.io", true);
    assert_ne!(
        check_candidate(&approval, &r, &registrable_ancestor),
        ScopeVerdict::InScope
    );
}

// --- property tests -----------------------------------------------------------

fn label() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("example".to_string()),
        Just("sub".to_string()),
        Just("api".to_string()),
        Just("deep".to_string()),
        Just("notexample".to_string()),
        Just("evil".to_string()),
        Just("other".to_string()),
    ]
}

/// Builds a dot-joined host from 1-4 labels plus a fixed TLD, so every generated
/// string is a syntactically valid, PSL-non-suffix `CanonicalHost` input.
fn host_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(label(), 1..=4).prop_map(|labels| format!("{}.com", labels.join(".")))
}

proptest! {
    // authz/configured-scope, authz/psl-exact-root
    #[test]
    fn check_candidate_never_admits_outside_domain_tree_root(
        candidate_str in host_strategy(),
    ) {
        let approval = domain_tree("example.com");
        let r = run("example.com", true);
        let candidate = host(&candidate_str);
        let verdict = check_candidate(&approval, &r, &candidate);

        let is_root_or_descendant =
            candidate.as_str() == "example.com" || candidate.is_descendant_of(&host("example.com"));

        if !is_root_or_descendant {
            prop_assert_ne!(verdict, ScopeVerdict::InScope);
        }
    }

    #[test]
    fn check_candidate_never_admits_a_host_different_from_an_exact_host_root(
        candidate_str in host_strategy(),
    ) {
        let approval = exact("example.com");
        let r = run("example.com", true);
        let candidate = host(&candidate_str);
        let verdict = check_candidate(&approval, &r, &candidate);

        if candidate.as_str() != "example.com" {
            prop_assert_ne!(verdict, ScopeVerdict::InScope);
        }
    }

    #[test]
    fn check_candidate_never_admits_descendants_when_include_subdomains_is_false(
        candidate_str in host_strategy(),
    ) {
        let approval = domain_tree("example.com");
        let r = run("example.com", false);
        let candidate = host(&candidate_str);
        let verdict = check_candidate(&approval, &r, &candidate);

        if candidate.as_str() != "example.com" {
            prop_assert_ne!(verdict, ScopeVerdict::InScope);
        }
    }
}
