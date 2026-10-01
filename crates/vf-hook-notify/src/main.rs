#![forbid(unsafe_code)]
//! secureCodeBox scan-completion hook (TDD §8.4, §21.3).
//!
//! Runs inside the scan namespace as a pinned arm64 image and tells the control
//! plane that a scan's artifacts are ready. VulcanFlow writes this hook in Rust;
//! the secureCodeBox operator, lurker and stock hooks stay upstream (§2.5.3).
//!
//! # Invariants
//!
//! **Notify, do not interpret.** The hook reports that artifacts exist. Parsing and
//! normalisation happen in `vf-ingest`, inside the control plane, where the input
//! is treated as hostile.
//!
//! **At-least-once delivery.** The hook can run twice for one scan, so the
//! notification carries the scan identity secureCodeBox exposes and the receiver
//! deduplicates (§6.2, §25 `execution/scan-identity`).
//!
//! **Least privilege.** It needs no cluster write permission and no tenant
//! database credential.

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
