//! Durable event shapes (architecture §A3.3; TDD §9.2, §9.3).
//!
//! Progress events are a **view of durable state** (§2.4), not a side channel.
//! Each variant here is the `data:` payload of one SSE frame whose `event:` name
//! is the serde tag, and the `seq` that becomes the SSE `id:` is allocated by
//! `vf-db::outbox` inside the same transaction that commits the state change the
//! event describes (§A3.3). Nothing in this module reads a clock or a socket;
//! the carried timestamps are supplied by the committing transaction through the
//! [`Clock`](crate::ports::Clock) port.
//!
//! Two streams with two independent cursors (§9.2):
//!
//! * [`PipelineEvent`] on `GET /v1/pipeline-runs/{id}/events`, keyed
//!   `(pipeline_run_id, seq)`.
//! * [`TenantEvent`] on `GET /v1/events`, keyed `(tenant_id, seq)`. Report and
//!   allowance events belong here precisely so an unrelated report is never
//!   "assigned to a fictitious scan" (§9.2).
//!
//! The enums are `#[serde(tag = "event")]` with the exact §9.2 names, so the
//! stored `event jsonb` column is self-describing and a replay does not depend
//! on the column it was selected from. §9.2's illustrative `"Running"` and
//! `"Succeeded"` state spellings are pre-v2.3 prose; the binding values are the
//! `snake_case` forms of [`crate::state`], which is what these typed fields
//! serialize to.
//!
//! Two rules from §9.2 and §9.3 that this module exists to make unrepresentable:
//!
//! * **Report download URLs are never placed in events.** [`TenantEvent`]
//!   carries a [`ReportId`] and a [`ReportState`] and no URL field, so a replay
//!   buffer cannot leak a capability.
//! * **Sampling is declared, not silent.** [`PipelineEvent::LogLine`] carries
//!   `sampled` and `dropped` so a consumer can tell a quiet node from a
//!   throttled one (§9.3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{
    BillingPeriodId, FindingId, FpDecisionId, NodeId, PipelineRunId, ReportId, ScanFingerprint,
    VerificationRunId, WorkUnitId,
};
use crate::state::{
    ObservationState, OutcomeClass, PipelineState, ReportState, VerificationOutcome, WorkUnitStatus,
};

/// Which of a scanner's two output streams a log line came from (§9.2
/// `log.line`).
///
/// A closed two-variant enum rather than free text: the adapter reads exactly
/// the child process's `stdout` and `stderr` (§7.4), so an unknown value here
/// would mean the pipe plumbing is wrong and should be a hard parse error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    /// The tool's standard output.
    Stdout,
    /// The tool's standard error.
    Stderr,
}

/// A finding's severity as the scanner reported it (§6.3 `findings.severity`).
///
/// Deliberately **not** a closed enum. §6.3 declares the column
/// `severity text NOT NULL` with no `CHECK` list — unlike every other status
/// column of §A4, which is why this type is not in [`crate::state`]. Each
/// scanner names its own scale (§9.2 shows `"high"`; the nuclei node config of
/// §7.3 selects `medium|high|critical`), and rejecting an unrecognized value
/// would drop a real observation during ingest.
///
/// The value is stored unchanged. Normalizing or ranking severities across
/// tools is report-time work (§16) against `vf.remediation_content`, not an
/// ingest-time transformation that would lose what the tool actually said.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Severity(String);

impl Severity {
    /// Wraps a scanner-reported severity, storing it unchanged.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The wrapped value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Unwraps to the inner `String`.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl core::fmt::Display for Severity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::str::FromStr for Severity {
    /// Infallible, for the reason given on [`Severity`]: the column accepts any
    /// text and an unknown scale is not an error.
    type Err = core::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_owned()))
    }
}

