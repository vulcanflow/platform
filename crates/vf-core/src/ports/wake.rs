//! The [`WakeBus`] port and its value types (architecture §A6.1, task F3).
//!
//! # What this port is, and what it is not
//!
//! §A3.3 is explicit: progress events are a view of durable state, the SSE
//! stream is fed from `pipeline_events` in Postgres, and "the `WakeBus` only
//! shortens the poll interval". So this port is **lossy by contract**. A
//! wake-up that is not delivered costs latency — the next poll of the outbox
//! or the events table finds the work anyway — and never costs correctness.
//! Nothing may put state in a wake payload that is not also durable.
//!
//! That is what makes the Valkey restart in the T4 pack an acceptable
//! outcome to test: the requirement is that the *subscription survives* and
//! resumes delivering, not that messages published while the server was down
//! are replayed. Redis publish/subscribe cannot replay them, and a design
//! that needed it would belong in the outbox instead.
//!
//! Token buckets live on the same port because they need the same shared
//! state: §19 rate limits must hold across every replica of `vf-api`, so they
//! are evaluated in Valkey when it is configured and in-process otherwise.
//!
//! Tenant scope is a type, as §A7-7 requires: a tenant's topics and bucket
//! keys live under [`tenant_wake_namespace`], and code serving one tenant
//! holds a [`TenantWakeBus`], which refuses a name outside it before the call
//! reaches an adapter. So tenants neither see each other's wake-ups nor share
//! a §19 bucket.
//!
//! Implemented in `vf-db::adapters` (task F3): `RedisWakeBus`,
//! `InProcessWakeBus`.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::PortFuture;

/// Largest accepted wake payload, in bytes.
///
/// Small on purpose. A wake-up carries at most an identifier the receiver
/// looks up in Postgres (§A3.3); anything that does not fit in 4 KiB is state
/// that belongs in a table, not on a lossy bus.
pub const MAX_WAKE_PAYLOAD_BYTES: usize = 4 * 1024;

/// Longest accepted [`Topic`] or [`BucketKey`], in bytes.
pub const MAX_WAKE_NAME_BYTES: usize = 255;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Everything a [`WakeBus`] call can refuse or fail with.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WakeBusError {
    /// The topic name is not well formed. Never reached the backend.
    #[error("wake topic is invalid: {reason}")]
    InvalidTopic {
        /// Why the topic was refused.
        reason: String,
    },

    /// The token-bucket key is not well formed. Never reached the backend.
    #[error("token bucket key is invalid: {reason}")]
    InvalidBucketKey {
        /// Why the key was refused.
        reason: String,
    },

    /// The quota is not usable: a zero rate or a zero burst would make every
    /// call wait forever, which is a configuration bug and not a limit.
    #[error("token bucket quota is invalid: {reason}")]
    InvalidQuota {
        /// Why the quota was refused.
        reason: String,
    },

    /// The topic or bucket key is outside the tenant namespace the bus is
    /// scoped to. Refused by [`TenantWakeBus`] before any I/O (§A7-7).
    #[error("wake topic or bucket key is outside the tenant namespace {namespace}")]
    OutsideTenantNamespace {
        /// The namespace the bus is scoped to, `tenant:{tenant_id}:`, and
        /// therefore not a secret. Never the refused name, which can be
        /// attacker-influenced.
        namespace: String,
    },

    /// The payload is over [`MAX_WAKE_PAYLOAD_BYTES`].
    #[error("wake payload of {size} bytes is over the {limit} byte limit")]
    PayloadTooLarge {
        /// The payload size offered.
        size: usize,
        /// The limit, [`MAX_WAKE_PAYLOAD_BYTES`].
        limit: usize,
    },

    /// The subscription will deliver nothing more. Terminal: the bus has shut
    /// down, or the adapter gave up reconnecting. Callers fall back to polling.
    #[error("subscription to {topic} is closed")]
    Closed {
        /// The topic that closed.
        topic: Topic,
    },

    /// Wake-ups were dropped because the receiver fell behind, or because the
    /// connection was down. **Not terminal** — the subscription stays usable,
    /// and the caller's next poll picks up whatever state it missed. The T4
    /// pack relies on this: a Valkey restart surfaces here and the next
    /// [`WakeSubscription::recv`] keeps working.
    #[error("subscription to {topic} dropped {skipped} wake-ups; poll to resync")]
    Lagged {
        /// The topic that lagged.
        topic: Topic,
        /// How many wake-ups were dropped, or `0` when the adapter cannot
        /// count them (a reconnect, for instance).
        skipped: u64,
    },

    /// The backing service failed. Carries a rendered message rather than the
    /// backend error type, so `vf-core` stays free of `redis`.
    #[error("wake bus backend error: {message}")]
    Backend {
        /// Rendered backend error chain.
        message: String,
    },

    /// This adapter does not implement the operation. Used by the F3 API
    /// skeleton, and afterwards by an adapter that genuinely cannot offer an
    /// operation.
    #[error("wake bus operation `{operation}` is not supported by this adapter")]
    Unsupported {
        /// The port method that was called.
        operation: &'static str,
    },
}

