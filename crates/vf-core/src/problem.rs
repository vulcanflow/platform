//! The RFC 9457 Problem Details catalogue (architecture §A3.8; TDD §13.3).
//!
//! One Rust enum, one variant per stable `type` URI, each mapping to exactly one
//! URI and one HTTP status (§13.3). `vf-api` adds the `IntoResponse` impl and
//! the OpenAPI documentation (task A1); `vf-core` may not depend on an HTTP
//! crate (§A1.4), which is why [`Problem::status`] returns a bare [`u16`] rather
//! than a `StatusCode`.
//!
//! §13.3's defining requirement is that a refusal is **itemized and actionable**
//! rather than generic: "it tells the user which door is open to them rather
//! than issuing a generic denial". That is the `vf` extension member — every
//! variant carries a typed extension struct, so the hint cannot be forgotten at
//! a call site the way an optional free-form map would let it be.
//!
//! Serialization is RFC 9457's flat object with the six members the acceptance
//! criteria name:
//!
//! ```json
//! {
//!   "type": "https://vulcanflow.io/errors/authorization-required",
//!   "title": "Target is not authorized for scanning",
//!   "status": 403,
//!   "detail": "example.com has no verified authorization basis for this account.",
//!   "instance": "/v1/pipeline-runs",
//!   "vf": { "track_a_available": true, "track_b_available": false, "…": "…" }
//! }
//! ```
//!
//! `type` and `title` are fixed per variant and never carry caller data;
//! `detail` is the one human-readable, per-occurrence member. Two rules from
//! §13.3 and §19.2 that this module enforces by construction:
//!
//! * **Internal error details are never serialized to the client.**
//!   [`Problem::Internal`] has no `detail` field at all — its detail is a fixed
//!   string and its extension carries only a `trace_id` to correlate with the
//!   log that does hold the database error or panic.
//! * **`title` is not caller-controlled**, so a Problem body cannot be used to
//!   place attacker-chosen text in a position a UI might trust.

use serde::Serialize;
use serde::ser::{SerializeStruct, Serializer};

use crate::ids::TargetId;
use crate::state::{IllegalTransition, ObservationEvent, ObservationState};

/// Expands to the full `type` URI for one slug, at compile time.
///
/// The namespace literal is written here and nowhere else, so
/// [`ERROR_TYPE_BASE`] and every [`Problem::type_uri`] arm cannot drift apart.
macro_rules! error_type {
    ($slug:literal) => {
        concat!("https://vulcanflow.io/errors/", $slug)
    };
}

/// The namespace every `type` URI in this catalogue lives under (§A3.8).
///
/// The URIs are **stable identifiers, not fetchable documentation**: clients
/// match on them, so a value here may never change once released.
pub const ERROR_TYPE_BASE: &str = error_type!("");

/// The fixed `detail` of [`Problem::Internal`].
///
/// §13.3: database errors and panics "are logged with a trace ID and never
/// serialized to the client".
const INTERNAL_DETAIL: &str = concat!(
    "An internal error occurred. The failure has been logged ",
    "with the trace identifier in this response.",
);

/// Which authorization door is open to the caller (§13.3, §5.1).
///
/// The point of §13.3's track-specific refusal: Track A records control
/// evidence over the host, Track B records a permission attestation behind the
/// identity and payment gates, and a caller needs to know which one they can
/// actually complete. Both may be unavailable at once — an unverifiable host for
/// a tenant with incomplete KYC — which is a legitimate state, not an error in
/// the hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrackHints {
    /// Whether the caller may verify control of the host now (DNS-TXT or
    /// HTTP-file challenge, §5.2).
    pub track_a_available: bool,
    /// Whether the caller may record a permission attestation now (§5.4).
    pub track_b_available: bool,
    /// Why Track B is closed, as a stable machine-readable reason such as
    /// `"kyc_incomplete"`. `None` when `track_b_available` is `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_b_reason: Option<String>,
    /// The itemized steps the caller can take, in the order to take them. §13.3
    /// requires at least one whenever either track is available.
    pub next_actions: Vec<NextAction>,
}

