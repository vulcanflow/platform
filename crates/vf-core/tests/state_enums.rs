//! T1a (VFL-112, covers C1 / VFL-14): status enums and the observation state
//! machine of `vf-core::state`.
//!
//! Architecture §A3.2 (VFL-8#document-architecture), TDD §6.3 / §8.2 / §10.4
//! / §15.1, and the architect ruling on
//! [VFL-235](/VFL/issues/VFL-235#document-decision), which corrects the
//! §15.1 diagram's edge set against §10.4, §6.3 and §A3.8 and is the
//! recorded decision that authorizes the edge-set and `Action` changes
//! below.
//!
//! * Every `status_enum!` type: `as_str`/`from_str` round-trip over every
//!   declared variant, serde agrees with `as_str` (via `tests/support`, since
//!   `vf-core` has no `serde_json` dev-dependency available to a test-only
//!   pull request), and an unknown string is a hard [`UnknownValue`] error
//!   naming the enum and listing every accepted value — never a silent
//!   default (§6.3).
//! * [`observation_transition`] admits exactly the twelve edges ruled on
//!   VFL-235 and rejects every other `(state, event)` pair. The §15.1
//!   diagram is corrected by that ruling, not narrowed by it.
//! * [`apply_verification`] resolves a verification outcome against every
//!   possible prior state.
//! * [`outcome_consumes`] is true only for [`OutcomeClass::Success`].
//! * [`WorkUnitStatus::is_terminal`] is true only for `Skipped` and
//!   `Terminal`.
//! * Every `vf_core::state` enum's `VALUES` is pinned against a literal
//!   quoted from the contract (VFL-236, VFL-233 MEDIUM-2), independently of
//!   the type's own `as_str()` — the round-trip tests above all derive their
//!   expectation from `as_str()`/`VALUES` and so would stay green through a
//!   wire-string rename. `Action` has no such assertion: §4.2 and
//!   architecture §A3.2 give it only a non-exhaustive illustrative comment,
//!   not a complete ordered variant list or any wire string to quote.
//! * [`RequestedObservationState`] round-trips the same way and is pinned to
//!   its four §A3.8 request-body literals; its `From` conversion to
//!   [`ObservationEvent`] matches the VFL-235 four-way mapping, and
//!   `from_str` rejects `new`, `verifying` and `fixed` because no request
//!   body can name an event-only or exit-only state.

mod support;

use vf_core::state::{
    Action, IllegalTransition, ObservationEvent, ObservationState, OutcomeClass, PipelineState,
    ReportState, RequestedObservationState, ReservationState, Role, UnknownValue,
    VerificationOutcome, WorkUnitStatus, apply_verification, observation_transition,
    outcome_consumes,
};

/// Generates the shared round-trip and unknown-value tests for one
/// `status_enum!` type.
macro_rules! status_enum_tests {
    ($mod_name:ident, $ty:ty, $type_name:literal) => {
        mod $mod_name {
            use super::*;

            #[test]
            fn as_str_values_are_distinct() {
                let values = <$ty>::VALUES;
                for (i, a) in values.iter().enumerate() {
                    for (j, b) in values.iter().enumerate() {
                        if i != j {
                            assert_ne!(a, b, "duplicate wire value {a:?} in {}", $type_name);
                        }
                    }
                }
            }

            #[test]
            fn every_variant_round_trips_through_as_str_and_from_str() {
                for variant in <$ty>::ALL {
                    let text = variant.as_str();
                    let parsed: $ty = text.parse().unwrap();
                    assert_eq!(parsed, *variant);
                }
            }

            #[test]
            fn display_matches_as_str() {
                for variant in <$ty>::ALL {
                    assert_eq!(variant.to_string(), variant.as_str());
                }
            }

            #[test]
            fn every_variant_round_trips_through_serde_as_the_as_str_wire_form() {
                for variant in <$ty>::ALL {
                    let value = support::to_value(variant);
                    assert_eq!(value.as_str(), Some(variant.as_str()));
                    assert_eq!(support::from_value::<$ty>(&value), *variant);
                }
            }

            #[test]
            fn from_str_rejects_an_unknown_value_naming_this_enum_and_every_accepted_value() {
                let err: UnknownValue = "totally-unknown-value".parse::<$ty>().unwrap_err();
                assert_eq!(err.enum_name, $type_name);
                assert_eq!(err.value, "totally-unknown-value");
                assert_eq!(err.expected, <$ty>::VALUES);
            }

            #[test]
            fn from_str_rejects_the_empty_string() {
                assert!("".parse::<$ty>().is_err());
            }

            #[test]
            fn from_str_is_case_sensitive() {
                // The wire/database form is fixed snake_case (module docs); an
                // uppercased variant name is not a second accepted spelling.
                for variant in <$ty>::ALL {
                    let upper = variant.as_str().to_uppercase();
                    if upper != variant.as_str() {
                        assert!(upper.parse::<$ty>().is_err());
                    }
                }
            }
        }
    };
}

