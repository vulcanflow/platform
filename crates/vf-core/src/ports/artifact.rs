//! The [`ArtifactStore`] port and its value types (architecture §A6.1, task
//! F3).
//!
//! Scanner output is the one payload that crosses from the execution plane to
//! the control plane (§A1.1), and it is written by a hostile workload. So the
//! types in this module, not the adapters, carry the rules: an
//! [`ArtifactKey`] is built from an allow-listed alphabet and cannot be a
//! traversal or an absolute path, an
//! [`ArtifactPrefix`] always ends at a segment boundary so it cannot widen
//! into a sibling tenant, and [`TenantArtifactStore`] refuses a key outside
//! `{tenant_id}/` before the call reaches an adapter (§A7-2, §A7-7).
//!
//! Every artifact write carries the caller's declared SHA-256
//! ([`Sha256Digest`]) and every whole-object read returns the digest the
//! adapter computed: §A7-6 requires the checksum to be verified against the
//! trusted work record before a parse. A streamed read returns no digest;
//! its caller hashes as it reads (see [`ArtifactReader`]).
//!
//! Implemented in `vf-db::adapters` (task F3): `S3ArtifactStore`,
//! `LocalFsArtifactStore`, `InMemoryArtifactStore`.

use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::PortFuture;

/// Longest accepted [`ArtifactKey`], in bytes.
///
/// [`ArtifactKey::parse`] enforces two bounds: 1024 bytes for the whole key,
/// which is the S3 object-key limit, and 255 bytes for each `/`-separated
/// segment, which is `NAME_MAX` for one path component on ext4, xfs and btrfs.
/// Together they make a key this crate accepts storable on every backend in
/// §A6.1 (Ceph RGW, RustFS, MinIO) and on a local filesystem under a short
/// root. The one filesystem divergence left is contract item 7 on
/// [`ArtifactStore`].
pub const MAX_ARTIFACT_KEY_BYTES: usize = 1024;

/// Longest accepted [`ArtifactKey`] segment, in bytes: the per-segment bound
/// the [`MAX_ARTIFACT_KEY_BYTES`] rustdoc states.
///
/// Public rustdoc cannot link to a private item, so three doc sites spell this
/// value out as `255`: [`MAX_ARTIFACT_KEY_BYTES`], [`ArtifactKey`] and the
/// `# Errors` section of [`ArtifactKey::parse`]. Change them with it.
const MAX_ARTIFACT_KEY_SEGMENT_BYTES: usize = 255;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Everything an [`ArtifactStore`] call can refuse or fail with.
///
/// The variants are split so a caller can tell a *refusal* (`InvalidKey`,
/// `OutsideTenantPrefix`, `ChecksumMismatch`, `TooLarge`) from a *backend
/// failure* (`Backend`). §A7-6 turns the first group into a platform outcome
/// with a visible reason and never into a target outcome.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ArtifactStoreError {
    /// The key is not a well-formed artifact key. Never reached storage.
    #[error("artifact key is invalid: {reason}")]
    InvalidKey {
        /// Why the key was refused. Never echoes the key itself, which is
        /// attacker-influenced on the ingest path.
        reason: String,
    },

    /// The prefix is not a well-formed artifact prefix. Never reached storage.
    #[error("artifact prefix is invalid: {reason}")]
    InvalidPrefix {
        /// Why the prefix was refused.
        reason: String,
    },

    /// The key is outside the tenant prefix the store is scoped to. Refused by
    /// [`TenantArtifactStore`] before any I/O (§A7-7).
    #[error("artifact key is outside the tenant prefix {prefix}")]
    OutsideTenantPrefix {
        /// The prefix the store is scoped to, which is `{tenant_id}/` and
        /// therefore not a secret.
        prefix: ArtifactPrefix,
    },

    /// No object is stored under this key. Returned by `head`, `get` and
    /// `get_stream`, never by `delete` (contract item 4 on [`ArtifactStore`]).
    #[error("artifact not found: {key}")]
    NotFound {
        /// The key that was looked up.
        key: ArtifactKey,
    },

    /// Reserved: no method returns it yet. `put` is last-writer-wins
    /// (contract item 2 on [`ArtifactStore`]); a conditional put, if one is
    /// ever added, is what would raise it.
    #[error("artifact already exists: {key}")]
    AlreadyExists {
        /// The key that was written.
        key: ArtifactKey,
    },

    /// The bytes do not hash to the digest the caller declared. On `put` the
    /// object is not stored; on `get` the bytes are not returned.
    #[error("artifact {key} checksum mismatch: declared {declared}, computed {computed}")]
    ChecksumMismatch {
        /// The key involved.
        key: ArtifactKey,
        /// The digest the caller declared.
        declared: Sha256Digest,
        /// The digest of the bytes actually seen.
        computed: Sha256Digest,
    },

    /// The object is larger than the adapter's read or write limit. §A7-6
    /// requires every artifact path to be size-bounded.
    #[error("artifact {key} is {size} bytes, over the {limit} byte limit")]
    TooLarge {
        /// The key involved.
        key: ArtifactKey,
        /// The size seen, or the size the backend reported.
        size: u64,
        /// The configured limit.
        limit: u64,
    },

    /// The backing service failed. Carries a rendered message rather than the
    /// backend error type, so `vf-core` stays free of `object_store`.
    #[error("artifact store backend error: {message}")]
    Backend {
        /// Rendered backend error chain.
        message: String,
    },

    /// This adapter does not implement the operation. Used by the F3 API
    /// skeleton, and afterwards by an adapter that genuinely cannot offer an
    /// operation.
    #[error("artifact store operation `{operation}` is not supported by this adapter")]
    Unsupported {
        /// The port method that was called.
        operation: &'static str,
    },
}

