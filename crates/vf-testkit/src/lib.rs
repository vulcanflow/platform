#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used, clippy::expect_used)]

//! `vf-testkit` — the local test harness (architecture §A1.3, §A6.2).
//!
//! A dev-dependency everywhere and shipped nowhere: the testcontainers
//! Postgres bootstrap with a migrated template schema, the fixture loaders, the
//! deterministic `Clock` and `IdGen`, dev token minting, and the fixture corpus
//! under `fixtures/`.
//!
//! `fixtures/` is a neutral-lane directory, so a coder may add a scanner output
//! sample without crossing into the test lane.
//!
//! # Layout
//!
//! | Module | §A6 role |
//! |---|---|
//! | [`db`] | Postgres for a test: migrated, templated, direct and pooled (§A6.1 Postgres row) |
//! | [`fixtures`] | the corpus under `crates/vf-testkit/fixtures/` |
//! | [`clock`] | the deterministic `Clock` of the §A6.1 `Clock` row |
//! | [`ids`] | the deterministic `IdGen` of the §A6.1 `IdGen` row |
//! | [`identity`] | dev-issuer token minting (§A6.4) |
//! | [`entitlements`] | illustrative package entitlements (open question O9) |
//!
//! # Nothing here is shipped, everything here is gated
//!
//! The crate is a `dev-dependencies` entry in every consumer and appears in no
//! binary. It is still held to the production gate — `#![forbid(unsafe_code)]`,
//! clippy `-D warnings`, no inline `cfg(test)` module in `src` — because a
//! harness that misleads a test is worse than no harness.
//!
//! (That phrase is spelled without the attribute brackets on purpose: the lane
//! gate greps production source for the literal attribute, and it is right to
//! — a mention is indistinguishable from a use to a grep, and the mention is
//! the cheaper thing to reword.)

pub mod clock;
pub mod db;
pub mod entitlements;
pub mod fixtures;
pub mod identity;
pub mod ids;

pub use clock::DeterministicClock;
pub use db::{Backend, TestDb};
pub use identity::{Claims, token};
pub use ids::SeqIdGen;

/// The result type every fallible entry point in this crate returns.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong in the harness.
///
/// A test that fails because the harness itself could not start has to say so
/// in a way a developer can act on without reading this crate, so every
/// variant carries the thing it was working on — the fixture name, the
/// database, the path — rather than a bare source error.
///
/// `#[non_exhaustive]`: a consumer matches the variants it handles and falls
/// through on the rest, so adding one when a later task fills a body in does
/// not break a test pack that already compiles.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A skeleton body that the F2 implementation commits replace.
    ///
    /// Present so the public API could be committed, reviewed and written
    /// against before the bodies existed (the "API skeleton first" rule of
    /// task F2). No body returns it since slice F2b; the variant stays because
    /// removing a public variant breaks any pack that names it, and the next
    /// skeleton-first change to this crate needs it again. The payload is the
    /// item that is missing, so a test that trips one names it in the failure
    /// rather than reporting a bare "not implemented".
    #[error("{0} is not implemented yet")]
    NotImplemented(&'static str),

    /// A fixture exists but could not be used as asked — not UTF-8 when read
    /// as text, not the JSON shape the caller named, unreadable.
    #[error("fixture `{name}`: {message}")]
    Fixture {
        /// The fixture name, relative to [`fixtures::root`].
        name: String,
        /// What was wrong with it.
        message: String,
    },

    /// A fixture name resolved outside the fixture tree.
    ///
    /// Its own variant rather than a not-found, because a `..` segment or an
    /// absolute path in a fixture name is a mistake in the *test*, and saying
    /// "no such fixture" would send the reader looking for a missing file.
    #[error("fixture name `{0}` escapes the fixture tree")]
    FixtureEscape(String),

    /// No fixture by that name.
    #[error("no fixture `{name}` (looked at {path})")]
    FixtureNotFound {
        /// The fixture name, relative to [`fixtures::root`].
        name: String,
        /// The absolute path that was tried, so it can be pasted into an editor.
        path: std::path::PathBuf,
    },

    /// The container runtime, or the compose stand-ins, could not give us a
    /// database. Carries the human-readable reason rather than the backend's
    /// own error type, so the backend can change without changing this enum.
    #[error("test backend: {0}")]
    Backend(String),

    /// A slug [`TestDb::create_tenant_schema`] cannot turn into a schema
    /// name: empty, longer than 56 bytes, or not lower-case ASCII letters,
    /// digits and `_`. A mistake in the test, so it is its own variant rather
    /// than the error Postgres would give for the resulting identifier.
    #[error("tenant slug `{0}` cannot name a schema: use 1 to 56 of [a-z0-9_]")]
    TenantSlug(String),

    /// Postgres said no.
    #[error("postgres: {0}")]
    Db(#[from] sqlx::Error),

    /// The `vf-db` migrations did not apply.
    ///
    /// Until task C3 lands there are no migration files and the migration step
    /// applies an empty set, so this variant is unreachable today. It is
    /// declared now because the signatures it belongs to are already public.
    #[error("migrations: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    /// Minting a dev-issuer token, or publishing its JWKS, failed (§A6.4).
    #[error("dev issuer: {0}")]
    Identity(String),

    /// A [`DeterministicClock`] was asked to move outside the range `chrono`
    /// can represent. Always a mistake in the test — a real deadline is never
    /// a quarter of a million years out — so it is reported rather than
    /// saturated.
    #[error("clock: {0}")]
    Clock(String),

    /// Filesystem trouble, with the path that caused it.
    #[error("i/o at {path}: {source}")]
    Io {
        /// The path being read or written.
        path: std::path::PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
}