/// One event on a single pipeline run's stream (§A3.3, §9.2).
///
/// Keyed `(pipeline_run_id, seq)` in `pipeline_events`; `seq` is the SSE `id:`.
/// `Last-Event-ID` replays from that table for events newer than 15 minutes,
/// and an older cursor receives one `resync` event carrying a full `RunSnapshot`
/// instead — `resync` is a `vf-api` stream frame built from the `GET /v1/…`
/// shape (task A2), not a variant here, because it reports current state rather
/// than a committed transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum PipelineEvent {
    /// The run as a whole changed state (§8.2).
    #[serde(rename = "pipeline.state")]
    PipelineState {
        /// The run this stream belongs to.
        pipeline_run_id: PipelineRunId,
        /// The committed new state.
        state: PipelineState,
        /// When the committing transaction recorded the change.
        at: DateTime<Utc>,
    },

    /// One work unit changed state (§6.3 `scan_work_units.status`).
    #[serde(rename = "scan.state")]
    ScanState {
        /// The run this stream belongs to.
        pipeline_run_id: PipelineRunId,
        /// The unit that moved. It exists from submission onwards (§8.1), which
        /// is why it is the stable identifier on this event.
        work_unit_id: WorkUnitId,
        /// The secureCodeBox attempt, once there is one. `None` before the Scan
        /// object is created and for a unit that never reaches it — a scope or
        /// allowance skip, for instance (§17.3).
        scan_fingerprint: Option<ScanFingerprint>,
        /// The committed new status.
        state: WorkUnitStatus,
        /// Why the unit ended. `Some` only once `state` is terminal.
        outcome_class: Option<OutcomeClass>,
    },

    /// Per-node progress counters (§9.2).
    ///
    /// §9.3 coalesces these to at most four updates per second per node, so a
    /// consumer must treat each as a snapshot of the counters and never add
    /// successive payloads together.
    #[serde(rename = "node.counter")]
    NodeCounter {
        /// The run this stream belongs to.
        pipeline_run_id: PipelineRunId,
        /// The graph node these counters describe.
        node_id: NodeId,
        /// Work units registered for the node so far. Discovery may still raise
        /// this while the node's producers are open (§8.2).
        registered: u32,
        /// Units that reached a terminal outcome.
        completed: u32,
        /// Units skipped with a recorded reason (§8.2).
        skipped: u32,
    },

    /// One line of scanner output (§9.2).
    ///
    /// Not a state change, so it carries no `seq`-bearing durable row of its
    /// own in the sense the other variants do; §9.3's sampling applies here and
    /// only here.
    #[serde(rename = "log.line")]
    LogLine {
        /// The attempt that produced the line.
        scan_fingerprint: ScanFingerprint,
        /// The graph node the attempt belongs to.
        node_id: NodeId,
        /// Which output stream the line came from.
        stream: LogStream,
        /// The line, already treated as attacker-influenced text by whatever
        /// renders it (§16.7).
        line: String,
        /// When the line was observed.
        ts: DateTime<Utc>,
        /// `true` when server-side sampling is active for this node, i.e. the
        /// node exceeded 100 lines/second (§9.3).
        sampled: bool,
        /// How many lines were dropped before this one while sampling. `0` when
        /// `sampled` is `false`.
        dropped: u32,
    },

    /// A new observation was ingested (§15.1).
    #[serde(rename = "finding.new")]
    FindingNew {
        /// The new observation. Never reused to merge observations from later
        /// scans (§6.2).
        finding_id: FindingId,
        /// The attempt it was parsed from.
        scan_fingerprint: ScanFingerprint,
        /// Its state as ingested. Normally [`ObservationState::New`]; a
        /// false-positive decision matched at ingest makes it
        /// [`ObservationState::FalsePositive`] (§10.2).
        state: ObservationState,
        /// Severity as the scanner reported it.
        severity: Severity,
    },

    /// An observation's state changed (§15.1).
    #[serde(rename = "finding.state")]
    FindingState {
        /// The observation that moved.
        finding_id: FindingId,
        /// The committed new state.
        state: ObservationState,
        /// The false-positive decision responsible, when
        /// `state == ObservationState::FalsePositive` (§10.2). `None` for every
        /// other transition.
        decision_id: Option<FpDecisionId>,
    },

    /// A verification run finished (§15.4).
    ///
    /// The resulting observation state is **not** on this event: a
    /// [`VerificationOutcome::Inconclusive`] result restores the state the
    /// observation held before it began verifying, which only
    /// [`apply_verification`](crate::state::apply_verification) and the stored
    /// `verification_runs.prior_observation_state` know. Consumers that need the
    /// new state read the `finding.state` event committed alongside this one.
    #[serde(rename = "verification.completed")]
    VerificationCompleted {
        /// The observation that was verified.
        finding_id: FindingId,
        /// The verification run that finished.
        verification_run_id: VerificationRunId,
        /// What the eligible check concluded.
        outcome: VerificationOutcome,
    },
}

/// One event on the tenant-wide stream (§A3.3, §9.2).
///
/// Keyed `(tenant_id, seq)` in `vf.tenant_events`, with a cursor separate from
/// any pipeline stream. These are the events that outlive a single run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum TenantEvent {
    /// The billing period's allowance balance moved (§17.3).
    ///
    /// Emitted on reserve, settle and release, so `consumed + reserved +
    /// remaining` is the period's entitlement at the instant the transaction
    /// committed. §17.3: only a successful unit is ever `consumed`.
    #[serde(rename = "allowance.updated")]
    AllowanceUpdated {
        /// The billing period whose balance this is.
        period_id: BillingPeriodId,
        /// Units settled as consumed in the period.
        consumed: u64,
        /// Units currently held by live reservations.
        reserved: u64,
        /// Units still available to reserve.
        remaining: u64,
    },

    /// A report changed state (§16.8).
    ///
    /// Carries no download URL by design (§9.2): the artifact is fetched through
    /// the authorized report endpoint, so a replayed event is not a capability.
    #[serde(rename = "report.state")]
    ReportState {
        /// The report that moved.
        report_id: ReportId,
        /// The committed new state.
        state: ReportState,
    },
}
