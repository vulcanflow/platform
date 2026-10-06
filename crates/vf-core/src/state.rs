//! State enums, transition functions and the role x action policy
//! (architecture §A3.2; TDD §4.2, §6.3, §8.2, §15.1, §17.3).
//!
//! Three rules hold for everything in this module.
//!
//! 1. **Every text-typed status column of §A4 is one enum here**, with
//!    `as_str`/`from_str` that errors on unknown values (§6.3). An unknown
//!    string read from the database is a hard error, never a silent default,
//!    which is why serde goes through the same two functions rather than a
//!    second `rename_all` derive that could drift from them.
//! 2. **Transitions are total functions and a transition absent from the TDD is
//!    unrepresentable** (§A3.2). [`observation_transition`] has one arm per edge
//!    of the §15.1 diagram and no catch-all that would admit a new edge by
//!    accident.
//! 3. **The policy is one exhaustive `match` with no wildcard arm** (§4.2), so
//!    adding a [`Role`] or an [`Action`] fails compilation until the policy
//!    decides the new cells.
//!
//! String forms are `snake_case` throughout, which makes the database text, the
//! `as_str` form and the serde/SSE form one single spelling. §9.2's illustrative
//! `"Running"` and `"Succeeded"` are pre-v2.3 prose; the binding values are the
//! `CHECK` lists of §6.3 (`new`, `fix_pending`, `false_positive`, `reserved`,
//! `consumed`, `released`, `success|target|platform|tool|scope|limit|cancelled`)
//! and §A3.3's typed event fields.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A status string that is not a member of the enum it was parsed into.
///
/// Returned by every `from_str` in this module. §6.3 requires an unknown value
/// read from the database to be a hard error, and this is that error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown {enum_name} value {value:?}; expected one of: {}", expected.join(", "))]
pub struct UnknownValue {
    /// The enum that rejected the value, for example `"PipelineState"`.
    pub enum_name: &'static str,
    /// The rejected input.
    pub value: String,
    /// The accepted values, in declaration order. Also what the OpenAPI
    /// `enum` list for the column is generated from.
    pub expected: &'static [&'static str],
}

/// Defines one status enum with `as_str`, `from_str`, `ALL`, `Display` and
/// serde.
///
/// serde is implemented **through** `as_str`/`from_str` rather than derived
/// with `rename_all`, so the wire form and the database form cannot drift apart
/// and the round-trip property of the acceptance criteria holds by construction
/// rather than by a test that happens to cover both.
macro_rules! status_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$vmeta:meta])* $variant:ident => $text:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)+
        }

        impl $name {
            /// The enum's name, used in [`UnknownValue`] and in log fields.
            pub const TYPE_NAME: &'static str = stringify!($name);

            /// Every variant, in declaration order.
            ///
            /// Exhaustiveness tests and the role x action matrix test iterate
            /// this instead of hard-coding a list that could fall behind the
            /// enum.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            /// Every accepted string value, in the same order as [`Self::ALL`].
            pub const VALUES: &'static [&'static str] = &[$($text,)+];

            /// The database and wire form of this variant (§6.3).
            #[must_use]
            pub const fn as_str(&self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = UnknownValue;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok(Self::$variant),)+
                    other => Err(UnknownValue {
                        enum_name: Self::TYPE_NAME,
                        value: other.to_owned(),
                        expected: Self::VALUES,
                    }),
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
                raw.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

