//! T1a (VFL-112, covers C1 / VFL-14): the RFC 9457 Problem Details catalogue
//! of `vf-core::problem`.
//!
//! Architecture §A3.8, TDD §13.3 (VFL-8#document-architecture). Every
//! `Problem` variant serializes to the same flat six-member envelope —
//! `type`, `title`, `status`, `detail`, `instance`, `vf` — in that order,
//! with variant-specific data nested under `vf` (never flattened into the
//! envelope itself). `vf-core` has no `serde_json` dev-dependency available
//! to a test-only pull request, so `tests/support`'s minimal `Serialize`
//! target stands in for it (see that module's docs).
//!
//! Covers the ten URIs named in the C1 scope bullet, plus `TenantSuspended`:
//! the skeleton's own doc comment flags it as "addition to the ten URIs
//! named in the C1 scope" per the T7 acceptance criteria and "called out for
//! review" — not yet an architect ruling either way, so this pack tests it
//! against the skeleton's own stated contract (§4.1, the T7 criteria) rather
//! than leaving the eleventh variant uncovered. A ruling that it should not
//! exist is a dispute for Cortana, not a reason to skip testing what is
//! actually on the public API today.

mod support;

use vf_core::ids::TargetId;
use vf_core::problem::{
    AllowanceHints, ChallengeHints, ERROR_TYPE_BASE, ForbiddenHints, GraphError, NextAction,
    NotFoundHints, Problem, RateLimitHints, TrackHints, TransitionHints,
};
use vf_core::state::{IllegalTransition, ObservationEvent, ObservationState};

const ENVELOPE_KEYS: &[&str] = &["type", "title", "status", "detail", "instance", "vf"];

fn target_id() -> TargetId {
    TargetId::from_uuid(uuid::Uuid::parse_str("01890a5d-ac96-774b-bf71-ca4fde3b9c2b").unwrap())
}

// ---------------------------------------------------------------------------
// One builder per variant, reused by the per-variant shape tests and by the
// cross-cutting consistency test at the bottom of this file.
// ---------------------------------------------------------------------------

fn authorization_required_full() -> Problem {
    Problem::AuthorizationRequired {
        detail: "example.com has no verified authorization basis for this account.".into(),
        instance: None,
        target_id: Some(target_id()),
        hints: TrackHints {
            track_a_available: true,
            track_b_available: false,
            track_b_reason: Some("kyc_incomplete".into()),
            next_actions: vec![NextAction {
                action: "verify_ownership".into(),
                href: "/v1/targets/verify".into(),
            }],
        },
    }
}

fn authorization_required_minimal() -> Problem {
    Problem::AuthorizationRequired {
        detail: "no basis".into(),
        instance: None,
        target_id: None,
        hints: TrackHints {
            track_a_available: false,
            track_b_available: true,
            track_b_reason: None,
            next_actions: vec![],
        },
    }
}

fn target_allowance_exceeded() -> Problem {
    Problem::TargetAllowanceExceeded {
        detail: "adding example.com would exceed the package target allowance.".into(),
        instance: None,
        hints: AllowanceHints {
            current: 10,
            limit: 10,
            package: "starter".into(),
            upgrade_href: "/v1/billing/upgrade".into(),
        },
    }
}

fn graph_invalid() -> Problem {
    Problem::GraphInvalid {
        detail: "the submitted graph has 2 violations.".into(),
        instance: None,
        errors: vec![
            GraphError {
                code: "cycle".into(),
                node_id: Some("n2".into()),
                message: "n2 participates in a cycle".into(),
            },
            GraphError {
                code: "empty_graph".into(),
                node_id: None,
                message: "the graph has no nodes".into(),
            },
        ],
    }
}

fn idempotency_conflict() -> Problem {
    Problem::IdempotencyConflict {
        detail: "the Idempotency-Key was replayed with a different request body.".into(),
        instance: None,
        request_key: "client-key-123".into(),
    }
}

fn illegal_transition() -> Problem {
    Problem::IllegalTransition {
        detail: "illegal observation transition: start_fix is not admitted from verifying".into(),
        instance: None,
        hints: TransitionHints {
            from: ObservationState::Verifying,
            attempted: ObservationEvent::StartFix,
            admitted: vec![],
        },
    }
}