status_enum_tests!(pipeline_state, PipelineState, "PipelineState");
status_enum_tests!(work_unit_status, WorkUnitStatus, "WorkUnitStatus");
status_enum_tests!(outcome_class, OutcomeClass, "OutcomeClass");
status_enum_tests!(observation_state, ObservationState, "ObservationState");
status_enum_tests!(
    verification_outcome,
    VerificationOutcome,
    "VerificationOutcome"
);
status_enum_tests!(reservation_state, ReservationState, "ReservationState");
status_enum_tests!(report_state, ReportState, "ReportState");
status_enum_tests!(role, Role, "Role");
status_enum_tests!(action, Action, "Action");
status_enum_tests!(observation_event, ObservationEvent, "ObservationEvent");
status_enum_tests!(
    requested_observation_state,
    RequestedObservationState,
    "RequestedObservationState"
);

// ---------------------------------------------------------------------------
// Literal wire values, pinned from the contract (VFL-236, VFL-233 MEDIUM-2)
// ---------------------------------------------------------------------------
//
// Each assertion quotes a literal from TDD v2.3 or the architecture
// document rather than deriving its expectation from `vf_core::state`
// itself, so a rename of a wire value (for example `partially_completed` to
// `partiallyCompleted`) fails here even though every round-trip test above
// stays green.
//
// Two enums have no §6.3/§15.4/§16.8/§17.3 `CHECK` list naming their wire
// strings: `PipelineState` (`pipeline_runs.status text NOT NULL`, no
// `CHECK`) and `WorkUnitStatus` (`scan_work_units.status text NOT NULL`, no
// `CHECK`). For both, architecture §A3.2 gives a complete, ordered Rust
// enum declaration with no illustrative `/* e.g. ... */` qualifier — "Names
// are binding" (§A3.2 preamble) — and every other enum below confirms,
// byte-for-byte against its own `CHECK` list, that this codebase's one wire
// convention is the snake_case form of that binding name. `PipelineState`'s
// order and names are also confirmed by the §8.2 prose list.

#[test]
fn pipeline_state_values_are_pinned_to_the_contract() {
    assert_eq!(
        PipelineState::VALUES,
        &[
            "validating",
            "refused",
            "dispatching",
            "running",
            "completed",
            "partially_completed",
            "target_failed",
            "platform_failed",
            "cancelled",
            "timed_out",
        ]
    );
}

#[test]
fn work_unit_status_values_are_pinned_to_the_contract() {
    assert_eq!(
        WorkUnitStatus::VALUES,
        &[
            "registered",
            "reserved",
            "skipped",
            "admitted",
            "running",
            "terminal",
        ]
    );
}

#[test]
fn outcome_class_values_are_pinned_to_the_contract() {
    // TDD §6.3 `scan_work_units.outcome_class` comment, quoted verbatim:
    // "success|target|platform|tool|scope|limit|cancelled".
    assert_eq!(
        OutcomeClass::VALUES,
        &[
            "success",
            "target",
            "platform",
            "tool",
            "scope",
            "limit",
            "cancelled",
        ]
    );
}

#[test]
fn observation_state_values_are_pinned_to_the_contract() {
    // TDD §6.3 `finding_states.state` CHECK list, quoted verbatim.
    assert_eq!(
        ObservationState::VALUES,
        &[
            "new",
            "acknowledged",
            "fix_pending",
            "verifying",
            "fixed",
            "false_positive",
            "accepted_risk",
        ]
    );
}