status_enum! {
    /// The state of one submitted graph (§8.2, `pipeline_runs.status`).
    ///
    /// The first four are in-flight; the rest are final. Which final state a
    /// finished run takes is derived from durable unit records by
    /// [`pipeline_outcome`], never from a process exit code (§8.2).
    PipelineState {
        /// Submitted; the graph and the run scope are being checked. No work
        /// unit exists yet.
        Validating => "validating",
        /// The graph or the scope was refused. §A3.8: a refused graph writes no
        /// run row at all, so this state exists for runs refused after the row
        /// was created.
        Refused => "refused",
        /// Accepted; work units are registered and outbox intent is written
        /// (§8.1).
        Dispatching => "dispatching",
        /// At least one work unit is executing.
        Running => "running",
        /// Every registered work unit succeeded.
        Completed => "completed",
        /// Some units succeeded and some did not. The successful units are
        /// consumed; §8.2 neither refunds nor charges the whole graph.
        PartiallyCompleted => "partially_completed",
        /// No unit succeeded and the dominant reason was the target
        /// (unreachable, DNS failure, blocked or rate-limiting).
        TargetFailed => "target_failed",
        /// No unit succeeded and the dominant reason was ours (operator, image,
        /// storage, parser, ingest or scanner).
        PlatformFailed => "platform_failed",
        /// Stopped on a durable stop intent (§8.3) with no successful unit.
        Cancelled => "cancelled",
        /// Exceeded the run-level time budget. Set by the run-level timeout, not
        /// derived from unit summaries: §8.2 gives work units one shared
        /// `Cancelled / TimedOut` outcome row, so [`pipeline_outcome`] cannot
        /// tell the two apart and never returns this variant.
        TimedOut => "timed_out",
    }
}

status_enum! {
    /// The lifecycle of one logical work unit (§6.3 `scan_work_units.status`).
    ///
    /// `Skipped` and `Terminal` are both final — see [`Self::is_terminal`].
    /// §8.2: "an unauthorized/skipped branch is terminal with a recorded
    /// reason".
    WorkUnitStatus {
        /// Created inside the submission transaction (§8.1); no allowance held.
        Registered => "registered",
        /// Holds one allowance reservation (§17.3). The reservation survives
        /// bounded infrastructure retries.
        Reserved => "reserved",
        /// Final. Scope, allowance, unsupported connectivity or an unmet
        /// dependency prevented execution; any reservation was released.
        Skipped => "skipped",
        /// Past the start barrier (§5.7): a live authorization basis and a
        /// reservation were both confirmed.
        Admitted => "admitted",
        /// A secureCodeBox Scan attempt is in flight.
        Running => "running",
        /// Final. The unit's outcome class is recorded and its reservation is
        /// settled or released.
        Terminal => "terminal",
    }
}

status_enum! {
    /// Why a work unit ended, and therefore whether it consumes allowance
    /// (§6.3 `scan_work_units.outcome_class`, §8.2 table).
    ///
    /// §17.3 is unconditional: failed scans consume no allowance, including
    /// unreachable targets, blocks and platform errors. Only [`Self::Success`]
    /// consumes — see [`outcome_consumes`].
    OutcomeClass {
        /// The intended scan executed and required parsing and ingestion
        /// completed. Zero findings is a valid success (§8.2).
        Success => "success",
        /// Unreachable, a DNS failure relevant to the intended operation, or a
        /// blocked or rate-limiting target.
        Target => "target",
        /// Operator, image, storage, parser or ingest failure. Bounded
        /// infrastructure retries reuse the reservation (§17.3).
        Platform => "platform",
        /// The scanner could not complete the intended operation. §8.2 leaves
        /// mixed success/failure within one unit open; the adapter emits this
        /// class when it cannot classify, and never infers success from exit
        /// zero alone.
        Tool => "tool",
        /// Refused by the configured scan scope (§5.3) or by a missing
        /// authorization basis.
        Scope => "scope",
        /// Refused by allowance, entitlement or unsupported connectivity.
        /// Recorded as a coverage limit, not a platform error (§8.2).
        Limit => "limit",
        /// A durable stop intent or a timeout ended the unit (§8.3).
        Cancelled => "cancelled",
    }
}