fn challenge_failed() -> Problem {
    Problem::ChallengeFailed {
        detail: "the DNS-TXT challenge token was not found.".into(),
        instance: None,
        hints: ChallengeHints {
            reason: "token_not_found".into(),
            retryable: true,
        },
    }
}

fn not_found() -> Problem {
    Problem::NotFound {
        detail: "no such target for this tenant.".into(),
        instance: None,
        hints: NotFoundHints {
            resource: "target".into(),
        },
    }
}

fn forbidden() -> Problem {
    Problem::Forbidden {
        detail: "member role does not permit member_manage.".into(),
        instance: None,
        hints: ForbiddenHints {
            required_action: "member_manage".into(),
        },
    }
}

fn tenant_suspended_with_reason() -> Problem {
    Problem::TenantSuspended {
        detail: "the account is suspended; reads remain available.".into(),
        instance: None,
        reason: Some("payment_failed".into()),
    }
}

fn tenant_suspended_no_reason() -> Problem {
    Problem::TenantSuspended {
        detail: "the account is suspended; reads remain available.".into(),
        instance: None,
        reason: None,
    }
}

fn rate_limited() -> Problem {
    Problem::RateLimited {
        detail: "the dispatch bucket refused this request.".into(),
        instance: None,
        hints: RateLimitHints {
            bucket: "dispatch".into(),
            retry_after_seconds: 30,
        },
    }
}

fn internal() -> Problem {
    Problem::internal("trace-abc-123")
}

/// Exhaustive over every `Problem` variant, with no wildcard arm (LOW-7):
/// adding a variant to `vf_core::problem::Problem` fails this file to
/// compile until [`all_variants`] is updated to cover it too.
fn assert_every_problem_variant_is_covered(p: &Problem) {
    match p {
        Problem::AuthorizationRequired { .. }
        | Problem::TargetAllowanceExceeded { .. }
        | Problem::GraphInvalid { .. }
        | Problem::IdempotencyConflict { .. }
        | Problem::IllegalTransition { .. }
        | Problem::ChallengeFailed { .. }
        | Problem::NotFound { .. }
        | Problem::Forbidden { .. }
        | Problem::TenantSuspended { .. }
        | Problem::RateLimited { .. }
        | Problem::Internal { .. } => {}
    }
}

/// Every variant, each with distinct shapes where relevant, for the
/// cross-cutting consistency test.
fn all_variants() -> Vec<Problem> {
    let variants = vec![
        authorization_required_full(),
        authorization_required_minimal(),
        target_allowance_exceeded(),
        graph_invalid(),
        idempotency_conflict(),
        illegal_transition(),
        challenge_failed(),
        not_found(),
        forbidden(),
        tenant_suspended_with_reason(),
        tenant_suspended_no_reason(),
        rate_limited(),
        internal(),
    ];
    for variant in &variants {
        assert_every_problem_variant_is_covered(variant);
    }
    variants
}

// ---------------------------------------------------------------------------
// Per-variant: stable type URI, title and status
// ---------------------------------------------------------------------------

#[test]
fn authorization_required_type_title_status() {
    let p = authorization_required_full();
    assert_eq!(
        p.type_uri(),
        "https://vulcanflow.io/errors/authorization-required"
    );
    assert_eq!(p.title(), "Target is not authorized for scanning");
    assert_eq!(p.status(), 403);
}

#[test]
fn target_allowance_exceeded_type_title_status() {
    let p = target_allowance_exceeded();
    assert_eq!(
        p.type_uri(),
        "https://vulcanflow.io/errors/target-allowance-exceeded"
    );
    assert_eq!(p.title(), "Target allowance exceeded");
    assert_eq!(p.status(), 403);
}

#[test]
fn graph_invalid_type_title_status() {
    let p = graph_invalid();
    assert_eq!(p.type_uri(), "https://vulcanflow.io/errors/graph-invalid");
    assert_eq!(p.title(), "Graph is not valid");
    assert_eq!(p.status(), 422);
}

#[test]
fn idempotency_conflict_type_title_status() {
    let p = idempotency_conflict();
    assert_eq!(
        p.type_uri(),
        "https://vulcanflow.io/errors/idempotency-conflict"
    );
    assert_eq!(p.title(), "Idempotency key reused with different content");
    assert_eq!(p.status(), 409);
}

