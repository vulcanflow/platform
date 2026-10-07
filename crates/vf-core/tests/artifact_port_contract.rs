//! Port-contract tests for `vf_core::ports::artifact` — VFL-314 and its
//! rev2 follow-up VFL-355, the F3a slices of T4 (VFL-44) for F3a (VFL-310).
//!
//! Rebased onto `426b848` on `jorge/f3a-ports` (one commit on `f9af174`,
//! not pushed), F3a rev2 (VFL-310). That revision adds the required
//! `ArtifactStore::get_stream` method and the `ArtifactReader` trait it
//! returns, and changes `delete` of an absent key from `NotFound` to
//! `Ok(())` (Cortana's ruling on card `b4a430a9`, VFL-339).
//!
//! Scope is VFL-310's criteria exactly: (1) `TenantArtifactStore` refuses a
//! key or list prefix outside `{tenant_id}/` before any I/O, rewriting a
//! root list prefix to the tenant prefix rather than refusing it, and this
//! now covers `get_stream` as well as `put`/`get`/`head`/`list`/`delete`;
//! (2) key, prefix and digest validation behave as their rustdoc states.
//! There is no concrete `ArtifactStore` adapter yet — that is F3b
//! (VFL-311) — so criterion 1 is tested against a hand-written inner store
//! that panics if any method it must not reach is ever called, which turns
//! a refusal bug into a test failure directly rather than one inferred from
//! an I/O side effect. Adapter conformance (including `get_stream`'s
//! `NotFound`/`TooLarge`-before-first-chunk and finished-reader-stays-
//! finished behavior, contract items 8-9) and production refusal are out of
//! scope here by the issue's own words and belong to the F3b/F3c slices.
//!
//! `vf-core` carries no async runtime (§A1.3): `support::block_on` drives
//! the returned `PortFuture`s by polling once, which every future here needs
//! only once (see that module's doc comment).

mod support;

use std::sync::{Arc, Mutex};

use uuid::Uuid;
use vf_core::ports::{
    ArtifactBody, ArtifactContent, ArtifactKey, ArtifactMeta, ArtifactPrefix, ArtifactReader,
    ArtifactSource, ArtifactStore, ArtifactStoreError, DigestParseError, MAX_ARTIFACT_KEY_BYTES,
    PortFuture, PutReceipt, Sha256Digest, TenantArtifactStore,
};

// ---------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------

/// An inner `ArtifactStore` that panics on every method except `list` and
/// `get_stream`, which instead record what they were called with.
///
/// `TenantArtifactStore`'s rustdoc promises refusal "before any I/O"; wiring
/// this in as the inner store turns a refusal bug straight into a test
/// failure (the panic) instead of something that would need an I/O
/// assertion to catch. `list` and `get_stream` cannot panic unconditionally
/// because each also has a non-refused path under test (a rewritten root
/// list prefix; an in-tenant key forwarded to `get_stream`), so for those two
/// the recorded calls stand in for the panic: empty means never reached,
/// non-empty names exactly what got through.
#[derive(Default)]
struct PanicUnlessListed {
    list_calls: Mutex<Vec<ArtifactPrefix>>,
    get_stream_calls: Mutex<Vec<ArtifactKey>>,
}

/// A canned `ArtifactReader` with no chunks, returned by a forwarded
/// `PanicUnlessListed::get_stream` call so the caller has something to hold.
struct EmptyReader(ArtifactMeta);

impl ArtifactReader for EmptyReader {
    fn meta(&self) -> &ArtifactMeta {
        &self.0
    }

    fn next_chunk(&mut self) -> PortFuture<'_, Result<Option<Vec<u8>>, ArtifactStoreError>> {
        Box::pin(std::future::ready(Ok(None)))
    }
}