status_enum! {
    /// The state of **one historical observation** (§15.1, §6.3
    /// `finding_states.state`).
    ///
    /// Every later scan creates fresh observations; there is no persistent
    /// regression model and no inherited fixed state (§15.1). `PriorState` in
    /// the §15.1 diagram is restoration behaviour, not a variant — see
    /// [`apply_verification`].
    ObservationState {
        /// As ingested, or re-opened because verification still found it.
        New => "new",
        /// Seen and accepted as real, with no fix underway.
        Acknowledged => "acknowledged",
        /// A fix is being worked on.
        FixPending => "fix_pending",
        /// A verification run is in flight (§15.4).
        Verifying => "verifying",
        /// An eligible verification completed and did not detect the issue.
        Fixed => "fixed",
        /// An explicit false-positive decision was recorded (§10.2).
        FalsePositive => "false_positive",
        /// The risk was accepted. §6.3 lists this value and §A3.8's
        /// `POST /v1/findings/{id}/state` accepts it, but the §15.1 diagram
        /// draws no edge to it, so [`observation_transition`] admits none. See
        /// the module note on the open contract gap.
        AcceptedRisk => "accepted_risk",
    }
}

status_enum! {
    /// The result of one verification run (§15.1, §15.4).
    ///
    /// Note that `Inconclusive` is a verification **outcome**, not an
    /// observation state: §15.1 restores the previous observation state instead
    /// of inventing one. [`apply_verification`] is where that happens.
    VerificationOutcome {
        /// The eligible check completed and did not detect the issue.
        NotDetected => "not_detected",
        /// The issue is still present.
        StillPresent => "still_present",
        /// The check could not decide. Retain the previous observation state.
        Inconclusive => "inconclusive",
    }
}

status_enum! {
    /// The state of one allowance reservation (§17.3
    /// `scan_usage_reservations.state`).
    ///
    /// The database state machine is the authoritative guarantee across
    /// processes and retries. `vf-meter`'s move-only `Reservation` value is a
    /// coding aid on top of it (§17.3), not a replacement.
    ReservationState {
        /// Held against the period balance; the unit has not settled.
        Reserved => "reserved",
        /// Settled as one consumption entry after confirmed successful
        /// execution, required parsing and ingestion.
        Consumed => "consumed",
        /// Returned to the period balance because the unit never executed or
        /// failed terminally.
        Released => "released",
    }
}

status_enum! {
    /// The state of one generated report (§16.8 `reports.status`).
    ///
    /// Report *logic* is M4 work; the schema and this enum exist now because
    /// §A3.3's `report.state` tenant event carries the value and §A4 creates the
    /// table in the first migration. §16.8: a report becomes [`Self::Ready`]
    /// only after its required artifacts and checksums are committed, and
    /// delivery follows that durable state rather than the render finishing.
    ReportState {
        /// The report job is committed and waiting for a worker to claim its
        /// lease (§16.9).
        Queued => "queued",
        /// Selecting observations and assembling content against the cutoff.
        Assembling => "assembling",
        /// Rendering in the isolated headless browser (§16.4).
        Rendering => "rendering",
        /// Final. Artifacts and checksums are committed, so the report may be
        /// downloaded through the authorized endpoint. §9.2: download URLs are
        /// never placed in events.
        Ready => "ready",
        /// Final. §16.9 requires an actionable failure reason to be preserved
        /// alongside this state.
        Failed => "failed",
    }
}

status_enum! {
    /// A tenant role (§4.2). GA ships exactly these two; `viewer` moves to P2
    /// and is deliberately absent, so no policy cell exists for it and no code
    /// path can grant it anything by default.
    Role {
        /// Full control of the tenant.
        Admin => "admin",
        /// An ordinary member: own scans, own templates, read-only billing, no
        /// member management.
        Member => "member",
    }
}