// ---------------------------------------------------------------------------
// Keys and prefixes
// ---------------------------------------------------------------------------

/// A validated artifact object key, relative to the bucket root.
///
/// Keys are `/`-separated and are built by the platform from the §A3.6 layout
/// `{tenant_id}/{pipeline_run_id}/{scan_fingerprint}/{name}`. The constructor
/// is the only way to make one. Each segment is drawn from the allow-list
/// `[A-Za-z0-9._-]`, which every §A6.1 backend stores verbatim (no percent
/// encoding, no case folding, no reserved filename characters), and a segment
/// may not be empty, `.`, `..` or longer than 255 bytes. So a key cannot be
/// absolute, cannot traverse, and cannot mean a different object on a
/// different backend; that is what keeps [`ArtifactPrefix::contains`] a sound
/// containment test rather than a string guess.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ArtifactKey(String);

impl ArtifactKey {
    /// Validates `raw` and returns the key.
    ///
    /// # Errors
    ///
    /// [`ArtifactStoreError::InvalidKey`] when `raw` is empty, longer than
    /// [`MAX_ARTIFACT_KEY_BYTES`], absolute, ends in `/`, has an empty, `.` or
    /// `..` segment, has a segment longer than 255 bytes, or contains a byte
    /// outside `[A-Za-z0-9._/-]`.
    pub fn parse(raw: &str) -> Result<Self, ArtifactStoreError> {
        let invalid = |reason: &str| ArtifactStoreError::InvalidKey {
            reason: reason.to_owned(),
        };

        if raw.is_empty() {
            return Err(invalid("empty"));
        }
        if raw.len() > MAX_ARTIFACT_KEY_BYTES {
            return Err(invalid(&format!(
                "longer than {MAX_ARTIFACT_KEY_BYTES} bytes"
            )));
        }
        if raw.starts_with('/') {
            return Err(invalid("absolute: starts with `/`"));
        }
        if raw.ends_with('/') {
            return Err(invalid("ends with `/`, which names a prefix not an object"));
        }
        // The offending byte is attacker-influenced on the ingest path, so
        // only the rule is reported, never the byte.
        if !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
        {
            return Err(invalid("contains a byte outside [A-Za-z0-9._/-]"));
        }
        for segment in raw.split('/') {
            match segment {
                "" => return Err(invalid("contains an empty path segment")),
                "." | ".." => {
                    return Err(invalid("contains a `.` or `..` path segment"));
                }
                // The rule, never the segment, as for the byte check above.
                _ if segment.len() > MAX_ARTIFACT_KEY_SEGMENT_BYTES => {
                    return Err(invalid(&format!(
                        "segment longer than {MAX_ARTIFACT_KEY_SEGMENT_BYTES} bytes"
                    )));
                }
                _ => {}
            }
        }

        Ok(Self(raw.to_owned()))
    }

