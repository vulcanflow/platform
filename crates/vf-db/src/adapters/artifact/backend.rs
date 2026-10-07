//! The one implementation of the [`ArtifactStore`] contract behind all three
//! adapters (task F3).
//!
//! S3, the local filesystem and memory are all `object_store` backends, so
//! the contract on [`ArtifactStore`] is implemented once, here, over
//! `dyn ObjectStore`, and each adapter only chooses the backend. The places
//! where the backends disagree are normalised here rather than left to leak
//! through:
//!
//! * `delete` of an absent key succeeds on S3 and in memory, and fails on the
//!   local filesystem. The port requires success everywhere (contract item
//!   4), so the filesystem's `NotFound` is mapped to `Ok(())`.
//! * `list` order is backend-defined. The port requires key order, so the
//!   result is sorted.
//! * The checksum is computed here over the bytes actually sent or received,
//!   whatever the backend verifies on its own.
//! * A body's length is counted here against the size the backend reported
//!   when the read was opened, in both directions (contract item 8), because
//!   no backend promises that its stream and its metadata agree.

use std::fmt;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use object_store::path::Path;
use object_store::{ObjectMeta, ObjectStore, ObjectStoreExt as _, PutPayload, WriteMultipart};
use sha2::{Digest as _, Sha256};
use vf_core::ports::{
    ArtifactBody, ArtifactContent, ArtifactKey, ArtifactMeta, ArtifactPrefix, ArtifactReader,
    ArtifactSource, ArtifactStore, ArtifactStoreError, PortFuture, PutReceipt, Sha256Digest,
};

use super::ArtifactLimits;

/// How many multipart parts may be in flight at once.
///
/// Bounds the memory a streamed upload holds to about
/// `(MAX_PARTS_IN_FLIGHT + 1) * multipart_part_bytes`, whatever the object
/// size.
const MAX_PARTS_IN_FLIGHT: usize = 4;

/// An [`ArtifactStore`] over any `object_store` backend: the one
/// implementation that [`S3ArtifactStore`](super::S3ArtifactStore),
/// [`LocalFsArtifactStore`](super::LocalFsArtifactStore) and
/// [`InMemoryArtifactStore`](super::InMemoryArtifactStore) delegate every
/// call to.
///
/// Public so the T4 conformance pack can put a backend double under the same
/// code the three adapters run. Contract item 8 has a case no real backend
/// can be made to produce on demand, a body longer than the size reported at
/// open, and the count that refuses it lives here and not in the backend.
///
/// Not selectable by `VF_ARTIFACT_STORE`: a binary builds its store with
/// [`artifact_store_from_env`](crate::adapters::artifact_store_from_env), which
/// names only the three adapters and refuses the test ones in production.
pub struct ObjectStoreArtifactStore {
    store: Arc<dyn ObjectStore>,
    limits: ArtifactLimits,
}

impl ObjectStoreArtifactStore {
    /// An adapter over `store`, enforcing `limits`.
    ///
    /// A zero multipart threshold or part size is raised to one.
    #[must_use]
    pub fn new(store: Arc<dyn ObjectStore>, mut limits: ArtifactLimits) -> Self {
        // A zero part size would never fill a part, and a zero threshold would
        // send an empty object through a multipart upload with no parts,
        // which S3 refuses.
        limits.multipart_threshold_bytes = limits.multipart_threshold_bytes.max(1);
        limits.multipart_part_bytes = limits.multipart_part_bytes.max(1);
        Self { store, limits }
    }

    /// The limits this store enforces.
    #[must_use]
    pub fn limits(&self) -> ArtifactLimits {
        self.limits
    }