impl WakeBusError {
    /// Whether the subscription that produced this error is still usable.
    ///
    /// `true` only for [`Self::Lagged`]. A caller loops on `recv`, logs a
    /// recoverable error and continues; anything else ends the loop and falls
    /// back to polling.
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::Lagged { .. })
    }
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

/// Validates a topic or bucket name.
///
/// The accepted set is ASCII alphanumerics and `.`, `_`, `-`, `:`, `/`. It is
/// an allow-list, and the thing it excludes matters: Redis glob metacharacters
/// (`*`, `?`, `[`, `]`, `\`) cannot appear, so a name can never be read as a
/// pattern by a `PSUBSCRIBE` elsewhere in the codebase, and whitespace and
/// newlines cannot appear, so a name cannot smuggle a second RESP token.
fn validate_name(raw: &str) -> Result<(), String> {
    if raw.is_empty() {
        return Err("empty".to_owned());
    }
    if raw.len() > MAX_WAKE_NAME_BYTES {
        return Err(format!("longer than {MAX_WAKE_NAME_BYTES} bytes"));
    }
    if !raw
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b':' | b'/'))
    {
        return Err(
            "contains a character outside [A-Za-z0-9._:/-]; glob metacharacters and whitespace \
             are refused so a name is never read as a pattern"
                .to_owned(),
        );
    }
    Ok(())
}

/// The wake namespace of one tenant: `tenant:{tenant_id}:`, with the tenant id
/// as a hyphenated lowercase UUID.
///
/// This is the bus namespace `LocalTenantProvisioner` provisions for a tenant
/// (§A6.1), and every topic and bucket key a [`TenantWakeBus`] accepts lies
/// under it. The provisioner is to call this function rather than restate the
/// format, so the two cannot drift. Takes a [`Uuid`] rather than the
/// `vf-core::ids` `TenantId` of §A3.1, which task C1 has not published yet.
///
/// The trailing `:` is the boundary, for the same reason
/// [`ArtifactPrefix`](super::ArtifactPrefix) ends in `/`: a name under
/// `tenant:{a}:` can never be read as lying under a longer id that merely
/// starts with `{a}`.
#[must_use]
pub fn tenant_wake_namespace(tenant_id: Uuid) -> String {
    format!("tenant:{}:", tenant_id.as_hyphenated())
}

/// Builds `{namespace}{name}` and validates the whole name.
///
/// An empty `name` is refused: the bare namespace names no topic and no
/// bucket, and [`TenantWakeBus`] refuses it too, so every name built here is
/// one the guard accepts.
fn tenant_name(tenant_id: Uuid, name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("empty name inside the tenant namespace".to_owned());
    }
    let full = format!("{}{name}", tenant_wake_namespace(tenant_id));
    validate_name(&full)?;
    Ok(full)
}