    /// The key as stored, with no leading slash.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first path segment, which is the tenant prefix under the §A3.6
    /// layout. Never empty, because [`Self::parse`] refuses empty segments.
    #[must_use]
    pub fn first_segment(&self) -> &str {
        self.0.split('/').next().unwrap_or(&self.0)
    }

    /// The prefix made of everything up to and including the last `/`, or
    /// [`ArtifactPrefix::root`] when the key has a single segment.
    #[must_use]
    pub fn parent_prefix(&self) -> ArtifactPrefix {
        match self.0.rfind('/') {
            // `..= i` keeps the separator, which is what makes the result a
            // segment-boundary prefix.
            Some(i) => ArtifactPrefix(self.0[..=i].to_owned()),
            None => ArtifactPrefix::root(),
        }
    }
}

impl fmt::Display for ArtifactKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ArtifactKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ArtifactKey({:?})", self.0)
    }
}

impl TryFrom<String> for ArtifactKey {
    type Error = ArtifactStoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ArtifactKey> for String {
    fn from(value: ArtifactKey) -> Self {
        value.0
    }
}

/// A validated artifact key prefix: either the bucket root, or a string
/// ending at a `/` segment boundary.
///
/// The trailing `/` is the whole point. `tenant-a/` cannot match
/// `tenant-abcd/findings.json`, so [`Self::contains`] is a tenant-isolation
/// test and not a `starts_with` that silently widens — the same rule §A7-2
/// states for host scope, applied to object keys.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ArtifactPrefix(String);

impl ArtifactPrefix {
    /// The bucket root: contains every key.
    #[must_use]
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Validates `raw` and returns the prefix, appending the trailing `/` when
    /// it is missing. The empty string is [`Self::root`].
    ///
    /// The result can be one byte over [`MAX_ARTIFACT_KEY_BYTES`]: a legal
    /// key of the full length plus the boundary `/`. Such a prefix names no
    /// object and lists nothing, which is harmless.
    ///
    /// # Errors
    ///
    /// [`ArtifactStoreError::InvalidPrefix`] under the same rules
    /// [`ArtifactKey::parse`] applies, minus the trailing-slash rule.
    pub fn parse(raw: &str) -> Result<Self, ArtifactStoreError> {
        if raw.is_empty() {
            return Ok(Self::root());
        }

        let trimmed = raw.strip_suffix('/').unwrap_or(raw);
        // Reuse the key rules so a prefix and a key can never disagree about
        // what a legal segment is; then put the boundary slash back.
        let key = ArtifactKey::parse(trimmed).map_err(|source| match source {
            ArtifactStoreError::InvalidKey { reason } => {
                ArtifactStoreError::InvalidPrefix { reason }
            }
            other => other,
        })?;

        Ok(Self(format!("{}/", key.as_str())))
    }

    /// The prefix for one tenant: `{tenant_id}/`.
    ///
    /// Infallible, because a hyphenated lowercase UUID is always a legal
    /// single segment. This takes a [`Uuid`] rather than the `vf-core::ids`
    /// `TenantId` newtype of §A3.1, which task C1 has not published yet; when
    /// it lands the call site becomes `for_tenant(tenant.as_uuid())`.
    #[must_use]
    pub fn for_tenant(tenant_id: Uuid) -> Self {
        Self(format!("{}/", tenant_id.as_hyphenated()))
    }

    /// The prefix as stored: empty, or ending in `/`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this prefix is the root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `key` lies under this prefix.
    ///
    /// Because the prefix ends at a `/` boundary, this never matches a sibling
    /// whose name merely starts with the same characters.
    #[must_use]
    pub fn contains(&self, key: &ArtifactKey) -> bool {
        key.as_str().starts_with(&self.0)
    }

    /// `self` extended by `relative`, as a key.
    ///
    /// # Errors
    ///
    /// [`ArtifactStoreError::InvalidKey`] when the concatenation is not a
    /// legal key — which is how a `../` in `relative` is refused.
    pub fn join(&self, relative: &str) -> Result<ArtifactKey, ArtifactStoreError> {
        ArtifactKey::parse(&format!("{}{relative}", self.0))
    }

