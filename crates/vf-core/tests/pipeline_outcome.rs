//! T1a (VFL-112, covers C1 / VFL-14): `vf-core::state::pipeline_outcome`.
//!
//! TDD §8.2 completion rule (via VFL-8#document-architecture): a run is
//! finalizable only once discovery is closed, there is no pending fan-in,
//! every registered work unit is terminal and every unit's required
//! ingestion is complete — all four checked before any state is derived, so
//! `None` means "leave the run running" and `Some` is safe to write once.
//! With all four satisfied, the final state follows from the units' outcome
//! classes per the table in `pipeline_outcome`'s doc comment.

use vf_core::state::{OutcomeClass, PipelineState, UnitSummary, WorkUnitStatus, pipeline_outcome};

fn success() -> UnitSummary {
    UnitSummary::terminal(WorkUnitStatus::Terminal, OutcomeClass::Success)
}

fn terminal(class: OutcomeClass) -> UnitSummary {
    UnitSummary::terminal(WorkUnitStatus::Terminal, class)
}

// ---------------------------------------------------------------------------
// The four completion preconditions
// ---------------------------------------------------------------------------

#[test]
fn none_while_discovery_is_open_even_with_no_units_and_no_fan_in() {
    assert_eq!(pipeline_outcome(&[], false, 0), None);
}

#[test]
fn none_while_discovery_is_open_regardless_of_unit_state() {
    assert_eq!(pipeline_outcome(&[success()], false, 0), None);
}

#[test]
fn none_while_fan_in_is_pending() {
    assert_eq!(pipeline_outcome(&[success()], true, 1), None);
}

#[test]
fn none_while_any_unit_is_not_terminal() {
    for status in [
        WorkUnitStatus::Registered,
        WorkUnitStatus::Reserved,
        WorkUnitStatus::Admitted,
        WorkUnitStatus::Running,
    ] {
        let units = [success(), UnitSummary::in_flight(status)];
        assert_eq!(
            pipeline_outcome(&units, true, 0),
            None,
            "status: {status:?}"
        );
    }
}

#[test]
fn none_while_a_terminal_unit_is_missing_required_ingestion() {
    let incomplete = UnitSummary {
        status: WorkUnitStatus::Terminal,
        outcome_class: Some(OutcomeClass::Success),
        required_ingestion_complete: false,
    };
    assert_eq!(pipeline_outcome(&[incomplete], true, 0), None);
}

#[test]
fn none_while_a_terminal_unit_has_no_recorded_outcome_class() {
    let incomplete = UnitSummary {
        status: WorkUnitStatus::Terminal,
        outcome_class: None,
        required_ingestion_complete: true,
    };
    assert_eq!(pipeline_outcome(&[incomplete], true, 0), None);
}

// ---------------------------------------------------------------------------
// With all four preconditions satisfied
// ---------------------------------------------------------------------------

#[test]
fn empty_slice_is_completed_with_no_output() {
    assert_eq!(
        pipeline_outcome(&[], true, 0),
        Some(PipelineState::Completed)
    );
}

#[test]
fn all_success_is_completed() {
    let units = [success(), success(), success()];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::Completed)
    );
}

#[test]
fn some_but_not_all_success_is_partially_completed() {
    let units = [success(), terminal(OutcomeClass::Target)];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::PartiallyCompleted)
    );
}

#[test]
fn one_success_among_many_failures_is_still_partially_completed() {
    let units = [
        success(),
        terminal(OutcomeClass::Target),
        terminal(OutcomeClass::Platform),
        terminal(OutcomeClass::Cancelled),
    ];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::PartiallyCompleted)
    );
}

#[test]
fn no_success_and_any_cancelled_is_cancelled() {
    let units = [
        terminal(OutcomeClass::Cancelled),
        terminal(OutcomeClass::Target),
    ];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::Cancelled)
    );
}

#[test]
fn cancelled_outranks_platform_and_target_together() {
    let units = [
        terminal(OutcomeClass::Cancelled),
        terminal(OutcomeClass::Platform),
        terminal(OutcomeClass::Target),
        terminal(OutcomeClass::Tool),
    ];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::Cancelled)
    );
}

#[test]
fn no_success_no_cancelled_and_any_platform_is_platform_failed() {
    let units = [
        terminal(OutcomeClass::Platform),
        terminal(OutcomeClass::Target),
    ];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::PlatformFailed)
    );
}

#[test]
fn tool_outcome_also_maps_to_platform_failed() {
    let units = [terminal(OutcomeClass::Tool), terminal(OutcomeClass::Target)];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::PlatformFailed)
    );
}

#[test]
fn platform_and_tool_outrank_target() {
    let units = [
        terminal(OutcomeClass::Target),
        terminal(OutcomeClass::Platform),
        terminal(OutcomeClass::Tool),
    ];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::PlatformFailed)
    );
}

#[test]
fn no_success_no_cancelled_no_platform_no_tool_and_any_target_is_target_failed() {
    let units = [
        terminal(OutcomeClass::Target),
        terminal(OutcomeClass::Scope),
    ];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::TargetFailed)
    );
}

#[test]
fn scope_and_limit_skips_alone_are_completed_with_no_platform_or_target_failure() {
    let units = [terminal(OutcomeClass::Scope), terminal(OutcomeClass::Limit)];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::Completed)
    );
}

#[test]
fn scope_only_is_completed() {
    let units = [terminal(OutcomeClass::Scope)];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::Completed)
    );
}

#[test]
fn limit_only_is_completed() {
    let units = [terminal(OutcomeClass::Limit)];
    assert_eq!(
        pipeline_outcome(&units, true, 0),
        Some(PipelineState::Completed)
    );
}

#[test]
fn timed_out_is_never_derived_from_unit_records() {
    // §8.2: work units share one Cancelled/TimedOut outcome row, so the
    // run-level timeout sets `TimedOut` directly and this function cannot
    // produce it from any combination of `OutcomeClass`.
    for class in OutcomeClass::ALL {
        let units = [terminal(*class)];
        assert_ne!(
            pipeline_outcome(&units, true, 0),
            Some(PipelineState::TimedOut)
        );
    }
}