/// Whether `name` lies strictly under `namespace`.
///
/// `namespace` always ends in `:`, so this is exact at the boundary: it never
/// matches a sibling whose id merely starts with the same characters, nor the
/// bare namespace itself. Comparison is byte-for-byte, so a name spelling the
/// tenant id in upper case is outside the namespace: fail closed.
fn in_namespace(namespace: &str, name: &str) -> bool {
    name.len() > namespace.len() && name.starts_with(namespace)
}

/// A validated publish/subscribe topic.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Topic(String);

impl Topic {
    /// Validates `raw` and returns the topic.
    ///
    /// # Errors
    ///
    /// [`WakeBusError::InvalidTopic`] when `raw` is empty, over
    /// [`MAX_WAKE_NAME_BYTES`], or contains a character outside
    /// `[A-Za-z0-9._:/-]`.
    pub fn parse(raw: &str) -> Result<Self, WakeBusError> {
        validate_name(raw)
            .map_err(|reason| WakeBusError::InvalidTopic { reason })
            .map(|()| Self(raw.to_owned()))
    }

    /// The topic `name` inside one tenant's namespace:
    /// `tenant:{tenant_id}:{name}` (see [`tenant_wake_namespace`]).
    ///
    /// # Errors
    ///
    /// [`WakeBusError::InvalidTopic`] when `name` is empty, or when the whole
    /// topic fails [`Self::parse`].
    pub fn for_tenant(tenant_id: Uuid, name: &str) -> Result<Self, WakeBusError> {
        tenant_name(tenant_id, name)
            .map_err(|reason| WakeBusError::InvalidTopic { reason })
            .map(Self)
    }

    /// The topic as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated token-bucket key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BucketKey(String);

impl BucketKey {
    /// Validates `raw` and returns the key.
    ///
    /// # Errors
    ///
    /// [`WakeBusError::InvalidBucketKey`] under the same rules as
    /// [`Topic::parse`].
    pub fn parse(raw: &str) -> Result<Self, WakeBusError> {
        validate_name(raw)
            .map_err(|reason| WakeBusError::InvalidBucketKey { reason })
            .map(|()| Self(raw.to_owned()))
    }

    /// The bucket key `name` inside one tenant's namespace:
    /// `tenant:{tenant_id}:{name}` (see [`tenant_wake_namespace`]).
    ///
    /// # Errors
    ///
    /// [`WakeBusError::InvalidBucketKey`] when `name` is empty, or when the
    /// whole key fails [`Self::parse`].
    pub fn for_tenant(tenant_id: Uuid, name: &str) -> Result<Self, WakeBusError> {
        tenant_name(tenant_id, name)
            .map_err(|reason| WakeBusError::InvalidBucketKey { reason })
            .map(Self)
    }

    /// The key as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! name_newtype_impls {
    ($name:ident, $parse_error:ty) => {
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({:?})"), self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = $parse_error;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(&value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

name_newtype_impls!(Topic, WakeBusError);
name_newtype_impls!(BucketKey, WakeBusError);

// ---------------------------------------------------------------------------
// Messages and subscriptions
// ---------------------------------------------------------------------------

/// One delivered wake-up.
#[derive(Clone, PartialEq, Eq)]
pub struct WakeMessage {
    /// The topic it arrived on. Always the subscribed topic, because the
    /// adapters subscribe exactly and never by pattern.
    pub topic: Topic,
    /// The payload, at most [`MAX_WAKE_PAYLOAD_BYTES`] bytes.
    pub payload: Vec<u8>,
}

impl WakeMessage {
    /// A message with an empty payload: the common case, since §A3.3 wake-ups
    /// usually only need to say "look again".
    #[must_use]
    pub fn empty(topic: Topic) -> Self {
        Self {
            topic,
            payload: Vec::new(),
        }
    }
}

impl fmt::Debug for WakeMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The payload is not printed: it is small, but it is still data the
        // control plane put on a bus and logs are a different trust domain.
        f.debug_struct("WakeMessage")
            .field("topic", &self.topic)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

/// The receiving half of a subscription.
///
/// A trait with a boxed future rather than a `futures::Stream` because
/// `vf-core` carries no async crate (§A1.3). [`WakeSubscription`] wraps it so
/// callers write `while let Ok(msg) = sub.recv().await` and never name this
/// trait.
pub trait WakeStream: Send + 'static {
    /// The next wake-up.
    ///
    /// # Errors
    ///
    /// [`WakeBusError::Lagged`] when wake-ups were dropped — the caller logs
    /// and continues. [`WakeBusError::Closed`] when nothing more will arrive.
    /// [`WakeBusError::Backend`] for an unrecoverable transport failure.
    /// `Closed` and `Backend` are sticky: every later call returns the same
    /// error at once (bus contract item 7).
    fn recv(&mut self) -> PortFuture<'_, Result<WakeMessage, WakeBusError>>;
}

/// A live subscription to one topic.
///
/// Dropping it unsubscribes. An adapter that multiplexes one connection
/// across subscriptions is free to keep the server-side subscription until
/// the last one for that topic goes away.
pub struct WakeSubscription {
    topic: Topic,
    stream: Box<dyn WakeStream>,
}

impl WakeSubscription {
    /// Wraps an adapter's stream. Called by adapters, not by callers.
    #[must_use]
    pub fn new(topic: Topic, stream: impl WakeStream) -> Self {
        Self {
            topic,
            stream: Box::new(stream),
        }
    }

