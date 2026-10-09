//! [`ArtifactStore`](vf_core::ports::ArtifactStore) adapters (task F3).
//!
//! Three implementations of one trait, exercised by one T4 conformance
//! module: [`S3ArtifactStore`] against Ceph RGW or RustFS,
//! [`LocalFsArtifactStore`] against a directory, and
//! [`InMemoryArtifactStore`] against a map. All three delegate to
//! [`ObjectStoreArtifactStore`], which the conformance module can also put
//! over a backend double.
//!
//! # Limits are the adapter's, not the caller's
//!
//! §A7-6 requires every artifact path to be size-bounded. The bound lives on
//! the adapter ([`ArtifactLimits`]) rather than in each call, because the
//! thing being defended is the process's memory and the caller on the ingest
//! path is the least trusted party in the system. `get` and `get_stream`
//! refuse an oversized object by its metadata, before reading bytes, and then
//! count the body in case the metadata was wrong.

mod backend;
pub mod local_fs;
pub mod memory;
pub mod s3;

pub use backend::ObjectStoreArtifactStore;
pub use local_fs::{LocalFsArtifactStore, LocalFsArtifactStoreConfig};
pub use memory::InMemoryArtifactStore;
pub use s3::{S3ArtifactStore, S3ArtifactStoreConfig, S3Credentials, install_crypto_provider};

/// Largest object an adapter will write or read, in bytes.
///
/// 128 MiB: the F3 acceptance criteria put a 100 MiB artifact through
/// multipart upload, so the limit has to clear it with headroom. It bounds
/// `get_stream` as well as `get` (contract item 8 on
/// [`ArtifactStore`](vf_core::ports::ArtifactStore)), so a streamed read
/// still caps what a caller can be made to buffer.
pub const DEFAULT_MAX_OBJECT_BYTES: u64 = 128 * 1024 * 1024;

/// Size at or above which a `put` uses a multipart upload.
///
/// 16 MiB is above every control-plane artifact and below the 100 MiB
/// scanner-output case, so the conformance pack can select either path by size
/// alone.
pub const DEFAULT_MULTIPART_THRESHOLD_BYTES: u64 = 16 * 1024 * 1024;

/// Part size for a multipart upload, in bytes.
///
/// 8 MiB clears S3's 5 MiB minimum for every part but the last, and keeps a
/// 100 MiB object inside the 10 000-part ceiling with room to spare.
pub const DEFAULT_MULTIPART_PART_BYTES: usize = 8 * 1024 * 1024;

/// The size bounds an adapter enforces.
///
/// Shared by all three adapters so the conformance pack can set one limit and
/// assert the same [`ArtifactStoreError::TooLarge`](vf_core::ports::ArtifactStoreError::TooLarge) from each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactLimits {
    /// Largest object read by `get` or `get_stream`, and largest accepted by
    /// `put`.
    pub max_object_bytes: u64,
    /// Size at or above which `put` goes multipart, on all three adapters:
    /// each `object_store` backend offers multipart, so the conformance pack
    /// exercises the same upload path everywhere. A zero is treated as one, so
    /// an empty object is always a single request.
    pub multipart_threshold_bytes: u64,
    /// Multipart part size. A zero is treated as one. S3 requires at least
    /// 5 MiB for every part but the last; the in-memory and filesystem
    /// adapters accept any size, which lets the T4 pack drive the multipart
    /// path with small objects.
    pub multipart_part_bytes: usize,
}

impl ArtifactLimits {
    /// The defaults above.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_object_bytes: DEFAULT_MAX_OBJECT_BYTES,
            multipart_threshold_bytes: DEFAULT_MULTIPART_THRESHOLD_BYTES,
            multipart_part_bytes: DEFAULT_MULTIPART_PART_BYTES,
        }
    }

    /// The same limits with a different object ceiling. Used by the T4 pack to
    /// assert [`ArtifactStoreError::TooLarge`](vf_core::ports::ArtifactStoreError::TooLarge) without writing 128 MiB.
    #[must_use]
    pub const fn with_max_object_bytes(mut self, bytes: u64) -> Self {
        self.max_object_bytes = bytes;
        self
    }
}

impl Default for ArtifactLimits {
    fn default() -> Self {
        Self::new()
    }
}