/// One actionable step offered by a refusal (§13.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextAction {
    /// A stable machine-readable step name such as `"verify_ownership"` or
    /// `"complete_verification"`. The UI maps it to its own wording.
    pub action: String,
    /// Where to take the step — an API route or an application path. A URI
    /// reference on this deployment, never an absolute off-site URL.
    pub href: String,
}

/// The target-allowance position that produced the refusal (§13.3, §17.1).
///
/// §13.3 singles this out as the refusal "a growing account will hit" most
/// often, and requires the current count, the package limit and an upgrade
/// link, so the numbers are not optional.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AllowanceHints {
    /// Hostnames currently counted against the allowance (§17.2: separately
    /// scanned hostnames, canonical duplicates reusing one entry).
    pub current: u64,
    /// The subscribed package's ceiling.
    pub limit: u64,
    /// The package in force, for the message the UI renders.
    pub package: String,
    /// Where to upgrade. A URI reference on this deployment.
    pub upgrade_href: String,
}

/// Why a graph was refused (§A3.8 `POST /v1/graphs/validate`).
///
/// The same shape the validator returns on the success path as
/// `{valid, errors: [ValidationError]}`, so a refusal at submission and a
/// pre-flight validation itemize a graph identically. `vf-graph` (task C3) owns
/// the `code` vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphError {
    /// A stable machine-readable rule name such as `"cycle"` or
    /// `"unknown_node_type"`.
    pub code: String,
    /// The offending node, when the rule is about one. `None` for a
    /// whole-graph rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// A human-readable explanation of this one violation.
    pub message: String,
}

/// A refused observation state change, as the caller can act on it (§15.1,
/// §A3.2).
///
/// Carries the admitted alternatives rather than only the rejection, which is
/// §13.3's "structured remediation hint" applied to the §A3.2 edge table: a
/// client that asked for the wrong edge is told which edges exist from where it
/// actually is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransitionHints {
    /// The observation's current state.
    pub from: ObservationState,
    /// The refused event.
    pub attempted: ObservationEvent,
    /// Every event the §A3.2 edge table admits from `from`. Empty for a
    /// terminal state, which tells the caller to stop rather than to retry
    /// differently, and for `verifying`, where only the running verification
    /// can move the observation on.
    pub admitted: Vec<ObservationEvent>,
}

/// Why a challenge verification failed (§A3.8, §5.2).
///
/// §5.2 fixes the failure set: HTTP downgrade, cross-host redirect, a
/// private or link-local destination, an oversized body, a timeout, and a token
/// that is simply absent or wrong. The reason is a stable machine-readable
/// string from that vocabulary, owned by `vf-authz` (task G3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChallengeHints {
    /// The stable reason code, for example `"token_not_found"`,
    /// `"cross_host_redirect"` or `"private_destination"`.
    pub reason: String,
    /// Whether the caller can usefully retry the same challenge — `true` for a
    /// propagation timeout, `false` for a cross-host redirect. §5.2 consumes a
    /// challenge only on success, so a retryable failure leaves it usable.
    pub retryable: bool,
}

/// What was not found (§A3.8).
///
/// Tenant isolation means a resource belonging to another tenant is reported as
/// absent, never as forbidden (§3): a 404 here must not let a caller probe for
/// the existence of another account's rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotFoundHints {
    /// The resource kind, for example `"target"` or `"pipeline_run"`.
    pub resource: String,
}

/// Why a permitted caller was refused (§4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ForbiddenHints {
    /// The policy action the route required, as
    /// [`Action::as_str`](crate::state::Action::as_str) spells it.
    pub required_action: String,
}

/// The rate-limit position that produced the refusal (§13.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RateLimitHints {
    /// The bucket that refused — `"dispatch"`, `"verification"`,
    /// `"report_generation"` or `"read"` (§13.4). Each is package-scaled and
    /// independent, so naming it tells the caller what to slow down.
    pub bucket: String,
    /// Seconds to wait, mirroring the `Retry-After` header `vf-api` sets.
    pub retry_after_seconds: u64,
}