impl ArtifactStore for PanicUnlessListed {
    fn put(
        &self,
        _key: &ArtifactKey,
        body: ArtifactBody,
        _sha256: Sha256Digest,
    ) -> PortFuture<'_, Result<PutReceipt, ArtifactStoreError>> {
        drop(body);
        panic!("TenantArtifactStore must refuse before the inner store's put is ever called");
    }

    fn get(
        &self,
        _key: &ArtifactKey,
    ) -> PortFuture<'_, Result<ArtifactContent, ArtifactStoreError>> {
        panic!("TenantArtifactStore must refuse before the inner store's get is ever called");
    }

    fn get_stream(
        &self,
        key: &ArtifactKey,
    ) -> PortFuture<'_, Result<Box<dyn ArtifactReader>, ArtifactStoreError>> {
        self.get_stream_calls.lock().unwrap().push(key.clone());
        let meta = ArtifactMeta {
            key: key.clone(),
            size: 0,
            last_modified: None,
            e_tag: None,
        };
        let reader: Box<dyn ArtifactReader> = Box::new(EmptyReader(meta));
        Box::pin(std::future::ready(Ok(reader)))
    }

    fn head(&self, _key: &ArtifactKey) -> PortFuture<'_, Result<ArtifactMeta, ArtifactStoreError>> {
        panic!("TenantArtifactStore must refuse before the inner store's head is ever called");
    }

    fn list(
        &self,
        prefix: &ArtifactPrefix,
    ) -> PortFuture<'_, Result<Vec<ArtifactMeta>, ArtifactStoreError>> {
        self.list_calls.lock().unwrap().push(prefix.clone());
        Box::pin(std::future::ready(Ok(Vec::new())))
    }

    fn delete(&self, _key: &ArtifactKey) -> PortFuture<'_, Result<(), ArtifactStoreError>> {
        panic!("TenantArtifactStore must refuse before the inner store's delete is ever called");
    }
}

/// An `ArtifactSource` that panics if it is ever polled, used to prove a
/// refused `put`'s body is dropped unread rather than consumed.
struct PanicIfPolled;

impl ArtifactSource for PanicIfPolled {
    fn next_chunk(&mut self) -> PortFuture<'_, Result<Option<Vec<u8>>, ArtifactStoreError>> {
        panic!(
            "ArtifactSource::next_chunk must never be polled when \
             TenantArtifactStore refuses first"
        );
    }
}

fn tenant_store(tenant_id: Uuid) -> (Arc<PanicUnlessListed>, TenantArtifactStore) {
    let inner = Arc::new(PanicUnlessListed::default());
    let store = TenantArtifactStore::new(Arc::clone(&inner) as Arc<dyn ArtifactStore>, tenant_id);
    (inner, store)
}

fn zero_digest() -> Sha256Digest {
    Sha256Digest::from_bytes([0u8; 32])
}

// ---------------------------------------------------------------------------
// TenantArtifactStore: refusal before any I/O (criterion 1)
// ---------------------------------------------------------------------------

#[test]
fn refuses_put_outside_its_prefix_before_touching_the_inner_store_or_the_body() {
    let tenant_id = Uuid::from_u128(1);
    let other_tenant_id = Uuid::from_u128(2);
    let (_inner, store) = tenant_store(tenant_id);
    let foreign_key = ArtifactPrefix::for_tenant(other_tenant_id)
        .join("findings.json")
        .unwrap();

    let result = support::block_on(store.put(
        &foreign_key,
        ArtifactBody::stream(PanicIfPolled),
        zero_digest(),
    ));

    assert!(matches!(
        result,
        Err(ArtifactStoreError::OutsideTenantPrefix { .. })
    ));
}

#[test]
fn refuses_get_outside_its_prefix_before_touching_the_inner_store() {
    let tenant_id = Uuid::from_u128(1);
    let other_tenant_id = Uuid::from_u128(2);
    let (_inner, store) = tenant_store(tenant_id);
    let foreign_key = ArtifactPrefix::for_tenant(other_tenant_id)
        .join("findings.json")
        .unwrap();

    let result = support::block_on(store.get(&foreign_key));

    assert!(matches!(
        result,
        Err(ArtifactStoreError::OutsideTenantPrefix { .. })
    ));
}

#[test]
fn refuses_head_outside_its_prefix_before_touching_the_inner_store() {
    let tenant_id = Uuid::from_u128(1);
    let other_tenant_id = Uuid::from_u128(2);
    let (_inner, store) = tenant_store(tenant_id);
    let foreign_key = ArtifactPrefix::for_tenant(other_tenant_id)
        .join("findings.json")
        .unwrap();

    let result = support::block_on(store.head(&foreign_key));

    assert!(matches!(
        result,
        Err(ArtifactStoreError::OutsideTenantPrefix { .. })
    ));
}

#[test]
fn refuses_delete_outside_its_prefix_before_touching_the_inner_store() {
    let tenant_id = Uuid::from_u128(1);
    let other_tenant_id = Uuid::from_u128(2);
    let (_inner, store) = tenant_store(tenant_id);
    let foreign_key = ArtifactPrefix::for_tenant(other_tenant_id)
        .join("findings.json")
        .unwrap();

    let result = support::block_on(store.delete(&foreign_key));

    assert!(matches!(
        result,
        Err(ArtifactStoreError::OutsideTenantPrefix { .. })
    ));
}