    async fn put_impl(
        &self,
        key: ArtifactKey,
        body: ArtifactBody,
        declared: Sha256Digest,
    ) -> Result<PutReceipt, ArtifactStoreError> {
        let path = object_path(&key)?;
        let mut upload = Upload::new(&key, declared, self.limits);

        match body {
            ArtifactBody::Bytes(bytes) => {
                // Everything is already in memory, so the size and the digest
                // are checked before the first request.
                upload.observe(&bytes)?;
                upload.verify()?;
                if upload.size < self.limits.multipart_threshold_bytes {
                    return self.put_single(upload, &path, bytes).await;
                }
                let mut writer = self.start_multipart(&path).await?;
                let written = self.write_parts(&mut writer, &bytes).await;
                self.finish_multipart(upload, writer, written).await
            }
            ArtifactBody::Stream(mut source) => {
                // Buffer up to the multipart threshold. A stream that ends
                // before it is uploaded in one request, exactly like bytes.
                let mut buffer = Vec::new();
                loop {
                    let Some(chunk) = source.next_chunk().await? else {
                        upload.verify()?;
                        return self.put_single(upload, &path, buffer).await;
                    };
                    upload.observe(&chunk)?;
                    buffer.extend_from_slice(&chunk);
                    if upload.size >= self.limits.multipart_threshold_bytes {
                        break;
                    }
                }

                let mut writer = self.start_multipart(&path).await?;
                let written = self
                    .stream_parts(&mut writer, &mut upload, buffer, source.as_mut())
                    .await
                    .and_then(|()| upload.verify());
                self.finish_multipart(upload, writer, written).await
            }
        }
    }

    async fn put_single(
        &self,
        upload: Upload,
        path: &Path,
        bytes: Vec<u8>,
    ) -> Result<PutReceipt, ArtifactStoreError> {
        let result = self
            .store
            .put(path, PutPayload::from(bytes))
            .await
            .map_err(|e| map_error(e, &upload.key))?;
        Ok(upload.receipt(false, result.e_tag))
    }

    async fn start_multipart(&self, path: &Path) -> Result<WriteMultipart, ArtifactStoreError> {
        let multipart = self
            .store
            .put_multipart(path)
            .await
            .map_err(|e| backend_error(&e))?;
        Ok(WriteMultipart::new_with_chunk_size(
            multipart,
            self.limits.multipart_part_bytes,
        ))
    }

    /// Writes `bytes` part by part, never with more than
    /// [`MAX_PARTS_IN_FLIGHT`] parts outstanding.
    async fn write_parts(
        &self,
        writer: &mut WriteMultipart,
        bytes: &[u8],
    ) -> Result<(), ArtifactStoreError> {
        for part in bytes.chunks(self.limits.multipart_part_bytes) {
            writer
                .wait_for_capacity(MAX_PARTS_IN_FLIGHT)
                .await
                .map_err(|e| backend_error(&e))?;
            writer.write(part);
        }
        Ok(())
    }

    /// Writes the buffered head of a stream, then the rest of it, counting
    /// and hashing every chunk.
    async fn stream_parts(
        &self,
        writer: &mut WriteMultipart,
        upload: &mut Upload,
        head: Vec<u8>,
        source: &mut dyn ArtifactSource,
    ) -> Result<(), ArtifactStoreError> {
        self.write_parts(writer, &head).await?;
        drop(head);
        while let Some(chunk) = source.next_chunk().await? {
            upload.observe(&chunk)?;
            self.write_parts(writer, &chunk).await?;
        }
        Ok(())
    }

    /// Completes the upload when everything before it succeeded, and aborts
    /// it otherwise, so a failed `put` never leaves an object behind
    /// (contract point 2).
    async fn finish_multipart(
        &self,
        upload: Upload,
        writer: WriteMultipart,
        written: Result<(), ArtifactStoreError>,
    ) -> Result<PutReceipt, ArtifactStoreError> {
        if let Err(error) = written {
            if let Err(abort) = writer.abort().await {
                // The upload was never completed, so no object is visible;
                // what may be left is unreferenced parts, which the bucket's
                // lifecycle rule removes.
                tracing::warn!(error = %render(&abort), "aborting a failed multipart upload failed");
            }
            return Err(error);
        }
        // `finish` aborts on its own when completing fails.
        let result = writer.finish().await.map_err(|e| backend_error(&e))?;
        Ok(upload.receipt(true, result.e_tag))
    }