    /// The topic subscribed to.
    #[must_use]
    pub fn topic(&self) -> &Topic {
        &self.topic
    }

    /// The next wake-up.
    ///
    /// # Errors
    ///
    /// As [`WakeStream::recv`]. Check [`WakeBusError::is_recoverable`] before
    /// giving up on the subscription.
    pub async fn recv(&mut self) -> Result<WakeMessage, WakeBusError> {
        self.stream.recv().await
    }
}

impl fmt::Debug for WakeSubscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WakeSubscription")
            .field("topic", &self.topic)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Token buckets
// ---------------------------------------------------------------------------

/// A refill rate: `tokens` tokens every `period`.
///
/// Expressed as a pair of integers rather than a float so the type is `Eq`,
/// so two quotas read from configuration compare exactly, and so the Redis
/// and in-process adapters compute the refill with the same integer
/// arithmetic from the same inputs. It is not `Deserialize`: a configuration
/// loader reads the two numbers and calls [`Self::new`], so a zero rate is
/// refused where it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TokenRate {
    tokens: u32,
    period: Duration,
}

impl TokenRate {
    /// `tokens` tokens every `period`.
    ///
    /// # Errors
    ///
    /// [`WakeBusError::InvalidQuota`] when `tokens` is zero (no call would
    /// ever be granted) or `period` is zero (the refill would be unbounded).
    pub fn new(tokens: u32, period: Duration) -> Result<Self, WakeBusError> {
        if tokens == 0 {
            return Err(WakeBusError::InvalidQuota {
                reason: "a rate of zero tokens never grants a permit".to_owned(),
            });
        }
        if period.is_zero() {
            return Err(WakeBusError::InvalidQuota {
                reason: "a zero refill period is an unbounded rate, not a limit".to_owned(),
            });
        }
        Ok(Self { tokens, period })
    }

    /// `tokens` tokens per second.
    ///
    /// # Errors
    ///
    /// As [`Self::new`].
    pub fn per_second(tokens: u32) -> Result<Self, WakeBusError> {
        Self::new(tokens, Duration::from_secs(1))
    }

    /// Tokens added per [`Self::period`].
    #[must_use]
    pub const fn tokens(&self) -> u32 {
        self.tokens
    }

    /// The refill period.
    #[must_use]
    pub const fn period(&self) -> Duration {
        self.period
    }
}

/// The answer to one [`WakeBus::token_bucket`] call.
///
/// A value and not a guard: the token is spent when the bucket is consulted,
/// never returned on drop. §19 rate limits count attempts, and a guard that
/// refunded on an early return would let a caller retry a failed request for
/// free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permit {
    /// The call may proceed. One token was consumed.
    Granted {
        /// Tokens left in the bucket after this call.
        remaining: u32,
    },
    /// The call must not proceed. No token was consumed.
    Denied {
        /// How long until one token is available, rounded up to a whole
        /// number of seconds and never less than one. The value a
        /// `Retry-After` header is built from (§13.3); the header counts
        /// whole seconds, so a sub-second wait truncated to `0` would invite
        /// an immediate retry.
        retry_after: Duration,
    },
}

