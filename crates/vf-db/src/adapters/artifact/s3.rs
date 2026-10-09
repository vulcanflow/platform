//! [`S3ArtifactStore`] — the S3 adapter, used against Ceph RGW in production
//! and against RustFS from `just dev-up` (task F3).
//!
//! # Why one adapter for both
//!
//! The local suite must test the code that runs in production, so RustFS is
//! driven through this adapter rather than through a lookalike. The only
//! difference between the two deployments is [`S3ArtifactStoreConfig`].
//!
//! # The three settings that are not defaults
//!
//! * **Path-style addressing.** `virtual_hosted_style_request = false`
//!   unless [`S3ArtifactStoreConfig::path_style`] says otherwise. RustFS and
//!   a typical RGW install are reached by endpoint and bucket path, not by a
//!   `{bucket}.` DNS prefix that would need a wildcard record.
//! * **Explicit checksums.** `object_store`'s S3 backend can be asked to send
//!   a checksum with every upload. §A7-6 requires the artifact's SHA-256 to be
//!   verified against the trusted work record, so the adapter hashes the bytes
//!   it writes itself *and* asks the backend to verify its own, which is what
//!   makes a silent corruption on the wire a failed `put` instead of a stored
//!   object with the wrong contents.
//! * **No credential discovery.** Endpoint, region and credentials come from
//!   configuration only, and the client is built from an empty builder rather
//!   than `from_env`, so the `AWS_*` variables, a stray `~/.aws` and the
//!   instance-metadata endpoint (an SSRF target in the cluster) are never
//!   consulted.
//!
//! The rustls `ring` provider is installed once per process by
//! [`install_crypto_provider`], as the §A5 `object_store` pin requires.

use std::fmt;
use std::sync::Arc;

use object_store::aws::{AmazonS3Builder, Checksum};
use vf_core::ports::{
    ArtifactBody, ArtifactContent, ArtifactKey, ArtifactMeta, ArtifactPrefix, ArtifactReader,
    ArtifactStore, ArtifactStoreError, PortFuture, PutReceipt, Sha256Digest,
};

use super::ArtifactLimits;
use super::backend::{ObjectStoreArtifactStore, backend_error};
use crate::adapters::{AdapterSelectionError, DeployEnv, Lookup, optional_bool, required};

/// The S3 endpoint URL.
pub const ENV_S3_ENDPOINT: &str = "VF_S3_ENDPOINT";
/// The S3 region.
pub const ENV_S3_REGION: &str = "VF_S3_REGION";
/// The bucket artifacts are stored in.
pub const ENV_S3_BUCKET: &str = "VF_S3_BUCKET";
/// The S3 access key id.
pub const ENV_S3_ACCESS_KEY: &str = "VF_S3_ACCESS_KEY";
/// The S3 secret access key.
pub const ENV_S3_SECRET_KEY: &str = "VF_S3_SECRET_KEY";
/// An S3 session token, for temporary credentials. Optional.
pub const ENV_S3_SESSION_TOKEN: &str = "VF_S3_SESSION_TOKEN";
/// Whether to use path-style addressing. Optional, default `true`.
pub const ENV_S3_PATH_STYLE: &str = "VF_S3_PATH_STYLE";

/// S3 credentials for [`S3ArtifactStore`].
///
/// [`fmt::Debug`] is hand-written: this struct ends up inside a config that
/// gets logged at startup, and a derived `Debug` would put the secret key in
/// the log.
#[derive(Clone, PartialEq, Eq)]
pub struct S3Credentials {
    /// The access key id.
    pub access_key_id: String,
    /// The secret access key. Never logged.
    pub secret_access_key: String,
    /// A session token, when the credentials are temporary.
    pub session_token: Option<String>,
}

impl fmt::Debug for S3Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Credentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"<redacted>")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// How to reach the S3 endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3ArtifactStoreConfig {
    /// Endpoint URL, for instance `http://127.0.0.1:9000` for RustFS from
    /// `just dev-up`.
    pub endpoint: String,
    /// Region. RustFS ignores it; SigV4 still needs a value, so this is
    /// required rather than defaulted, to keep the signing inputs explicit.
    pub region: String,
    /// Bucket name. Every key this store sees is relative to its root.
    pub bucket: String,
    /// Credentials.
    pub credentials: S3Credentials,
    /// Path-style addressing (`{endpoint}/{bucket}/{key}`). `true` for every
    /// §A6.1 backend: RustFS, MinIO and a typical RGW install are reached by
    /// endpoint and bucket path, not by a `{bucket}.` DNS prefix that would
    /// need a wildcard record.
    pub path_style: bool,
    /// Whether to allow an endpoint that is not `https://`. `true` for RustFS
    /// on loopback; `false` in production, where the endpoint is TLS.
    pub allow_http: bool,
    /// Size bounds.
    pub limits: ArtifactLimits,
}