#[test]
fn verification_outcome_values_are_pinned_to_the_contract() {
    // TDD §15.4 `verification_runs.outcome` CHECK list, quoted verbatim.
    assert_eq!(
        VerificationOutcome::VALUES,
        &["not_detected", "still_present", "inconclusive"]
    );
}

#[test]
fn reservation_state_values_are_pinned_to_the_contract() {
    // TDD §17.3 `scan_usage_reservations.state` CHECK list, quoted
    // verbatim.
    assert_eq!(
        ReservationState::VALUES,
        &["reserved", "consumed", "released"]
    );
}

#[test]
fn report_state_values_are_pinned_to_the_contract() {
    // TDD §16.8 `reports.status` CHECK list, quoted verbatim.
    assert_eq!(
        ReportState::VALUES,
        &["queued", "assembling", "rendering", "ready", "failed"]
    );
}

#[test]
fn role_values_are_pinned_to_the_contract() {
    // TDD §4.2: "GA ships `admin` and `member`"; the role matrix's two
    // columns use the same two literals in the same order.
    assert_eq!(Role::VALUES, &["admin", "member"]);
}

#[test]
fn observation_event_values_are_pinned_to_the_ruled_five() {
    // Per the VFL-235 ruling: `AcceptRisk` ("accept_risk") is a fifth
    // caller-requested event, inserted before `RequestVerification` in the
    // ruled `ObservationEvent::ALL` order.
    assert_eq!(
        ObservationEvent::VALUES,
        &[
            "acknowledge",
            "start_fix",
            "decide_false_positive",
            "accept_risk",
            "request_verification",
        ]
    );
}

#[test]
fn requested_observation_state_values_are_pinned_to_the_a3_8_request_body() {
    // §A3.8 `POST /v1/findings/{id}/state` body, quoted verbatim in the
    // VFL-235 ruling: `state: acknowledged|fix_pending|accepted_risk|false_positive`.
    assert_eq!(
        RequestedObservationState::VALUES,
        &[
            "acknowledged",
            "fix_pending",
            "accepted_risk",
            "false_positive",
        ]
    );
}

// ---------------------------------------------------------------------------
// `From<RequestedObservationState> for ObservationEvent` — the VFL-235
// four-way mapping
// ---------------------------------------------------------------------------

#[test]
fn requested_observation_state_maps_to_the_ruled_observation_event() {
    use ObservationEvent as E;
    use RequestedObservationState as R;

    assert_eq!(ObservationEvent::from(R::Acknowledged), E::Acknowledge);
    assert_eq!(ObservationEvent::from(R::FixPending), E::StartFix);
    assert_eq!(ObservationEvent::from(R::AcceptedRisk), E::AcceptRisk);
    assert_eq!(
        ObservationEvent::from(R::FalsePositive),
        E::DecideFalsePositive
    );
}

#[test]
fn requested_observation_state_from_str_rejects_new_verifying_and_fixed() {
    // §A3.8's request body can only name a caller-requested target state;
    // `new`, `verifying` and `fixed` are an `ObservationState`'s own
    // event-only or exit-only values and no request body can name them
    // (VFL-235 ruling §2). `RequestVerification` is reachable only through
    // `POST /v1/findings/{id}/verify`, never through this enum.
    for value in ["new", "verifying", "fixed"] {
        let err = value.parse::<RequestedObservationState>().unwrap_err();
        assert_eq!(err.enum_name, "RequestedObservationState");
        assert_eq!(err.value, value);
        assert_eq!(err.expected, RequestedObservationState::VALUES);
    }
}

// ---------------------------------------------------------------------------
// `observation_transition` — exactly the twelve edges ruled on VFL-235
// ---------------------------------------------------------------------------

/// The complete ruled edge set (VFL-235 decision, table in §1): the §15.1
/// diagram's six edges plus the six `accepted_risk`/backfilled edges that
/// ruling adds against §10.4, §6.3 and §A3.8.
fn admitted_edges() -> Vec<(ObservationState, ObservationEvent, ObservationState)> {
    use ObservationEvent as E;
    use ObservationState as S;
    vec![
        (S::New, E::Acknowledge, S::Acknowledged),
        (S::New, E::StartFix, S::FixPending),
        (S::New, E::DecideFalsePositive, S::FalsePositive),
        (S::New, E::AcceptRisk, S::AcceptedRisk),
        (S::New, E::RequestVerification, S::Verifying),
        (S::Acknowledged, E::StartFix, S::FixPending),
        (S::Acknowledged, E::DecideFalsePositive, S::FalsePositive),
        (S::Acknowledged, E::AcceptRisk, S::AcceptedRisk),
        (S::Acknowledged, E::RequestVerification, S::Verifying),
        (S::FixPending, E::DecideFalsePositive, S::FalsePositive),
        (S::FixPending, E::AcceptRisk, S::AcceptedRisk),
        (S::FixPending, E::RequestVerification, S::Verifying),
    ]
}

