//! Dev-issuer tokens for tests (architecture §A6.4).
//!
//! §A6.4 puts the dev issuer behind the `vf-api` feature `dev-auth`, as a
//! `vf-api dev-issuer` subcommand that serves a JWKS and mints tokens, and
//! says `vf-testkit::token(tenant, role)` wraps it for tests.
//!
//! # Why this module does not depend on `vf-api`
//!
//! Two reasons, and they both point the same way. `vf-api` is a binary crate
//! that task A1 has not written yet, and `vf-testkit` is a dev-dependency of
//! `vf-api` itself — a dependency back the other way would be a cycle. So the
//! shared artefact between the subcommand and this module is the *key
//! material and the claim set*, not the binary: both sign the §A6.4 claims
//! (`sub`, `tenant_id`, `roles[]`, `package`, `kyc_level`) with the same dev
//! issuer key, so a token minted here verifies against a JWKS served there and
//! the other way round. Task A1 reuses [`Claims`] rather than redeclaring it.
//!
//! # The key material is not in the repository
//!
//! No private key is committed. [`DevIssuer::fixture`] derives the dev
//! keypair deterministically from [`DevIssuer::FIXTURE_SEED`], so every
//! machine and every process gets the same key and the same `kid` without a
//! PEM in git; [`DevIssuer::from_pem_env`] exists for the developer who wants
//! to point the harness at a key they generated themselves, read from the
//! environment and never from a tracked file.
//!
//! These tokens are accepted only when `VF_AUTH=dev`, and §A6.4 requires every
//! binary to refuse that setting under `VF_ENV=production`.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, TimeDelta, Utc};

use crate::Result;

/// The `iss` claim the dev issuer signs, and the issuer a test configures a
/// `TokenVerifier` with.
///
/// A `.test` host, which RFC 2606 reserves and no resolver will answer, so a
/// misconfigured verifier fails to reach it instead of reaching something.
pub const DEV_ISSUER_URL: &str = "https://dev-issuer.vulcanflow.test/realms/vulcanflow";

/// The `aud` claim the dev issuer signs.
pub const DEV_AUDIENCE: &str = "vf-api";

/// Mints a token for `role` in `tenant` against the dev issuer keys.
///
/// The §A6.4 spelling, with the third argument that task F2's scope adds for
/// the claims §A6.4 lists beyond the first two:
///
/// ```ignore
/// let t = token("acme", "owner", Claims::default())?;          // the common case
/// let t = token("acme", "analyst", Claims::default().kyc_level(2))?;
/// ```
///
/// `tenant` is the tenant slug; it is written to `tenant_id`. `role` is added
/// to [`Claims::roles`], so a single-role token — almost every token in a test
/// — needs no builder call at all.
pub fn token(tenant: &str, role: &str, claims: Claims) -> Result<String> {
    DevIssuer::fixture()?.mint(tenant, role, claims)
}

/// What to put in the token beyond the tenant and the role.
///
/// The minted body is the §A6.4 claim set — `sub`, `tenant_id`, `roles`,
/// `package`, `kyc_level`, plus `iss`, `aud`, `iat`, `exp` and anything in
/// [`Claims::extra`]. This type is the *input* to that, not its serialized
/// shape: [`Claims::ttl`] becomes `exp`, and [`DevIssuer`] supplies `iss` and
/// `aud`. Assembling the body is [`DevIssuer::mint`]'s job, so the wire names
/// live in one place and this builder can stay readable.
///
/// Every field has a usable default, so a test that does not care about an
/// attribute does not mention it — and, more importantly, a test that *does*
/// care names exactly the attribute it is about, which is what makes the
/// failure readable.
#[derive(Debug, Clone, Default)]
pub struct Claims {
    /// `sub`. Defaults to a stable fixture subject derived from the tenant and
    /// role, so two tokens for one role in one tenant are the same principal.
    pub subject: Option<String>,
    /// `roles`. [`token`] pushes its `role` argument here; add more for a
    /// principal that holds several.
    pub roles: Vec<String>,
    /// `package`. Left `None` by default rather than given an invented value:
    /// the package names are open question O9 and belong to Product (see
    /// [`crate::entitlements`]).
    pub package: Option<String>,
    /// `kyc_level`. Defaults to 0 — unverified — because that is the value a
    /// path that forgot to check should fail on.
    pub kyc_level: u8,
    /// How long the token is valid for, from [`Claims::issued_at`]. Defaults
    /// to one hour.
    pub ttl: Option<TimeDelta>,
    /// `iat`. Defaults to [`crate::clock::epoch`], so a signed token is itself
    /// reproducible; set it from a [`crate::DeterministicClock`] when the test
    /// is about expiry.
    pub issued_at: Option<DateTime<Utc>>,
    /// Claims beyond the §A6.4 set.
    ///
    /// Sorted, so the serialized body — and therefore the signature — is
    /// byte-identical across runs. That is what lets a test assert on a token
    /// string at all.
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Claims {
    /// Sets `sub`.
    #[must_use]
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Adds a role on top of the one [`token`] supplies.
    #[must_use]
    pub fn role(mut self, role: impl Into<String>) -> Self {
        self.roles.push(role.into());
        self
    }

    /// Sets `package`.
    #[must_use]
    pub fn package(mut self, package: impl Into<String>) -> Self {
        self.package = Some(package.into());
        self
    }

    /// Sets `kyc_level`.
    #[must_use]
    pub fn kyc_level(mut self, level: u8) -> Self {
        self.kyc_level = level;
        self
    }

    /// Sets the validity window, and therefore `exp`.
    ///
    /// A negative or zero `ttl` is allowed and is the point of the method: it
    /// is how a test gets an already-expired token to prove the verifier
    /// refuses it.
    #[must_use]
    pub fn ttl(mut self, ttl: TimeDelta) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Sets `iat`.
    #[must_use]
    pub fn issued_at(mut self, at: DateTime<Utc>) -> Self {
        self.issued_at = Some(at);
        self
    }

    /// Adds a claim outside the §A6.4 set.
    #[must_use]
    pub fn claim(mut self, name: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.extra.insert(name.into(), value.into());
        self
    }
}

/// The dev issuer's signing key and the JWKS that matches it (§A6.4).
///
/// Held as a value rather than reached through a global, so a test that needs
/// two issuers — to prove a token signed by the wrong one is refused — makes
/// two.
///
/// `Debug` is written by hand and redacts the key. It is a throwaway key for a
/// throwaway issuer, but a harness that prints private keys into test output
/// teaches the wrong habit, and test output ends up in CI logs.
pub struct DevIssuer {
    issuer: String,
    audience: String,
    key_id: String,
    /// PKCS#8 PEM. Private, so the representation can become
    /// `secrecy::SecretString` when the bodies land without that being an API
    /// change.
    private_key_pem: String,
}

impl fmt::Debug for DevIssuer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DevIssuer")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("key_id", &self.key_id)
            .field("private_key_pem", &"<redacted>")
            .finish()
    }
}