status_enum! {
    /// One cell of the §4.2 role matrix.
    ///
    /// The §4.2 table has six columns and the cells distinguish scope, so
    /// "cancel" and "cancel own" and "CRUD" and "CRUD own" are separate
    /// actions. Enforcement is a single middleware layer with this declarative
    /// table, not per-handler checks (§4.2).
    Action {
        /// Submit a pipeline run. Both roles.
        ScanDispatch => "scan_dispatch",
        /// Cancel any run in the tenant. `admin` column "cancel".
        ScanCancelAny => "scan_cancel_any",
        /// Cancel a run the caller requested. `member` column "cancel own".
        ScanCancelOwn => "scan_cancel_own",
        /// Read findings.
        FindingRead => "finding_read",
        /// Change an observation state, including false-positive decisions
        /// (§15.1, §10.2).
        FindingTriage => "finding_triage",
        /// Request verification of an observation (§15.4).
        FindingVerify => "finding_verify",
        /// Create, read, update and delete any template in the tenant.
        /// `admin` column "CRUD".
        TemplateCrudAny => "template_crud_any",
        /// The same for templates the caller owns. `member` column "CRUD own".
        TemplateCrudOwn => "template_crud_own",
        /// Generate a report (§16).
        ReportGenerate => "report_generate",
        /// Configure report branding. `admin` only.
        ReportConfigureBranding => "report_configure_branding",
        /// Read usage, allowance and billing state.
        BillingRead => "billing_read",
        /// Change billing configuration. `admin` column "full".
        BillingManage => "billing_manage",
        /// Invite, remove and re-role tenant members. `admin` only.
        MemberManage => "member_manage",
    }
}

impl WorkUnitStatus {
    /// Whether this status is final.
    ///
    /// Both [`Self::Skipped`] and [`Self::Terminal`] are: §8.2 counts an
    /// unauthorized or skipped branch as terminal with a recorded reason, and
    /// [`pipeline_outcome`] must not wait for a skipped unit to move again.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Skipped | Self::Terminal)
    }
}

impl ObservationState {
    /// The events §15.1 admits from this state, in [`ObservationEvent::ALL`]
    /// order.
    ///
    /// Derived from [`observation_transition`] rather than written out a second
    /// time, so the two can never disagree about the edge set. Returned in
    /// [`Problem::IllegalTransition`](crate::problem::Problem::IllegalTransition)
    /// so a rejected state change tells the caller what it could have asked for
    /// instead — the "structured remediation hint" of §13.3.
    #[must_use]
    pub fn admitted_events(self) -> Vec<ObservationEvent> {
        ObservationEvent::ALL
            .iter()
            .copied()
            .filter(|event| observation_transition(self, *event).is_ok())
            .collect()
    }
}

status_enum! {
    /// What happened to an observation, as one edge label of the §15.1 diagram.
    ///
    /// There is deliberately no `Inconclusive` event: §15.1 calls inconclusive a
    /// verification outcome whose effect is to retain the *previous* state,
    /// which [`observation_transition`] cannot know from `from` alone — it is
    /// always [`ObservationState::Verifying`] by then. That restoration is
    /// [`apply_verification`]'s job.
    ObservationEvent {
        /// §15.1 "acknowledge this observation".
        Acknowledge => "acknowledge",
        /// §15.1 "work on this observation".
        StartFix => "start_fix",
        /// §15.1 "explicit false-positive decision".
        DecideFalsePositive => "decide_false_positive",
        /// §15.1 "request verification".
        RequestVerification => "request_verification",
        /// §15.1 "eligible check completed; issue not detected", i.e.
        /// [`VerificationOutcome::NotDetected`].
        VerifiedNotDetected => "verified_not_detected",
        /// §15.1 "issue still present", i.e.
        /// [`VerificationOutcome::StillPresent`].
        VerifiedStillPresent => "verified_still_present",
    }
}

/// An observation state change that §15.1 does not draw.
///
/// Maps to
/// [`Problem::IllegalTransition`](crate::problem::Problem::IllegalTransition)
/// and HTTP 409 at the API boundary (§A3.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("illegal observation transition: {event} is not admitted from {from}")]
pub struct IllegalTransition {
    /// The observation's current state.
    pub from: ObservationState,
    /// The event that was refused.
    pub event: ObservationEvent,
}

