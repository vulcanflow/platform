//! Identity newtypes (architecture §A3.1, TDD §6.2).
//!
//! §6.2 fixes five distinct identities and the v2.3 decision that each one is a
//! distinct Rust newtype "rather than a bare `Uuid`/`String`, so passing a
//! pipeline-run ID where a scan fingerprint is expected is a compile error".
//! This module is that decision: nothing here is a type alias, and no newtype
//! converts into another.
//!
//! Two families:
//!
//! * UUID identities — a newtype over [`Uuid`]. The value is a **v7** UUID, so
//!   identities are time-ordered and index-friendly; generation belongs to the
//!   [`IdGen`](crate::ports::IdGen) port, never to a `Default` impl, so tests
//!   are deterministic. `Display`, `FromStr` and serde are the plain UUID
//!   string in all cases (§A3.1).
//! * String identities — [`ScanFingerprint`], [`CandidateKey`] and
//!   [`RequestKey`]. These carry values the platform does not mint from a
//!   CSPRNG: the secureCodeBox Scan identifier, a deterministic per-candidate
//!   key, and a client-supplied idempotency key.
//!
//! [`ScanFingerprint`] in particular is **stored unchanged and never derived**
//! (§6.2, §10.2): it is the identifier secureCodeBox supplies, not a content
//! hash, so the constructor normalizes nothing.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// A UUID identity string that is not a valid UUID.
///
/// Carries the newtype's name so a parse failure in a request body or a
/// database row names the column it came from.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {type_name}: {value:?} is not a UUID: {source}")]
pub struct IdParseError {
    /// The newtype that rejected the value, for example `"PipelineRunId"`.
    pub type_name: &'static str,
    /// The rejected input, kept for the error message only.
    pub value: String,
    /// The underlying `uuid` parse failure.
    #[source]
    pub source: uuid::Error,
}

/// Defines one newtype over [`Uuid`] with `Display`, `FromStr` and serde as the
/// plain UUID string (§A3.1).
///
/// There is deliberately no `Default`, no `new()` that calls a random source and
/// no `From<OtherId>`: identities come from the [`IdGen`](crate::ports::IdGen)
/// port or from a parsed string, and the whole point of §6.2 is that they do not
/// convert into one another.
macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// The newtype's name, used in [`IdParseError`] and in log fields.
            pub const TYPE_NAME: &'static str = stringify!($name);

            /// Wraps an already-generated UUID.
            ///
            /// The caller is responsible for the value being a v7 UUID; the
            /// [`IdGen`](crate::ports::IdGen) port is the only sanctioned
            /// source in production code.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// The wrapped UUID, for the database and wire boundaries.
            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// Unwraps to the inner UUID.
            #[must_use]
            pub const fn into_uuid(self) -> Uuid {
                self.0
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self).map_err(|source| IdParseError {
                    type_name: Self::TYPE_NAME,
                    value: s.to_owned(),
                    source,
                })
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.0.serialize(serializer)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Uuid::deserialize(deserializer).map(Self)
            }
        }
    };
}