    /// `self` extended by `relative`, as another prefix.
    ///
    /// # Errors
    ///
    /// [`ArtifactStoreError::InvalidPrefix`] when the concatenation is not a
    /// legal prefix.
    pub fn child(&self, relative: &str) -> Result<Self, ArtifactStoreError> {
        Self::parse(&format!("{}{relative}", self.0))
    }
}

impl fmt::Display for ArtifactPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ArtifactPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ArtifactPrefix({:?})", self.0)
    }
}

impl TryFrom<String> for ArtifactPrefix {
    type Error = ArtifactStoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ArtifactPrefix> for String {
    fn from(value: ArtifactPrefix) -> Self {
        value.0
    }
}

// ---------------------------------------------------------------------------
// Checksums
// ---------------------------------------------------------------------------

/// A SHA-256 digest.
///
/// Hex formatting and parsing are hand-written rather than taken from the
/// `hex` crate: §A1.3 fixes `vf-core`'s dependency list, and 30 lines is a
/// cheaper price than an entry on it.
///
/// The derived `PartialEq` is an ordinary byte comparison, not a
/// constant-time one. That is right for a content checksum and wrong for a
/// MAC: the §A3.6 `X-VF-Signature` check must not be built on this type.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// Wraps 32 raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Parses 64 lowercase or uppercase hex characters.
    ///
    /// # Errors
    ///
    /// [`DigestParseError`], deliberately not an [`ArtifactStoreError`]: a
    /// malformed digest in a §A3.6 notification body is a bad request at the
    /// ingest boundary, and reporting it as a store failure would send an
    /// attacker-triggered 400 down the platform-error path instead.
    pub fn parse_hex(raw: &str) -> Result<Self, DigestParseError> {
        let bytes = raw.as_bytes();
        if bytes.len() != 64 {
            return Err(DigestParseError::Length { len: raw.len() });
        }

        let mut out = [0u8; 32];
        // 64 bytes make exactly 32 pairs, so there is no remainder to check.
        let (pairs, _) = bytes.as_chunks::<2>();
        for (slot, &[hi, lo]) in out.iter_mut().zip(pairs) {
            let (Some(hi), Some(lo)) = (hex_nibble(hi), hex_nibble(lo)) else {
                return Err(DigestParseError::NotHex);
            };
            *slot = (hi << 4) | lo;
        }

        Ok(Self(out))
    }

    /// The 64-character lowercase hex form.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(hex_digit(byte >> 4));
            out.push(hex_digit(byte & 0x0f));
        }
        out
    }

    /// The raw 32 bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Why a hex digest could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DigestParseError {
    /// Not 64 characters long.
    #[error("a sha-256 digest is 64 hex characters, got {len}")]
    Length {
        /// The length seen.
        len: usize,
    },
    /// Contains a character outside `[0-9a-fA-F]`.
    #[error("a sha-256 digest contains only hex characters")]
    NotHex,
}

const fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

const fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'a' + nibble - 10) as char,
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256Digest({})", self.to_hex())
    }
}

impl TryFrom<String> for Sha256Digest {
    type Error = DigestParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse_hex(&value)
    }
}

impl From<Sha256Digest> for String {
    fn from(value: Sha256Digest) -> Self {
        value.to_hex()
    }
}

// ---------------------------------------------------------------------------
// Bodies, metadata and receipts
// ---------------------------------------------------------------------------

