//! Port-contract tests for `vf_core::ports::wake` — VFL-314 and its rev2
//! follow-up VFL-355 (the F3a slices of T4 (VFL-44) for F3a (VFL-310)), plus
//! VFL-420 (the F3d slice of T4 for F3d (VFL-346)).
//!
//! Rebased onto `5ca5dc3` on `jorge/f3d-tenant-wake-bus` (one commit on
//! `a8ad7ae`, F3a, not pushed); see `artifact_port_contract.rs`'s header for
//! the revision and split note shared by both files in this pack. F3d adds
//! `tenant_wake_namespace`, `Topic`/`BucketKey::for_tenant`, `TenantWakeBus`
//! and `WakeBusError::OutsideTenantNamespace`, resolving the pending-
//! tenant-scope note F3a rev2 left in this module's rustdoc.
//!
//! Scope through the `TokenRate validation` section is unchanged: VFL-310's
//! criterion 2 for the wake side (topic, bucket-key and rate validation; the
//! two wake-only byte caps). The `TenantWakeBus` section below is VFL-346's
//! criteria: (1) `publish`, `subscribe` and `token_bucket` refuse a topic or
//! key outside the tenant namespace before any I/O, against an inner bus
//! that panics if touched, and forward an in-namespace call; (2) the guard
//! is exact at the `tenant:{id}:` boundary; (3) `for_tenant` values pass
//! `Topic::parse`/`BucketKey::parse`, are accepted by their own tenant's
//! guard and refused by another's, and the 255-byte cap applies to the whole
//! name; (4) the refusal never echoes the refused name. There is still no
//! concrete `WakeBus` adapter — that is F3c (VFL-312) — so delivery
//! semantics (publish/subscribe, `Lagged` vs `Closed`, token accounting
//! against a real bucket) remain out of scope here and belong to that
//! slice's conformance pack; the `TenantWakeBus` tests use a hand-written
//! inner double, the same technique `artifact_port_contract.rs` uses for
//! `TenantArtifactStore`.
//!
//! `vf-core` carries no async runtime (§A1.3): `support::block_on` drives
//! the `PortFuture`s `TenantWakeBus` returns by polling once, as
//! `artifact_port_contract.rs`'s header describes.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use uuid::Uuid;
use vf_core::ports::{
    BucketKey, MAX_WAKE_NAME_BYTES, MAX_WAKE_PAYLOAD_BYTES, Permit, PortFuture, TenantWakeBus,
    TokenRate, Topic, WakeBus, WakeBusError, WakeMessage, WakeStream, WakeSubscription,
    tenant_wake_namespace,
};

// ---------------------------------------------------------------------------
// Topic / BucketKey validation
// ---------------------------------------------------------------------------

/// (raw input, whether it should parse). `Topic` and `BucketKey` share one
/// `validate_name` implementation (wake.rs), so one table drives both.
const NAME_CASES: &[(&str, bool)] = &[
    ("", false),
    ("scan.completed", true),
    ("tenant:123/run-1_v2.json", true),
    ("has space", false),
    ("glob*", false),
    ("glob?", false),
    ("glob[0]", false),
    ("glob]", false),
    ("glob\\x", false),
    ("new\nline", false),
];

#[test]
fn topic_and_bucket_key_share_the_same_validation_rules() {
    for (raw, expect_ok) in NAME_CASES {
        assert_eq!(
            Topic::parse(raw).is_ok(),
            *expect_ok,
            "Topic::parse({raw:?}) expected ok={expect_ok}"
        );
        assert_eq!(
            BucketKey::parse(raw).is_ok(),
            *expect_ok,
            "BucketKey::parse({raw:?}) expected ok={expect_ok}"
        );
    }
}

#[test]
fn topic_and_bucket_key_accept_exactly_the_name_byte_cap() {
    let raw = "a".repeat(MAX_WAKE_NAME_BYTES);
    assert!(Topic::parse(&raw).is_ok());
    assert!(BucketKey::parse(&raw).is_ok());
}

#[test]
fn topic_and_bucket_key_reject_one_byte_over_the_name_cap() {
    let raw = "a".repeat(MAX_WAKE_NAME_BYTES + 1);
    assert!(matches!(
        Topic::parse(&raw),
        Err(WakeBusError::InvalidTopic { .. })
    ));
    assert!(matches!(
        BucketKey::parse(&raw),
        Err(WakeBusError::InvalidBucketKey { .. })
    ));
}