    /// Opens `key` for reading and applies the checks contract item 8 makes
    /// before the first chunk: `NotFound` for an absent key, `TooLarge` for a
    /// reported size over the read limit. `get` and `get_stream` both start
    /// here, so they refuse the same objects and count bodies the same way.
    async fn open(&self, key: ArtifactKey) -> Result<ObjectReader, ArtifactStoreError> {
        let path = object_path(&key)?;
        let limit = self.limits.max_object_bytes;
        let result = self
            .store
            .get(&path)
            .await
            .map_err(|e| map_error(e, &key))?;

        // Refused by its metadata, before a byte of the body is read.
        if result.meta.size > limit {
            return Err(ArtifactStoreError::TooLarge {
                key,
                size: result.meta.size,
                limit,
            });
        }

        let meta = artifact_meta(key, &result.meta);
        // Mapped here so the reader never names `object_store`'s chunk type.
        // Every failure after open is the transfer's, so it is `Backend`,
        // even a `NotFound` from an object deleted mid-read: item 8 puts
        // `NotFound` before the first chunk only.
        let body = result
            .into_stream()
            .map(|chunk| chunk.map(Vec::from).map_err(|e| backend_error(&e)))
            .boxed();
        Ok(ObjectReader {
            meta,
            limit,
            yielded: 0,
            state: ReaderState::Reading(body),
        })
    }

    async fn get_impl(&self, key: ArtifactKey) -> Result<ArtifactContent, ArtifactStoreError> {
        let mut reader = self.open(key).await?;
        let mut hasher = Sha256::new();
        let mut bytes = Vec::with_capacity(usize::try_from(reader.meta.size).unwrap_or(0));
        while let Some(chunk) = reader.read().await? {
            hasher.update(&chunk);
            bytes.extend_from_slice(&chunk);
        }

        Ok(ArtifactContent {
            meta: reader.meta,
            sha256: Sha256Digest::from_bytes(hasher.finalize().into()),
            bytes,
        })
    }

    async fn get_stream_impl(
        &self,
        key: ArtifactKey,
    ) -> Result<Box<dyn ArtifactReader>, ArtifactStoreError> {
        let reader = self.open(key).await?;
        Ok(Box::new(reader))
    }

    async fn head_impl(&self, key: ArtifactKey) -> Result<ArtifactMeta, ArtifactStoreError> {
        let path = object_path(&key)?;
        let meta = self
            .store
            .head(&path)
            .await
            .map_err(|e| map_error(e, &key))?;
        Ok(artifact_meta(key, &meta))
    }

    async fn list_impl(
        &self,
        prefix: ArtifactPrefix,
    ) -> Result<Vec<ArtifactMeta>, ArtifactStoreError> {
        // `object_store` prefixes are whole path segments without the
        // trailing `/`, which is the same boundary `ArtifactPrefix` keeps.
        let prefix_path = if prefix.is_root() {
            None
        } else {
            Some(
                Path::parse(prefix.as_str().trim_end_matches('/')).map_err(|_| {
                    ArtifactStoreError::InvalidPrefix {
                        reason: "not a valid object path".to_owned(),
                    }
                })?,
            )
        };

        let mut listing = self.store.list(prefix_path.as_ref());
        let mut found = Vec::new();
        let mut foreign = 0_u64;
        while let Some(meta) = listing.next().await {
            let meta = meta.map_err(|e| backend_error(&e))?;
            match ArtifactKey::parse(meta.location.as_ref()) {
                Ok(key) if prefix.contains(&key) => found.push(artifact_meta(key, &meta)),
                // An object this crate could not have written: something else
                // shares the bucket, or a scanner wrote outside the key rules.
                // It cannot be named through the port, so it is not listed.
                _ => foreign += 1,
            }
        }
        if foreign > 0 {
            tracing::warn!(
                skipped = foreign,
                prefix = %prefix,
                "objects whose keys are not valid artifact keys were left out of a listing"
            );
        }

        found.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(found)
    }