/// A source of artifact bytes for a streaming [`ArtifactStore::put`].
///
/// This exists so a 100 MiB findings artifact can be uploaded in parts
/// without ever being whole in memory, which is what §A7-6's "size-bounded
/// and streaming" requires on the write side; [`ArtifactReader`] mirrors its
/// chunked shape on the read side. It is a trait with a boxed future rather
/// than a `futures::Stream` because `vf-core` carries no async crate;
/// `vf-db::adapters` adapts it to `object_store`'s multipart writer.
pub trait ArtifactSource: Send + 'static {
    /// The next chunk, or `Ok(None)` at end of stream.
    ///
    /// A chunk may be any size; the adapter buffers to its own part size. An
    /// empty `Vec` is not end of stream and is skipped.
    ///
    /// # Errors
    ///
    /// Whatever the underlying producer failed with, as an
    /// [`ArtifactStoreError`]. The adapter aborts the upload and propagates it.
    fn next_chunk(&mut self) -> PortFuture<'_, Result<Option<Vec<u8>>, ArtifactStoreError>>;

    /// The total length when the producer knows it, used only to pick
    /// single-shot over multipart. Never trusted as a limit: the adapter
    /// counts bytes as they arrive.
    fn size_hint(&self) -> Option<u64> {
        None
    }
}

/// What to write in an [`ArtifactStore::put`].
///
/// `#[non_exhaustive]` is deliberately absent: the conformance pack matches on
/// both variants, and a third way to supply bytes would be a contract change
/// to §A6.1 rather than an additive one.
pub enum ArtifactBody {
    /// Bytes already in memory. The adapter still chooses multipart when they
    /// are over its threshold.
    Bytes(Vec<u8>),
    /// Bytes produced on demand.
    Stream(Box<dyn ArtifactSource>),
}

impl ArtifactBody {
    /// An in-memory body.
    #[must_use]
    pub fn bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(bytes.into())
    }

    /// A streamed body.
    #[must_use]
    pub fn stream(source: impl ArtifactSource) -> Self {
        Self::Stream(Box::new(source))
    }

    /// The length when it is already known.
    #[must_use]
    pub fn size_hint(&self) -> Option<u64> {
        match self {
            Self::Bytes(bytes) => Some(bytes.len() as u64),
            Self::Stream(source) => source.size_hint(),
        }
    }
}

impl fmt::Debug for ArtifactBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes(bytes) => f
                .debug_struct("ArtifactBody::Bytes")
                .field("len", &bytes.len())
                .finish(),
            Self::Stream(source) => f
                .debug_struct("ArtifactBody::Stream")
                .field("size_hint", &source.size_hint())
                .finish(),
        }
    }
}

/// What a backend knows about a stored object without reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactMeta {
    /// The object's key.
    pub key: ArtifactKey,
    /// Size in bytes.
    pub size: u64,
    /// Last modification time, when the backend reports one.
    pub last_modified: Option<DateTime<Utc>>,
    /// The backend's entity tag, when it reports one. Opaque: never parsed as
    /// a checksum, because for a multipart object it is not one.
    pub e_tag: Option<String>,
}

/// The outcome of a successful [`ArtifactStore::put`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutReceipt {
    /// The key written.
    pub key: ArtifactKey,
    /// Bytes written, counted by the adapter rather than taken from a hint.
    pub size: u64,
    /// The digest the adapter computed over the bytes it wrote. Equal to the
    /// digest the caller declared, or the put failed with
    /// [`ArtifactStoreError::ChecksumMismatch`].
    pub sha256: Sha256Digest,
    /// Whether the adapter used a multipart upload.
    pub multipart: bool,
    /// The backend's entity tag, when it reports one.
    pub e_tag: Option<String>,
}

/// The outcome of a successful [`ArtifactStore::get`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactContent {
    /// What the backend knows about the object.
    pub meta: ArtifactMeta,
    /// The digest the adapter computed over the bytes it is returning.
    pub sha256: Sha256Digest,
    /// The bytes.
    pub bytes: Vec<u8>,
}