#[test]
fn illegal_transition_type_title_status() {
    let p = illegal_transition();
    assert_eq!(
        p.type_uri(),
        "https://vulcanflow.io/errors/illegal-transition"
    );
    assert_eq!(p.title(), "Observation state transition is not allowed");
    assert_eq!(p.status(), 409);
}

#[test]
fn challenge_failed_type_title_status() {
    let p = challenge_failed();
    assert_eq!(
        p.type_uri(),
        "https://vulcanflow.io/errors/challenge-failed"
    );
    assert_eq!(p.title(), "Authorization challenge failed");
    assert_eq!(p.status(), 422);
}

#[test]
fn not_found_type_title_status() {
    let p = not_found();
    assert_eq!(p.type_uri(), "https://vulcanflow.io/errors/not-found");
    assert_eq!(p.title(), "Resource not found");
    assert_eq!(p.status(), 404);
}

#[test]
fn forbidden_type_title_status() {
    let p = forbidden();
    assert_eq!(p.type_uri(), "https://vulcanflow.io/errors/forbidden");
    assert_eq!(p.title(), "Role does not permit this action");
    assert_eq!(p.status(), 403);
}

#[test]
fn tenant_suspended_type_title_status() {
    let p = tenant_suspended_with_reason();
    assert_eq!(
        p.type_uri(),
        "https://vulcanflow.io/errors/tenant-suspended"
    );
    assert_eq!(p.title(), "Account is suspended");
    assert_eq!(p.status(), 403);
}

#[test]
fn rate_limited_type_title_status() {
    let p = rate_limited();
    assert_eq!(p.type_uri(), "https://vulcanflow.io/errors/rate-limited");
    assert_eq!(p.title(), "Rate limit exceeded");
    assert_eq!(p.status(), 429);
}

#[test]
fn internal_type_title_status() {
    let p = internal();
    assert_eq!(p.type_uri(), "https://vulcanflow.io/errors/internal");
    assert_eq!(p.title(), "Internal error");
    assert_eq!(p.status(), 500);
}

// ---------------------------------------------------------------------------
// Per-variant: the serialized envelope and `vf` extension shape
// ---------------------------------------------------------------------------

#[test]
fn authorization_required_serializes_target_id_and_flattens_track_hints() {
    let value = support::to_value(&authorization_required_full());
    assert!(value.has_keys_in_order(ENVELOPE_KEYS));
    assert_eq!(value.get("status").unwrap().as_u64(), Some(403));
    assert!(value.get("instance").unwrap().is_none());

    let vf = value.get("vf").unwrap();
    assert!(vf.has_keys_in_order(&[
        "target_id",
        "track_a_available",
        "track_b_available",
        "track_b_reason",
        "next_actions",
    ]));
    assert_eq!(
        vf.get("target_id").unwrap().as_str(),
        Some("01890a5d-ac96-774b-bf71-ca4fde3b9c2b")
    );
    assert_eq!(vf.get("track_a_available").unwrap().as_bool(), Some(true));
    assert_eq!(vf.get("track_b_available").unwrap().as_bool(), Some(false));
    assert_eq!(
        vf.get("track_b_reason").unwrap().as_str(),
        Some("kyc_incomplete")
    );

    let actions = vf.get("next_actions").unwrap().as_seq().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0].get("action").unwrap().as_str(),
        Some("verify_ownership")
    );
    assert_eq!(
        actions[0].get("href").unwrap().as_str(),
        Some("/v1/targets/verify")
    );
}

#[test]
fn authorization_required_omits_target_id_and_track_b_reason_when_absent() {
    let value = support::to_value(&authorization_required_minimal());
    let vf = value.get("vf").unwrap();
    assert!(vf.get("target_id").is_none());
    assert!(vf.get("track_b_reason").is_none());
    assert!(vf.has_keys_in_order(&["track_a_available", "track_b_available", "next_actions"]));
}