#[test]
fn refuses_get_stream_outside_its_prefix_before_touching_the_inner_store() {
    let tenant_id = Uuid::from_u128(1);
    let other_tenant_id = Uuid::from_u128(2);
    let (inner, store) = tenant_store(tenant_id);
    let foreign_key = ArtifactPrefix::for_tenant(other_tenant_id)
        .join("findings.json")
        .unwrap();

    let result = support::block_on(store.get_stream(&foreign_key));

    assert!(matches!(
        result,
        Err(ArtifactStoreError::OutsideTenantPrefix { .. })
    ));
    assert!(
        inner.get_stream_calls.lock().unwrap().is_empty(),
        "a refused get_stream must never reach the inner store"
    );
}

#[test]
fn forwards_an_in_tenant_key_to_the_inner_stores_get_stream() {
    let tenant_id = Uuid::from_u128(1);
    let (inner, store) = tenant_store(tenant_id);
    let key = store.key("findings.json").unwrap();

    let result = support::block_on(store.get_stream(&key));

    assert!(
        result.is_ok(),
        "an in-tenant key must be forwarded to the inner store, not refused: {result:?}"
    );
    assert_eq!(
        inner.get_stream_calls.lock().unwrap().as_slice(),
        std::slice::from_ref(&key)
    );
}

#[test]
fn refuses_a_list_prefix_outside_its_prefix_before_touching_the_inner_store() {
    let tenant_id = Uuid::from_u128(1);
    let other_tenant_id = Uuid::from_u128(2);
    let (inner, store) = tenant_store(tenant_id);
    let foreign_prefix = ArtifactPrefix::for_tenant(other_tenant_id);

    let result = support::block_on(store.list(&foreign_prefix));

    assert!(matches!(
        result,
        Err(ArtifactStoreError::OutsideTenantPrefix { .. })
    ));
    assert!(
        inner.list_calls.lock().unwrap().is_empty(),
        "a refused list must never reach the inner store"
    );
}

#[test]
fn rewrites_a_root_list_prefix_to_its_own_prefix_instead_of_refusing_it() {
    let tenant_id = Uuid::from_u128(1);
    let (inner, store) = tenant_store(tenant_id);

    let result = support::block_on(store.list(&ArtifactPrefix::root()));

    assert!(
        result.is_ok(),
        "a root list on a tenant-scoped store must mean \"everything I can \
         see\", not be refused: {result:?}"
    );
    assert_eq!(
        inner.list_calls.lock().unwrap().as_slice(),
        std::slice::from_ref(store.prefix())
    );
}

#[test]
fn accepts_a_list_prefix_at_or_below_its_own_prefix_unchanged() {
    let tenant_id = Uuid::from_u128(1);
    let (inner, store) = tenant_store(tenant_id);
    let own_subprefix = store.prefix().child("run-1").unwrap();

    let result = support::block_on(store.list(&own_subprefix));

    assert!(
        result.is_ok(),
        "a sub-prefix of the tenant's own prefix must not be refused: {result:?}"
    );
    assert_eq!(
        inner.list_calls.lock().unwrap().as_slice(),
        std::slice::from_ref(&own_subprefix)
    );
}

#[test]
fn key_builds_a_key_inside_its_own_prefix() {
    let tenant_id = Uuid::from_u128(42);
    let (_inner, store) = tenant_store(tenant_id);

    let key = store
        .key("run-1/findings.json")
        .expect("a relative path with no `..` is a legal key inside the tenant prefix");

    assert!(store.prefix().contains(&key));
    assert_eq!(
        key.as_str(),
        format!("{}run-1/findings.json", store.prefix().as_str())
    );
}

#[test]
fn key_refuses_a_relative_path_that_escapes_via_dot_dot() {
    let tenant_id = Uuid::from_u128(42);
    let (_inner, store) = tenant_store(tenant_id);

    let result = store.key("../other-tenant/findings.json");

    assert!(matches!(result, Err(ArtifactStoreError::InvalidKey { .. })));
}

#[test]
fn exposes_the_tenant_id_and_prefix_it_was_constructed_with() {
    let tenant_id = Uuid::from_u128(7);
    let (_inner, store) = tenant_store(tenant_id);

    assert_eq!(store.tenant_id(), tenant_id);
    assert_eq!(store.prefix(), &ArtifactPrefix::for_tenant(tenant_id));
}