/// One stored object, read in chunks: what [`ArtifactStore::get_stream`]
/// returns.
///
/// The read-side mirror of [`ArtifactSource`]: a §A3.6 `findings.json` at its
/// 64 MiB bound arrives in chunks, so the caller decides where the bytes go
/// rather than receiving one whole buffer per concurrent ingest.
///
/// **Verify, then parse.** §A7-6 puts the checksum comparison before any
/// parse, so a caller runs one pipeline in this order: it hashes each chunk
/// as it reads it and keeps the bytes, within the read limit; at `Ok(None)` it
/// compares the hash with the digest declared in the trusted work record
/// (§A3.6); only on a match does it parse what it kept, and that parse is
/// itself size-bounded and streaming (§A7-6). Nothing is parsed while the
/// object is still arriving, and nothing from a reader that ended in an error.
///
/// **No digest.** Unlike [`ArtifactContent`], a reader reports no SHA-256. A
/// digest the adapter computed would be known only at end of stream, and the
/// digest that counts is the declared one, not one the store reports.
pub trait ArtifactReader: Send + 'static {
    /// What the backend reported about the object when the read was opened.
    fn meta(&self) -> &ArtifactMeta;

    /// The next chunk, or `Ok(None)` at end of stream.
    ///
    /// A chunk is never empty and may be any size. Once this returns
    /// `Ok(None)` or an error, every later call returns the same value at
    /// once (contract item 9 on [`ArtifactStore`]).
    ///
    /// # Errors
    ///
    /// [`ArtifactStoreError::TooLarge`] instead of a chunk that would carry
    /// the total past the adapter's read limit. [`ArtifactStoreError::Backend`]
    /// when the transfer fails, and instead of `Ok(None)` when the stream ends
    /// with the total below the size the backend reported at open
    /// ([`Self::meta`]; contract item 8 on [`ArtifactStore`]).
    fn next_chunk(&mut self) -> PortFuture<'_, Result<Option<Vec<u8>>, ArtifactStoreError>>;
}

impl fmt::Debug for dyn ArtifactReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never the bytes: they are scanner output, written by a hostile
        // workload.
        f.debug_struct("ArtifactReader")
            .field("meta", self.meta())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// The port
// ---------------------------------------------------------------------------

/// Artifact object storage (§A6.1).
///
/// Real: `object_store` S3 to Ceph RGW or RustFS. Local: `object_store` S3 to
/// a RustFS container. Test: in-memory and local-filesystem doubles.
/// Implemented in `vf-db::adapters` (task F3).
///
/// # Contract every adapter honours
///
/// The T4 conformance pack runs one module against all three implementations,
/// so these are requirements and not suggestions:
///
/// 1. `put` counts the bytes it writes and hashes them. If the result is not
///    the declared digest, it fails with
///    [`ArtifactStoreError::ChecksumMismatch`] and the object is not
///    observable afterwards.
/// 2. `put` is last-writer-wins for the same key. A reader never observes a
///    partial object, neither while a `put` is in flight nor after one fails.
/// 3. `get` returns the digest it computed over the bytes it returns, and
///    refuses an object over the adapter's read limit with
///    [`ArtifactStoreError::TooLarge`] rather than buffering it. The read
///    limit is adapter construction configuration (§A3.6 sets the
///    `findings.json` default at 64 MiB), so the conformance pack builds each
///    adapter with a known limit and asserts the boundary there.
/// 4. `head`, `get` and `get_stream` on an absent key return
///    [`ArtifactStoreError::NotFound`]. `delete` of an absent key returns
///    `Ok(())`: S3 `DeleteObject` carries no signal that the key existed, so
///    the production adapter could only fake one with a racy extra `HEAD`. A
///    repeated delete is a done one.
/// 5. `list` returns keys under `prefix` at any depth, sorted by key, and an
///    empty `Vec` for a prefix with nothing under it. Never an error for
///    "nothing there". A scoping decorator may narrow the prefix it is given:
///    [`TenantArtifactStore`] lists `{tenant_id}/` for the root.
/// 6. Nothing in this trait is tenant-scoped. Scoping is
///    [`TenantArtifactStore`]'s job, so an adapter is never the thing that
///    has to remember it.
/// 7. Callers never store a key that is a segment-boundary prefix of another
///    stored key (`T/a` beside `T/a/b`). A flat keyspace (S3, memory) accepts
///    both and a filesystem cannot, so the outcome is unspecified and not part
///    of the conformance pack. The §A3.6 layout always has exactly four
///    segments, so it never produces such a pair.
/// 8. `get_stream` refuses before the first chunk: an absent key is
///    [`ArtifactStoreError::NotFound`], and an object whose reported size is
///    over the read limit `get` applies is [`ArtifactStoreError::TooLarge`],
///    both from `get_stream` itself. The reader's [`ArtifactReader::meta`]
///    names the key and the size the backend reported at open, and the reader
///    yields exactly the object's bytes, in order, then `Ok(None)`. It counts
///    what it yields. It returns `TooLarge` rather than a chunk that crosses
///    the limit, so a backend that under-reports a size cannot widen the
///    bound. It returns [`ArtifactStoreError::Backend`] rather than `Ok(None)`
///    when the total it yielded is below the size the backend reported at
///    open, so a truncated body never reads as a complete one. It returns no
///    digest; the caller verifies the declared one.
/// 9. A finished reader stays finished. Once `next_chunk` returns `Ok(None)`
///    or an error, every later call returns the same value at once: it never
///    hangs, panics or resumes yielding bytes.
pub trait ArtifactStore: Send + Sync + 'static {
    /// Writes `body` under `key`, verifying it hashes to `sha256`.
    fn put(
        &self,
        key: &ArtifactKey,
        body: ArtifactBody,
        sha256: Sha256Digest,
    ) -> PortFuture<'_, Result<PutReceipt, ArtifactStoreError>>;

    /// Reads the whole object under `key`, bounded by the adapter's read limit.
    /// For small objects; a caller that parses an artifact uses
    /// [`Self::get_stream`].
    fn get(&self, key: &ArtifactKey)
    -> PortFuture<'_, Result<ArtifactContent, ArtifactStoreError>>;

    /// Opens the object under `key` for a chunked read, bounded by the same
    /// read limit as [`Self::get`]. Returns no digest: the caller verifies
    /// the declared one (contract item 8).
    fn get_stream(
        &self,
        key: &ArtifactKey,
    ) -> PortFuture<'_, Result<Box<dyn ArtifactReader>, ArtifactStoreError>>;

    /// Reads the metadata of the object under `key` without its bytes.
    fn head(&self, key: &ArtifactKey) -> PortFuture<'_, Result<ArtifactMeta, ArtifactStoreError>>;

    /// Lists the objects under `prefix`, at any depth, sorted by key.
    fn list(
        &self,
        prefix: &ArtifactPrefix,
    ) -> PortFuture<'_, Result<Vec<ArtifactMeta>, ArtifactStoreError>>;

    /// Deletes the object under `key`. Succeeds when there is none.
    fn delete(&self, key: &ArtifactKey) -> PortFuture<'_, Result<(), ArtifactStoreError>>;
}

