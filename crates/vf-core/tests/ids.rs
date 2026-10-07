//! T1a (VFL-112, covers C1 / VFL-14): identity newtypes of `vf-core::ids`.
//!
//! Architecture §A3.1, TDD §6.2 (VFL-8#document-architecture). §6.2 fixes two
//! families and one rule per family:
//!
//! * The fourteen UUID identities (plus `BillingPeriodId`, the §A3.1 addition
//!   the skeleton documents) are newtypes over `Uuid`. `Display`, `FromStr`
//!   and serde are all the plain UUID string, and none converts into another
//!   — passing one identity where a different one is expected is a compile
//!   error, not a runtime defect, so there is nothing to test for that half
//!   of §6.2; this file tests the half that *does* run: the string forms
//!   agree with each other and a malformed string is rejected with the
//!   newtype's own name attached.
//! * `ScanFingerprint`, `CandidateKey`, `RequestKey` and `NodeId` are
//!   newtypes over `String`, stored and round-tripped **unchanged** — no
//!   normalization, case-folding or trimming.
//!
//! `tests/support` supplies the serde round trip: `vf-core` has no
//! `serde_json` dev-dependency available to a test-only pull request (see
//! `support`'s module docs).

mod support;

use uuid::Uuid;
use vf_core::ids::{
    AuthorizationBasisId, BillingPeriodId, CandidateKey, FindingId, FpDecisionId, IdParseError,
    NodeId, OutboxId, PipelineRunId, ProjectId, ReportId, RequestKey, ScanFingerprint, ScanId,
    ScheduleId, TargetId, TenantId, UserId, VerificationRunId, WorkUnitId,
};

// ---------------------------------------------------------------------------
// UUID identities
// ---------------------------------------------------------------------------

/// Generates one module of tests for one UUID-backed id newtype.
///
/// A macro rather than a generic function: `from_uuid`/`as_uuid`/`into_uuid`
/// and `TYPE_NAME` are inherent `impl` items with no shared trait (§6.2
/// deliberately gives these types no common conversion trait), so a generic
/// helper could not name them. One `#[test]` per property, so the lane gate's
/// erosion check (`ci/lane-gate.sh erosion`) counts each id type's coverage
/// separately.
macro_rules! uuid_id_tests {
    ($mod_name:ident, $ty:ty, $type_name:literal) => {
        mod $mod_name {
            use super::*;

            const SAMPLE: &str = "01890a5d-ac96-774b-bf71-ca4fde3b9c2b";

            fn sample() -> $ty {
                <$ty>::from_uuid(Uuid::parse_str(SAMPLE).unwrap())
            }

            #[test]
            fn type_name_is_the_rust_identifier() {
                assert_eq!(<$ty>::TYPE_NAME, $type_name);
            }

            #[test]
            fn from_uuid_as_uuid_into_uuid_agree() {
                let uuid = Uuid::parse_str(SAMPLE).unwrap();
                let id = <$ty>::from_uuid(uuid);
                assert_eq!(*id.as_uuid(), uuid);
                assert_eq!(id.into_uuid(), uuid);
            }

            #[test]
            fn from_and_into_uuid_conversions_agree_with_from_uuid() {
                let uuid = Uuid::parse_str(SAMPLE).unwrap();
                let via_from: $ty = uuid.into();
                assert_eq!(via_from, <$ty>::from_uuid(uuid));
                let back: Uuid = via_from.into();
                assert_eq!(back, uuid);
            }

            #[test]
            fn display_is_the_plain_uuid_string() {
                assert_eq!(sample().to_string(), SAMPLE);
            }

            #[test]
            fn from_str_round_trips_through_display() {
                let id = sample();
                let parsed: $ty = id.to_string().parse().unwrap();
                assert_eq!(parsed, id);
            }

            #[test]
            fn from_str_rejects_a_non_uuid_and_names_this_type() {
                let err: IdParseError = "not-a-uuid".parse::<$ty>().unwrap_err();
                assert_eq!(err.type_name, $type_name);
                assert_eq!(err.value, "not-a-uuid");
            }

            #[test]
            fn from_str_rejects_the_empty_string() {
                assert!("".parse::<$ty>().is_err());
            }

            #[test]
            fn serde_round_trips_as_the_plain_uuid_string() {
                let id = sample();
                let value = support::to_value(&id);
                assert_eq!(value.as_str(), Some(SAMPLE));
                assert_eq!(support::from_value::<$ty>(&value), id);
            }

            #[test]
            fn distinct_values_are_not_equal() {
                let other: $ty = <$ty>::from_uuid(
                    Uuid::parse_str("00000000-0000-7000-8000-000000000000").unwrap(),
                );
                assert_ne!(sample(), other);
            }
        }
    };
}