// ---------------------------------------------------------------------------
// ArtifactKey validation (criterion 2)
// ---------------------------------------------------------------------------

#[test]
fn artifact_key_rejects_empty() {
    assert!(matches!(
        ArtifactKey::parse(""),
        Err(ArtifactStoreError::InvalidKey { .. })
    ));
}

#[test]
fn artifact_key_accepts_exactly_the_byte_cap() {
    let raw = "a".repeat(MAX_ARTIFACT_KEY_BYTES);
    assert!(ArtifactKey::parse(&raw).is_ok());
}

#[test]
fn artifact_key_rejects_one_byte_over_the_cap() {
    let raw = "a".repeat(MAX_ARTIFACT_KEY_BYTES + 1);
    assert!(matches!(
        ArtifactKey::parse(&raw),
        Err(ArtifactStoreError::InvalidKey { .. })
    ));
}

#[test]
fn artifact_key_rejects_an_absolute_path() {
    assert!(matches!(
        ArtifactKey::parse("/tenant-a/findings.json"),
        Err(ArtifactStoreError::InvalidKey { .. })
    ));
}

#[test]
fn artifact_key_rejects_a_trailing_slash() {
    assert!(matches!(
        ArtifactKey::parse("tenant-a/findings/"),
        Err(ArtifactStoreError::InvalidKey { .. })
    ));
}

#[test]
fn artifact_key_rejects_a_byte_outside_the_allowed_alphabet() {
    assert!(matches!(
        ArtifactKey::parse("tenant-a/report?.json"),
        Err(ArtifactStoreError::InvalidKey { .. })
    ));
}

#[test]
fn artifact_key_rejects_an_empty_segment() {
    assert!(matches!(
        ArtifactKey::parse("tenant-a//findings.json"),
        Err(ArtifactStoreError::InvalidKey { .. })
    ));
}

#[test]
fn artifact_key_rejects_a_dot_segment() {
    assert!(matches!(
        ArtifactKey::parse("tenant-a/./findings.json"),
        Err(ArtifactStoreError::InvalidKey { .. })
    ));
}

#[test]
fn artifact_key_rejects_a_dot_dot_segment() {
    assert!(matches!(
        ArtifactKey::parse("tenant-a/../findings.json"),
        Err(ArtifactStoreError::InvalidKey { .. })
    ));
}

#[test]
fn artifact_key_accepts_every_character_in_the_allowed_alphabet() {
    let raw = "tenant-A9/run_1.v2/report-final.JSON";
    assert_eq!(ArtifactKey::parse(raw).unwrap().as_str(), raw);
}

#[test]
fn artifact_key_first_segment_is_the_tenant_segment() {
    let key = ArtifactKey::parse("tenant-a/run-1/findings.json").unwrap();
    assert_eq!(key.first_segment(), "tenant-a");
}

#[test]
fn artifact_key_parent_prefix_keeps_everything_up_to_the_last_slash() {
    let key = ArtifactKey::parse("tenant-a/run-1/findings.json").unwrap();
    assert_eq!(key.parent_prefix().as_str(), "tenant-a/run-1/");
}

#[test]
fn artifact_key_parent_prefix_of_a_single_segment_key_is_root() {
    let key = ArtifactKey::parse("findings.json").unwrap();
    assert_eq!(key.parent_prefix(), ArtifactPrefix::root());
}

#[test]
fn artifact_key_try_from_string_matches_parse_and_into_string_round_trips() {
    let key = ArtifactKey::try_from("tenant-a/findings.json".to_string()).unwrap();
    assert_eq!(key.as_str(), "tenant-a/findings.json");
    let round_tripped: String = key.into();
    assert_eq!(round_tripped, "tenant-a/findings.json");

    assert!(matches!(
        ArtifactKey::try_from("/absolute".to_string()),
        Err(ArtifactStoreError::InvalidKey { .. })
    ));
}

// ---------------------------------------------------------------------------
// ArtifactPrefix validation (criterion 2)
// ---------------------------------------------------------------------------

#[test]
fn artifact_prefix_empty_string_is_root() {
    let prefix = ArtifactPrefix::parse("").unwrap();
    assert!(prefix.is_root());
    assert_eq!(prefix.as_str(), "");
    assert_eq!(prefix, ArtifactPrefix::root());
}