#[test]
fn topic_parse_reports_its_own_error_variant() {
    assert!(matches!(
        Topic::parse(""),
        Err(WakeBusError::InvalidTopic { .. })
    ));
}

#[test]
fn bucket_key_parse_reports_its_own_error_variant() {
    assert!(matches!(
        BucketKey::parse(""),
        Err(WakeBusError::InvalidBucketKey { .. })
    ));
}

#[test]
fn topic_as_str_returns_the_validated_name_unchanged() {
    let topic = Topic::parse("scan.completed").unwrap();
    assert_eq!(topic.as_str(), "scan.completed");
}

#[test]
fn bucket_key_as_str_returns_the_validated_name_unchanged() {
    let key = BucketKey::parse("tenant-a:webhook-deliver").unwrap();
    assert_eq!(key.as_str(), "tenant-a:webhook-deliver");
}

#[test]
fn topic_try_from_string_matches_parse_and_into_string_round_trips() {
    let topic = Topic::try_from("scan.completed".to_string()).unwrap();
    assert_eq!(topic.as_str(), "scan.completed");
    let round_tripped: String = topic.into();
    assert_eq!(round_tripped, "scan.completed");

    assert!(matches!(
        Topic::try_from(String::new()),
        Err(WakeBusError::InvalidTopic { .. })
    ));
}

#[test]
fn bucket_key_try_from_string_matches_parse_and_into_string_round_trips() {
    let key = BucketKey::try_from("tenant-a:webhook-deliver".to_string()).unwrap();
    assert_eq!(key.as_str(), "tenant-a:webhook-deliver");
    let round_tripped: String = key.into();
    assert_eq!(round_tripped, "tenant-a:webhook-deliver");

    assert!(matches!(
        BucketKey::try_from(String::new()),
        Err(WakeBusError::InvalidBucketKey { .. })
    ));
}

// ---------------------------------------------------------------------------
// TokenRate validation
// ---------------------------------------------------------------------------

#[test]
fn token_rate_rejects_a_zero_token_count() {
    assert!(matches!(
        TokenRate::new(0, Duration::from_secs(1)),
        Err(WakeBusError::InvalidQuota { .. })
    ));
}

#[test]
fn token_rate_rejects_a_zero_period() {
    assert!(matches!(
        TokenRate::new(1, Duration::ZERO),
        Err(WakeBusError::InvalidQuota { .. })
    ));
}

#[test]
fn token_rate_new_keeps_the_tokens_and_period_it_was_given() {
    let rate = TokenRate::new(5, Duration::from_millis(250)).unwrap();
    assert_eq!(rate.tokens(), 5);
    assert_eq!(rate.period(), Duration::from_millis(250));
}

#[test]
fn token_rate_per_second_is_tokens_per_one_second_period() {
    let rate = TokenRate::per_second(10).unwrap();
    assert_eq!(rate.tokens(), 10);
    assert_eq!(rate.period(), Duration::from_secs(1));
}

#[test]
fn token_rate_per_second_rejects_a_zero_token_count() {
    assert!(matches!(
        TokenRate::per_second(0),
        Err(WakeBusError::InvalidQuota { .. })
    ));
}

// ---------------------------------------------------------------------------
// Named byte caps (criterion 2)
// ---------------------------------------------------------------------------

#[test]
fn max_wake_payload_bytes_is_four_kibibytes() {
    assert_eq!(MAX_WAKE_PAYLOAD_BYTES, 4 * 1024);
}

#[test]
fn max_wake_name_bytes_is_255() {
    assert_eq!(MAX_WAKE_NAME_BYTES, 255);
}

// ---------------------------------------------------------------------------
// Debug redaction (S4): a wake payload never reaches a log line
// ---------------------------------------------------------------------------

#[test]
fn wake_message_debug_prints_payload_len_never_the_payload() {
    let topic = Topic::parse("scan.completed").unwrap();
    let message = WakeMessage {
        topic: topic.clone(),
        payload: b"snitch".to_vec(),
    };

    assert_eq!(
        format!("{message:?}"),
        format!("WakeMessage {{ topic: {topic:?}, payload_len: 6 }}")
    );
}

// ---------------------------------------------------------------------------
// Optional additions per Cortana's VFL-394 decision §3-§4
// ---------------------------------------------------------------------------