uuid_id_tests!(tenant_id, TenantId, "TenantId");
uuid_id_tests!(user_id, UserId, "UserId");
uuid_id_tests!(target_id, TargetId, "TargetId");
uuid_id_tests!(project_id, ProjectId, "ProjectId");
uuid_id_tests!(
    authorization_basis_id,
    AuthorizationBasisId,
    "AuthorizationBasisId"
);
uuid_id_tests!(pipeline_run_id, PipelineRunId, "PipelineRunId");
uuid_id_tests!(work_unit_id, WorkUnitId, "WorkUnitId");
uuid_id_tests!(scan_id, ScanId, "ScanId");
uuid_id_tests!(finding_id, FindingId, "FindingId");
uuid_id_tests!(fp_decision_id, FpDecisionId, "FpDecisionId");
uuid_id_tests!(verification_run_id, VerificationRunId, "VerificationRunId");
uuid_id_tests!(report_id, ReportId, "ReportId");
uuid_id_tests!(schedule_id, ScheduleId, "ScheduleId");
uuid_id_tests!(outbox_id, OutboxId, "OutboxId");
uuid_id_tests!(billing_period_id, BillingPeriodId, "BillingPeriodId");

// ---------------------------------------------------------------------------
// String identities — stored and round-tripped unchanged, never derived
// ---------------------------------------------------------------------------

macro_rules! string_id_tests {
    ($mod_name:ident, $ty:ty, $type_name:literal) => {
        mod $mod_name {
            use super::*;

            #[test]
            fn type_name_is_the_rust_identifier() {
                assert_eq!(<$ty>::TYPE_NAME, $type_name);
            }

            #[test]
            fn new_and_as_str_store_the_value_unchanged() {
                let id = <$ty>::new("Mixed-Case.Example_123");
                assert_eq!(id.as_str(), "Mixed-Case.Example_123");
            }

            #[test]
            fn stores_leading_and_trailing_whitespace_unchanged() {
                let id = <$ty>::new("  padded  ");
                assert_eq!(id.as_str(), "  padded  ");
            }

            #[test]
            fn stores_the_empty_string() {
                let id = <$ty>::new("");
                assert_eq!(id.as_str(), "");
            }

            #[test]
            fn into_string_unwraps_to_the_same_value() {
                let id = <$ty>::new("abc-DEF");
                assert_eq!(id.into_string(), "abc-DEF");
            }

            #[test]
            fn display_is_the_stored_value() {
                let id = <$ty>::new("abc-DEF");
                assert_eq!(id.to_string(), "abc-DEF");
            }

            #[test]
            fn from_str_is_infallible_and_round_trips_exactly() {
                let parsed: $ty = "Weird Value/with:punctuation".parse().unwrap();
                assert_eq!(parsed.as_str(), "Weird Value/with:punctuation");
            }

            #[test]
            fn serde_round_trips_the_value_unchanged() {
                let id = <$ty>::new("value-with-DASHES_and_underscores");
                let value = support::to_value(&id);
                assert_eq!(value.as_str(), Some("value-with-DASHES_and_underscores"));
                assert_eq!(support::from_value::<$ty>(&value), id);
            }

            #[test]
            fn equality_is_byte_for_byte() {
                assert_ne!(<$ty>::new("example.com"), <$ty>::new("Example.com"));
            }
        }
    };
}

string_id_tests!(scan_fingerprint, ScanFingerprint, "ScanFingerprint");
string_id_tests!(candidate_key, CandidateKey, "CandidateKey");
string_id_tests!(request_key, RequestKey, "RequestKey");
string_id_tests!(node_id, NodeId, "NodeId");