    async fn delete_impl(&self, key: ArtifactKey) -> Result<(), ArtifactStoreError> {
        let path = object_path(&key)?;
        // Contract item 4: deleting an absent key succeeds. S3 and the memory
        // store already report success; the local filesystem reports
        // `NotFound`, which means the same thing here.
        match self.store.delete(&path).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(backend_error(&error)),
        }
    }
}

impl fmt::Debug for ObjectStoreArtifactStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObjectStoreArtifactStore")
            .field("store", &self.store.to_string())
            .field("limits", &self.limits)
            .finish()
    }
}

impl ArtifactStore for ObjectStoreArtifactStore {
    fn put(
        &self,
        key: &ArtifactKey,
        body: ArtifactBody,
        sha256: Sha256Digest,
    ) -> PortFuture<'_, Result<PutReceipt, ArtifactStoreError>> {
        Box::pin(self.put_impl(key.clone(), body, sha256))
    }

    fn get(
        &self,
        key: &ArtifactKey,
    ) -> PortFuture<'_, Result<ArtifactContent, ArtifactStoreError>> {
        Box::pin(self.get_impl(key.clone()))
    }

    fn get_stream(
        &self,
        key: &ArtifactKey,
    ) -> PortFuture<'_, Result<Box<dyn ArtifactReader>, ArtifactStoreError>> {
        Box::pin(self.get_stream_impl(key.clone()))
    }

    fn head(&self, key: &ArtifactKey) -> PortFuture<'_, Result<ArtifactMeta, ArtifactStoreError>> {
        Box::pin(self.head_impl(key.clone()))
    }

    fn list(
        &self,
        prefix: &ArtifactPrefix,
    ) -> PortFuture<'_, Result<Vec<ArtifactMeta>, ArtifactStoreError>> {
        Box::pin(self.list_impl(prefix.clone()))
    }

    fn delete(&self, key: &ArtifactKey) -> PortFuture<'_, Result<(), ArtifactStoreError>> {
        Box::pin(self.delete_impl(key.clone()))
    }
}

/// One object being read: what `get_stream` returns, and what `get` drains.
struct ObjectReader {
    meta: ArtifactMeta,
    limit: u64,
    yielded: u64,
    state: ReaderState,
}

enum ReaderState {
    /// The body is still arriving.
    Reading(BoxStream<'static, Result<Vec<u8>, ArtifactStoreError>>),
    /// The body ended (`None`) or failed. Every later call returns this again
    /// (contract item 9); the stream was dropped on the way in, which
    /// releases the connection.
    Finished(Option<ArtifactStoreError>),
}

impl ObjectReader {
    /// The next non-empty chunk, counted against the read limit and against
    /// the size the backend reported at open (contract item 8).
    async fn read(&mut self) -> Result<Option<Vec<u8>>, ArtifactStoreError> {
        let body = match &mut self.state {
            ReaderState::Reading(body) => body,
            ReaderState::Finished(end) => return end.clone().map_or(Ok(None), Err),
        };
        let end = loop {
            match body.next().await {
                Some(Ok(chunk)) if chunk.is_empty() => {}
                Some(Ok(chunk)) => {
                    let total = self.yielded.saturating_add(chunk.len() as u64);
                    // The limit first: `open` refused a reported size over
                    // it, so a chunk that crosses the limit also crosses the
                    // reported size, and that case is `TooLarge`. `Backend`
                    // is the body outgrowing the size reported at open while
                    // still inside the limit.
                    if total > self.limit {
                        break Some(ArtifactStoreError::TooLarge {
                            key: self.meta.key.clone(),
                            size: total,
                            limit: self.limit,
                        });
                    }
                    if total > self.meta.size {
                        break Some(ArtifactStoreError::Backend {
                            message: format!(
                                "artifact {} body is longer than the {} bytes reported at open",
                                self.meta.key, self.meta.size
                            ),
                        });
                    }
                    self.yielded = total;
                    return Ok(Some(chunk));
                }
                Some(Err(error)) => break Some(error),
                None if self.yielded < self.meta.size => {
                    break Some(ArtifactStoreError::Backend {
                        message: format!(
                            "artifact {} body ended at {} of the {} bytes reported at open",
                            self.meta.key, self.yielded, self.meta.size
                        ),
                    });
                }
                None => break None,
            }
        };
        self.state = ReaderState::Finished(end.clone());
        end.map_or(Ok(None), Err)
    }
}

impl ArtifactReader for ObjectReader {
    fn meta(&self) -> &ArtifactMeta {
        &self.meta
    }