impl DevIssuer {
    /// The seed [`DevIssuer::fixture`] derives its keypair from.
    ///
    /// Public, constant and useless: it signs tokens that only a verifier
    /// configured with `VF_AUTH=dev` accepts, and §A6.4 forbids that setting
    /// in production. It is spelled out here, rather than hidden, so nobody
    /// mistakes it for a secret that leaked.
    pub const FIXTURE_SEED: &[u8] = b"vulcanflow dev issuer fixture key, not a secret";

    /// The shared dev issuer: [`DevIssuer::FIXTURE_SEED`], [`DEV_ISSUER_URL`],
    /// [`DEV_AUDIENCE`].
    ///
    /// Deterministic — the same key and the same `kid` in every process — so a
    /// token minted in one test binary verifies in another, and the JWKS can
    /// be written into a fixture.
    pub fn fixture() -> Result<Self> {
        Err(crate::Error::NotImplemented("DevIssuer::fixture"))
    }

    /// An issuer that signs with the PKCS#8 PEM private key in the environment
    /// variable `var`.
    ///
    /// For the developer running against their own `vf-api dev-issuer`, or a
    /// Keycloak from the optional compose profile. The key is named by
    /// variable rather than passed as a string so that the only way to use this
    /// is the way that keeps the key out of the repository and out of the
    /// process's argv.
    pub fn from_pem_env(var: &str) -> Result<Self> {
        let _ = var;
        Err(crate::Error::NotImplemented("DevIssuer::from_pem_env"))
    }

    /// Overrides `iss`, for the test that proves a token from an unexpected
    /// issuer is refused.
    #[must_use]
    pub fn with_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = issuer.into();
        self
    }

    /// Overrides `aud`, for the equivalent audience test.
    #[must_use]
    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = audience.into();
        self
    }

    /// The `iss` this issuer signs.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The `aud` this issuer signs.
    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// The `kid` in the header of every token this issuer mints, and the key
    /// id in its [`DevIssuer::jwks`].
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// The public JWKS document, in the shape a `TokenVerifier` fetches from a
    /// real issuer's `/protocol/openid-connect/certs`.
    ///
    /// Serve it from a `wiremock` host to test the JWKS path end to end, or
    /// hand it to a verifier directly to test only the signature check.
    pub fn jwks(&self) -> Result<serde_json::Value> {
        Err(crate::Error::NotImplemented("DevIssuer::jwks"))
    }

    /// Mints a signed compact JWT.
    ///
    /// `role` is appended to `claims.roles`; the rest of the §A6.4 claim set
    /// comes from `claims` and this issuer's `iss`/`aud`.
    pub fn mint(&self, tenant: &str, role: &str, claims: Claims) -> Result<String> {
        let _ = (tenant, role, claims);
        Err(crate::Error::NotImplemented("DevIssuer::mint"))
    }

    /// Mints a token whose signature does not match its body.
    ///
    /// Separate from [`DevIssuer::mint`] because "the verifier refuses a
    /// tampered token" is a test every protected path needs, and building the
    /// broken token by hand in each one is how a test ends up asserting on a
    /// malformed token instead of a mis-signed one.
    pub fn mint_mis_signed(&self, tenant: &str, role: &str, claims: Claims) -> Result<String> {
        let _ = (tenant, role, claims);
        Err(crate::Error::NotImplemented("DevIssuer::mint_mis_signed"))
    }
}