// ---------------------------------------------------------------------------
// Tenant scoping
// ---------------------------------------------------------------------------

/// An [`ArtifactStore`] that can only see one tenant's prefix.
///
/// §A7-7 makes tenant context a type for database access; this is the same
/// rule for object storage. A worker that holds a `TenantArtifactStore`
/// cannot name another tenant's object even by accident, and a key that
/// arrives from an untrusted notification is refused here rather than deep in
/// an adapter.
///
/// **Refusal happens before any I/O.** Every method validates first and
/// returns an already-resolved future on refusal, so the inner store's method
/// is never called, no future that would do I/O is ever constructed, and the
/// T4 pack can assert the refusal against an inner store that panics if
/// touched.
pub struct TenantArtifactStore {
    inner: Arc<dyn ArtifactStore>,
    tenant_id: Uuid,
    prefix: ArtifactPrefix,
}

impl TenantArtifactStore {
    /// Scopes `inner` to `{tenant_id}/`.
    ///
    /// Takes a [`Uuid`] rather than the `vf-core::ids` `TenantId` of §A3.1,
    /// which task C1 has not published yet.
    #[must_use]
    pub fn new(inner: Arc<dyn ArtifactStore>, tenant_id: Uuid) -> Self {
        Self {
            inner,
            tenant_id,
            prefix: ArtifactPrefix::for_tenant(tenant_id),
        }
    }

    /// The tenant this store is scoped to.
    #[must_use]
    pub fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    /// The prefix this store is scoped to: `{tenant_id}/`.
    #[must_use]
    pub fn prefix(&self) -> &ArtifactPrefix {
        &self.prefix
    }

