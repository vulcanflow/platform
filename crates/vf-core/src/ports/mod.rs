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
//! F1 published the trait names only; §A6.1 fixes no method signatures, and
//! each method arrives with the task that owns its port. Task F3 fills in
//! [`ArtifactStore`] and [`WakeBus`] (see the [`artifact`] and [`wake`]
//! submodules); the traits still listed inline below are empty until their own
//! task lands (C1 for [`Clock`] and [`IdGen`], G3 for [`ChallengeProbe`], O1
//! for [`ScanRuntime`], O2 for [`TenantProvisioner`], A1 for
//! [`TokenVerifier`]). The `Send + Sync + 'static` bounds are part of the
//! §A6.1 contract, because every port is held behind `Arc<dyn Port>`.
//!
//! # Why the port methods are not `async fn`
//!
//! Every method returns [`PortFuture`], a boxed future, rather than being an
//! `async fn` in a trait. Two reasons, both structural:
//!
//! 1. A port is always held as `Arc<dyn Port>` (§A6.1). An `async fn` in a
//!    trait is not dyn-compatible without the same boxing written at every
//!    call site, so the boxing goes in the signature once.
//! 2. §A1.3 keeps `vf-core` free of every async crate, which rules out
//!    `async-trait` and `futures::future::BoxFuture`. [`PortFuture`] is the
//!    same shape built from `core` alone.

use std::future::Future;
use std::pin::Pin;

pub mod artifact;
pub mod wake;

pub use artifact::{
    ArtifactBody, ArtifactContent, ArtifactKey, ArtifactMeta, ArtifactPrefix, ArtifactSource,
    ArtifactStore, ArtifactStoreError, DigestParseError, MAX_ARTIFACT_KEY_BYTES, PutReceipt,
    Sha256Digest, TenantArtifactStore,
};
pub use wake::{
    BucketKey, MAX_WAKE_NAME_BYTES, MAX_WAKE_PAYLOAD_BYTES, Permit, TokenRate, Topic, WakeBus,
    WakeBusError, WakeMessage, WakeStream, WakeSubscription,
};

/// The return type of every asynchronous port method.
///
/// A pinned, boxed, `Send` future borrowing the port for `'a`. Written out of
/// `core` rather than taken from `futures`, because §A1.3 forbids an async
/// dependency in this crate; it is the same type `futures::future::BoxFuture`
/// names.
///
/// `Send` is required: the adapters run on the multi-threaded tokio runtime and
/// every port future is spawned or awaited across a `.await` that may move
/// between workers.
pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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
