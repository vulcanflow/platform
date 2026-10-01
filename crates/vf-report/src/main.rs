#![forbid(unsafe_code)]
//! Report assembly and rendering (TDD §16).
//!
//! # Invariants
//!
//! **Assembly is durable and snapshot-based.** A report renders from an immutable
//! input snapshot, so a worker restart resumes rather than re-derives, and two
//! reports over the same cutoff agree (§16.4, §16.8,
//! §25 `report/cross-consistency`, `report/worker-and-queue-loss`).
//!
//! **Escaping is per template context.** `askama` escapes HTML bodies by default;
//! an attribute, a URL, a `<script>` block and a CSS value each need different
//! treatment, and `|safe` disables escaping entirely. Scanner output is
//! attacker-influenced text that lands in a customer-facing report, which is why
//! §25 `report/xss-network-corpus` exists. Per ADR-0002 §5, a `|safe` in a report
//! template is a required change at review unless the pull request explains why
//! the value cannot carry hostile content.
//!
//! **The renderer has no network.** Not a configuration preference — an isolated
//! renderer with egress denied, so a report can never fetch a remote resource.
//!
//! **The disclaimer is versioned and mandatory** (§16.7).

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