#[test]
fn wake_bus_error_is_recoverable_is_true_only_for_lagged() {
    let lagged = WakeBusError::Lagged {
        topic: Topic::parse("scan.completed").unwrap(),
        skipped: 3,
    };
    assert!(lagged.is_recoverable());

    let closed = WakeBusError::Closed {
        topic: Topic::parse("scan.completed").unwrap(),
    };
    assert!(!closed.is_recoverable());

    // L6 (VFL-428): every other variant is also non-recoverable, not just
    // `Closed` — `is_recoverable` must name `Lagged` specifically rather
    // than some broader "not a hard failure" rule.
    let invalid_topic = WakeBusError::InvalidTopic {
        reason: "empty".to_owned(),
    };
    assert!(!invalid_topic.is_recoverable());

    let invalid_bucket_key = WakeBusError::InvalidBucketKey {
        reason: "empty".to_owned(),
    };
    assert!(!invalid_bucket_key.is_recoverable());

    let invalid_quota = WakeBusError::InvalidQuota {
        reason: "zero rate".to_owned(),
    };
    assert!(!invalid_quota.is_recoverable());

    let payload_too_large = WakeBusError::PayloadTooLarge {
        size: MAX_WAKE_PAYLOAD_BYTES + 1,
        limit: MAX_WAKE_PAYLOAD_BYTES,
    };
    assert!(!payload_too_large.is_recoverable());

    let backend = WakeBusError::Backend {
        message: "connection refused".to_owned(),
    };
    assert!(!backend.is_recoverable());

    assert!(!WakeBusError::Unsupported { operation: "recv" }.is_recoverable());
}

#[test]
fn permit_is_granted_is_true_only_for_granted() {
    assert!(Permit::Granted { remaining: 1 }.is_granted());
    assert!(
        !Permit::Denied {
            retry_after: Duration::from_secs(1)
        }
        .is_granted()
    );
}

// ---------------------------------------------------------------------------
// Test doubles (TenantWakeBus)
// ---------------------------------------------------------------------------

/// A stream that panics if `recv` is ever polled.
///
/// Stands in for the subscription a successful `subscribe` must return;
/// nothing in this pack calls `recv`, so a panic there would mean a test
/// reached further than intended, not that the double is missing a feature.
struct PanicIfPolledStream;

impl WakeStream for PanicIfPolledStream {
    fn recv(&mut self) -> PortFuture<'_, Result<WakeMessage, WakeBusError>> {
        panic!("PanicIfPolledStream::recv must never be polled in this pack");
    }
}

/// An inner `WakeBus` that records every call it receives and answers with a
/// canned success.
///
/// `TenantWakeBus`'s rustdoc promises refusal "before any I/O", and every one
/// of its three methods also has a forwarding path under test (unlike
/// `PanicUnlessListed` in `artifact_port_contract.rs`, where most methods
/// only need a refusal test and can panic unconditionally). So here an empty
/// call list after a refused call is what proves the inner bus was never
/// touched, the same technique that file uses for `list` and `get_stream`.
#[derive(Default)]
struct RecordingWakeBus {
    publish_calls: Mutex<Vec<(Topic, Vec<u8>)>>,
    subscribe_calls: Mutex<Vec<Topic>>,
    token_bucket_calls: Mutex<Vec<(BucketKey, TokenRate, u32)>>,
}

impl WakeBus for RecordingWakeBus {
    fn publish(&self, topic: &Topic, payload: &[u8]) -> PortFuture<'_, Result<(), WakeBusError>> {
        self.publish_calls
            .lock()
            .unwrap()
            .push((topic.clone(), payload.to_vec()));
        Box::pin(std::future::ready(Ok(())))
    }

    fn subscribe(&self, topic: &Topic) -> PortFuture<'_, Result<WakeSubscription, WakeBusError>> {
        self.subscribe_calls.lock().unwrap().push(topic.clone());
        let subscription = WakeSubscription::new(topic.clone(), PanicIfPolledStream);
        Box::pin(std::future::ready(Ok(subscription)))
    }

    fn token_bucket(
        &self,
        key: &BucketKey,
        rate: TokenRate,
        burst: u32,
    ) -> PortFuture<'_, Result<Permit, WakeBusError>> {
        self.token_bucket_calls
            .lock()
            .unwrap()
            .push((key.clone(), rate, burst));
        Box::pin(std::future::ready(Ok(Permit::Granted {
            remaining: burst.saturating_sub(1),
        })))
    }
}

fn tenant_bus(tenant_id: Uuid) -> (Arc<RecordingWakeBus>, TenantWakeBus) {
    let inner = Arc::new(RecordingWakeBus::default());
    let bus = TenantWakeBus::new(Arc::clone(&inner) as Arc<dyn WakeBus>, tenant_id);
    (inner, bus)
}

