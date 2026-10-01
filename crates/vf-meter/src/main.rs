#![forbid(unsafe_code)]
//! Target and scan allowance accounting and the usage ledger (TDD §17).
//!
//! # Invariants
//!
//! **Integers, and only integers.** Reservations and settlements are `i64` with
//! checked arithmetic. No `f64`, no decimal crate — ADR-0002 §3.5 makes this a
//! hard constraint because a §17.3 accounting error is a billing bug.
//!
//! **Reservation is atomic and happens before work starts** (§5.7, §8.1,
//! §25 `execution/gates-before-start`).
//!
//! **Failed work settles to zero; successful work settles exactly once.** Retries
//! must not double-charge, and concurrent cascades must respect the remaining
//! allowance rather than each other's optimism (§25
//! `usage/failure-retry-settlement`, `usage/concurrent-reservations`).
//!
//! **A limit permits partial work with visible skips** rather than failing the
//! whole scan silently (§17.2, §25 `usage/remaining-work-notification`).
//!
//! The arithmetic itself lives in `vf-core` so it can be property-tested without a
//! database.

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