    fn next_chunk(&mut self) -> PortFuture<'_, Result<Option<Vec<u8>>, ArtifactStoreError>> {
        Box::pin(self.read())
    }
}

/// The running size and digest of one `put`.
struct Upload {
    key: ArtifactKey,
    declared: Sha256Digest,
    limit: u64,
    size: u64,
    hasher: Sha256,
}

impl Upload {
    fn new(key: &ArtifactKey, declared: Sha256Digest, limits: ArtifactLimits) -> Self {
        Self {
            key: key.clone(),
            declared,
            limit: limits.max_object_bytes,
            size: 0,
            hasher: Sha256::new(),
        }
    }

    /// Counts and hashes `chunk`, refusing it if it takes the object over
    /// the limit.
    fn observe(&mut self, chunk: &[u8]) -> Result<(), ArtifactStoreError> {
        let size = self.size.saturating_add(chunk.len() as u64);
        if size > self.limit {
            return Err(ArtifactStoreError::TooLarge {
                key: self.key.clone(),
                size,
                limit: self.limit,
            });
        }
        self.size = size;
        self.hasher.update(chunk);
        Ok(())
    }

    /// Compares the digest of everything observed with the declared one.
    fn verify(&self) -> Result<(), ArtifactStoreError> {
        let computed = self.computed();
        if computed == self.declared {
            Ok(())
        } else {
            Err(ArtifactStoreError::ChecksumMismatch {
                key: self.key.clone(),
                declared: self.declared,
                computed,
            })
        }
    }

    fn computed(&self) -> Sha256Digest {
        Sha256Digest::from_bytes(self.hasher.clone().finalize().into())
    }

    fn receipt(self, multipart: bool, e_tag: Option<String>) -> PutReceipt {
        PutReceipt {
            sha256: self.computed(),
            key: self.key,
            size: self.size,
            multipart,
            e_tag,
        }
    }
}

/// The `object_store` path for `key`.
///
/// Cannot fail for a key [`ArtifactKey::parse`] accepted, since its alphabet
/// is a subset of what `Path::parse` accepts; mapped rather than unwrapped so
/// that a future change to either side fails a request instead of the process.
fn object_path(key: &ArtifactKey) -> Result<Path, ArtifactStoreError> {
    Path::parse(key.as_str()).map_err(|_| ArtifactStoreError::InvalidKey {
        reason: "not a valid object path".to_owned(),
    })
}

fn artifact_meta(key: ArtifactKey, meta: &ObjectMeta) -> ArtifactMeta {
    ArtifactMeta {
        key,
        size: meta.size,
        last_modified: Some(meta.last_modified),
        e_tag: meta.e_tag.clone(),
    }
}

/// Maps the backend's error onto the port's, keeping `NotFound` and
/// `AlreadyExists` distinguishable.
fn map_error(error: object_store::Error, key: &ArtifactKey) -> ArtifactStoreError {
    match error {
        object_store::Error::NotFound { .. } => ArtifactStoreError::NotFound { key: key.clone() },
        object_store::Error::AlreadyExists { .. } => {
            ArtifactStoreError::AlreadyExists { key: key.clone() }
        }
        other => backend_error(&other),
    }
}

pub(super) fn backend_error(error: &object_store::Error) -> ArtifactStoreError {
    ArtifactStoreError::Backend {
        message: render(error),
    }
}

/// The rendered error. `object_store`'s `Display` already includes its
/// source, and carries the request URL but never the credentials, which
/// travel in headers.
fn render(error: &object_store::Error) -> String {
    error.to_string()
}