/// Applies one §15.1 edge to one observation.
///
/// The single encoding of the §15.1 diagram, and a total function: every
/// `(from, event)` pair either names an edge of the diagram or is an
/// [`IllegalTransition`]. There is **no wildcard arm** — every non-terminal
/// state matches exhaustively over [`ObservationEvent`], so adding an event
/// variant fails compilation until the diagram says where it leads from each
/// state. The three terminal states admit nothing by construction: §15.1 ends
/// there and a later scan produces a fresh observation rather than reviving
/// this one.
///
/// # Errors
///
/// [`IllegalTransition`] when §15.1 draws no edge for the pair. §A3.8 maps that
/// to HTTP 409 and
/// [`Problem::IllegalTransition`](crate::problem::Problem::IllegalTransition).
pub fn observation_transition(
    from: ObservationState,
    event: ObservationEvent,
) -> Result<ObservationState, IllegalTransition> {
    use ObservationEvent as E;
    use ObservationState as S;

    let illegal = Err(IllegalTransition { from, event });
    match from {
        S::New => match event {
            E::Acknowledge => Ok(S::Acknowledged),
            E::StartFix => Ok(S::FixPending),
            E::DecideFalsePositive => Ok(S::FalsePositive),
            E::RequestVerification => Ok(S::Verifying),
            E::VerifiedNotDetected | E::VerifiedStillPresent => illegal,
        },
        S::Acknowledged | S::FixPending => match event {
            E::RequestVerification => Ok(S::Verifying),
            E::Acknowledge
            | E::StartFix
            | E::DecideFalsePositive
            | E::VerifiedNotDetected
            | E::VerifiedStillPresent => illegal,
        },
        S::Verifying => match event {
            E::VerifiedNotDetected => Ok(S::Fixed),
            E::VerifiedStillPresent => Ok(S::New),
            E::Acknowledge | E::StartFix | E::DecideFalsePositive | E::RequestVerification => {
                illegal
            }
        },
        S::Fixed | S::FalsePositive | S::AcceptedRisk => illegal,
    }
}

/// Resolves a completed verification run into an observation state (§15.1).
///
/// `prior` is the state the observation held **before** it entered
/// [`ObservationState::Verifying`] — §A4 persists it as
/// `verification_runs.prior_observation_state` for exactly this call. It is used
/// only by [`VerificationOutcome::Inconclusive`], which retains it; the other
/// two outcomes are absolute.
///
/// Total function, so there is no error case: §15.1 draws an edge for all three
/// outcomes.
#[must_use]
pub fn apply_verification(
    prior: ObservationState,
    outcome: VerificationOutcome,
) -> ObservationState {
    match outcome {
        VerificationOutcome::NotDetected => ObservationState::Fixed,
        VerificationOutcome::StillPresent => ObservationState::New,
        VerificationOutcome::Inconclusive => prior,
    }
}

/// Whether a work unit with this outcome consumes one allowance unit (§8.2,
/// §17.3).
///
/// True only for [`OutcomeClass::Success`]. §17.3 is `[CONFIRMED]`: failed
/// scans consume no allowance, including unreachable targets, blocks and
/// platform errors, and infrastructure retries never add consumption for the
/// same logical scan.
#[must_use]
pub const fn outcome_consumes(class: OutcomeClass) -> bool {
    match class {
        OutcomeClass::Success => true,
        OutcomeClass::Target
        | OutcomeClass::Platform
        | OutcomeClass::Tool
        | OutcomeClass::Scope
        | OutcomeClass::Limit
        | OutcomeClass::Cancelled => false,
    }
}

/// The durable record of one work unit, as [`pipeline_outcome`] reads it.
///
/// §8.2 requires the overall outcome to be derived "from durable unit and
/// coverage records", so this is a projection of a `scan_work_units` row plus
/// the ingestion state of its artifacts — not a live view of a running job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitSummary {
    /// The unit's lifecycle status (`scan_work_units.status`).
    pub status: WorkUnitStatus,
    /// Why it ended (`scan_work_units.outcome_class`). `None` while the unit is
    /// still in flight; a terminal unit without one is not finalizable.
    pub outcome_class: Option<OutcomeClass>,
    /// Whether the artifact ingestion this unit requires has completed (§8.2
    /// "required artifact ingestion complete"). A unit that produces no
    /// artifact — a skip, for instance — sets this `true`.
    pub required_ingestion_complete: bool,
}