#[test]
fn target_allowance_exceeded_serializes_the_allowance_hints() {
    let value = support::to_value(&target_allowance_exceeded());
    assert!(value.has_keys_in_order(ENVELOPE_KEYS));
    let vf = value.get("vf").unwrap();
    assert!(vf.has_keys_in_order(&["current", "limit", "package", "upgrade_href"]));
    assert_eq!(vf.get("current").unwrap().as_u64(), Some(10));
    assert_eq!(vf.get("limit").unwrap().as_u64(), Some(10));
    assert_eq!(vf.get("package").unwrap().as_str(), Some("starter"));
    assert_eq!(
        vf.get("upgrade_href").unwrap().as_str(),
        Some("/v1/billing/upgrade")
    );
}

#[test]
fn graph_invalid_serializes_every_error_and_omits_absent_node_id() {
    let value = support::to_value(&graph_invalid());
    let vf = value.get("vf").unwrap();
    let errors = vf.get("errors").unwrap().as_seq().unwrap();
    assert_eq!(errors.len(), 2);

    assert_eq!(errors[0].get("code").unwrap().as_str(), Some("cycle"));
    assert_eq!(errors[0].get("node_id").unwrap().as_str(), Some("n2"));
    assert_eq!(
        errors[0].get("message").unwrap().as_str(),
        Some("n2 participates in a cycle")
    );

    assert_eq!(errors[1].get("code").unwrap().as_str(), Some("empty_graph"));
    assert!(errors[1].get("node_id").is_none());
}

#[test]
fn idempotency_conflict_serializes_the_request_key() {
    let value = support::to_value(&idempotency_conflict());
    let vf = value.get("vf").unwrap();
    assert!(vf.has_keys_in_order(&["request_key"]));
    assert_eq!(
        vf.get("request_key").unwrap().as_str(),
        Some("client-key-123")
    );
}

#[test]
fn illegal_transition_serializes_from_attempted_and_admitted_as_wire_strings() {
    let value = support::to_value(&illegal_transition());
    let vf = value.get("vf").unwrap();
    assert!(vf.has_keys_in_order(&["from", "attempted", "admitted"]));
    assert_eq!(vf.get("from").unwrap().as_str(), Some("verifying"));
    assert_eq!(vf.get("attempted").unwrap().as_str(), Some("start_fix"));
    assert!(vf.get("admitted").unwrap().as_seq().unwrap().is_empty());
}

#[test]
fn illegal_transition_from_domain_error_fills_admitted_from_the_state_machine() {
    let domain_error = IllegalTransition {
        from: ObservationState::FixPending,
        event: ObservationEvent::StartFix,
    };
    let problem: Problem = domain_error.into();

    match &problem {
        Problem::IllegalTransition { detail, hints, .. } => {
            assert_eq!(
                detail,
                "illegal observation transition: start_fix is not admitted from fix_pending"
            );
            assert_eq!(hints.from, ObservationState::FixPending);
            assert_eq!(hints.attempted, ObservationEvent::StartFix);
            assert_eq!(
                hints.admitted,
                vec![
                    ObservationEvent::DecideFalsePositive,
                    ObservationEvent::AcceptRisk,
                    ObservationEvent::RequestVerification,
                ]
            );
        }
        other => panic!("expected IllegalTransition, got {other:?}"),
    }
    assert_eq!(
        problem.type_uri(),
        "https://vulcanflow.io/errors/illegal-transition"
    );
    assert_eq!(problem.status(), 409);
}

#[test]
fn challenge_failed_serializes_reason_and_retryable() {
    let value = support::to_value(&challenge_failed());
    let vf = value.get("vf").unwrap();
    assert!(vf.has_keys_in_order(&["reason", "retryable"]));
    assert_eq!(vf.get("reason").unwrap().as_str(), Some("token_not_found"));
    assert_eq!(vf.get("retryable").unwrap().as_bool(), Some(true));
}

#[test]
fn not_found_serializes_the_resource_kind() {
    let value = support::to_value(&not_found());
    let vf = value.get("vf").unwrap();
    assert!(vf.has_keys_in_order(&["resource"]));
    assert_eq!(vf.get("resource").unwrap().as_str(), Some("target"));
}

#[test]
fn forbidden_serializes_the_required_action() {
    let value = support::to_value(&forbidden());
    let vf = value.get("vf").unwrap();
    assert!(vf.has_keys_in_order(&["required_action"]));
    assert_eq!(
        vf.get("required_action").unwrap().as_str(),
        Some("member_manage")
    );
}