/// The platform's error catalogue (§13.3, §A3.8).
///
/// Every variant maps to exactly one stable `type` URI and one HTTP status; see
/// [`Problem::type_uri`], [`Problem::title`] and [`Problem::status`], each of
/// which is a single exhaustive `match` with no wildcard arm, so a new variant
/// fails compilation until its URI, title and status are all decided.
///
/// `instance` is the request path the failure occurred on (RFC 9457's
/// occurrence identifier). It is `Option` because the catalogue is also used
/// outside a request — the outbox worker and the operator reconcile loop build
/// Problems for durable failure records, where no request path exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The target has no live authorization basis, so no scanner may start
    /// (§5.1). The refusal is track-specific (§13.3).
    AuthorizationRequired {
        /// Which host, and what is missing.
        detail: String,
        /// The request path.
        instance: Option<String>,
        /// The target that needs authorizing, when the caller named one that
        /// exists.
        target_id: Option<TargetId>,
        /// Which door is open.
        hints: TrackHints,
    },

    /// Registering or admitting this hostname would exceed the package's target
    /// allowance (§17.2).
    TargetAllowanceExceeded {
        /// Which hostname was refused and against which ceiling.
        detail: String,
        /// The request path.
        instance: Option<String>,
        /// The current count, the limit and the upgrade link.
        hints: AllowanceHints,
    },

    /// The submitted graph is not valid, so no run row is written at all
    /// (§A3.8).
    GraphInvalid {
        /// A summary; the itemized rules are in `errors`.
        detail: String,
        /// The request path.
        instance: Option<String>,
        /// Every violation found, not just the first.
        errors: Vec<GraphError>,
    },

    /// The `Idempotency-Key` was replayed with a different request body (§8.1).
    ///
    /// The same key with the *same* hash is a success returning the original
    /// run, not this Problem (§A3.8).
    IdempotencyConflict {
        /// That the key is in use for different content.
        detail: String,
        /// The request path.
        instance: Option<String>,
        /// The key that conflicted. The value the client already holds, so
        /// echoing it discloses nothing new.
        request_key: String,
    },

    /// The requested observation state change is not an edge of §15.1.
    IllegalTransition {
        /// Which change was refused.
        detail: String,
        /// The request path.
        instance: Option<String>,
        /// The current state, the refused event and the admitted alternatives.
        hints: TransitionHints,
    },

    /// A challenge verification did not succeed (§5.2).
    ChallengeFailed {
        /// Which check failed.
        detail: String,
        /// The request path.
        instance: Option<String>,
        /// The stable reason and whether a retry is worthwhile.
        hints: ChallengeHints,
    },

    /// No such resource for this tenant (§3).
    NotFound {
        /// What was looked for.
        detail: String,
        /// The request path.
        instance: Option<String>,
        /// The resource kind.
        hints: NotFoundHints,
    },

    /// The caller is authenticated and in the tenant, but their role does not
    /// permit the action (§4.2).
    Forbidden {
        /// What was required.
        detail: String,
        /// The request path.
        instance: Option<String>,
        /// The policy action the route required.
        hints: ForbiddenHints,
    },

    /// The tenant is suspended, so write paths are refused (§4.1).
    ///
    /// **Addition to the ten URIs named in the C1 scope.** §4.1 refuses writes
    /// for a suspended tenant while still serving reads, and the T7 acceptance
    /// criteria require that refusal to carry the distinct
    /// `403 tenant-suspended` type rather than the generic `forbidden`. Folding
    /// it into [`Self::Forbidden`] would make the two indistinguishable to a
    /// client, so it is a variant here and is called out for review.
    TenantSuspended {
        /// That the account is suspended and reads remain available.
        detail: String,
        /// The request path.
        instance: Option<String>,
        /// The stable suspension reason (`vf.tenants.suspended_reason`), when
        /// one is recorded.
        reason: Option<String>,
    },

    /// A token bucket refused the request (§13.4).
    RateLimited {
        /// Which bucket and for how long.
        detail: String,
        /// The request path.
        instance: Option<String>,
        /// The bucket and the retry delay.
        hints: RateLimitHints,
    },

    /// An unexpected failure on our side (§13.3).
    ///
    /// Carries no caller-facing detail by construction: its `detail` is one
    /// fixed sentence and the extension holds only a trace identifier.
    Internal {
        /// The request path.
        instance: Option<String>,
        /// The trace identifier the real error was logged under.
        trace_id: String,
    },
}