uuid_id! {
    /// A tenant. The isolation boundary of §3: every tenant-owned row carries
    /// one, and it is the prefix of the artifact layout of §6.5.
    TenantId
}
uuid_id! {
    /// A user, as carried by the `sub` claim of §A3.8.
    UserId
}
uuid_id! {
    /// A registered scan target — one canonical host (§6.3 `targets`).
    TargetId
}
uuid_id! {
    /// A project. A label grouping targets, with no authorization meaning
    /// (§3.1).
    ProjectId
}
uuid_id! {
    /// One recorded authorization basis for a target (§5.6). Work units cite
    /// the basis they were admitted under.
    AuthorizationBasisId
}
uuid_id! {
    /// One submitted graph, represented by a ScanFlow. **Not a metered scan**
    /// (§6.2).
    PipelineRunId
}
uuid_id! {
    /// One logical scan type against one target and relevant port/endpoint
    /// (§6.2). This is the unit of reservation and of retry deduplication, so
    /// it exists before any secureCodeBox object does (§17.3).
    WorkUnitId
}
uuid_id! {
    /// One secureCodeBox Scan attempt row (§6.3 `scans`). Distinct from
    /// [`ScanFingerprint`], which is the identifier secureCodeBox itself
    /// supplies for that attempt.
    ScanId
}
uuid_id! {
    /// One observation from one particular scan attempt. **Never reused to
    /// merge observations from later scans** (§6.2).
    FindingId
}
uuid_id! {
    /// One explicit false-positive decision (§6.3
    /// `false_positive_decisions`).
    FpDecisionId
}
uuid_id! {
    /// One verification run over a single observation (§15.4).
    VerificationRunId
}
uuid_id! {
    /// One generated report (§16.8).
    ReportId
}
uuid_id! {
    /// One recurring scan schedule (§11.1).
    ScheduleId
}
uuid_id! {
    /// One transactional-outbox row (§A3.3 `vf.outbox`).
    OutboxId
}
uuid_id! {
    /// One billing period's usage row (§17.3 `billing_period_usage`).
    ///
    /// Addition to the §A3.1 list: §A3.3's `allowance.updated` event and
    /// §17.3's `scan_usage_reservations.period_id` both reference this row, and
    /// typing them as a bare `Uuid` would be the single exception to §6.2 in
    /// the crate.
    BillingPeriodId
}

/// Defines one newtype over [`String`] with `Display`, `FromStr` and serde as
/// the plain string.
///
/// The value is stored exactly as given. None of these three identities is
/// normalized, lowercased or hashed: [`ScanFingerprint`] is supplied by
/// secureCodeBox (§6.2), [`CandidateKey`] is derived by the dispatcher and the
/// operator from `(run, node, operation scope)` and must compare byte-for-byte
/// against the `UNIQUE (pipeline_run_id, candidate_key)` of §6.3, and
/// [`RequestKey`] is the client's `Idempotency-Key` header, which §8.1 binds to
/// a request hash rather than interpreting.
macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// The newtype's name, for log fields.
            pub const TYPE_NAME: &'static str = stringify!($name);

            /// Wraps a value, storing it unchanged.
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

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            /// Infallible: the value is stored unchanged, so there is nothing
            /// to reject here. Length and charset limits belong to the request
            /// validation layer, not to the identity type.
            type Err = core::convert::Infallible;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(s.to_owned()))
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer).map(Self)
            }
        }
    };
}

string_id! {
    /// The identifier of the actual secureCodeBox Scan attempt, stored
    /// unchanged and never derived (§6.2, §10.2).
    ///
    /// It does not exist before the Scan object is created, which is why §17.3
    /// reserves against [`WorkUnitId`] and binds the fingerprint afterwards.
    /// Possessing a fingerprint is not a capability: §A3.8's
    /// `GET /v1/scans/{scan_fingerprint}` is tenant-authorized like every other
    /// route.
    ScanFingerprint
}
string_id! {
    /// A deterministic key per `(run, node, operation scope)` that deduplicates
    /// retries of the same graph candidate (§6.3 `scan_work_units`).
    CandidateKey
}
string_id! {
    /// The client's idempotency key for a submission, bound to a request hash
    /// (§8.1). Replaying it with different content is
    /// [`Problem::IdempotencyConflict`](crate::problem::Problem::IdempotencyConflict),
    /// not success (§17.3).
    RequestKey
}
string_id! {
    /// One node of a submitted graph, as the graph author labelled it (`"n2"`
    /// in the §9.2 examples).
    ///
    /// Addition to the §A3.1 list. §A3.3's `node.counter` and `log.line` events
    /// both carry a node identifier and `vf-core` may not depend on `vf-graph`
    /// (§A1.4), so the newtype lives here rather than in the graph crate. It is
    /// a client-supplied label, not a platform-minted UUID, which is why it is
    /// a string identity stored unchanged: the graph validator (`vf-graph`,
    /// task C3) owns the charset and uniqueness rules.
    NodeId
}
