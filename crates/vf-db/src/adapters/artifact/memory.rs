//! [`InMemoryArtifactStore`] — artifacts in a map, for the duration of the
//! process (task F3).
//!
//! A test adapter of §A6.1, and refused under `VF_ENV=production` by
//! [`ArtifactStoreKind::is_stand_in`](crate::adapters::ArtifactStoreKind::is_stand_in).
//!
//! It is held to the same contract as the other two, including the
//! [`ArtifactStoreError::TooLarge`] limit: a test double that accepted an
//! object the real store refuses would let a size bug through the unit suite
//! and fail in the integration suite instead.

use std::fmt;
use std::sync::Arc;

use object_store::memory::InMemory;
use vf_core::ports::{
    ArtifactBody, ArtifactContent, ArtifactKey, ArtifactMeta, ArtifactPrefix, ArtifactReader,
    ArtifactStore, ArtifactStoreError, PortFuture, PutReceipt, Sha256Digest,
};

use super::ArtifactLimits;
use super::backend::ObjectStoreArtifactStore;

/// An [`ArtifactStore`] backed by `object_store`'s in-memory store.
///
/// Cheap to share behind an `Arc`: the conformance pack can drive one store
/// from several tasks.
pub struct InMemoryArtifactStore {
    backend: ObjectStoreArtifactStore,
}

impl InMemoryArtifactStore {
    /// An empty store with the default limits.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(ArtifactLimits::new())
    }

    /// An empty store with explicit limits, so the T4 pack can assert
    /// [`ArtifactStoreError::TooLarge`] cheaply.
    #[must_use]
    pub fn with_limits(limits: ArtifactLimits) -> Self {
        Self {
            backend: ObjectStoreArtifactStore::new(Arc::new(InMemory::new()), limits),
        }
    }

    /// The limits this store enforces.
    #[must_use]
    pub fn limits(&self) -> ArtifactLimits {
        self.backend.limits()
    }
}

impl Default for InMemoryArtifactStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for InMemoryArtifactStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemoryArtifactStore")
            .field("limits", &self.backend.limits())
            .finish_non_exhaustive()
    }
}

impl ArtifactStore for InMemoryArtifactStore {
    fn put(
        &self,
        key: &ArtifactKey,
        body: ArtifactBody,
        sha256: Sha256Digest,
    ) -> PortFuture<'_, Result<PutReceipt, ArtifactStoreError>> {
        self.backend.put(key, body, sha256)
    }

    fn get(
        &self,
        key: &ArtifactKey,
    ) -> PortFuture<'_, Result<ArtifactContent, ArtifactStoreError>> {
        self.backend.get(key)
    }

    fn get_stream(
        &self,
        key: &ArtifactKey,
    ) -> PortFuture<'_, Result<Box<dyn ArtifactReader>, ArtifactStoreError>> {
        self.backend.get_stream(key)
    }

    fn head(&self, key: &ArtifactKey) -> PortFuture<'_, Result<ArtifactMeta, ArtifactStoreError>> {
        self.backend.head(key)
    }

    fn list(
        &self,
        prefix: &ArtifactPrefix,
    ) -> PortFuture<'_, Result<Vec<ArtifactMeta>, ArtifactStoreError>> {
        self.backend.list(prefix)
    }

    fn delete(&self, key: &ArtifactKey) -> PortFuture<'_, Result<(), ArtifactStoreError>> {
        self.backend.delete(key)
    }
}
