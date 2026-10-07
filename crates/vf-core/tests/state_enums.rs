//! T1a (VFL-112, covers C1 / VFL-14): status enums and the §15.1 observation
//! state machine of `vf-core::state`.
//!
//! Architecture §A3.2, TDD §6.3 / §8.2 / §15.1 (VFL-8#document-architecture).
//!
//! * Every `status_enum!` type: `as_str`/`from_str` round-trip over every
//!   declared variant, serde agrees with `as_str` (via `tests/support`, since
//!   `vf-core` has no `serde_json` dev-dependency available to a test-only
//!   pull request), and an unknown string is a hard [`UnknownValue`] error
//!   naming the enum and listing every accepted value — never a silent
//!   default (§6.3).
//! * [`observation_transition`] admits exactly the six edges the §15.1
//!   diagram draws and rejects every other `(state, event)` pair.
//! * [`apply_verification`] resolves a verification outcome against every
//!   possible prior state.
//! * [`outcome_consumes`] is true only for [`OutcomeClass::Success`].
//! * [`WorkUnitStatus::is_terminal`] is true only for `Skipped` and
//!   `Terminal`.

mod support;

use vf_core::state::{
    Action, IllegalTransition, ObservationEvent, ObservationState, OutcomeClass, PipelineState,
    ReportState, ReservationState, Role, UnknownValue, VerificationOutcome, WorkUnitStatus,
    apply_verification, observation_transition, outcome_consumes,
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

// ---------------------------------------------------------------------------
// `observation_transition` — exactly the six §15.1 edges
// ---------------------------------------------------------------------------

/// The complete §15.1 edge set, as the acceptance criteria name it.
fn admitted_edges() -> Vec<(ObservationState, ObservationEvent, ObservationState)> {
    use ObservationEvent as E;
    use ObservationState as S;
    vec![
        (S::New, E::Acknowledge, S::Acknowledged),
        (S::New, E::StartFix, S::FixPending),
        (S::New, E::DecideFalsePositive, S::FalsePositive),
        (S::New, E::RequestVerification, S::Verifying),
        (S::Acknowledged, E::RequestVerification, S::Verifying),
        (S::FixPending, E::RequestVerification, S::Verifying),
    ]
}

#[test]
fn observation_transition_admits_exactly_the_six_diagram_edges() {
    let edges = admitted_edges();

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
            E::RequestVerification
        ]
    );
    assert_eq!(
        S::Acknowledged.admitted_events(),
        vec![E::RequestVerification]
    );
    assert_eq!(
        S::FixPending.admitted_events(),
        vec![E::RequestVerification]
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