impl Permit {
    /// Whether the call may proceed.
    #[must_use]
    pub const fn is_granted(&self) -> bool {
        matches!(self, Self::Granted { .. })
    }
}

// ---------------------------------------------------------------------------
// The port
// ---------------------------------------------------------------------------

/// Wake-up publish/subscribe and token buckets (§A6.1).
///
/// Real and local: `redis` against Valkey. Test: an in-process tokio
/// broadcast bus. Implemented in `vf-db::adapters` (task F3).
///
/// # Contract every adapter honours
///
/// The T4 conformance pack runs one module against both implementations:
///
/// 1. `publish` to a topic nobody is subscribed to succeeds. There is no
///    delivery guarantee to report, so there is no error to return.
/// 2. A subscriber created before a `publish` receives it, unless the
///    delivery is lost as item 3 describes. A subscriber created after it
///    does not: there is no replay.
/// 3. Every subscriber to a topic receives every message published after it
///    subscribed, unless it falls behind or the adapter reconnects. Either
///    loss surfaces as [`WakeBusError::Lagged`] and leaves the subscription
///    usable. Every delivered [`WakeMessage`] carries the subscription's
///    topic.
/// 4. A payload over [`MAX_WAKE_PAYLOAD_BYTES`] is refused with
///    [`WakeBusError::PayloadTooLarge`] and nothing is published.
/// 5. `token_bucket` consumes one token per granted call. A bucket starts
///    full at `burst`. Calls are granted while tokens remain and denied once
///    they do not, with a `retry_after` that is a whole number of seconds and
///    at least one. A `burst` of zero is refused with
///    [`WakeBusError::InvalidQuota`]. `key` alone names the bucket: an
///    adapter never derives the backend key from `rate` or `burst`, because
///    then a caller could mint a fresh, full bucket by varying `burst`. A call
///    with a different quota re-measures the same bucket, so callers must not
///    vary the quota per call.
/// 6. Everything on this port is best-effort with respect to delivery and
///    exact with respect to accounting: a dropped wake-up is a latency cost
///    (§A3.3), but a token is never granted twice.
/// 7. Terminal subscription errors are sticky. Once `recv` returns
///    [`WakeBusError::Closed`] or [`WakeBusError::Backend`], every later
///    `recv` returns the same error at once: it never hangs, panics or
///    resumes delivering.
///
/// # Tenant scope
///
/// An adapter does not interpret names and enforces no tenant boundary. Code
/// that serves one tenant holds a [`TenantWakeBus`], which refuses a topic or
/// bucket key outside that tenant's [`tenant_wake_namespace`] before any I/O.
pub trait WakeBus: Send + Sync + 'static {
    /// Publishes `payload` on `topic`.
    fn publish(&self, topic: &Topic, payload: &[u8]) -> PortFuture<'_, Result<(), WakeBusError>>;

    /// Subscribes to `topic`. The subscription begins before this returns, so
    /// a `publish` that happens after the await is delivered.
    fn subscribe(&self, topic: &Topic) -> PortFuture<'_, Result<WakeSubscription, WakeBusError>>;

    /// Takes one token from the bucket named `key`, refilling at `rate` with
    /// capacity `burst`. Refused with [`WakeBusError::InvalidQuota`] when
    /// `burst` is zero.
    fn token_bucket(
        &self,
        key: &BucketKey,
        rate: TokenRate,
        burst: u32,
    ) -> PortFuture<'_, Result<Permit, WakeBusError>>;
}

// ---------------------------------------------------------------------------
// Tenant scoping
// ---------------------------------------------------------------------------

