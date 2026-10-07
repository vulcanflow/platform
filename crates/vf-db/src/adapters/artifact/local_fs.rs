//! [`LocalFsArtifactStore`] — artifacts in a directory on this machine
//! (task F3).
//!
//! A test adapter: refused under `VF_ENV=production` by
//! [`ArtifactStoreKind::is_stand_in`](crate::adapters::ArtifactStoreKind::is_stand_in).
//!
//! # Why the key validation in `vf-core` is load-bearing here
//!
//! This is the one adapter where an artifact key becomes a filesystem path, so
//! it is the one where a traversal would escape rather than merely name a
//! surprising object. [`ArtifactKey`] only admits `[A-Za-z0-9._-]` segments
//! that are neither `.` nor `..`, which is why that validation lives in the
//! port's constructor and not in each adapter: no key can name a path outside
//! the root. The root is created and canonicalised once at construction.
//!
//! What this adapter does **not** defend against is a symlink planted inside
//! the root by another process, which `object_store`'s local backend follows.
//! The root must be a directory only this process writes; that is acceptable
//! for a test adapter and one more reason it is refused in production.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use object_store::local::LocalFileSystem;
use vf_core::ports::{
    ArtifactBody, ArtifactContent, ArtifactKey, ArtifactMeta, ArtifactPrefix, ArtifactReader,
    ArtifactStore, ArtifactStoreError, PortFuture, PutReceipt, Sha256Digest,
};

use super::ArtifactLimits;
use super::backend::{ObjectStoreArtifactStore, backend_error};
use crate::adapters::{AdapterSelectionError, Lookup, required};

/// The directory a [`LocalFsArtifactStore`] keeps its objects in.
pub const ENV_ARTIFACT_ROOT: &str = "VF_ARTIFACT_ROOT";

/// Where a [`LocalFsArtifactStore`] keeps its objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFsArtifactStoreConfig {
    /// The root directory. Created if absent, then canonicalised.
    pub root: PathBuf,
    /// Size bounds.
    pub limits: ArtifactLimits,
}

impl LocalFsArtifactStoreConfig {
    /// Default limits under `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            limits: ArtifactLimits::new(),
        }
    }

    /// Reads the root from `VF_ARTIFACT_ROOT`, with the default limits.
    ///
    /// # Errors
    ///
    /// [`AdapterSelectionError::MissingSetting`] when `VF_ARTIFACT_ROOT` is
    /// unset or empty.
    pub fn from_lookup(lookup: Lookup<'_>) -> Result<Self, AdapterSelectionError> {
        Ok(Self::new(required(lookup, ENV_ARTIFACT_ROOT)?))
    }
}

/// An [`ArtifactStore`] backed by a directory.
///
/// Keys map to paths under the root, segment for segment. A `put` writes to a
/// staging file in the same directory and renames it into place, so a failed
/// or interrupted write never leaves a partial object visible — contract point
/// 2 on [`ArtifactStore`].
pub struct LocalFsArtifactStore {
    root: PathBuf,
    backend: ObjectStoreArtifactStore,
}

impl LocalFsArtifactStore {
    /// Creates the root if it does not exist, canonicalises it, and returns
    /// the store.
    ///
    /// # Errors
    ///
    /// [`ArtifactStoreError::Backend`] when the root cannot be created or
    /// canonicalised.
    pub fn new(config: LocalFsArtifactStoreConfig) -> Result<Self, ArtifactStoreError> {
        let prepare = |error: std::io::Error| ArtifactStoreError::Backend {
            message: format!("the artifact root could not be prepared: {error}"),
        };
        std::fs::create_dir_all(&config.root).map_err(prepare)?;
        let root = std::fs::canonicalize(&config.root).map_err(prepare)?;
        // Automatic cleanup removes the directories a delete leaves empty, so
        // the tree does not keep one empty directory per deleted run.
        let store = LocalFileSystem::new_with_prefix(&root)
            .map_err(|e| backend_error(&e))?
            .with_automatic_cleanup(true);
        Ok(Self {
            root,
            backend: ObjectStoreArtifactStore::new(Arc::new(store), config.limits),
        })
    }

    /// The canonical root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The limits this store enforces.
    #[must_use]
    pub fn limits(&self) -> ArtifactLimits {
        self.backend.limits()
    }
}

impl fmt::Debug for LocalFsArtifactStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalFsArtifactStore")
            .field("root", &self.root)
            .field("limits", &self.backend.limits())
            .finish()
    }
}

impl ArtifactStore for LocalFsArtifactStore {
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