#[test]
fn observation_transition_admits_exactly_the_twelve_ruled_edges() {
    let edges = admitted_edges();
    assert_eq!(edges.len(), 12, "VFL-235 rules exactly twelve edges");

    for (from, event, to) in &edges {
        assert_eq!(observation_transition(*from, *event), Ok(*to));
    }

    for &from in ObservationState::ALL {
        for &event in ObservationEvent::ALL {
            if !edges.iter().any(|(f, e, _)| *f == from && *e == event) {
                assert_eq!(
                    observation_transition(from, event),
                    Err(IllegalTransition { from, event }),
                    "expected {from:?} + {event:?} to be illegal"
                );
            }
        }
    }
}

#[test]
fn admitted_events_is_derived_from_observation_transition() {
    use ObservationEvent as E;
    use ObservationState as S;

    assert_eq!(
        S::New.admitted_events(),
        vec![
            E::Acknowledge,
            E::StartFix,
            E::DecideFalsePositive,
            E::AcceptRisk,
            E::RequestVerification,
        ]
    );
    assert_eq!(
        S::Acknowledged.admitted_events(),
        vec![
            E::StartFix,
            E::DecideFalsePositive,
            E::AcceptRisk,
            E::RequestVerification,
        ]
    );
    assert_eq!(
        S::FixPending.admitted_events(),
        vec![
            E::DecideFalsePositive,
            E::AcceptRisk,
            E::RequestVerification,
        ]
    );
    assert_eq!(S::Verifying.admitted_events(), Vec::new());
    assert_eq!(S::Fixed.admitted_events(), Vec::new());
    assert_eq!(S::FalsePositive.admitted_events(), Vec::new());
    assert_eq!(S::AcceptedRisk.admitted_events(), Vec::new());
}

#[test]
fn the_three_terminal_states_admit_no_event() {
    use ObservationState as S;
    for &terminal in &[S::Fixed, S::FalsePositive, S::AcceptedRisk] {
        for &event in ObservationEvent::ALL {
            assert!(observation_transition(terminal, event).is_err());
        }
    }
}

// ---------------------------------------------------------------------------
// `apply_verification` — total over every prior state
// ---------------------------------------------------------------------------

#[test]
fn not_detected_always_resolves_to_fixed_regardless_of_prior() {
    for &prior in ObservationState::ALL {
        assert_eq!(
            apply_verification(prior, VerificationOutcome::NotDetected),
            ObservationState::Fixed
        );
    }
}

#[test]
fn still_present_always_resolves_to_new_regardless_of_prior() {
    for &prior in ObservationState::ALL {
        assert_eq!(
            apply_verification(prior, VerificationOutcome::StillPresent),
            ObservationState::New
        );
    }
}

#[test]
fn inconclusive_always_restores_the_prior_state() {
    for &prior in ObservationState::ALL {
        assert_eq!(
            apply_verification(prior, VerificationOutcome::Inconclusive),
            prior
        );
    }
}

// ---------------------------------------------------------------------------
// `outcome_consumes` — true only for `Success`
// ---------------------------------------------------------------------------

#[test]
fn outcome_consumes_is_true_only_for_success() {
    for &class in OutcomeClass::ALL {
        assert_eq!(outcome_consumes(class), class == OutcomeClass::Success);
    }
}

// ---------------------------------------------------------------------------
// `WorkUnitStatus::is_terminal` — true only for `Skipped` and `Terminal`
// ---------------------------------------------------------------------------

#[test]
fn work_unit_status_is_terminal_only_for_skipped_and_terminal() {
    for &status in WorkUnitStatus::ALL {
        let expected = matches!(status, WorkUnitStatus::Skipped | WorkUnitStatus::Terminal);
        assert_eq!(status.is_terminal(), expected, "status: {status:?}");
    }
}