impl Problem {
    /// The stable `type` URI of this variant (§A3.8).
    ///
    /// Always [`ERROR_TYPE_BASE`] followed by the variant's slug, built from the
    /// same literal. These strings are part of the public API contract: clients
    /// match on them, so changing one is a breaking change.
    #[must_use]
    pub const fn type_uri(&self) -> &'static str {
        match self {
            Self::AuthorizationRequired { .. } => error_type!("authorization-required"),
            Self::TargetAllowanceExceeded { .. } => error_type!("target-allowance-exceeded"),
            Self::GraphInvalid { .. } => error_type!("graph-invalid"),
            Self::IdempotencyConflict { .. } => error_type!("idempotency-conflict"),
            Self::IllegalTransition { .. } => error_type!("illegal-transition"),
            Self::ChallengeFailed { .. } => error_type!("challenge-failed"),
            Self::NotFound { .. } => error_type!("not-found"),
            Self::Forbidden { .. } => error_type!("forbidden"),
            Self::TenantSuspended { .. } => error_type!("tenant-suspended"),
            Self::RateLimited { .. } => error_type!("rate-limited"),
            Self::Internal { .. } => error_type!("internal"),
        }
    }

    /// The short, human-readable summary of the *type* (RFC 9457 §3.1.3).
    ///
    /// Fixed per variant and never derived from caller input, so a Problem body
    /// cannot carry attacker-chosen text in a field a UI treats as a heading.
    /// Per-occurrence wording belongs in [`Self::detail`].
    #[must_use]
    pub const fn title(&self) -> &'static str {
        match self {
            Self::AuthorizationRequired { .. } => "Target is not authorized for scanning",
            Self::TargetAllowanceExceeded { .. } => "Target allowance exceeded",
            Self::GraphInvalid { .. } => "Graph is not valid",
            Self::IdempotencyConflict { .. } => "Idempotency key reused with different content",
            Self::IllegalTransition { .. } => "Observation state transition is not allowed",
            Self::ChallengeFailed { .. } => "Authorization challenge failed",
            Self::NotFound { .. } => "Resource not found",
            Self::Forbidden { .. } => "Role does not permit this action",
            Self::TenantSuspended { .. } => "Account is suspended",
            Self::RateLimited { .. } => "Rate limit exceeded",
            Self::Internal { .. } => "Internal error",
        }
    }

    /// The HTTP status code for this variant.
    ///
    /// A [`u16`] rather than a `StatusCode` because `vf-core` depends on no HTTP
    /// crate (§A1.4); `vf-api` converts (task A1).
    ///
    /// Four of these are fixed verbatim by §A3.8 and §13.x: `403` for
    /// authorization-required, `422` for graph-invalid, `409` for
    /// idempotency-conflict and illegal-transition, `429` for rate-limited.
    /// Three are this module's reading of the contract and are called out for
    /// review:
    ///
    /// * **`target-allowance-exceeded` is `403`**, not `402`. §17 makes it a
    ///   policy ceiling on a live subscription rather than a payment-protocol
    ///   failure, and it sits in the same family as authorization-required: the
    ///   request is understood and refused, with an upgrade path offered.
    /// * **`challenge-failed` is `422`.** The request was well-formed and
    ///   accepted for processing; the *evidence* did not hold. §A3.8 returns it
    ///   in place of the `200 {basis_id, state}` body, so it reports a failed
    ///   verification, not a malformed call, which would be `400`.
    /// * **`tenant-suspended` is `403`**, as the T7 acceptance criteria state.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::AuthorizationRequired { .. }
            | Self::TargetAllowanceExceeded { .. }
            | Self::Forbidden { .. }
            | Self::TenantSuspended { .. } => 403,
            Self::GraphInvalid { .. } | Self::ChallengeFailed { .. } => 422,
            Self::IdempotencyConflict { .. } | Self::IllegalTransition { .. } => 409,
            Self::NotFound { .. } => 404,
            Self::RateLimited { .. } => 429,
            Self::Internal { .. } => 500,
        }
    }

    /// The per-occurrence, human-readable explanation (RFC 9457 §3.1.4).
    ///
    /// [`Self::Internal`] returns one fixed sentence: §13.3 forbids
    /// serializing a database error or a panic message to the client.
    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            Self::AuthorizationRequired { detail, .. }
            | Self::TargetAllowanceExceeded { detail, .. }
            | Self::GraphInvalid { detail, .. }
            | Self::IdempotencyConflict { detail, .. }
            | Self::IllegalTransition { detail, .. }
            | Self::ChallengeFailed { detail, .. }
            | Self::NotFound { detail, .. }
            | Self::Forbidden { detail, .. }
            | Self::TenantSuspended { detail, .. }
            | Self::RateLimited { detail, .. } => detail,
            Self::Internal { .. } => INTERNAL_DETAIL,
        }
    }

    /// The request path the failure occurred on, or `None` outside a request.
    #[must_use]
    pub fn instance(&self) -> Option<&str> {
        let instance = match self {
            Self::AuthorizationRequired { instance, .. }
            | Self::TargetAllowanceExceeded { instance, .. }
            | Self::GraphInvalid { instance, .. }
            | Self::IdempotencyConflict { instance, .. }
            | Self::IllegalTransition { instance, .. }
            | Self::ChallengeFailed { instance, .. }
            | Self::NotFound { instance, .. }
            | Self::Forbidden { instance, .. }
            | Self::TenantSuspended { instance, .. }
            | Self::RateLimited { instance, .. }
            | Self::Internal { instance, .. } => instance,
        };
        instance.as_deref()
    }

    /// Records the request path this Problem is being returned from.
    ///
    /// Lets a handler build the Problem where the failure is detected and let
    /// the error layer stamp the path once, instead of threading the path down
    /// into every call site (§A3.8).
    #[must_use]
    pub fn with_instance(mut self, path: impl Into<String>) -> Self {
        let path = path.into();
        let slot = match &mut self {
            Self::AuthorizationRequired { instance, .. }
            | Self::TargetAllowanceExceeded { instance, .. }
            | Self::GraphInvalid { instance, .. }
            | Self::IdempotencyConflict { instance, .. }
            | Self::IllegalTransition { instance, .. }
            | Self::ChallengeFailed { instance, .. }
            | Self::NotFound { instance, .. }
            | Self::Forbidden { instance, .. }
            | Self::TenantSuspended { instance, .. }
            | Self::RateLimited { instance, .. }
            | Self::Internal { instance, .. } => instance,
        };
        *slot = Some(path);
        self
    }

    /// Builds an [`Self::Internal`] carrying only a trace identifier.
    ///
    /// The only constructor offered as a helper, because it is the one whose
    /// misuse would leak: taking just the trace ID makes it awkward to pass an
    /// error message where §13.3 forbids one.
    #[must_use]
    pub fn internal(trace_id: impl Into<String>) -> Self {
        Self::Internal {
            instance: None,
            trace_id: trace_id.into(),
        }
    }
}

