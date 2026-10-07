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
//!
//! # Ed25519
//!
//! The fixture issuer signs `EdDSA` over Ed25519, because Ed25519 is the one
//! scheme whose private key *is* a 32-byte seed: deriving it from a constant
//! needs a hash and nothing else, where RSA would need a seeded prime search
//! and P-256 a scalar range check. Its signatures are deterministic too, so a
//! token minted from fixed claims is byte-identical on every run.
//! [`DevIssuer::from_pem_env`] also takes P-256, P-384 and RSA keys, for a
//! developer pointing the harness at an issuer that uses one of those.
//!
//! The `kid` is the RFC 7638 thumbprint of the public key, so it changes
//! exactly when the key does.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, TimeDelta, Utc};
use jsonwebtoken::jwk::{Jwk, JwkSet, PublicKeyUse, ThumbprintHash};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

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
    ///
    /// The default therefore puts `exp` at `2026-01-01T01:00:00Z`. A verifier
    /// that checks expiry against the injected `Clock` accepts it; one that
    /// reads the system clock refuses it, so set this to the wall clock when
    /// the code under test does.
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
    algorithm: Algorithm,
    /// The private key. Never printed; see the `Debug` impl.
    signing_key: EncodingKey,
    /// The public half, as the JWK [`DevIssuer::jwks`] publishes, `kid`
    /// included.
    public_key: Jwk,
}

impl fmt::Debug for DevIssuer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DevIssuer")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("key_id", &self.key_id)
            .field("algorithm", &self.algorithm)
            .field("signing_key", &"<redacted>")
            .finish()
    }
}

/// How long a token is valid for when [`Claims::ttl`] is not set.
fn default_ttl() -> TimeDelta {
    TimeDelta::hours(1)
}

