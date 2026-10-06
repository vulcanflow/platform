//! Postgres for a test: migrated, templated, and reachable from both sides of
//! PgBouncer (architecture §A6.1 Postgres row, §A6.2).
//!
//! # Why two pools
//!
//! §A5 pins PgBouncer into the local harness precisely so prepared-statement
//! behaviour is exercised now rather than first on Aether. A test that wants to
//! prove a query survives a pooled connection being handed to the next
//! transaction uses [`TestDb::pooled`]; a test that needs session-scoped state
//! (`SET`, `LISTEN`, a temp table) uses [`TestDb::direct`]. Both point at the
//! same database, so a row written through one is visible through the other.
//!
//! # Why a template schema
//!
//! §A4 creates one tenant schema per tenant from a single migration template,
//! and the sqlx offline query data in `.sqlx/` is generated against that
//! template. [`TestDb`] materialises it once per database, then clones it per
//! test with [`TestDb::create_tenant_schema`], which is a `CREATE SCHEMA` plus
//! a table copy rather than a migration replay.

use crate::Result;

/// Where the Postgres under a [`TestDb`] comes from.
///
/// Both backends present the same surface, PgBouncer included: the acceptance
/// criterion for the pooled pool is that it is genuinely pooled, so the
/// testcontainers backend starts a PgBouncer container of its own rather than
/// aliasing [`TestDb::pooled`] onto the direct pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// The long-lived `postgres` and `pgbouncer` services from
    /// `docker-compose.yml`, addressed through the `.env.example` variables.
    /// Fast, and the default when those variables are set.
    Compose,
    /// Throwaway containers started by `testcontainers` for this process.
    /// Needs only a container runtime, so it is what a developer with no
    /// `just dev-up` running gets.
    Testcontainers,
}

/// A migrated Postgres database owned by one test process.
///
/// Dropping a `TestDb` releases the pools. For the [`Backend::Testcontainers`]
/// backend it also stops the containers; for [`Backend::Compose`] the database
/// created for this handle is dropped, leaving the shared services running.
///
/// The fields are the final ones, not placeholders: no constructor in the
/// skeleton commit succeeds, so the accessors below can already borrow them
/// and their signatures do not change when the bodies arrive.
#[derive(Debug)]
pub struct TestDb {
    backend: Backend,
    database: String,
    direct: sqlx::PgPool,
    pooled: sqlx::PgPool,
}

impl TestDb {
    /// The schema that holds the control tables of §A4.
    pub const CONTROL_SCHEMA: &str = "vf";

    /// The schema the per-tenant schemas of §A4 are cloned from.
    pub const TENANT_TEMPLATE_SCHEMA: &str = "tenant_template";

    /// Brings up a database, applies the `vf-db` migrations to it, and
    /// materialises the tenant template schema.
    ///
    /// Picks [`Backend::Compose`] when the `.env.example` Postgres variables
    /// are set in the environment and the services answer, and
    /// [`Backend::Testcontainers`] otherwise.
    ///
    /// Until task C1 lands there are no migration files; the migration step
    /// applies an empty set and succeeds, and the resulting database has the
    /// two schemas and nothing in them. The acceptance test for this
    /// constructor is re-run when C1 closes.
    pub async fn new() -> Result<Self> {
        Err(crate::Error::NotImplemented("TestDb::new"))
    }

    /// As [`TestDb::new`], with the backend chosen by the caller rather than
    /// detected. A test that is specifically about pooled connection reuse
    /// pins [`Backend::Compose`] so it fails loudly when the harness is down,
    /// instead of silently falling back.
    pub async fn with_backend(backend: Backend) -> Result<Self> {
        let _ = backend;
        Err(crate::Error::NotImplemented("TestDb::with_backend"))
    }

    /// Which backend this handle actually got.
    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// The database name this handle owns.
    pub fn database(&self) -> &str {
        &self.database
    }

    /// A pool connected straight to Postgres, bypassing PgBouncer.
    ///
    /// Use it for migrations, for session-scoped state, and for any assertion
    /// about what the server itself did.
    pub fn direct(&self) -> &sqlx::PgPool {
        &self.direct
    }

    /// A pool connected through PgBouncer in transaction pooling mode.
    ///
    /// This is the pool the services use, so it is the one that proves §A5's
    /// claim about prepared statements and the one `vf-db`'s RLS tests reuse a
    /// connection across.
    pub fn pooled(&self) -> &sqlx::PgPool {
        &self.pooled
    }

    /// Clones the tenant template schema into `tenant_<slug>` and returns the
    /// schema name, as §A3.9's provisioner does for real.
    ///
    /// Idempotent: calling it twice for one slug returns the same schema
    /// without touching its contents.
    pub async fn create_tenant_schema(&self, slug: &str) -> Result<String> {
        let _ = slug;
        Err(crate::Error::NotImplemented("TestDb::create_tenant_schema"))
    }

    /// Truncates every table in the control schema and in every tenant schema,
    /// leaving the structure in place. Cheaper than a fresh [`TestDb`] for a
    /// test that only needs empty tables.
    pub async fn truncate_all(&self) -> Result<()> {
        Err(crate::Error::NotImplemented("TestDb::truncate_all"))
    }

    /// Closes both pools and releases the backend. Called for you on drop;
    /// call it explicitly when a test wants the teardown error rather than a
    /// best-effort drop.
    pub async fn close(self) -> Result<()> {
        Err(crate::Error::NotImplemented("TestDb::close"))
    }
}