impl From<IllegalTransition> for Problem {
    /// Lifts the domain error of [`crate::state::observation_transition`] into
    /// its catalogue entry, filling in the admitted alternatives from the §A3.2
    /// edge table.
    ///
    /// Having this conversion in `vf-core` means the handler for
    /// `POST /v1/findings/{id}/state` cannot accidentally report a refused
    /// transition as a different Problem (§A3.8).
    fn from(error: IllegalTransition) -> Self {
        Self::IllegalTransition {
            detail: error.to_string(),
            instance: None,
            hints: TransitionHints {
                from: error.from,
                attempted: error.event,
                admitted: error.from.admitted_events(),
            },
        }
    }
}

impl core::fmt::Display for Problem {
    /// `title: detail`, for logs. The serialized form is [`Serialize`].
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.title(), self.detail())
    }
}

impl core::error::Error for Problem {}

/// RFC 9457's flat Problem Details object.
///
/// Written by hand rather than derived because the shape is not the shape of the
/// enum: every variant serializes to the *same* six members, with the
/// variant-specific data nested under `vf` rather than flattened or
/// externally tagged. `type`, `title` and `status` come from the variant;
/// `detail` and `instance` are per-occurrence; `vf` is the §13.3 extension.
///
/// `instance` is always present, as `null` when there is no request path, so a
/// consumer sees a stable six-key object rather than a sometimes-five-key one.
impl Serialize for Problem {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        /// Emits the six members with `vf` bound to one typed extension value.
        ///
        /// Generic over the extension so each arm below passes its own struct
        /// and no `serde_json::Value` — and therefore no intermediate
        /// allocation or stringly-typed extension — is needed.
        fn emit<S: Serializer, E: Serialize + ?Sized>(
            serializer: S,
            problem: &Problem,
            extension: &E,
        ) -> Result<S::Ok, S::Error> {
            let mut state = serializer.serialize_struct("Problem", 6)?;
            state.serialize_field("type", problem.type_uri())?;
            state.serialize_field("title", problem.title())?;
            state.serialize_field("status", &problem.status())?;
            state.serialize_field("detail", problem.detail())?;
            state.serialize_field("instance", &problem.instance())?;
            state.serialize_field("vf", extension)?;
            state.end()
        }