/// The DER prefix that makes a 32-byte Ed25519 seed a PKCS#8 private key
/// (RFC 8410 §7): a version-0 `OneAsymmetricKey` whose algorithm is
/// id-Ed25519 (1.3.101.112) and whose `privateKey` is an OCTET STRING
/// wrapping the 32-byte `CurvePrivateKey` OCTET STRING. The seed follows it.
const ED25519_PKCS8_PREFIX: [u8; 16] = [
    0x30, 0x2e, // SEQUENCE, 46 bytes
    0x02, 0x01, 0x00, // INTEGER 0 (version)
    0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, // SEQUENCE { OID 1.3.101.112 }
    0x04, 0x22, 0x04, 0x20, // OCTET STRING { OCTET STRING, 32 bytes }
];

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
        // The Ed25519 private key is the 32-byte seed itself, so the
        // derivation is one hash.
        let seed = Sha256::digest(Self::FIXTURE_SEED);
        let mut der = Vec::with_capacity(ED25519_PKCS8_PREFIX.len() + seed.len());
        der.extend_from_slice(&ED25519_PKCS8_PREFIX);
        der.extend_from_slice(&seed);
        Self::new(EncodingKey::from_ed_der(&der), Algorithm::EdDSA)
    }

    /// An issuer that signs with the PKCS#8 PEM private key in the environment
    /// variable `var`.
    ///
    /// For the developer running against their own `vf-api dev-issuer`, or a
    /// Keycloak from the optional compose profile. The key is named by
    /// variable rather than passed as a string so that the only way to use this
    /// is the way that keeps the key out of the repository and out of the
    /// process's argv.
    ///
    /// The algorithm follows the key: `EdDSA` for Ed25519, `ES256` or `ES384`
    /// for a P-256 or P-384 key, `RS256` for RSA. No error message includes
    /// the variable's value.
    pub fn from_pem_env(var: &str) -> Result<Self> {
        let pem = std::env::var(var).map_err(|e| {
            Error::Identity(match e {
                std::env::VarError::NotPresent => {
                    format!("environment variable `{var}` is not set")
                }
                // Not `e.to_string()`: that would print the value.
                std::env::VarError::NotUnicode(_) => {
                    format!("environment variable `{var}` is not valid UTF-8")
                }
            })
        })?;
        let pem = pem.as_bytes();

        if let Ok(key) = EncodingKey::from_ed_pem(pem) {
            return Self::new(key, Algorithm::EdDSA);
        }
        if let Ok(key) = EncodingKey::from_ec_pem(pem) {
            // The curve decides the algorithm; deriving the public key under
            // the wrong one fails, so the first that succeeds is the curve.
            for algorithm in [Algorithm::ES256, Algorithm::ES384] {
                if let Ok(issuer) = Self::new(key.clone(), algorithm) {
                    return Ok(issuer);
                }
            }
        }
        if let Ok(key) = EncodingKey::from_rsa_pem(pem) {
            return Self::new(key, Algorithm::RS256);
        }
        Err(Error::Identity(format!(
            "`{var}` does not hold a PEM private key this issuer can sign with \
             (Ed25519, P-256, P-384 or RSA)"
        )))
    }

    /// An issuer at [`DEV_ISSUER_URL`] and [`DEV_AUDIENCE`] signing with `key`
    /// under `algorithm`.
    fn new(signing_key: EncodingKey, algorithm: Algorithm) -> Result<Self> {
        let mut public_key =
            Jwk::from_encoding_key(&signing_key, algorithm).map_err(identity_error)?;
        let key_id = public_key
            .thumbprint(ThumbprintHash::SHA256)
            .map_err(identity_error)?;
        public_key.common.key_id = Some(key_id.clone());
        public_key.common.public_key_use = Some(PublicKeyUse::Signature);
        Ok(Self {
            issuer: DEV_ISSUER_URL.to_owned(),
            audience: DEV_AUDIENCE.to_owned(),
            key_id,
            algorithm,
            signing_key,
            public_key,
        })
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
        serde_json::to_value(JwkSet {
            keys: vec![self.public_key.clone()],
        })
        .map_err(|e| Error::Identity(format!("serialising the JWKS: {e}")))
    }

    /// Mints a signed compact JWT.
    ///
    /// `role` is appended to `claims.roles`; the rest of the §A6.4 claim set
    /// comes from `claims` and this issuer's `iss`/`aud`.
    ///
    /// A name in [`Claims::extra`] that is also one of the claims above
    /// replaces it. That is deliberate: it is how a test mints a token whose
    /// `tenant_id` is a number, or whose `roles` is a string, to prove the
    /// verifier refuses the shape and not only the signature.
    pub fn mint(&self, tenant: &str, role: &str, claims: Claims) -> Result<String> {
        let header = Header {
            kid: Some(self.key_id.clone()),
            ..Header::new(self.algorithm)
        };
        let body = self.body(tenant, role, claims)?;
        jsonwebtoken::encode(&header, &body, &self.signing_key).map_err(identity_error)
    }

    /// Mints a token whose signature does not match its body.
    ///
    /// Separate from [`DevIssuer::mint`] because "the verifier refuses a
    /// tampered token" is a test every protected path needs, and building the
    /// broken token by hand in each one is how a test ends up asserting on a
    /// malformed token instead of a mis-signed one.
    ///
    /// The header and body are exactly what [`DevIssuer::mint`] produces for
    /// the same arguments. The signature is that token's with the first
    /// base64url character replaced by another, which changes its first byte
    /// and nothing else: still well-formed, still the right length, no longer
    /// valid.
    pub fn mint_mis_signed(&self, tenant: &str, role: &str, claims: Claims) -> Result<String> {
        let token = self.mint(tenant, role, claims)?;
        let (signed, signature) = token
            .rsplit_once('.')
            .ok_or_else(|| Error::Identity("a minted token has no signature segment".to_owned()))?;
        let mut chars = signature.chars();
        let replacement = match chars.next() {
            Some('A') => 'B',
            Some(_) => 'A',
            None => {
                return Err(Error::Identity(
                    "a minted token has an empty signature".to_owned(),
                ));
            }
        };
        Ok(format!("{signed}.{replacement}{}", chars.as_str()))
    }

    /// The JWT body: the §A6.4 claims, the registered ones, then
    /// [`Claims::extra`] over the top.
    fn body(
        &self,
        tenant: &str,
        role: &str,
        claims: Claims,
    ) -> Result<serde_json::Map<String, serde_json::Value>> {
        let Claims {
            subject,
            mut roles,
            package,
            kyc_level,
            ttl,
            issued_at,
            extra,
        } = claims;
        roles.push(role.to_owned());
        let issued_at = issued_at.unwrap_or_else(crate::clock::epoch);
        let ttl = ttl.unwrap_or_else(default_ttl);
        let expires_at = issued_at
            .checked_add_signed(ttl)
            .ok_or_else(|| Error::Identity(format!("{issued_at} + {ttl} is out of range")))?;

        let mut body = serde_json::Map::new();
        body.insert("iss".to_owned(), self.issuer.clone().into());
        body.insert("aud".to_owned(), self.audience.clone().into());
        body.insert(
            "sub".to_owned(),
            subject
                .unwrap_or_else(|| default_subject(tenant, role))
                .into(),
        );
        body.insert("iat".to_owned(), issued_at.timestamp().into());
        body.insert("exp".to_owned(), expires_at.timestamp().into());
        body.insert("tenant_id".to_owned(), tenant.into());
        body.insert("roles".to_owned(), roles.into());
        if let Some(package) = package {
            body.insert("package".to_owned(), package.into());
        }
        body.insert("kyc_level".to_owned(), kyc_level.into());
        body.extend(extra);
        Ok(body)
    }
}

/// The `sub` of a token whose [`Claims::subject`] is not set: a UUID derived
/// from the tenant and the role, so one role in one tenant is always the same
/// principal.
///
/// UUID-shaped because that is what a real issuer's `sub` is, so a verifier
/// that parses it as one does not need a test-only path. A version-8 (custom)
/// UUID over the first 16 bytes of a SHA-256: that is the RFC 9562 version for
/// "derived by a method of our own", and the hash is the method.
fn default_subject(tenant: &str, role: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"vf-testkit dev subject\0");
    hash.update(tenant.as_bytes());
    hash.update(b"\0");
    hash.update(role.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_custom_bytes(bytes)
        .into_uuid()
        .hyphenated()
        .to_string()
}

/// [`Error::Identity`] from a `jsonwebtoken` error.
fn identity_error(e: jsonwebtoken::errors::Error) -> Error {
    Error::Identity(e.to_string())
}