    /// Builds a key inside this tenant's prefix from a relative path.
    ///
    /// The preferred way to name an object, because it cannot produce a key
    /// outside the prefix: a `relative` containing `..` fails
    /// [`ArtifactKey::parse`].
    ///
    /// # Errors
    ///
    /// [`ArtifactStoreError::InvalidKey`] when `{tenant_id}/{relative}` is not
    /// a legal key.
    pub fn key(&self, relative: &str) -> Result<ArtifactKey, ArtifactStoreError> {
        self.prefix.join(relative)
    }

    /// Refuses a key outside the tenant prefix.
    fn guard_key(&self, key: &ArtifactKey) -> Result<(), ArtifactStoreError> {
        if self.prefix.contains(key) {
            Ok(())
        } else {
            Err(ArtifactStoreError::OutsideTenantPrefix {
                prefix: self.prefix.clone(),
            })
        }
    }

    /// Refuses a list prefix that is not inside the tenant prefix.
    ///
    /// A prefix is accepted when it is at or below `{tenant_id}/`. The root
    /// prefix is rewritten to the tenant prefix rather than refused, so
    /// `list(root)` on a tenant-scoped store means "everything I can see" and
    /// not "everything there is".
    fn guard_prefix(&self, prefix: &ArtifactPrefix) -> Result<ArtifactPrefix, ArtifactStoreError> {
        if prefix.is_root() {
            return Ok(self.prefix.clone());
        }
        if prefix.as_str().starts_with(self.prefix.as_str()) {
            Ok(prefix.clone())
        } else {
            Err(ArtifactStoreError::OutsideTenantPrefix {
                prefix: self.prefix.clone(),
            })
        }
    }
}

impl fmt::Debug for TenantArtifactStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TenantArtifactStore")
            .field("tenant_id", &self.tenant_id)
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

/// An already-resolved refusal, so a rejected call never builds a future that
/// could do I/O.
///
/// `T: Send` is not a restriction in practice — [`PortFuture`] is `Send` by
/// definition (§A6.1), so every success type a port method can return already
/// satisfies it.
fn refuse<'a, T: Send + 'a>(
    error: ArtifactStoreError,
) -> PortFuture<'a, Result<T, ArtifactStoreError>> {
    Box::pin(std::future::ready(Err(error)))
}

impl ArtifactStore for TenantArtifactStore {
    fn put(
        &self,
        key: &ArtifactKey,
        body: ArtifactBody,
        sha256: Sha256Digest,
    ) -> PortFuture<'_, Result<PutReceipt, ArtifactStoreError>> {
        if let Err(error) = self.guard_key(key) {
            // `body` is dropped here, unread: nothing is uploaded.
            drop(body);
            return refuse(error);
        }
        self.inner.put(key, body, sha256)
    }

    fn get(
        &self,
        key: &ArtifactKey,
    ) -> PortFuture<'_, Result<ArtifactContent, ArtifactStoreError>> {
        if let Err(error) = self.guard_key(key) {
            return refuse(error);
        }
        self.inner.get(key)
    }

    fn get_stream(
        &self,
        key: &ArtifactKey,
    ) -> PortFuture<'_, Result<Box<dyn ArtifactReader>, ArtifactStoreError>> {
        if let Err(error) = self.guard_key(key) {
            return refuse(error);
        }
        self.inner.get_stream(key)
    }

    fn head(&self, key: &ArtifactKey) -> PortFuture<'_, Result<ArtifactMeta, ArtifactStoreError>> {
        if let Err(error) = self.guard_key(key) {
            return refuse(error);
        }
        self.inner.head(key)
    }

    fn list(
        &self,
        prefix: &ArtifactPrefix,
    ) -> PortFuture<'_, Result<Vec<ArtifactMeta>, ArtifactStoreError>> {
        match self.guard_prefix(prefix) {
            Ok(scoped) => {
                // `scoped` is owned, and the inner future borrows only `self`,
                // so the temporary has to outlive this call: box the inner
                // call inside an async block that owns it.
                let inner = Arc::clone(&self.inner);
                Box::pin(async move { inner.list(&scoped).await })
            }
            Err(error) => refuse(error),
        }
    }

    fn delete(&self, key: &ArtifactKey) -> PortFuture<'_, Result<(), ArtifactStoreError>> {
        if let Err(error) = self.guard_key(key) {
            return refuse(error);
        }
        self.inner.delete(key)
    }
}