#[test]
fn tenant_suspended_serializes_the_reason_when_present() {
    let value = support::to_value(&tenant_suspended_with_reason());
    let vf = value.get("vf").unwrap();
    assert!(vf.has_keys_in_order(&["reason"]));
    assert_eq!(vf.get("reason").unwrap().as_str(), Some("payment_failed"));
}

#[test]
fn tenant_suspended_omits_the_reason_when_absent() {
    let value = support::to_value(&tenant_suspended_no_reason());
    let vf = value.get("vf").unwrap();
    assert!(vf.has_keys_in_order(&[]));
}

#[test]
fn rate_limited_serializes_bucket_and_retry_after_seconds() {
    let value = support::to_value(&rate_limited());
    let vf = value.get("vf").unwrap();
    assert!(vf.has_keys_in_order(&["bucket", "retry_after_seconds"]));
    assert_eq!(vf.get("bucket").unwrap().as_str(), Some("dispatch"));
    assert_eq!(vf.get("retry_after_seconds").unwrap().as_u64(), Some(30));
}

#[test]
fn internal_never_serializes_caller_facing_detail_only_a_trace_id() {
    let p = internal();
    assert_eq!(
        p.detail(),
        "An internal error occurred. The failure has been logged with the trace identifier in this response."
    );

    let value = support::to_value(&p);
    assert_eq!(
        value.get("detail").unwrap().as_str(),
        Some(
            "An internal error occurred. The failure has been logged with the trace identifier in this response."
        )
    );
    let vf = value.get("vf").unwrap();
    assert!(vf.has_keys_in_order(&["trace_id"]));
    assert_eq!(vf.get("trace_id").unwrap().as_str(), Some("trace-abc-123"));
}

#[test]
fn internal_constructor_sets_no_instance() {
    match internal() {
        Problem::Internal { instance, trace_id } => {
            assert_eq!(instance, None);
            assert_eq!(trace_id, "trace-abc-123");
        }
        other => panic!("expected Internal, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Cross-cutting: `instance`, `Display` and the envelope shape for every
// variant at once
// ---------------------------------------------------------------------------

#[test]
fn with_instance_sets_the_envelope_instance_and_nothing_else() {
    for problem in all_variants() {
        let before = support::to_value(&problem);
        let path = "/v1/pipeline-runs";
        let after_problem = problem.clone().with_instance(path);
        let after = support::to_value(&after_problem);

        assert_eq!(problem.instance(), None);
        assert_eq!(after_problem.instance(), Some(path));
        assert_eq!(after.get("instance").unwrap().as_str(), Some(path));

        // Everything else about the envelope is unchanged.
        assert_eq!(before.get("type"), after.get("type"));
        assert_eq!(before.get("title"), after.get("title"));
        assert_eq!(before.get("status"), after.get("status"));
        assert_eq!(before.get("detail"), after.get("detail"));
        assert_eq!(before.get("vf"), after.get("vf"));
    }
}

#[test]
fn display_is_title_colon_detail() {
    for problem in all_variants() {
        assert_eq!(
            problem.to_string(),
            format!("{}: {}", problem.title(), problem.detail())
        );
    }
}

#[test]
fn every_variant_serializes_the_six_member_envelope_in_order() {
    for problem in all_variants() {
        let value = support::to_value(&problem);
        assert!(
            value.has_keys_in_order(ENVELOPE_KEYS),
            "envelope shape mismatch for {problem:?}"
        );
        assert_eq!(
            value.get("type").unwrap().as_str(),
            Some(problem.type_uri())
        );
        assert_eq!(value.get("title").unwrap().as_str(), Some(problem.title()));
        assert_eq!(
            value.get("status").unwrap().as_u64(),
            Some(u64::from(problem.status()))
        );
        assert_eq!(
            value.get("detail").unwrap().as_str(),
            Some(problem.detail())
        );
    }
}

#[test]
fn every_variant_type_uri_is_under_the_stable_error_namespace() {
    for problem in all_variants() {
        assert!(problem.type_uri().starts_with(ERROR_TYPE_BASE));
    }
}

#[test]
fn every_variant_status_is_a_valid_http_error_status() {
    for problem in all_variants() {
        assert!(
            [403, 404, 409, 422, 429, 500].contains(&problem.status()),
            "unexpected status {} for {problem:?}",
            problem.status()
        );
    }
}