// ---------------------------------------------------------------------------
// TenantWakeBus: refusal before any I/O, in-namespace forwarding (criterion 1)
// ---------------------------------------------------------------------------

#[test]
fn refuses_publish_outside_its_namespace_before_touching_the_inner_bus() {
    let tenant_id = Uuid::from_u128(1);
    let other_tenant_id = Uuid::from_u128(2);
    let (inner, bus) = tenant_bus(tenant_id);
    let foreign_topic = Topic::for_tenant(other_tenant_id, "scan.completed").unwrap();

    let result = support::block_on(bus.publish(&foreign_topic, b"payload"));

    assert!(matches!(
        result,
        Err(WakeBusError::OutsideTenantNamespace { .. })
    ));
    assert!(
        inner.publish_calls.lock().unwrap().is_empty(),
        "a refused publish must never reach the inner bus"
    );
}

#[test]
fn refuses_subscribe_outside_its_namespace_before_touching_the_inner_bus() {
    let tenant_id = Uuid::from_u128(1);
    let other_tenant_id = Uuid::from_u128(2);
    let (inner, bus) = tenant_bus(tenant_id);
    let foreign_topic = Topic::for_tenant(other_tenant_id, "scan.completed").unwrap();

    let result = support::block_on(bus.subscribe(&foreign_topic));

    assert!(matches!(
        result,
        Err(WakeBusError::OutsideTenantNamespace { .. })
    ));
    assert!(
        inner.subscribe_calls.lock().unwrap().is_empty(),
        "a refused subscribe must never reach the inner bus"
    );
}

#[test]
fn refuses_token_bucket_outside_its_namespace_before_touching_the_inner_bus() {
    let tenant_id = Uuid::from_u128(1);
    let other_tenant_id = Uuid::from_u128(2);
    let (inner, bus) = tenant_bus(tenant_id);
    let foreign_key = BucketKey::for_tenant(other_tenant_id, "webhook-deliver").unwrap();
    let rate = TokenRate::per_second(1).unwrap();

    let result = support::block_on(bus.token_bucket(&foreign_key, rate, 1));

    assert!(matches!(
        result,
        Err(WakeBusError::OutsideTenantNamespace { .. })
    ));
    assert!(
        inner.token_bucket_calls.lock().unwrap().is_empty(),
        "a refused token_bucket call must never reach the inner bus"
    );
}

#[test]
fn forwards_an_in_namespace_publish_to_the_inner_bus() {
    let tenant_id = Uuid::from_u128(1);
    let (inner, bus) = tenant_bus(tenant_id);
    let topic = bus.topic("scan.completed").unwrap();

    let result = support::block_on(bus.publish(&topic, b"payload"));

    assert!(
        result.is_ok(),
        "an in-namespace publish must be forwarded, not refused: {result:?}"
    );
    assert_eq!(
        inner.publish_calls.lock().unwrap().as_slice(),
        &[(topic, b"payload".to_vec())]
    );
}

#[test]
fn forwards_an_in_namespace_subscribe_to_the_inner_bus() {
    let tenant_id = Uuid::from_u128(1);
    let (inner, bus) = tenant_bus(tenant_id);
    let topic = bus.topic("scan.completed").unwrap();

    let result = support::block_on(bus.subscribe(&topic));

    assert!(
        result.is_ok(),
        "an in-namespace subscribe must be forwarded, not refused: {result:?}"
    );
    assert_eq!(
        inner.subscribe_calls.lock().unwrap().as_slice(),
        std::slice::from_ref(&topic)
    );
}

#[test]
fn forwards_an_in_namespace_token_bucket_call_to_the_inner_bus() {
    let tenant_id = Uuid::from_u128(1);
    let (inner, bus) = tenant_bus(tenant_id);
    let key = bus.bucket_key("webhook-deliver").unwrap();
    let rate = TokenRate::per_second(1).unwrap();

    let result = support::block_on(bus.token_bucket(&key, rate, 1));

    assert!(
        result.is_ok(),
        "an in-namespace token_bucket call must be forwarded, not refused: {result:?}"
    );
    assert_eq!(
        inner.token_bucket_calls.lock().unwrap().as_slice(),
        &[(key, rate, 1)]
    );
}

// ---------------------------------------------------------------------------
// TenantWakeBus: exact namespace boundary (criterion 2)
// ---------------------------------------------------------------------------

