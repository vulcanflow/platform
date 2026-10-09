//! Infrastructure adapters for the §A6.1 ports (task F3).
//!
//! Owner: Jorge. The rest of `vf-db` is Fred's; this module is the review
//! boundary named in F3.
//!
//! # What lives here
//!
//! One adapter per backing service for the ports F3 owns:
//!
//! | Port | Real | Local stand-in | Test double |
//! |---|---|---|---|
//! | [`ArtifactStore`] | [`S3ArtifactStore`] (Ceph RGW) | [`S3ArtifactStore`] (RustFS) | [`LocalFsArtifactStore`], [`InMemoryArtifactStore`] |
//!
//! The `WakeBus` adapters follow in slice F3c.
//!
//! The real and the local artifact store are the *same* adapter against a
//! different endpoint, which is the point: RustFS is exercised through the S3
//! code path, so the local suite tests the production adapter rather than a
//! lookalike. All three artifact adapters delegate to
//! [`ObjectStoreArtifactStore`], which is not selectable by configuration.
//!
//! # Selection and the production refusal
//!
//! [`ArtifactStoreKind`] is read from `VF_ARTIFACT_STORE`, and [`DeployEnv`]
//! from `VF_ENV`. Under `VF_ENV=production` a test adapter (`memory`, `fs`) is
//! refused at selection time with
//! [`AdapterSelectionError::StandInRefusedInProduction`] — before any
//! connection is attempted, so a misconfigured production binary fails to
//! start rather than starting with a test double. §A6.1 states the rule for
//! `fake`, `memory` and `dev` adapters generally and lists `fs` as a test
//! adapter; the names this module owns are enforced here.
//!
//! The refusal guards configuration, not construction. Every adapter's
//! constructor is public and takes no [`DeployEnv`], so code in the process
//! can still build a test adapter under `VF_ENV=production`; a binary gets
//! the guard by building its store through [`artifact_store_from_env`], and
//! only through it. It also arms only on `VF_ENV=production`: see
//! [`DeployEnv`].
//!
//! Every entry point that reads configuration has a `*_from_lookup` twin that
//! takes the variables from a closure instead of the process environment, so
//! the refusal can be tested without mutating the environment of a running
//! test binary.

use std::sync::Arc;

use vf_core::ports::{ArtifactStore, ArtifactStoreError};

pub mod artifact;

pub use artifact::{
    ArtifactLimits, InMemoryArtifactStore, LocalFsArtifactStore, LocalFsArtifactStoreConfig,
    ObjectStoreArtifactStore, S3ArtifactStore, S3ArtifactStoreConfig, S3Credentials,
    install_crypto_provider,
};

/// The environment variable naming the [`ArtifactStore`] adapter.
pub const ENV_ARTIFACT_STORE: &str = "VF_ARTIFACT_STORE";

/// The environment variable naming the deployment environment.
pub const ENV_DEPLOY_ENV: &str = "VF_ENV";

/// A source of configuration variables: the process environment in a binary,
/// a map in a test. Returns `None` for an unset variable.
///
/// `Sync` so that a reference to it can be held across an `.await` by an
/// asynchronous constructor.
pub type Lookup<'a> = &'a (dyn Fn(&str) -> Option<String> + Sync);

/// [`Lookup`] over the process environment. A variable that is set but not
/// valid Unicode reads as unset, and then fails as a missing setting.
#[must_use]
pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why an adapter could not be selected or constructed from configuration.
///
/// Every variant is a startup failure. None of them is retried: a binary that
/// cannot name its adapters has no safe default to fall back to.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AdapterSelectionError {
    /// A required setting is absent from the environment.
    #[error("{name} is required but not set")]
    MissingSetting {
        /// The variable that was expected.
        name: &'static str,
    },

    /// A setting is present but not usable.
    ///
    /// `value` is never echoed: these variables can carry an endpoint or a
    /// path, and the message goes to logs.
    #[error("{name} is set but not usable: {reason}")]
    InvalidSetting {
        /// The variable involved.
        name: &'static str,
        /// Why it was refused.
        reason: String,
    },

    /// The adapter name is not one this build offers.
    ///
    /// The value given is not echoed, for the same reason as
    /// [`Self::InvalidSetting`]; the accepted names are enough to fix it.
    #[error("{name} is not a known adapter; expected one of {expected}")]
    UnknownAdapter {
        /// The variable involved.
        name: &'static str,
        /// The accepted names, comma separated.
        expected: &'static str,
    },

    /// A test adapter was selected under `VF_ENV=production` (§A6.1).
    #[error(
        "{name}={value} is a test adapter and is refused under {env}=production; \
         set a real adapter"
    )]
    StandInRefusedInProduction {
        /// The variable involved.
        name: &'static str,
        /// The adapter that was refused.
        value: &'static str,
        /// [`ENV_DEPLOY_ENV`], named so the message says which variable to
        /// look at.
        env: &'static str,
    },

    /// The adapter was selected but could not be constructed.
    #[error("the {} artifact store could not be built", kind.as_str())]
    ArtifactStore {
        /// Which adapter failed.
        kind: ArtifactStoreKind,
        /// The failure.
        #[source]
        source: ArtifactStoreError,
    },
}

