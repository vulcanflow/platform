#![forbid(unsafe_code)]
//! VulcanFlow control-plane API (TDD §13).
//!
//! REST only at GA — Connect-RPC is withdrawn with the rest of the Go stack
//! (TDD §2.5.2, §27 item 19, ADR-0003). The OpenAPI 3.1 document is generated
//! from handler types with `utoipa` so it cannot drift from the handlers, and
//! §25 `api/openapi-3_1-conformance` validates the emitted document against the
//! 3.1 metaschema in CI.
//!
//! # Invariants
//!
//! **Enforcement is a layer, not a convention.** Role policy (§4.2) is a `tower`
//! layer, so a handler added without thinking about authorization is still
//! covered.
//!
//! **Safety-relevant decisions are delegated, never re-implemented.** Scope
//! matching and allowance reservation live in `vf-core`; tenant data is reached
//! only through `vf-db`'s `TenantTx`. `vf-dispatcher` is a module of this binary
//! (§2.3), not a separate service, and its reservation logic calls `vf-core`.
//!
//! **A panic is a platform error.** Handler panics are caught at the service
//! boundary (`tower-http`'s `catch-panic`) and reported as platform errors, never
//! as a target outcome (§2.5.2). This is why the release profile unwinds.

/// Installs the process-wide rustls cryptographic provider.
///
/// `rustls 0.23` declares both `aws-lc-rs` and `ring` as optional backends. This
/// workspace reaches rustls through `reqwest`, `sqlx`, `kube` and `aws-sdk-s3`,
/// and if feature unification ever enables both backends then both compile,
/// rustls cannot pick a process default, and the failure is a **panic at the
/// first TLS handshake** rather than a build error (ADR-0002 §3.6 / A1).
///
/// Installing the provider explicitly, before any client is constructed, moves
/// that from "a property of feature resolution" to "a line we wrote": a
/// collision surfaces here, at startup, naming the cause. The CI assertions
/// `cargo tree -i ring` (empty) and `cargo tree -d` (no duplicate `rustls`) are
/// the build-time half of the same check.
///
/// # Errors
///
/// Returns an error if a provider is already installed, which means something
/// constructed a TLS client before this call or installed a different provider.
fn install_crypto_provider() -> anyhow::Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| {
            anyhow::anyhow!(
                "a rustls crypto provider was already installed; \
                 install_crypto_provider must run before any TLS client is built"
            )
        })
}

fn main() -> anyhow::Result<()> {
    // `try_init` rather than `init`: a second initialisation returns an error
    // instead of panicking, and `clippy::panic` is denied workspace-wide.
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    // Before anything that could build a TLS client.
    install_crypto_provider()?;

    tracing::info!(
        crate_name = env!("CARGO_PKG_NAME"),
        "scaffold binary: no behaviour implemented yet"
    );
    Ok(())
}