impl UnitSummary {
    /// Builds a summary for a unit that is still in flight.
    #[must_use]
    pub const fn in_flight(status: WorkUnitStatus) -> Self {
        Self {
            status,
            outcome_class: None,
            required_ingestion_complete: false,
        }
    }

    /// Builds a summary for a finished unit whose required ingestion is done.
    #[must_use]
    pub const fn terminal(status: WorkUnitStatus, outcome_class: OutcomeClass) -> Self {
        Self {
            status,
            outcome_class: Some(outcome_class),
            required_ingestion_complete: true,
        }
    }
}

/// Derives a run's final state from durable unit records (§8.2 completion
/// rule), or `None` while the run is not finalizable.
///
/// §8.2: "Completion requires closed discovery/cascade producers, no pending
/// fan-in inputs, every registered work unit terminal, and required artifact
/// ingestion complete." All four are checked before any state is derived, so a
/// caller can treat `None` as "leave the run in [`PipelineState::Running`]" and
/// a `Some` as a value safe to write once. Late hooks must not create work
/// after the run is finalized (§8.2); this function is the finalization test
/// that ordering is proven against.
///
/// With all four satisfied, the state follows from the units' outcome classes:
///
/// | Units | State |
/// |---|---|
/// | none registered | [`Completed`](PipelineState::Completed) — §8.2's "completed-with-no-output" |
/// | all [`Success`](OutcomeClass::Success) | [`Completed`](PipelineState::Completed) |
/// | some, not all, `Success` | [`PartiallyCompleted`](PipelineState::PartiallyCompleted) |
/// | no `Success`, any [`Cancelled`](OutcomeClass::Cancelled) | [`Cancelled`](PipelineState::Cancelled) |
/// | no `Success`, any [`Platform`](OutcomeClass::Platform) or [`Tool`](OutcomeClass::Tool) | [`PlatformFailed`](PipelineState::PlatformFailed) |
/// | no `Success`, any [`Target`](OutcomeClass::Target) | [`TargetFailed`](PipelineState::TargetFailed) |
/// | no `Success`, only [`Scope`](OutcomeClass::Scope) / [`Limit`](OutcomeClass::Limit) | [`Completed`](PipelineState::Completed) |
///
/// Three choices in that table are this function's reading of §8.2 rather than
/// text quoted from it, and are called out for review:
///
/// * **Cancelled outranks the failure classes.** A run the user stopped is
///   reported as stopped; §8.3 records the stop intent durably before anything
///   else, so a platform or target failure observed while winding down is a
///   consequence of the cancel, not the headline.
/// * **`Tool` maps to [`PlatformFailed`](PipelineState::PlatformFailed).** §8.2
///   separates `ToolFailed` from `PlatformFailed` for *units*, but gives the
///   pipeline no `ToolFailed` state. A scanner that cannot complete is our
///   component failing, not the tenant's target, and must not be reported as
///   [`TargetFailed`](PipelineState::TargetFailed).
/// * **Scope and limit skips alone complete the run.** §8.2 requires quota and
///   unsupported skips to be recorded "as coverage limits, not platform
///   errors", and a skipped branch is terminal with a recorded reason, so the
///   run finished — it just covered nothing.
///
/// [`TimedOut`](PipelineState::TimedOut) is never returned: §8.2 gives work
/// units one shared `Cancelled / TimedOut` outcome row, so the distinction does
/// not exist in the unit records this function reads. The run-level timeout sets
/// it directly.
#[must_use]
pub fn pipeline_outcome(
    units: &[UnitSummary],
    discovery_closed: bool,
    pending_fan_in: usize,
) -> Option<PipelineState> {
    // §8.2's four completion preconditions, in the order it states them.
    if !discovery_closed || pending_fan_in > 0 {
        return None;
    }
    if units.iter().any(|unit| !unit.status.is_terminal()) {
        return None;
    }
    if units.iter().any(|unit| !unit.required_ingestion_complete) {
        return None;
    }
    // A terminal unit with no recorded outcome class is an incomplete durable
    // record, not an outcome to guess at.
    if units.iter().any(|unit| unit.outcome_class.is_none()) {
        return None;
    }

    let successes = units
        .iter()
        .filter(|unit| unit.outcome_class.is_some_and(outcome_consumes))
        .count();
    if successes == units.len() {
        // Covers the empty slice: a graph that emitted no candidate is
        // explicitly completed with no output (§8.2).
        return Some(PipelineState::Completed);
    }
    if successes > 0 {
        return Some(PipelineState::PartiallyCompleted);
    }

    let has = |class: OutcomeClass| units.iter().any(|unit| unit.outcome_class == Some(class));
    if has(OutcomeClass::Cancelled) {
        Some(PipelineState::Cancelled)
    } else if has(OutcomeClass::Platform) || has(OutcomeClass::Tool) {
        Some(PipelineState::PlatformFailed)
    } else if has(OutcomeClass::Target) {
        Some(PipelineState::TargetFailed)
    } else {
        Some(PipelineState::Completed)
    }
}

