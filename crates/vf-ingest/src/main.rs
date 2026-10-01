#![forbid(unsafe_code)]
//! Findings artifact ingest (TDD §8.4, §10).
//!
//! # Invariants
//!
//! **Fresh observations per scan.** Each scan produces its own observations; ingest
//! does not mutate a previous scan's rows (§6.3, §10.4,
//! §25 `findings/new-per-scan`).
//!
//! **Replay is idempotent.** The same artifact delivered twice produces one
//! outcome, because secureCodeBox hooks retry (§25 `findings/replayed-artifact`).
//!
//! **Only explicit false positives carry forward.** Nothing else inherits state
//! from an earlier scan (§10.2, §25 `findings/fp-only-persistence`).
//!
//! **Scanner output is attacker-influenced.** Every parser here is a fuzz target
//! (§23.1, §25 `fuzz/untrusted-input`) and must not panic on hostile input.

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