/// Reads a required variable.
pub(crate) fn required(
    lookup: Lookup<'_>,
    name: &'static str,
) -> Result<String, AdapterSelectionError> {
    match lookup(name) {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(AdapterSelectionError::MissingSetting { name }),
    }
}

/// Reads an optional boolean variable: `true`/`false`/`1`/`0`, ignoring
/// ASCII case and surrounding whitespace. Unset or empty is `default`.
pub(crate) fn optional_bool(
    lookup: Lookup<'_>,
    name: &'static str,
    default: bool,
) -> Result<bool, AdapterSelectionError> {
    let Some(raw) = lookup(name) else {
        return Ok(default);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(AdapterSelectionError::InvalidSetting {
            name,
            reason: "expected true or false".to_owned(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Deployment environment
// ---------------------------------------------------------------------------

/// The deployment environment, from `VF_ENV`.
///
/// Only `production` has a behavioural meaning in this module, so everything
/// else collapses into [`Self::NonProduction`] rather than becoming a list
/// that has to be kept in step with the deployment tooling.
///
/// This fails open. An unset `VF_ENV`, or any value other than
/// `production` (`prod`, `production-eu`), is [`Self::NonProduction`], and
/// nothing else catches it: with `VF_ARTIFACT_STORE=memory` as well, a
/// production deployment starts with a test adapter. A production deployment
/// must therefore set `VF_ENV=production` exactly, and its manifests are where
/// that is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeployEnv {
    /// `VF_ENV=production`. Test adapters are refused.
    Production,
    /// Anything else, including unset.
    NonProduction,
}

impl DeployEnv {
    /// Reads `VF_ENV` from the process environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_lookup(&process_env)
    }

    /// Reads `VF_ENV` from `lookup`.
    #[must_use]
    pub fn from_lookup(lookup: Lookup<'_>) -> Self {
        lookup(ENV_DEPLOY_ENV).map_or(Self::NonProduction, |raw| Self::parse(&raw))
    }

    /// Classifies a `VF_ENV` value, ignoring ASCII case and surrounding
    /// whitespace.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        if raw.trim().eq_ignore_ascii_case("production") {
            Self::Production
        } else {
            Self::NonProduction
        }
    }

    /// Whether test adapters are refused.
    #[must_use]
    pub const fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

// ---------------------------------------------------------------------------
// Adapter names
// ---------------------------------------------------------------------------

/// The [`ArtifactStore`] adapters this build offers, named as in §A6.1
/// (`VF_ARTIFACT_STORE=s3|fs|memory`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactStoreKind {
    /// [`S3ArtifactStore`]: Ceph RGW in production, RustFS locally.
    S3,
    /// [`LocalFsArtifactStore`]: a directory on this machine. Test adapter.
    Fs,
    /// [`InMemoryArtifactStore`]: a process-lifetime map. Test adapter.
    Memory,
}

impl ArtifactStoreKind {
    /// The accepted `VF_ARTIFACT_STORE` values, for error messages.
    pub const EXPECTED: &'static str = "s3, fs, memory";

    /// The canonical name, which is also the accepted `VF_ARTIFACT_STORE`
    /// value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::Fs => "fs",
            Self::Memory => "memory",
        }
    }

    /// Whether this adapter is a test adapter, and so refused under
    /// `VF_ENV=production`.
    ///
    /// [`Self::Fs`] is one: §A6.1 lists it in the test-adapter column, it has
    /// no replication and no backup story, and its root is only as private as
    /// the directory permissions of one machine.
    #[must_use]
    pub const fn is_stand_in(self) -> bool {
        matches!(self, Self::Fs | Self::Memory)
    }

    /// Parses a `VF_ARTIFACT_STORE` value, ignoring ASCII case and
    /// surrounding whitespace. Only the §A6.1 names are accepted, so a typo is
    /// a startup error rather than a guess.
    ///
    /// # Errors
    ///
    /// [`AdapterSelectionError::UnknownAdapter`] for anything else.
    pub fn parse(raw: &str) -> Result<Self, AdapterSelectionError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "s3" => Ok(Self::S3),
            "fs" => Ok(Self::Fs),
            "memory" => Ok(Self::Memory),
            _ => Err(AdapterSelectionError::UnknownAdapter {
                name: ENV_ARTIFACT_STORE,
                expected: Self::EXPECTED,
            }),
        }
    }

    /// Reads `VF_ARTIFACT_STORE` from the process environment and refuses a
    /// test adapter under `VF_ENV=production`.
    ///
    /// # Errors
    ///
    /// As [`Self::from_lookup`].
    pub fn from_env(env: DeployEnv) -> Result<Self, AdapterSelectionError> {
        Self::from_lookup(&process_env, env)
    }

    /// Reads `VF_ARTIFACT_STORE` from `lookup` and refuses a test adapter
    /// under `VF_ENV=production`.
    ///
    /// # Errors
    ///
    /// [`AdapterSelectionError::MissingSetting`] when `VF_ARTIFACT_STORE` is
    /// unset, [`AdapterSelectionError::UnknownAdapter`] when it names no
    /// adapter, [`AdapterSelectionError::StandInRefusedInProduction`] when it
    /// names a test adapter and `env` is [`DeployEnv::Production`].
    pub fn from_lookup(lookup: Lookup<'_>, env: DeployEnv) -> Result<Self, AdapterSelectionError> {
        let kind = Self::parse(&required(lookup, ENV_ARTIFACT_STORE)?)?;
        kind.check_allowed(env)?;
        Ok(kind)
    }

    /// Refuses this adapter when it is a test adapter and `env` is
    /// production.
    ///
    /// # Errors
    ///
    /// [`AdapterSelectionError::StandInRefusedInProduction`].
    pub fn check_allowed(self, env: DeployEnv) -> Result<(), AdapterSelectionError> {
        if env.is_production() && self.is_stand_in() {
            return Err(AdapterSelectionError::StandInRefusedInProduction {
                name: ENV_ARTIFACT_STORE,
                value: self.as_str(),
                env: ENV_DEPLOY_ENV,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Construction from configuration
// ---------------------------------------------------------------------------

/// Builds the [`ArtifactStore`] named by `VF_ARTIFACT_STORE` in the process
/// environment.
///
/// This is the one function a binary's startup path calls. The result is not
/// tenant-scoped: each worker wraps it in
/// [`vf_core::ports::TenantArtifactStore`] for the tenant it is serving.
///
/// # Errors
///
/// As [`artifact_store_from_lookup`].
pub fn artifact_store_from_env() -> Result<Arc<dyn ArtifactStore>, AdapterSelectionError> {
    artifact_store_from_lookup(&process_env)
}

/// Builds the [`ArtifactStore`] named by `VF_ARTIFACT_STORE` in `lookup`.
///
/// Never returns a test adapter under `VF_ENV=production`, and reads every
/// setting the adapter needs before building it, so a configuration mistake
/// surfaces as a startup error rather than as a timeout on the first request.
///
/// # Errors
///
/// [`AdapterSelectionError`]: the selection errors of
/// [`ArtifactStoreKind::from_lookup`], the configuration errors of the
/// selected adapter, and [`AdapterSelectionError::ArtifactStore`] when the
/// adapter cannot be built from its configuration.
pub fn artifact_store_from_lookup(
    lookup: Lookup<'_>,
) -> Result<Arc<dyn ArtifactStore>, AdapterSelectionError> {
    let env = DeployEnv::from_lookup(lookup);
    let kind = ArtifactStoreKind::from_lookup(lookup, env)?;
    let built: Result<Arc<dyn ArtifactStore>, ArtifactStoreError> = match kind {
        ArtifactStoreKind::S3 => {
            let config = S3ArtifactStoreConfig::from_lookup(lookup, env)?;
            S3ArtifactStore::new(config).map(|store| Arc::new(store) as Arc<dyn ArtifactStore>)
        }
        ArtifactStoreKind::Fs => {
            let config = LocalFsArtifactStoreConfig::from_lookup(lookup)?;
            LocalFsArtifactStore::new(config).map(|store| Arc::new(store) as Arc<dyn ArtifactStore>)
        }
        ArtifactStoreKind::Memory => Ok(Arc::new(InMemoryArtifactStore::new())),
    };
    built.map_err(|source| AdapterSelectionError::ArtifactStore { kind, source })
}
