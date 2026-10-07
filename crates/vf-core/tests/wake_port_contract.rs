//! Port-contract tests for `vf_core::ports::wake` — VFL-314, the F3a slice
//! of T4 (VFL-44) for F3a (VFL-310).
//!
//! Written against `f9af174` on `jorge/f3a-ports`; see
//! `artifact_port_contract.rs`'s header for the revision and split note
//! shared by both files in this pack.
//!
//! Scope is VFL-310's criterion 2 for the wake side: topic, bucket-key and
//! rate validation behave as their rustdoc states, and the two wake-only
//! byte caps the criterion names (`MAX_WAKE_NAME_BYTES`,
//! `MAX_WAKE_PAYLOAD_BYTES`) hold the values their rustdoc states. There is
//! no concrete `WakeBus` yet — that is F3c (VFL-312) — so delivery semantics
//! (publish/subscribe, `Lagged` vs `Closed`, token accounting against a real
//! bucket) need a real adapter and are out of scope here; they belong to
//! that slice's conformance pack. Nothing here needs an async driver: every
//! type under test is a synchronous validator.

use std::time::Duration;

use vf_core::ports::{
    BucketKey, MAX_WAKE_NAME_BYTES, MAX_WAKE_PAYLOAD_BYTES, TokenRate, Topic, WakeBusError,
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