#[test]
fn guard_is_exact_at_the_tenant_namespace_boundary() {
    // Chosen to end in the same digits as the issue's own example (`…0001`)
    // while also containing hex letters, so upper-casing it actually changes
    // the string instead of being a no-op on an all-digit id.
    let tenant_id = Uuid::from_u128(0xABCD_0001);
    let other_tenant_id = Uuid::from_u128(0xABCD_0002);
    let namespace = tenant_wake_namespace(tenant_id);
    let bare_namespace = namespace.trim_end_matches(':').to_owned();
    let (_inner, bus) = tenant_bus(tenant_id);

    let cases: Vec<(String, bool)> = vec![
        (format!("{namespace}scan.completed"), true),
        (
            format!("{}scan.completed", tenant_wake_namespace(other_tenant_id)),
            false,
        ),
        (bare_namespace.clone(), false),
        (namespace.clone(), false),
        (format!("{bare_namespace}0:x"), false),
        (
            format!(
                "tenant:{}:scan.completed",
                tenant_id.as_hyphenated().to_string().to_uppercase()
            ),
            false,
        ),
        ("scan.completed".to_owned(), false),
    ];

    for (raw, expect_ok) in &cases {
        let topic = Topic::parse(raw).unwrap_or_else(|error| {
            panic!("case {raw:?} must itself be a well-formed Topic: {error:?}")
        });

        let result = support::block_on(bus.publish(&topic, b""));

        assert_eq!(
            result.is_ok(),
            *expect_ok,
            "publish({raw:?}) against namespace {namespace:?} expected ok={expect_ok}, got {result:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// TenantWakeBus: `for_tenant` names (criterion 3)
// ---------------------------------------------------------------------------

#[test]
fn for_tenant_topic_passes_parse_and_is_accepted_by_its_own_guard_and_refused_by_anothers() {
    let tenant_id = Uuid::from_u128(1);
    let other_tenant_id = Uuid::from_u128(2);
    let topic = Topic::for_tenant(tenant_id, "scan.completed").unwrap();

    assert!(Topic::parse(topic.as_str()).is_ok());

    let (_own_inner, own_bus) = tenant_bus(tenant_id);
    assert!(support::block_on(own_bus.publish(&topic, b"")).is_ok());

    let (_other_inner, other_bus) = tenant_bus(other_tenant_id);
    assert!(matches!(
        support::block_on(other_bus.publish(&topic, b"")),
        Err(WakeBusError::OutsideTenantNamespace { .. })
    ));
}

#[test]
fn for_tenant_bucket_key_passes_parse_and_is_accepted_by_its_own_guard_and_refused_by_anothers() {
    let tenant_id = Uuid::from_u128(1);
    let other_tenant_id = Uuid::from_u128(2);
    let key = BucketKey::for_tenant(tenant_id, "webhook-deliver").unwrap();
    let rate = TokenRate::per_second(1).unwrap();

    assert!(BucketKey::parse(key.as_str()).is_ok());

    let (_own_inner, own_bus) = tenant_bus(tenant_id);
    assert!(support::block_on(own_bus.token_bucket(&key, rate, 1)).is_ok());

    let (_other_inner, other_bus) = tenant_bus(other_tenant_id);
    assert!(matches!(
        support::block_on(other_bus.token_bucket(&key, rate, 1)),
        Err(WakeBusError::OutsideTenantNamespace { .. })
    ));
}

#[test]
fn topic_for_tenant_name_cap_applies_to_the_whole_namespaced_name() {
    let tenant_id = Uuid::from_u128(1);
    let namespace_len = tenant_wake_namespace(tenant_id).len();
    let exactly_at_cap = "a".repeat(MAX_WAKE_NAME_BYTES - namespace_len);
    let one_over_cap = "a".repeat(MAX_WAKE_NAME_BYTES - namespace_len + 1);

    assert!(Topic::for_tenant(tenant_id, &exactly_at_cap).is_ok());
    assert!(matches!(
        Topic::for_tenant(tenant_id, &one_over_cap),
        Err(WakeBusError::InvalidTopic { .. })
    ));
}

#[test]
fn bucket_key_for_tenant_name_cap_applies_to_the_whole_namespaced_name() {
    let tenant_id = Uuid::from_u128(1);
    let namespace_len = tenant_wake_namespace(tenant_id).len();
    let exactly_at_cap = "a".repeat(MAX_WAKE_NAME_BYTES - namespace_len);
    let one_over_cap = "a".repeat(MAX_WAKE_NAME_BYTES - namespace_len + 1);

    assert!(BucketKey::for_tenant(tenant_id, &exactly_at_cap).is_ok());
    assert!(matches!(
        BucketKey::for_tenant(tenant_id, &one_over_cap),
        Err(WakeBusError::InvalidBucketKey { .. })
    ));
}

#[test]
fn topic_for_tenant_refuses_an_empty_name() {
    assert!(matches!(
        Topic::for_tenant(Uuid::from_u128(1), ""),
        Err(WakeBusError::InvalidTopic { .. })
    ));
}

#[test]
fn bucket_key_for_tenant_refuses_an_empty_name() {
    assert!(matches!(
        BucketKey::for_tenant(Uuid::from_u128(1), ""),
        Err(WakeBusError::InvalidBucketKey { .. })
    ));
}

// ---------------------------------------------------------------------------
// TenantWakeBus: the refusal never echoes the refused name (criterion 4)
// ---------------------------------------------------------------------------

#[test]
fn outside_tenant_namespace_error_never_echoes_the_refused_name() {
    let tenant_id = Uuid::from_u128(1);
    let (_inner, bus) = tenant_bus(tenant_id);
    let secret_name = "tenant:00000000-0000-0000-0000-000000000002:leaked-topic-name";
    let foreign_topic = Topic::parse(secret_name).unwrap();

    let result = support::block_on(bus.publish(&foreign_topic, b""));
    let error = result.expect_err("a foreign topic must be refused");

    let display = error.to_string();
    let debug = format!("{error:?}");

    assert!(
        !display.contains("leaked-topic-name") && !debug.contains("leaked-topic-name"),
        "the refusal must never echo the refused name: display={display:?} debug={debug:?}"
    );
    assert!(
        display.contains(bus.namespace()),
        "the refusal should name the namespace it is scoped to: {display:?}"
    );
}

#[test]
fn outside_tenant_namespace_display_matches_its_documented_wording() {
    let error = WakeBusError::OutsideTenantNamespace {
        namespace: "tenant:00000000-0000-0000-0000-000000000001:".to_owned(),
    };

    assert_eq!(
        error.to_string(),
        "wake topic or bucket key is outside the tenant namespace \
         tenant:00000000-0000-0000-0000-000000000001:"
    );
}

// ---------------------------------------------------------------------------
// TenantWakeBus: accessors and builders (New API surface)
// ---------------------------------------------------------------------------

#[test]
fn exposes_the_tenant_id_and_namespace_it_was_constructed_with() {
    let tenant_id = Uuid::from_u128(7);
    let (_inner, bus) = tenant_bus(tenant_id);

    assert_eq!(bus.tenant_id(), tenant_id);
    assert_eq!(bus.namespace(), tenant_wake_namespace(tenant_id));
}

#[test]
fn topic_builds_a_topic_inside_its_own_namespace() {
    let tenant_id = Uuid::from_u128(42);
    let (_inner, bus) = tenant_bus(tenant_id);

    let topic = bus.topic("scan.completed").unwrap();

    assert_eq!(topic.as_str(), format!("{}scan.completed", bus.namespace()));
}

#[test]
fn bucket_key_builds_a_key_inside_its_own_namespace() {
    let tenant_id = Uuid::from_u128(42);
    let (_inner, bus) = tenant_bus(tenant_id);

    let key = bus.bucket_key("webhook-deliver").unwrap();

    assert_eq!(key.as_str(), format!("{}webhook-deliver", bus.namespace()));
}

#[test]
fn tenant_wake_bus_topic_refuses_an_empty_name() {
    let (_inner, bus) = tenant_bus(Uuid::from_u128(42));

    assert!(matches!(
        bus.topic(""),
        Err(WakeBusError::InvalidTopic { .. })
    ));
}

#[test]
fn tenant_wake_bus_bucket_key_refuses_an_empty_name() {
    let (_inner, bus) = tenant_bus(Uuid::from_u128(42));

    assert!(matches!(
        bus.bucket_key(""),
        Err(WakeBusError::InvalidBucketKey { .. })
    ));
}

#[test]
fn tenant_wake_namespace_is_tenant_colon_hyphenated_id_colon() {
    let tenant_id = Uuid::from_u128(9);

    assert_eq!(
        tenant_wake_namespace(tenant_id),
        format!("tenant:{}:", tenant_id.as_hyphenated())
    );
}