impl S3ArtifactStoreConfig {
    /// Reads the configuration from `VF_S3_ENDPOINT`, `VF_S3_REGION`,
    /// `VF_S3_BUCKET`, `VF_S3_ACCESS_KEY`, `VF_S3_SECRET_KEY`, the optional
    /// `VF_S3_SESSION_TOKEN` and the optional `VF_S3_PATH_STYLE` (default
    /// `true`), with the default limits.
    ///
    /// `allow_http` is not a variable: it is `true` exactly when `env` is not
    /// production, so a production binary cannot be pointed at a plaintext
    /// endpoint by configuration alone.
    ///
    /// # Errors
    ///
    /// [`AdapterSelectionError::MissingSetting`] for an absent required
    /// variable, [`AdapterSelectionError::InvalidSetting`] for a
    /// `VF_S3_PATH_STYLE` that is not a boolean.
    pub fn from_lookup(lookup: Lookup<'_>, env: DeployEnv) -> Result<Self, AdapterSelectionError> {
        Ok(Self {
            endpoint: required(lookup, ENV_S3_ENDPOINT)?,
            region: required(lookup, ENV_S3_REGION)?,
            bucket: required(lookup, ENV_S3_BUCKET)?,
            credentials: S3Credentials {
                access_key_id: required(lookup, ENV_S3_ACCESS_KEY)?,
                secret_access_key: required(lookup, ENV_S3_SECRET_KEY)?,
                session_token: lookup(ENV_S3_SESSION_TOKEN).filter(|t| !t.trim().is_empty()),
            },
            path_style: optional_bool(lookup, ENV_S3_PATH_STYLE, true)?,
            allow_http: !env.is_production(),
            limits: ArtifactLimits::new(),
        })
    }
}

/// Installs the rustls `ring` crypto provider for this process.
///
/// The §A5 `object_store` pin selects rustls with no provider compiled in, so
/// exactly one place in each binary has to choose one, and §A5 names task F3
/// as that place. [`S3ArtifactStore::new`] calls it, so a binary that builds
/// its store through this module never has to. Idempotent: a second call, or
/// a call after something else installed a provider, does nothing.
pub fn install_crypto_provider() {
    // An error means a provider is already installed, by an earlier call or
    // by the binary itself. Either way the process has one, which is all this
    // function promises.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// An [`ArtifactStore`] backed by an S3-compatible endpoint.
pub struct S3ArtifactStore {
    bucket: String,
    backend: ObjectStoreArtifactStore,
}

impl S3ArtifactStore {
    /// Builds the client from `config`, installing the crypto provider first.
    ///
    /// Does not contact the endpoint: a `put` or `get` is the first request,
    /// so a binary starts even when the object store is briefly unreachable
    /// and fails the request rather than the boot.
    ///
    /// # Errors
    ///
    /// [`ArtifactStoreError::Backend`] when `config` cannot be turned into a
    /// client — a malformed endpoint, or `allow_http: false` with an
    /// endpoint that does not start with `https://`.
    pub fn new(config: S3ArtifactStoreConfig) -> Result<Self, ArtifactStoreError> {
        install_crypto_provider();

        // Refused here rather than left to the first request, so that a
        // production binary pointed at a plaintext endpoint fails to start.
        // Only a literal `https://` passes: a test for `http://` would let a
        // schemeless endpoint through, and one with a tab or newline inside
        // the scheme, which the URL parser strips before it reads the scheme.
        if !config.allow_http
            && !config
                .endpoint
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("https://")
        {
            return Err(ArtifactStoreError::Backend {
                message: "the endpoint is not https and allow_http is false".to_owned(),
            });
        }

        // `AmazonS3Builder::new`, not `from_env`: nothing but `config` is read.
        let mut builder = AmazonS3Builder::new()
            .with_endpoint(config.endpoint)
            .with_region(config.region)
            .with_bucket_name(config.bucket.clone())
            .with_access_key_id(config.credentials.access_key_id)
            .with_secret_access_key(config.credentials.secret_access_key)
            .with_virtual_hosted_style_request(!config.path_style)
            .with_allow_http(config.allow_http)
            .with_checksum_algorithm(Checksum::SHA256);
        if let Some(token) = config.credentials.session_token {
            builder = builder.with_token(token);
        }
        let store = builder.build().map_err(|e| backend_error(&e))?;

        Ok(Self {
            bucket: config.bucket,
            backend: ObjectStoreArtifactStore::new(Arc::new(store), config.limits),
        })
    }

    /// The bucket this store writes to.
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// The limits this store enforces.
    #[must_use]
    pub fn limits(&self) -> ArtifactLimits {
        self.backend.limits()
    }
}

impl fmt::Debug for S3ArtifactStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3ArtifactStore")
            .field("bucket", &self.bucket)
            .field("limits", &self.backend.limits())
            .finish_non_exhaustive()
    }
}

impl ArtifactStore for S3ArtifactStore {
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
