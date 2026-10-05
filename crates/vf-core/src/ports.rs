//! Port traits — the single boundary between application logic and
//! infrastructure (architecture §A6.1).
//!
//! Application code depends on `Arc<dyn Port>` injected at startup from
//! configuration; no application crate names a concrete adapter. Each trait is
//! implemented in the crate named below, once for the real backing service,
//! once for the local stand-in from `just dev-up`, and once as a test double.
//!
//! Configuration selects the implementation by name (`VF_ARTIFACT_STORE`,
//! `VF_SCAN_RUNTIME`, `VF_WAKE_BUS`, `VF_AUTH`). The binaries refuse to start
//! with a `fake`, `memory` or `dev` adapter when `VF_ENV=production`.
//!
//! F1 publishes the trait names only; §A6.1 fixes no method signatures, and
//! each method arrives with the task that owns its port (F3 for
//! [`ArtifactStore`] and [`WakeBus`], C1 for [`Clock`] and [`IdGen`], G3 for
//! [`ChallengeProbe`], O1 for [`ScanRuntime`], O2 for [`TenantProvisioner`],
//! A1 for [`TokenVerifier`]). The `Send + Sync + 'static` bounds are part of
//! the §A6.1 contract, because every port is held behind `Arc<dyn Port>`.

/// Artifact object storage. Real: `object_store` S3 to Ceph RGW or RustFS.
/// Local: `object_store` S3 to a RustFS container. Test: in-memory and
/// local-filesystem doubles. Implemented in `vf-db::adapters` (task F3).
pub trait ArtifactStore: Send + Sync + 'static {}

/// Wake-up publish/subscribe and token buckets. Real and local: `redis`
/// against Valkey. Test: an in-process tokio broadcast bus. Implemented in
/// `vf-db::adapters` (task F3).
pub trait WakeBus: Send + Sync + 'static {}

/// Scan execution. Real: `kube-rs` against secureCodeBox CRDs. Local and test:
/// a fixture-scripted fake. Implemented in `vf-operator` (task O1).
pub trait ScanRuntime: Send + Sync + 'static {}

/// Tenant provisioning. Real: Kubernetes namespace plus schema plus bucket
/// prefix. Local and test: schema, prefix and bus namespace, Kubernetes steps
/// skipped. Implemented in `vf-operator` (task O2).
pub trait TenantProvisioner: Send + Sync + 'static {}

/// Track A challenge verification. Real: `hickory-resolver` and `reqwest`
/// against the internet. Local: the same against a `hickory-server` DNS fixture
/// and a wiremock HTTP host. Test: a fixture probe. Implemented in `vf-authz`
/// (task G3).
pub trait ChallengeProbe: Send + Sync + 'static {}

/// Bearer token verification. Real: Keycloak JWKS. Local: the dev issuer of
/// §A6.4 or an optional Keycloak compose profile. Test: static keys.
/// Implemented in `vf-api` (task A1).
pub trait TokenVerifier: Send + Sync + 'static {}

/// Wall-clock time. Real and local: the system clock. Test: deterministic.
/// Implemented in `vf-core` (task C1).
pub trait Clock: Send + Sync + 'static {}

/// Identifier generation (UUID v7). Real and local: the system CSPRNG. Test:
/// deterministic. Implemented in `vf-core` (task C1).
pub trait IdGen: Send + Sync + 'static {}

/// Outbound mail. M4; real: an SMTP provider, local: Mailpit. No task yet.
pub trait Mailer: Send + Sync + 'static {}

/// PDF rendering. M4; real and local: headless Chromium. No task yet.
pub trait Renderer: Send + Sync + 'static {}