/// A [`WakeBus`] that can only reach one tenant's namespace.
///
/// The bus counterpart of [`TenantArtifactStore`](super::TenantArtifactStore),
/// for the same §A7-7 rule: a worker that holds a `TenantWakeBus` cannot
/// subscribe to another tenant's topic, publish into it, or spend another
/// tenant's §19 token bucket, even by accident. A topic or bucket key is
/// accepted only when it lies strictly under [`tenant_wake_namespace`] for the
/// tenant this bus is scoped to.
///
/// **Refusal happens before any I/O.** Every method validates first and
/// returns an already-resolved future on refusal, so the inner bus's method
/// is never called, no future that would do I/O is ever constructed, and the
/// T4 pack can assert the refusal against an inner bus that panics if
/// touched.
pub struct TenantWakeBus {
    inner: Arc<dyn WakeBus>,
    tenant_id: Uuid,
    namespace: String,
}

impl TenantWakeBus {
    /// Scopes `inner` to `tenant:{tenant_id}:`.
    ///
    /// Takes a [`Uuid`] rather than the `vf-core::ids` `TenantId` of §A3.1,
    /// which task C1 has not published yet.
    #[must_use]
    pub fn new(inner: Arc<dyn WakeBus>, tenant_id: Uuid) -> Self {
        Self {
            inner,
            tenant_id,
            namespace: tenant_wake_namespace(tenant_id),
        }
    }

    /// The tenant this bus is scoped to.
    #[must_use]
    pub fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    /// The namespace this bus is scoped to: `tenant:{tenant_id}:`.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Builds a topic inside this tenant's namespace. The preferred way to
    /// name one, because it cannot produce a topic the guard refuses.
    ///
    /// # Errors
    ///
    /// As [`Topic::for_tenant`].
    pub fn topic(&self, name: &str) -> Result<Topic, WakeBusError> {
        Topic::for_tenant(self.tenant_id, name)
    }

    /// Builds a bucket key inside this tenant's namespace. The preferred way
    /// to name one, because it cannot produce a key the guard refuses.
    ///
    /// # Errors
    ///
    /// As [`BucketKey::for_tenant`].
    pub fn bucket_key(&self, name: &str) -> Result<BucketKey, WakeBusError> {
        BucketKey::for_tenant(self.tenant_id, name)
    }

    /// Refuses a topic or bucket key outside the tenant namespace.
    fn guard(&self, name: &str) -> Result<(), WakeBusError> {
        if in_namespace(&self.namespace, name) {
            Ok(())
        } else {
            Err(WakeBusError::OutsideTenantNamespace {
                namespace: self.namespace.clone(),
            })
        }
    }
}

impl fmt::Debug for TenantWakeBus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TenantWakeBus")
            .field("tenant_id", &self.tenant_id)
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

/// An already-resolved refusal, so a rejected call never builds a future that
/// could do I/O. The wake-bus twin of the helper behind
/// [`TenantArtifactStore`](super::TenantArtifactStore).
fn refuse<'a, T: Send + 'a>(error: WakeBusError) -> PortFuture<'a, Result<T, WakeBusError>> {
    Box::pin(std::future::ready(Err(error)))
}

impl WakeBus for TenantWakeBus {
    fn publish(&self, topic: &Topic, payload: &[u8]) -> PortFuture<'_, Result<(), WakeBusError>> {
        if let Err(error) = self.guard(topic.as_str()) {
            return refuse(error);
        }
        self.inner.publish(topic, payload)
    }

    fn subscribe(&self, topic: &Topic) -> PortFuture<'_, Result<WakeSubscription, WakeBusError>> {
        if let Err(error) = self.guard(topic.as_str()) {
            return refuse(error);
        }
        self.inner.subscribe(topic)
    }

    fn token_bucket(
        &self,
        key: &BucketKey,
        rate: TokenRate,
        burst: u32,
    ) -> PortFuture<'_, Result<Permit, WakeBusError>> {
        if let Err(error) = self.guard(key.as_str()) {
            return refuse(error);
        }
        self.inner.token_bucket(key, rate, burst)
    }
}