        match self {
            // `authorization-required` extends the track hints with the target
            // the caller should act on, which is what `next_actions`' `href`
            // templates need to be resolvable.
            Self::AuthorizationRequired {
                target_id, hints, ..
            } => {
                #[derive(Serialize)]
                struct Extension<'a> {
                    #[serde(skip_serializing_if = "Option::is_none")]
                    target_id: &'a Option<TargetId>,
                    #[serde(flatten)]
                    hints: &'a TrackHints,
                }
                emit(serializer, self, &Extension { target_id, hints })
            }
            Self::TargetAllowanceExceeded { hints, .. } => emit(serializer, self, hints),
            Self::GraphInvalid { errors, .. } => {
                #[derive(Serialize)]
                struct Extension<'a> {
                    errors: &'a [GraphError],
                }
                emit(serializer, self, &Extension { errors })
            }
            Self::IdempotencyConflict { request_key, .. } => {
                #[derive(Serialize)]
                struct Extension<'a> {
                    request_key: &'a str,
                }
                emit(serializer, self, &Extension { request_key })
            }
            Self::IllegalTransition { hints, .. } => emit(serializer, self, hints),
            Self::ChallengeFailed { hints, .. } => emit(serializer, self, hints),
            Self::NotFound { hints, .. } => emit(serializer, self, hints),
            Self::Forbidden { hints, .. } => emit(serializer, self, hints),
            Self::TenantSuspended { reason, .. } => {
                #[derive(Serialize)]
                struct Extension<'a> {
                    #[serde(skip_serializing_if = "Option::is_none")]
                    reason: &'a Option<String>,
                }
                emit(serializer, self, &Extension { reason })
            }
            Self::RateLimited { hints, .. } => emit(serializer, self, hints),
            // The trace identifier is the only thing about an internal error
            // that crosses the trust boundary (§13.3, §19.2).
            Self::Internal { trace_id, .. } => {
                #[derive(Serialize)]
                struct Extension<'a> {
                    trace_id: &'a str,
                }
                emit(serializer, self, &Extension { trace_id })
            }
        }
    }
}