/// The §4.2 role x action matrix, as one exhaustive `match` with no wildcard arm.
///
/// Adding a [`Role`] or an [`Action`] variant fails compilation here until the
/// policy decides every new cell — which is the whole reason §4.2 chose a Rust
/// match over an external policy engine, alongside keeping an availability
/// dependency out of the dispatch hot path.
///
/// One arm per cell, so the function reads as the table it implements:
///
/// | | scans | findings & fixes | templates | reports | billing | members |
/// |---|---|---|---|---|---|---|
/// | `admin` | dispatch, cancel | read, triage, verify | CRUD | generate, configure branding | full | manage |
/// | `member` | dispatch, cancel own | read, triage, verify | CRUD own | generate | read | — |
///
/// This decides the role cell only. Ownership ("own"), tenant membership,
/// suspension (§4.1) and the live authorization basis (§5.7) are separate
/// checks; `ScanCancelOwn` being allowed for a member means the member may
/// cancel *a run of theirs*, and the handler still has to confirm it is theirs.
#[must_use]
pub const fn allowed(role: Role, action: Action) -> bool {
    match (role, action) {
        // Scans: both roles dispatch; only admin cancels another member's run.
        (Role::Admin, Action::ScanDispatch) => true,
        (Role::Member, Action::ScanDispatch) => true,
        (Role::Admin, Action::ScanCancelAny) => true,
        (Role::Member, Action::ScanCancelAny) => false,
        (Role::Admin, Action::ScanCancelOwn) => true,
        (Role::Member, Action::ScanCancelOwn) => true,

        // Findings and fixes: identical for both roles in §4.2.
        (Role::Admin, Action::FindingRead) => true,
        (Role::Member, Action::FindingRead) => true,
        (Role::Admin, Action::FindingTriage) => true,
        (Role::Member, Action::FindingTriage) => true,
        (Role::Admin, Action::FindingVerify) => true,
        (Role::Member, Action::FindingVerify) => true,

        // Templates: admin over all, member over its own.
        (Role::Admin, Action::TemplateCrudAny) => true,
        (Role::Member, Action::TemplateCrudAny) => false,
        (Role::Admin, Action::TemplateCrudOwn) => true,
        (Role::Member, Action::TemplateCrudOwn) => true,

        // Reports: both generate; only admin configures branding.
        (Role::Admin, Action::ReportGenerate) => true,
        (Role::Member, Action::ReportGenerate) => true,
        (Role::Admin, Action::ReportConfigureBranding) => true,
        (Role::Member, Action::ReportConfigureBranding) => false,

        // Billing: admin full, member read.
        (Role::Admin, Action::BillingRead) => true,
        (Role::Member, Action::BillingRead) => true,
        (Role::Admin, Action::BillingManage) => true,
        (Role::Member, Action::BillingManage) => false,

        // Members: admin only; §4.2 leaves the member cell empty.
        (Role::Admin, Action::MemberManage) => true,
        (Role::Member, Action::MemberManage) => false,
    }
}