#[test]
fn artifact_prefix_appends_a_missing_trailing_slash() {
    let prefix = ArtifactPrefix::parse("tenant-a").unwrap();
    assert_eq!(prefix.as_str(), "tenant-a/");
}

#[test]
fn artifact_prefix_parse_is_idempotent_on_an_already_trailing_slash() {
    let prefix = ArtifactPrefix::parse("tenant-a/").unwrap();
    assert_eq!(prefix.as_str(), "tenant-a/");
}

#[test]
fn artifact_prefix_rejects_the_same_malformed_input_as_a_key() {
    assert!(matches!(
        ArtifactPrefix::parse("tenant-a/../tenant-b"),
        Err(ArtifactStoreError::InvalidPrefix { .. })
    ));
}

#[test]
fn artifact_prefix_for_tenant_is_the_hyphenated_uuid_with_a_trailing_slash() {
    let tenant_id = Uuid::from_u128(5);
    let prefix = ArtifactPrefix::for_tenant(tenant_id);
    assert_eq!(prefix.as_str(), format!("{}/", tenant_id.as_hyphenated()));
    assert!(!prefix.is_root());
}

#[test]
fn artifact_prefix_root_contains_every_key() {
    let key = ArtifactKey::parse("tenant-a/findings.json").unwrap();
    assert!(ArtifactPrefix::root().contains(&key));
}

#[test]
fn artifact_prefix_contains_is_a_segment_boundary_test_not_a_string_prefix_test() {
    let prefix = ArtifactPrefix::parse("tenant-a").unwrap();

    let sibling = ArtifactKey::parse("tenant-abcd/findings.json").unwrap();
    assert!(
        !prefix.contains(&sibling),
        "a segment-boundary prefix must not match a sibling tenant whose \
         name merely starts with the same characters"
    );

    let own = ArtifactKey::parse("tenant-a/findings.json").unwrap();
    assert!(prefix.contains(&own));
}

#[test]
fn artifact_prefix_join_builds_a_key_and_refuses_traversal() {
    let prefix = ArtifactPrefix::parse("tenant-a").unwrap();

    let key = prefix.join("run-1/findings.json").unwrap();
    assert_eq!(key.as_str(), "tenant-a/run-1/findings.json");

    assert!(matches!(
        prefix.join("../tenant-b/findings.json"),
        Err(ArtifactStoreError::InvalidKey { .. })
    ));
}

#[test]
fn artifact_prefix_child_builds_a_prefix_and_refuses_traversal() {
    let prefix = ArtifactPrefix::parse("tenant-a").unwrap();

    let child = prefix.child("run-1").unwrap();
    assert_eq!(child.as_str(), "tenant-a/run-1/");

    assert!(matches!(
        prefix.child(".."),
        Err(ArtifactStoreError::InvalidPrefix { .. })
    ));
}

// ---------------------------------------------------------------------------
// Sha256Digest validation (criterion 2)
// ---------------------------------------------------------------------------

#[test]
fn sha256_digest_round_trips_through_hex() {
    let bytes = [7u8; 32];
    let digest = Sha256Digest::from_bytes(bytes);

    let hex = digest.to_hex();
    assert_eq!(hex.len(), 64);

    let parsed = Sha256Digest::parse_hex(&hex).unwrap();
    assert_eq!(parsed, digest);
    assert_eq!(parsed.as_bytes(), &bytes);
}

#[test]
fn sha256_digest_parse_hex_accepts_uppercase_and_normalizes_to_lowercase() {
    let lower = "ab".repeat(32);
    let upper = lower.to_uppercase();

    let digest = Sha256Digest::parse_hex(&upper).unwrap();
    assert_eq!(digest.to_hex(), lower);
}

#[test]
fn sha256_digest_parse_hex_rejects_the_wrong_length() {
    assert!(matches!(
        Sha256Digest::parse_hex("ab"),
        Err(DigestParseError::Length { len: 2 })
    ));
}

#[test]
fn sha256_digest_parse_hex_rejects_non_hex_characters() {
    let not_hex = "g".repeat(64);
    assert!(matches!(
        Sha256Digest::parse_hex(&not_hex),
        Err(DigestParseError::NotHex)
    ));
}

#[test]
fn sha256_digest_try_from_string_uses_digest_parse_error() {
    assert!(matches!(
        Sha256Digest::try_from("too-short".to_string()),
        Err(DigestParseError::Length { len: 9 })
    ));
}
